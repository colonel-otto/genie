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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CensusStop {
    /// The update (1-based, as the slice counts its ticks).
    pub update: u32,
    /// The index of the object in the player's list.
    pub object: usize,
    /// What the slice does not model.
    pub reason: &'static str,
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
                && let Some((tick, object, why)) = turns.players.get(p).and_then(|q| q.unsupported)
            {
                player.stop = Some(CensusStop {
                    update: tick,
                    object,
                    reason: why.0,
                });
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

/// Where a run from the header first leaves the game ([`header_divergence`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The updates run when the player's objects first differ from the game's save.
    pub update: usize,
    /// What differs ([`crate::object_differences`]).
    pub differences: Vec<String>,
}

/// The recording run from its header as the census runs it, player `player`'s objects compared
/// after each update `u` from `first` to `last` with the game's save that holds the state after
/// update `u` (`save(u)`; the tick oracle's per-update saves name it `u - 1`, a missing save is
/// skipped): the first update whose objects differ, None when none of the saves does. The run
/// stops early at the player's stop, which ends what it can say.
///
/// # Errors
///
/// When the recording, its header or a save cannot be read.
pub fn header_divergence(
    recording: &[u8],
    masters: Option<&MasterTable>,
    player: u32,
    (first, last): (usize, usize),
    save: &mut dyn FnMut(usize) -> Option<Vec<u8>>,
) -> Result<Option<Divergence>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = header_slice(&header, masters);
    let mut decoder = Decoder::default();
    let p = player as usize;
    if turns.players.len() <= p {
        return Err(Error::NoSuchPlayer(player));
    }
    for (i, frame) in frames(&chapter, last)?.iter().enumerate() {
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(frame),
            &sections,
        );
        let update = i + 1;
        if turns.players[p].unsupported.is_some() {
            return Ok(None);
        }
        if update < first {
            continue;
        }
        let Some(game) = save(update) else {
            continue;
        };
        let differences = crate::object_differences(
            &crate::object_rows(&turns, p),
            &crate::saved_objects(&game, player)?,
        );
        if !differences.is_empty() {
            return Ok(Some(Divergence {
                update,
                differences,
            }));
        }
    }
    Ok(None)
}
