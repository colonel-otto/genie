//! PATROL (0x15): the player's units patrol through waypoints.

use crate::GenieError;
use crate::commands::UnitList;
use crate::world::World;

/// The waypoints of PATROL and `ATTACK_MOVE`: their count (`+0xb`) and the packet's ten slots
/// (x from `+0xc`, y from `+0x34`; the slots past the count are 0 in every recording).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Waypoints {
    /// How many of the slots the command uses.
    pub count: u8,
    /// The slots.
    pub points: [[f32; 2]; 10],
}

impl Waypoints {
    /// Waypoints of `points` (at most ten; the slots past them 0).
    ///
    /// # Panics
    ///
    /// More than ten points.
    #[must_use]
    pub fn of(points: &[[f32; 2]]) -> Self {
        let mut slots = [[0.0; 2]; 10];
        slots[..points.len()].copy_from_slice(points);
        Self {
            count: u8::try_from(points.len()).expect("at most ten waypoints"),
            points: slots,
        }
    }

    /// The waypoints the handlers read (`FUN_140e69cc0`, `FUN_140e66200`): the first `count`;
    /// None without 1 to 10, where they do nothing.
    #[must_use]
    pub fn used(&self) -> Option<&[[f32; 2]]> {
        (1..=10)
            .contains(&self.count)
            .then(|| &self.points[..usize::from(self.count)])
    }
}

/// PATROL (0x15, `FUN_140e69cc0`): the units the packet lists (the count at `+0x4`, the ids from
/// `+0x5c`) patrol through `waypoints` (1 to 10, their count at `+0xb`, x from `+0xc`, y from
/// `+0x34`); `reset` (`+0x8`) drops their unit AIs' saved patrols ([`World::patrol`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PatrolCommand {
    /// The player whose units patrol.
    pub player: usize,
    /// The units.
    pub units: UnitList,
    /// The waypoints.
    pub waypoints: Waypoints,
    /// `+0x8`.
    pub reset: bool,
}

impl PatrolCommand {
    /// The check (`FUN_140e55590`) refuses a count past the packet's ids; the handler reads the
    /// first of them ([`World::read_units`]) and does nothing without 1 to 10 waypoints
    /// ([`Waypoints::used`], [`World::patrol`]).
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let Some(ids) = w.read_units(self.player, &self.units) else {
            return Ok(());
        };
        let Some(waypoints) = self.waypoints.used() else {
            return Ok(());
        };
        w.patrol(self.player, ids, waypoints, self.reset)
    }
}
