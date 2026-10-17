//! The unit AI of an idle unit on its owner's turn: the timers and the AI-stream draws behind
//! checkpoint word 9.
//!
//! 158041: the full update is `UnitAIModule` slot 4 (`0x140c66b10`); slot 73 (`0x140c567c0`) redraws
//! the retarget timer. Only an idle unit is modelled: no order, no action, no target, no actions in
//! its list. Anything else is [`Unsupported`].

use crate::rng::Well512;

/// The unit-AI fields the idle update reads and writes, with their offsets in 158041.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitAi {
    /// The factory's type (`has_ai` in the save), which picks the class.
    pub ai_type: u32,
    /// `+0x3c`, −1 for none.
    pub current_order: i32,
    /// `+0x44`, −1 for none.
    pub current_action: i32,
    /// `+0x48`, −1 for none.
    pub current_target: i32,
    /// `+0x160`.
    pub idle_timer: u32,
    /// `+0x164`, the randomised timeout the idle timer is compared with.
    pub adjusted_idle_timeout: u32,
    /// `+0x168`, the base the adjusted timeout is drawn around.
    pub idle_timeout: u32,
    /// `+0x16c`.
    pub secondary_timer: u32,
    /// `+0x170`.
    pub look_around_timer: u32,
    /// `+0x174`.
    pub look_around_timeout: u32,
    /// `+0xbc`, the world time of the last retarget.
    pub last_retarget_time: u32,
    /// `+0x1cc`, `mRandomizedRetargetTimer`: the word-9 input.
    pub retarget_timer: u32,
    /// `+0x1a9`: 1 and 2 add leash checks to slot 73.
    pub stance: u8,
    /// `+0x1d6`: non-zero skips the retarget check.
    pub retarget_disabled: u8,
    /// `+0x1a8`, `stop_after_target_killed_value`: a move does not let the current action take its
    /// point (`FUN_140c41c10`).
    pub stop_after_kill: bool,
}

/// Why an update was not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

/// What slot 73 does for a factory type.
enum Retarget {
    /// The base slot 73: the retarget check.
    Base,
    /// An override that never reaches the check: villagers (8), livestock (19).
    Never,
}

fn retarget_kind(ai_type: u32) -> Result<Retarget, Unsupported> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 | 19 => Ok(Retarget::Never),
        // transport ship (11, slot 72 0x140c21d20), soldier (13), tower (15), building (17); their
        // slot 72 draws nothing for an idle unit
        11 | 13 | 15 | 17 => Ok(Retarget::Base),
        _ => Err(Unsupported("unit-AI type not modelled")),
    }
}

/// The base idle timeout (`+0x168`) the constructor sets; the adjusted one is drawn around it.
const IDLE_TIMEOUT: u32 = 3000;

/// `UpdateAdjustedIdleTimeout` (`0x140c41e00`): `r % (base / 2) + 3 * base / 4`, one AI draw.
fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

impl UnitAi {
    /// The unit AI a new object gets. `UnitAIModule`'s constructor (`0x140c2e080`) draws three
    /// AI-stream values, in this order: the last retarget time `r % 500`, the adjusted idle timeout
    /// `r % 1500 + 2250` and the retarget timer `750 + r % 500`. Only the building class (17,
    /// `TribeBuildingUnitAIModule` `0x140c11470`, which sets the stance to 0) is modelled; for any
    /// other type nothing is drawn.
    pub fn constructed(ai_type: u32, ai: &mut Well512) -> Result<UnitAi, Unsupported> {
        if ai_type != 17 {
            return Err(Unsupported("constructor of a unit-AI type not modelled"));
        }
        let last_retarget_time = ai.rand15() % 500;
        let adjusted_idle_timeout = adjusted_idle_timeout(IDLE_TIMEOUT, ai);
        let retarget_timer = 750 + ai.rand15() % 500;
        Ok(UnitAi {
            ai_type,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            idle_timer: 0,
            adjusted_idle_timeout,
            idle_timeout: IDLE_TIMEOUT,
            secondary_timer: 750,
            look_around_timer: 0,
            look_around_timeout: 1000,
            last_retarget_time,
            retarget_timer,
            // the constructor's two bytes at `+0x1a8`: 0x300
            stance: 3,
            retarget_disabled: 0,
            stop_after_kill: false,
        })
    }

    /// What an order (INTERACT, MOVE, RETREAT) does to each of its units' AI before it runs
    /// (`FUN_140e6cfc0`): an adjusted idle timeout of 750 or more is redrawn
    /// (`UpdateAdjustedIdleTimeout`, unit-AI slot 53, `0x140c41e00`), and the secondary timer is
    /// capped at 750.
    pub fn reset_for_order(&mut self, ai: &mut Well512) {
        if self.adjusted_idle_timeout > 749 {
            self.adjusted_idle_timeout = adjusted_idle_timeout(self.idle_timeout, ai);
        }
        self.secondary_timer = self.secondary_timer.min(750);
    }

    /// The work command's notify of its new action (`FUN_14106e4e0`, object slot 0x828): the unit
    /// AI takes the order and action values of the action's type and the target
    /// (`FUN_140c64b60`, `FUN_140c64b50`, `FUN_140c64be0`). Leaving order 711 or 733 redraws an
    /// adjusted idle timeout of exactly 750 (`UpdateAdjustedIdleTimeout`) and caps the secondary
    /// timer at 750. The last order, action and target, the target's class and point, the order
    /// history (`FUN_140c42080`) and the notifies (`FUN_140c5dc10`) are not held.
    pub fn notify_work(&mut self, order: i32, action: i32, target: i32, ai: &mut Well512) {
        let leaving = |o| self.current_order == o && order != o;
        if leaving(711) || leaving(733) {
            if self.adjusted_idle_timeout == 750 {
                self.adjusted_idle_timeout = adjusted_idle_timeout(self.idle_timeout, ai);
            }
            self.secondary_timer = self.secondary_timer.min(750);
        }
        self.current_order = order;
        self.current_action = action;
        self.current_target = target;
    }

    fn is_idle(&self) -> bool {
        self.current_order == -1 && self.current_action == -1 && self.current_target == -1
    }

    /// The full update of an idle unit: `dt` is the time its owner accumulated since its last turn
    /// (`world+0x18c[player]`), `computer` the owner's type test (`player+0x10 == 3`).
    pub fn update_idle(
        &mut self,
        dt: u32,
        world_time: u32,
        computer: bool,
        ai: &mut Well512,
    ) -> Result<(), Unsupported> {
        if !self.is_idle() {
            return Err(Unsupported("unit is not idle"));
        }
        let retarget = retarget_kind(self.ai_type)?;
        self.secondary_timer = self.secondary_timer.saturating_sub(dt);
        self.look_around_timer = self.look_around_timer.wrapping_add(dt);
        self.idle_timer = self.idle_timer.wrapping_add(dt);
        if let Retarget::Base = retarget {
            if self.stance == 1 {
                return Err(Unsupported("stance 1 leash check"));
            }
            // An idle stand-ground unit passes its leash check: it returns only with an action.
            if self.retarget_disabled == 0
                && world_time.wrapping_sub(self.last_retarget_time) >= self.retarget_timer
            {
                self.last_retarget_time = world_time;
                self.retarget_timer = 750 + ai.rand15() % 500;
            }
        }
        if self.idle_timer > self.adjusted_idle_timeout {
            // then slot 72, which for these types draws nothing for an idle unit
            self.idle_timer = 0;
        }
        self.look_around(computer, ai)
    }

    /// The look-around check at the end of the full update (`LAB_140c6809c`): a timer past its
    /// timeout restarts with a timeout drawn around 5 s.
    fn look_around(&mut self, computer: bool, ai: &mut Well512) -> Result<(), Unsupported> {
        if self.look_around_timer > self.look_around_timeout {
            // the human branch; `FUN_14152b470` sends players allied with a computer elsewhere
            if computer {
                return Err(Unsupported("look-around timer of a computer player"));
            }
            self.look_around_timer = 0;
            self.look_around_timeout = 4500 + ai.rand15() % 1000;
        }
        Ok(())
    }

    /// The full update (slot 4, `0x140c66b10`) of a villager on its build order: its order and
    /// action stand, so it only runs the timers. With actions in its list the order is not
    /// re-checked; its action keeps the idle timer at 0 (a player above Gaia); villagers do not
    /// retarget; the look-around check follows. Its target, a building, never counts as dead
    /// here. A unit in a group, inside another, with notifies or with other orders is not
    /// modelled (the caller checks the first two).
    pub fn update_working(
        &mut self,
        dt: u32,
        computer: bool,
        ai: &mut Well512,
    ) -> Result<(), Unsupported> {
        if !matches!(
            (self.current_order, self.current_action),
            (BUILD_ORDER, BUILD_ACTION) | (HUNT_ORDER, HUNT_ACTION)
        ) {
            return Err(Unsupported(
                "unit is not idle and not on a build or hunt order",
            ));
        }
        if !matches!(retarget_kind(self.ai_type)?, Retarget::Never) {
            return Err(Unsupported("a unit on a build order that retargets"));
        }
        self.secondary_timer = self.secondary_timer.saturating_sub(dt);
        self.look_around_timer = self.look_around_timer.wrapping_add(dt);
        self.idle_timer = 0;
        self.look_around(computer, ai)
    }
}

/// The build order (`0x2be`) and the build action value (`0x25a`) of a villager's unit AI.
pub const BUILD_ORDER: i32 = 702;
pub const BUILD_ACTION: i32 = 602;
/// The hunt order (`0x2c9`) and action value (`0x265`) of a villager's unit AI.
pub const HUNT_ORDER: i32 = 713;
pub const HUNT_ACTION: i32 = 613;

#[cfg(test)]
mod tests {
    use super::*;

    fn idle(ai_type: u32) -> UnitAi {
        UnitAi {
            ai_type,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            adjusted_idle_timeout: 3000,
            look_around_timeout: 1000,
            retarget_timer: 900,
            ..UnitAi::default()
        }
    }

    fn rng() -> Well512 {
        let mut r = Well512::default();
        r.set_seed(0);
        r
    }

    /// A builder on its build order (702, action 602): MarvinBlue/426048731's 9175 in update 681,
    /// whose look-around timer runs out and is redrawn from the AI stream.
    #[test]
    fn a_builder_s_idle_timer_stays_0_and_its_look_around_is_redrawn() {
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            current_target: 10028,
            idle_timer: 120,
            secondary_timer: 10,
            look_around_timer: 990,
            ..idle(8)
        };
        let mut r = rng();
        u.update_working(50, false, &mut r).unwrap();
        assert_eq!(
            (u.idle_timer, u.secondary_timer, u.look_around_timer),
            (0, 0, 0)
        );
        assert_eq!(u.look_around_timeout, 4500 + 7241 % 1000);
        assert_eq!(r.draws, 1);
        // another order is not modelled
        u.current_order = 700;
        assert!(u.update_working(50, false, &mut r).is_err());
    }

    #[test]
    fn a_work_notify_sets_the_order_and_redraws_a_timeout_left_by_order_711() {
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            current_target: 10028,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 900,
            ..idle(8)
        };
        let mut r = rng();
        u.notify_work(HUNT_ORDER, HUNT_ACTION, 9365, &mut r);
        assert_eq!(
            (u.current_order, u.current_action, u.current_target, r.draws),
            (HUNT_ORDER, HUNT_ACTION, 9365, 0)
        );
        u.current_order = 711;
        u.notify_work(HUNT_ORDER, HUNT_ACTION, 9365, &mut r);
        assert_eq!((r.draws, u.secondary_timer), (1, 750));
        assert_ne!(u.adjusted_idle_timeout, 750);
    }

    #[test]
    fn retarget_waits_for_the_timer() {
        let mut u = idle(17);
        let mut r = rng();
        u.update_idle(50, 899, false, &mut r).unwrap();
        assert_eq!((u.retarget_timer, r.draws), (900, 0));
        u.update_idle(50, 900, false, &mut r).unwrap();
        // the first rand15 after seed 0 is 7241
        assert_eq!(
            (u.last_retarget_time, u.retarget_timer),
            (900, 750 + 7241 % 500)
        );
    }

    #[test]
    fn a_last_retarget_ahead_of_the_world_time_wraps_and_retargets() {
        let mut u = UnitAi {
            last_retarget_time: 400,
            ..idle(15)
        };
        let mut r = rng();
        u.update_idle(50, 100, false, &mut r).unwrap();
        assert_eq!((u.last_retarget_time, r.draws), (100, 1));
    }

    #[test]
    fn villagers_and_livestock_never_retarget() {
        for t in [8, 19] {
            let mut u = idle(t);
            let mut r = rng();
            u.update_idle(50, 5000, false, &mut r).unwrap();
            assert_eq!((u.retarget_timer, r.draws), (900, 0), "type {t}");
        }
    }

    #[test]
    fn the_look_around_timeout_is_redrawn_once_exceeded() {
        let mut u = UnitAi {
            retarget_disabled: 1,
            ..idle(8)
        };
        let mut r = rng();
        u.update_idle(1000, 0, false, &mut r).unwrap();
        assert_eq!((u.look_around_timer, r.draws), (1000, 0));
        u.update_idle(1, 0, false, &mut r).unwrap();
        assert_eq!(
            (u.look_around_timer, u.look_around_timeout),
            (0, 4500 + 7241 % 1000)
        );
    }

    #[test]
    fn retarget_draws_before_look_around() {
        let mut u = UnitAi {
            look_around_timer: 1000,
            ..idle(13)
        };
        let mut r = rng();
        u.update_idle(1, 900, false, &mut r).unwrap();
        assert_eq!(u.retarget_timer, 750 + 7241 % 500);
        assert_eq!(u.look_around_timeout, 4500 + 25269 % 1000);
    }

    #[test]
    fn the_idle_timer_restarts_after_its_timeout_and_the_secondary_timer_stops_at_0() {
        let mut u = UnitAi {
            secondary_timer: 750,
            ..idle(8)
        };
        let mut r = rng();
        u.update_idle(3001, 0, false, &mut r).unwrap();
        assert_eq!((u.idle_timer, u.secondary_timer), (0, 0));
    }

    #[test]
    fn non_idle_units_and_unknown_types_are_unsupported() {
        let mut r = rng();
        let mut busy = UnitAi {
            current_action: 600,
            ..idle(17)
        };
        assert!(busy.update_idle(1, 0, false, &mut r).is_err());
        assert!(idle(2).update_idle(1, 0, false, &mut r).is_err());
        let mut defensive = UnitAi {
            stance: 1,
            ..idle(13)
        };
        assert!(defensive.update_idle(1, 0, false, &mut r).is_err());
    }

    #[test]
    fn a_new_building_unit_ai_draws_its_three_timers() {
        // UnitAIModule's constructor: last retarget r % 500, adjusted idle timeout r % 1500 + 2250,
        // retarget timer 750 + r % 500, in that order
        let mut r = rng();
        let ai = UnitAi::constructed(17, &mut r).unwrap();
        let mut expected = rng();
        let (a, b, c) = (expected.rand15(), expected.rand15(), expected.rand15());
        assert_eq!(r.draws, 3);
        assert_eq!(
            ai,
            UnitAi {
                ai_type: 17,
                current_order: -1,
                current_action: -1,
                current_target: -1,
                idle_timer: 0,
                adjusted_idle_timeout: b % 1500 + 2250,
                idle_timeout: 3000,
                secondary_timer: 750,
                look_around_timer: 0,
                look_around_timeout: 1000,
                last_retarget_time: a % 500,
                retarget_timer: 750 + c % 500,
                stance: 3,
                retarget_disabled: 0,
                stop_after_kill: false,
            }
        );
    }

    #[test]
    fn a_unit_ai_class_without_a_modelled_constructor_draws_nothing() {
        let mut r = rng();
        assert!(UnitAi::constructed(13, &mut r).is_err());
        assert_eq!(r.draws, 0);
    }

    #[test]
    fn an_order_redraws_an_adjusted_idle_timeout_of_750_or_more_and_caps_the_secondary_timer() {
        // FUN_140e6cfc0 for each unit of an INTERACT, MOVE or RETREAT
        let mut u = UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 2600,
            secondary_timer: 900,
            ..idle(13)
        };
        let mut r = rng();
        u.reset_for_order(&mut r);
        assert_eq!(
            (u.adjusted_idle_timeout, u.secondary_timer),
            (7241 % 1500 + 2250, 750)
        );
        let mut low = UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 749,
            ..idle(13)
        };
        let mut r = rng();
        low.reset_for_order(&mut r);
        assert_eq!((low.adjusted_idle_timeout, r.draws), (749, 0));
    }

    #[test]
    fn an_idle_transport_ship_retargets_like_a_soldier() {
        // type 11, TribeTransportShipUnitAIModule: base slots 4 and 73, slot 72 0x140c21d20 draws
        // nothing
        let mut u = idle(11);
        let mut r = rng();
        u.update_idle(50, 900, false, &mut r).unwrap();
        assert_eq!(
            (u.last_retarget_time, u.retarget_timer),
            (900, 750 + 7241 % 500)
        );
    }
}
