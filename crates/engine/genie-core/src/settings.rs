//! The game options the simulation reads: the object `FUN_140c87b90` returns (`GameOptions`,
//! 0x1088 bytes), which the header saves and its load `FUN_141305310` fills. Each field names the
//! option's member and offset as the load reads it (`&mWalkableFarms` into `+0x69`, …); the
//! recording's header holds them all (genie-save's `GameOptions`).

use crate::turn::standard_mode;

/// The game mode King of the Hill (5), which [`Settings::carry_rule`] passes over.
const KING_OF_THE_HILL: i32 = 5;

/// What the simulation reads of the game options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `mGameMode` (`+0x34`).
    pub game_mode: i32,
    /// `mFullTechTreeEnabled` (`+0x63`): a civilisation's own technologies of a mode other than
    /// 1 are disabled like another civilisation's ([`crate::turn::Turns::update_dependents`]).
    pub full_tech_tree: bool,
    /// `mWalkableFarms` (`+0x69`): farmers walk their farm's stops (a farm's Gather of four
    /// stops), and a Gather's `set_state(7)` leaves `mGatherX`, `mGatherY` at −1.
    pub walkable_farms: bool,
    /// `mSelectedContentMode` (`+0x98`). The option is a tagged value read through
    /// `FUN_140ec0cd0` (`+0x9c` the kind: 0 for the value itself, 1 for 2 where the value is 3,
    /// else 0); the load reads the value alone and the kind stays 0, as the object's
    /// constructor and `FUN_14130f2b0` leave it.
    pub selected_content_mode: i32,
    /// `mVillagerForceDrop` (`+0x1070`), which [`Settings::carry_rule`] reads.
    pub villager_force_drop: bool,
    /// `mCheatsEnabled` (`+0x1074`, `FUN_1412ffe00`): players may order each other's units.
    pub cheats_enabled: bool,
}

impl Settings {
    /// `FUN_141302150`: with the villagers' force drop on, a game mode other than King of the
    /// Hill whose descriptor's `+0x9c` is not 1 (not [`standard_mode`]): a hunter drops another
    /// resource it carries.
    #[must_use]
    pub const fn carry_rule(&self) -> bool {
        self.villager_force_drop
            && self.game_mode != KING_OF_THE_HILL
            && !standard_mode(self.game_mode)
    }

    /// The content mode read through `FUN_140ec0cd0` is 1 (`0x140ac8655`): a hunt whose Attack
    /// on a predator (class 10) ended ends too (`set_state` 1) when its unit AI has orders
    /// queued.
    #[must_use]
    pub const fn predator_hunts_end(&self) -> bool {
        self.selected_content_mode == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FUN_141302150`: the force drop counts in the scenario modes but King of the Hill; in
    /// the standard modes, and in any mode without it, a hunter keeps what it carries.
    #[test]
    fn the_carry_rule_needs_the_force_drop_in_a_scenario_mode_but_king_of_the_hill() {
        let with = |game_mode, villager_force_drop| Settings {
            game_mode,
            villager_force_drop,
            ..Settings::default()
        };
        for mode in [3, 4, 14, 15] {
            assert!(with(mode, true).carry_rule(), "mode {mode}");
            assert!(!with(mode, false).carry_rule(), "mode {mode}");
        }
        // King of the Hill and the standard modes, a mode above 16 (the first descriptor's)
        for mode in [5, 0, 1, 2, 6, 11, 13, 16, 17, -1] {
            assert!(!with(mode, true).carry_rule(), "mode {mode}");
        }
    }

    /// The content mode 1 alone ends a predator's hunt.
    #[test]
    fn a_predator_s_hunt_ends_in_content_mode_1() {
        let with = |selected_content_mode| Settings {
            selected_content_mode,
            ..Settings::default()
        };
        assert!(with(1).predator_hunts_end());
        for mode in [0, 2, 3] {
            assert!(!with(mode).predator_hunts_end());
        }
    }
}
