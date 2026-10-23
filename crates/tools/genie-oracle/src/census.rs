//! The census run (FD-025): a recording played from its header with nothing forced, against its
//! own checkpoints. Per player, the first update the slice cannot model (the player stops there)
//! and the first recorded checkpoint whose 11-word block differs; together over many recordings
//! they rank what blocks the replays. Dev only.

use genie_rec::de::{self, Op};

use crate::{Decoder, Error, MasterTable, frames, header_slice, lane_block, run};

/// Why a census run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CensusEnd {
    /// The recording's last update ran.
    Recording,
    /// The run reached its update limit.
    Limit,
    /// Every player had stopped or left its checkpoints: nothing more to learn.
    AllDone,
}

/// The first update a player's slice could not model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusStop {
    /// The update (1-based, as the slice counts its ticks).
    pub update: u32,
    /// The index of the object in the player's list.
    pub object: usize,
    /// What the slice does not model: the error's reason.
    pub reason: &'static str,
    /// Where the engine raised it (`file:line:column`).
    pub at: String,
    /// What its site attached (key and value).
    pub context: Vec<(&'static str, String)>,
}

impl CensusStop {
    /// The stop of `why` at `update` on the object at `object`.
    #[must_use]
    pub fn of(update: u32, object: usize, why: &genie_core::GenieError) -> Self {
        Self {
            update,
            object,
            reason: why.reason(),
            at: why.location().to_string(),
            context: why.context().map(|(k, v)| (k, v.to_owned())).collect(),
        }
    }
}

/// The first recorded checkpoint whose block differs from the slice's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusMismatch {
    /// The syncs before the checkpoint: it holds the state after that many updates.
    pub syncs: usize,
    /// The words that differ: (word, the slice's, the recording's).
    pub words: Vec<(usize, u32, u32)>,
}

/// One player's census.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CensusPlayer {
    /// The first update the slice could not model, if any.
    pub stop: Option<CensusStop>,
    /// The first checkpoint that differs while the player still ran, if any.
    pub mismatch: Option<CensusMismatch>,
    /// The checkpoints that held before either.
    pub checkpoints_held: usize,
}

impl CensusPlayer {
    /// The player stopped or left its checkpoints.
    #[must_use]
    pub const fn done(&self) -> bool {
        self.stop.is_some() || self.mismatch.is_some()
    }
}

/// A recording's census run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CensusRun {
    /// The updates run.
    pub updates: usize,
    /// The recording's updates (its syncs, a checkpoint's included).
    pub syncs: usize,
    /// The checkpoints compared.
    pub checkpoints: usize,
    /// Each player's census, Gaia first.
    pub players: Vec<CensusPlayer>,
    /// Why the run ended.
    pub end: CensusEnd,
}

/// The census run of `recording` (its first chapter) for at most `limit` updates: the slice
/// from the header as the game's load leaves it, one update per sync, each recorded checkpoint
/// compared lane by lane with the players' blocks (a checkpoint after N syncs holds the state
/// after N updates). A player's comparisons end at its stop or first mismatch; the run ends when
/// every player's have.
///
/// # Errors
///
/// When the recording or its header cannot be read.
pub fn census_run(
    recording: &[u8],
    masters: Option<&MasterTable>,
    limit: Option<usize>,
) -> Result<CensusRun, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut checkpoints = Vec::new();
    let mut syncs = 0;
    for op in chapter.ops() {
        match op? {
            Op::Sync { .. } => syncs += 1,
            Op::Checkpoint { checkpoint, .. } => {
                checkpoints.push((syncs, checkpoint.lanes));
                syncs += 1;
            }
            _ => {}
        }
    }
    let total = limit.map_or(syncs, |l| l.min(syncs));
    let mut turns = header_slice(&header, masters);
    let mut decoder = Decoder::default();
    let mut players = vec![CensusPlayer::default(); sections.len()];
    let mut next = 0;
    let (mut updates, mut compared) = (0, 0);
    let mut end = if total < syncs {
        CensusEnd::Limit
    } else {
        CensusEnd::Recording
    };
    for frame in frames(&chapter, total)? {
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(&frame),
            &sections,
        );
        updates += 1;
        for (p, player) in players.iter_mut().enumerate() {
            if player.stop.is_none()
                && let Some((tick, object, why)) =
                    turns.players.get(p).and_then(|q| q.unsupported.as_ref())
            {
                player.stop = Some(CensusStop::of(*tick, *object, why));
            }
        }
        while let Some(&(at, lanes)) = checkpoints.get(next).filter(|(at, _)| *at <= updates) {
            next += 1;
            if at < updates {
                continue;
            }
            compared += 1;
            for lane in lanes.iter().filter(|l| l.iter().any(|&w| w != 0)) {
                let p = lane[8] as usize;
                let (Some(section), Some(state), Some(player)) =
                    (sections.get(p), turns.players.get(p), players.get_mut(p))
                else {
                    continue;
                };
                if player.done() {
                    continue;
                }
                let block = lane_block(section, lane[8], state);
                let words: Vec<(usize, u32, u32)> = (0..11)
                    .filter(|&w| block[w] != lane[w])
                    .map(|w| (w, block[w], lane[w]))
                    .collect();
                if words.is_empty() {
                    player.checkpoints_held += 1;
                } else {
                    player.mismatch = Some(CensusMismatch { syncs: at, words });
                }
            }
        }
        if players.iter().all(CensusPlayer::done) {
            end = CensusEnd::AllDone;
            break;
        }
    }
    Ok(CensusRun {
        updates,
        syncs,
        checkpoints: compared,
        players,
        end,
    })
}

/// What a run from the header says of one player against the game's saves
/// ([`header_divergences`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followed {
    /// Its objects matched every save of the window.
    Follows,
    /// It stopped first (the update the slice could not model); what follows says nothing of
    /// the port.
    Stops(CensusStop),
    /// Its objects left the game.
    Leaves(Divergence),
}

/// Where a run from the header first leaves the game ([`header_divergences`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The updates run when the player's objects first differ from the game's save.
    pub update: usize,
    /// What differs ([`crate::object_field_differences`]).
    pub differences: Vec<crate::Difference>,
}

/// The recording run from its header as the census runs it, the objects of each of `players`
/// compared after each update `u` from `first` to `last` with the game's save that holds the state
/// after update `u` (`save(u)`, read once for all players; the tick oracle's per-update saves name
/// it `u - 1`, a missing save is skipped): per player, whether its objects followed, it stopped
/// first, or the first update whose objects differ. A player's comparisons end at its stop or
/// its first divergence; the run ends when every player's have.
///
/// # Errors
///
/// When the recording, its header or a save cannot be read, or a player is not in it.
pub fn header_divergences(
    recording: &[u8],
    masters: Option<&MasterTable>,
    players: &[u32],
    (first, last): (usize, usize),
    save: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
) -> Result<Vec<Followed>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = header_slice(&header, masters);
    let mut decoder = Decoder::default();
    if let Some(&p) = players.iter().find(|&&p| turns.players.len() <= p as usize) {
        return Err(Error::NoSuchPlayer(p));
    }
    let mut out: Vec<Followed> = vec![Followed::Follows; players.len()];
    let mut done = vec![false; players.len()];
    for (i, frame) in frames(&chapter, last)?.iter().enumerate() {
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(frame),
            &sections,
        );
        let update = i + 1;
        for (k, &p) in players.iter().enumerate() {
            if let (false, Some((tick, object, why))) =
                (done[k], turns.players[p as usize].unsupported.as_ref())
            {
                out[k] = Followed::Stops(CensusStop::of(*tick, *object, why));
                done[k] = true;
            }
        }
        if done.iter().all(|&d| d) {
            break;
        }
        if update < first {
            continue;
        }
        let Some(game) = save(update) else {
            continue;
        };
        let saved = de::chapters(&game)?[0];
        let saved = saved.header()?;
        for (k, &p) in players.iter().enumerate() {
            if done[k] {
                continue;
            }
            let differences = crate::object_field_differences(
                &crate::object_rows(&turns, p as usize),
                &crate::header_objects(&saved, p)?,
            );
            if !differences.is_empty() {
                out[k] = Followed::Leaves(Divergence {
                    update,
                    differences,
                });
                done[k] = true;
            }
        }
    }
    Ok(out)
}

/// The recording run from its header as the census runs it, to update `last`; `before` is called
/// before each update with its number (1 the first), so a caller can listen to one update's
/// trace ([`genie_core::trace`]). The engine as it stands after the run.
///
/// # Errors
///
/// When the recording or its header cannot be read.
pub fn header_run(
    recording: &[u8],
    masters: Option<&MasterTable>,
    last: usize,
    before: &mut dyn FnMut(usize),
) -> Result<genie_core::turn::Turns, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = header_slice(&header, masters);
    let mut decoder = Decoder::default();
    for (i, frame) in frames(&chapter, last)?.iter().enumerate() {
        before(i + 1);
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(frame),
            &sections,
        );
    }
    Ok(turns)
}

/// [`header_divergences`] of one player.
///
/// # Errors
///
/// As [`header_divergences`].
pub fn header_divergence(
    recording: &[u8],
    masters: Option<&MasterTable>,
    player: u32,
    window: (usize, usize),
    save: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
) -> Result<Followed, Error> {
    Ok(
        header_divergences(recording, masters, &[player], window, save)?
            .pop()
            .unwrap_or(Followed::Follows),
    )
}
