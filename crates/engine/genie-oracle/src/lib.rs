//! The replay oracle: state read from a recording's header save, checked against the checkpoints
//! the recording stores in its body.
//!
//! The first checkpoint comes 500 syncs into the game; a recording has none at tick 0. Ten of its
//! eleven words per player still equal the start-of-game state ([`SAVE_WORDS`]). Word 9 does not:
//! it sums each object's top action and unit-AI retarget timer, which the engine redraws while the
//! game runs, so reproducing it needs the simulation.

use genie_core::checkpoint::{ObjectState, PlayerState, player_block};
use genie_rec::de::{self, Op, header::PlayerSection};

/// The words of the first checkpoint that the start-of-game save determines: all but word 9.
pub const SAVE_WORDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10];

/// Main object ids in the save that the first checkpoint does not count: PLACEHOLDER2, FLARE3 and
/// the empty town-centre annex. Why they drop out (removed in the first ticks, or never in the
/// counted list) is not known yet.
pub const NOT_COUNTED: [u16; 3] = [1909, 332, 890];

/// One player's block, computed from the save and as the recording stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerComparison {
    pub id: u32,
    pub from_save: [u32; 11],
    pub checkpoint: [u32; 11],
}

/// Why a recording could not be compared.
#[derive(Debug)]
pub enum Error {
    Rec(de::Error),
    /// The recording has no checkpoint (it ends within the first interval).
    NoCheckpoint,
    /// A checkpoint lane names a player the header save does not have.
    NoSuchPlayer(u32),
}

impl From<de::Error> for Error {
    fn from(e: de::Error) -> Self {
        Error::Rec(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Rec(e) => write!(f, "{e}"),
            Error::NoCheckpoint => write!(f, "the recording has no checkpoint"),
            Error::NoSuchPlayer(id) => write!(f, "a checkpoint names player {id}, not in the save"),
        }
    }
}

impl std::error::Error for Error {}

/// The checkpoint inputs of one player section. Word 9's inputs stay empty (see the crate docs).
pub fn player_state(section: &PlayerSection, id: u32) -> PlayerState {
    let attributes = &section.prelude.attributes;
    let resource = |i: usize| attributes.get(i).copied().unwrap_or_default();
    PlayerState {
        id,
        resources: [resource(0), resource(1), resource(2), resource(3)],
        sleeping_count: section
            .tail
            .as_ref()
            .map_or(0, |t| t.sleeping_objects.len() as u32),
        live: section
            .objects
            .iter()
            .map(|o| o.static_part())
            .filter(|s| !NOT_COUNTED.contains(&s.main_id))
            .map(|s| ObjectState {
                id: s.id as i32,
                main_id: s.main_id as i16,
                state: s.state,
                amount_held: s.amount,
                position: s.position,
                top_action: None,
                retarget_timer: None,
            })
            .collect(),
    }
}

/// Compare every player of the first chapter's save with the recording's first checkpoint, in the
/// order of the checkpoint's lanes. Lanes that are all zero hold no player.
pub fn compare_first_checkpoint(recording: &[u8]) -> Result<Vec<PlayerComparison>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    let checkpoint = chapter
        .ops()
        .find_map(|op| match op {
            Ok(Op::Checkpoint { checkpoint, .. }) => Some(Ok(checkpoint)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .ok_or(Error::NoCheckpoint)??;
    checkpoint
        .lanes
        .iter()
        .filter(|lane| lane.iter().any(|&w| w != 0))
        .map(|lane| {
            let id = lane[8];
            let section = header
                .players
                .get(id as usize)
                .ok_or(Error::NoSuchPlayer(id))?;
            Ok(PlayerComparison {
                id,
                from_save: player_block(&player_state(section, id)),
                checkpoint: *lane,
            })
        })
        .collect()
}

/// Commands whose handlers can create a unit group (158041 `FUN_14152efc0`): INTERACT, WORK,
/// MOVE, FORM_FORMATION, GROUP_MULTI_WAYPOINTS, BUILD, REPAIR. More player-side callers exist and
/// are not mapped yet. A group draws from the World stream every tick while active and every 3 s
/// while idle (`FUN_14103d910` → `FUN_14102d370`), which the slice does not model. Whether a command
/// creates one depends on its units: the splitter (`FUN_140fd76d0`) groups only clusters of two or
/// more formation-capable units, and villagers never join a formation (as openage's
/// `doc/reverse_engineering/game_mechanics/formations.md` describes for AoC).
pub const GROUP_COMMANDS: [u8; 7] = [0x00, 0x02, 0x03, 0x17, 0x1f, 0x66, 0x6e];

/// A command that may create a unit group and so take the frame-seed chain out of the slice's model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCommand {
    /// The tick (1-based) the command executes in.
    pub tick: u32,
    pub player: u8,
    pub command: u8,
}

/// Word 9 of one player's first checkpoint, from the unit-AI slice and as recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word9 {
    pub id: u32,
    pub simulated: u32,
    pub checkpoint: u32,
    /// The first command before the checkpoint, by any player, that may create a unit group.
    pub group_command: Option<GroupCommand>,
    /// The first unit update of this player the slice could not model.
    pub unsupported: Option<(u32, usize, genie_core::unit_ai::Unsupported)>,
}

fn unit_ai(object: &de::header::Object) -> Option<genie_core::unit_ai::UnitAi> {
    use de::header::ObjectBody;
    let combat = match &object.body {
        ObjectBody::Combat(c) => &**c,
        ObjectBody::Building(b) => &b.combat,
        _ => return None,
    };
    let ai = combat.unit_ai.as_ref()?;
    Some(genie_core::unit_ai::UnitAi {
        ai_type: combat.has_ai,
        current_order: ai.current_order,
        current_action: ai.current_action,
        current_target: ai.current_target,
        idle_timer: ai.idle_timer,
        adjusted_idle_timeout: ai.adjusted_idle_timeout,
        secondary_timer: ai.secondary_timer,
        look_around_timer: ai.look_around_timer,
        look_around_timeout: ai.look_around_timeout,
        last_retarget_time: ai.last_retarget_time,
        retarget_timer: ai.randomized_target_timer,
        stance: ai.state,
        retarget_disabled: ai.unknown_de2,
    })
}

/// Run the unit-AI slice from the header save up to the first checkpoint and compare word 9.
///
/// The checkpoint block is built before the player loop of the tick that records it, so it shows
/// the state after one tick fewer than the syncs before it. Objects stay in their saved order; how
/// and when the [`NOT_COUNTED`] objects leave the live list is not known, so they stay in it.
pub fn simulate_word9(recording: &[u8]) -> Result<Vec<Word9>, Error> {
    use genie_core::turn::{Player, Turns};
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    let mut dts = Vec::new();
    let mut group_command = None;
    let mut checkpoint = None;
    for op in chapter.ops() {
        match op? {
            Op::Sync { dt } => dts.push(dt),
            Op::Command { packet, .. } => {
                if group_command.is_none() && GROUP_COMMANDS.contains(&packet[0]) {
                    group_command = Some(GroupCommand {
                        tick: dts.len() as u32 + 1,
                        player: packet.get(1).copied().unwrap_or_default(),
                        command: packet[0],
                    });
                }
            }
            Op::Checkpoint { checkpoint: c, .. } => {
                checkpoint = Some(c);
                break;
            }
            _ => {}
        }
    }
    let checkpoint = checkpoint.ok_or(Error::NoCheckpoint)?;
    let players = header
        .players
        .iter()
        .map(|p| Player {
            objects: p.objects.iter().map(unit_ai).collect(),
            computer: false,
            unsupported: None,
        })
        .collect();
    let replay = &header.replay;
    let mut turns = Turns::new(
        replay.world_time,
        replay.random_seed_2,
        replay.player_turn as usize,
        replay.player_turns.clone(),
        players,
    );
    for &dt in &dts {
        turns.tick(dt);
    }
    checkpoint
        .lanes
        .iter()
        .filter(|lane| lane.iter().any(|&w| w != 0))
        .map(|lane| {
            let id = lane[8];
            let section = header
                .players
                .get(id as usize)
                .ok_or(Error::NoSuchPlayer(id))?;
            let simulated = turns.players[id as usize].clone();
            let mut state = player_state(section, id);
            let timers = section
                .objects
                .iter()
                .zip(&simulated.objects)
                .filter(|(o, _)| !NOT_COUNTED.contains(&o.static_part().main_id))
                .map(|(_, ai)| ai.as_ref().map(|ai| ai.retarget_timer as i32));
            for (object, timer) in state.live.iter_mut().zip(timers) {
                object.retarget_timer = timer;
            }
            Ok(Word9 {
                id,
                simulated: player_block(&state)[9],
                checkpoint: lane[9],
                group_command,
                unsupported: simulated.unsupported,
            })
        })
        .collect()
}
