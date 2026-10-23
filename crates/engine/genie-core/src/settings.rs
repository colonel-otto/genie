//! The game options the simulation reads: the object `FUN_140c87b90` returns (`GameOptions`,
//! 0x1088 bytes), which the header saves and its load `FUN_141305310` fills. Each field names the
//! option's member and offset as the load reads it (`&mWalkableFarms` into `+0x69`, …); the
//! recording's header holds them all (genie-save's `GameOptions`).

use crate::turn::standard_mode;

/// The game mode King of the Hill (5), which [`Settings::carry_rule`] passes over.
const KING_OF_THE_HILL: i32 = 5;
/// The game mode Regicide (1), whose players lose with their last king ([`Settings::regicide`]).
const REGICIDE: i32 = 1;
/// The game mode Sudden Death (11), whose players lose with their last town centre
/// ([`Settings::sudden_death`]).
const SUDDEN_DEATH: i32 = 11;
/// `mSubGameModeFlag`'s bit of the sudden death rule (`FUN_141324b60(options + 0x18) & 2`).
const SUDDEN_DEATH_FLAG: i32 = 2;
/// `mSubGameModeFlag`'s bit of the regicide rule (`FUN_141302520`: `+0x94 & 4`).
const REGICIDE_FLAG: i32 = 4;
/// `FUN_1412fc110`'s last age for an `mEndingAge` of 0.
const DEFAULT_LAST_AGE: i32 = 5;
/// The visibility types whose players have explored the whole map (`mVisibilityType` 1, explored,
/// and 2, all visible): their attribute 22 is 100 ([`Settings::explored_all`]).
const EXPLORED: i32 = 1;
const ALL_VISIBLE: i32 = 2;

/// What the simulation reads of the game options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// `mGameMode` (`+0x34`).
    pub game_mode: i32,
    /// `mEndingAge` (`+0x40`): the last age of the game, 0 for the default
    /// ([`Settings::last_age`]).
    pub ending_age: i32,
    /// `mVisibilityType` (`+0x48`): 0 normal, 1 explored, 2 all visible.
    pub visibility_type: i32,
    /// `mSubGameModeFlag` (`+0x94`): the rules a mode adds to another (bit 1 sudden death, bit 2
    /// regicide), read as an int (`FUN_141324b60` on `+0x18` reads `+0x18 + 0x7c`).
    pub sub_game_mode_flag: i32,
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
    /// `mTreatyLength`: the treaty's minutes, 0 for none. The world's treaty monitor (`+0x820`,
    /// `FUN_141461190`) fills its players' zones only while a treaty holds (`FUN_141461c00`);
    /// the slice holds no treaty.
    pub treaty_length: u32,
    /// `mTeamsLocked` (`+0x64`): the world's teams stay as the game began
    /// ([`crate::turn::Turns::resolve_teams`]).
    pub teams_locked: bool,
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

    /// `FUN_1412fc110`: the game's last age, `mEndingAge` or 5 for 0. A tech tree's unit of a
    /// later age than the one before it is not in the tree ([`crate::world_player`]).
    #[must_use]
    pub const fn last_age(&self) -> i32 {
        if self.ending_age == 0 {
            DEFAULT_LAST_AGE
        } else {
            self.ending_age
        }
    }

    /// The players have explored the whole map from the start (`mVisibilityType` 1 or 2): the
    /// world player's update sets their attribute 22 to 100 rather than their explored share
    /// (`WorldPlayer::update`, `0x14163b100`).
    #[must_use]
    pub const fn explored_all(&self) -> bool {
        matches!(self.visibility_type, EXPLORED | ALL_VISIBLE)
    }

    /// `FUN_141302520`: the regicide rule holds, the game mode Regicide (1) or the sub-mode's
    /// bit 2: a player without a king (master 434) loses.
    #[must_use]
    pub const fn regicide(&self) -> bool {
        self.game_mode == REGICIDE || self.sub_game_mode_flag & REGICIDE_FLAG != 0
    }

    /// The sudden death rule holds (`WorldPlayer::update` at `0x14163b92a`): the game mode Sudden
    /// Death (11) or the sub-mode's bit 1: a player who had a town centre loses without one.
    #[must_use]
    pub const fn sudden_death(&self) -> bool {
        self.game_mode == SUDDEN_DEATH || self.sub_game_mode_flag & SUDDEN_DEATH_FLAG != 0
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

    /// `mVisibilityType` 1 (explored) and 2 (all visible) explore the whole map; 0 (normal) and
    /// the others do not.
    #[test]
    fn the_explored_and_all_visible_types_explore_the_whole_map() {
        let with = |visibility_type| Settings {
            visibility_type,
            ..Settings::default()
        };
        assert!(with(1).explored_all() && with(2).explored_all());
        for t in [0, 3, -1] {
            assert!(!with(t).explored_all(), "type {t}");
        }
    }

    /// `FUN_141302520`: Regicide (mode 1) or the sub-mode's bit 2; the sudden death rule: Sudden
    /// Death (mode 11) or the sub-mode's bit 1. Each bit alone, in any mode.
    #[test]
    fn regicide_and_sudden_death_come_from_the_mode_or_the_sub_mode_s_bits() {
        let with = |game_mode, sub_game_mode_flag| Settings {
            game_mode,
            sub_game_mode_flag,
            ..Settings::default()
        };
        assert!(with(1, 0).regicide() && !with(1, 0).sudden_death());
        assert!(with(11, 0).sudden_death() && !with(11, 0).regicide());
        assert!(with(0, 4).regicide() && !with(0, 4).sudden_death());
        assert!(with(0, 2).sudden_death() && !with(0, 2).regicide());
        assert!(with(13, 6).regicide() && with(13, 6).sudden_death());
        for mode in [0, 2, 3, 12, 13] {
            assert!(!with(mode, 1).regicide() && !with(mode, 1).sudden_death());
        }
    }

    /// `FUN_1412fc110`: an ending age of 0 is the fifth; any other is itself.
    #[test]
    fn the_last_age_is_the_ending_age_or_5() {
        let with = |ending_age| Settings {
            ending_age,
            ..Settings::default()
        };
        assert_eq!(with(0).last_age(), 5);
        for age in [1, 2, 4, 6, -1] {
            assert_eq!(with(age).last_age(), age);
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
