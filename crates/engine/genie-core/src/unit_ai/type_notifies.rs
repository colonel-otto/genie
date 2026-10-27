//! Each unit-AI type's notify handler (slot 0x238): the animals', the villager's, the ships',
//! the siege's, the soldier's, the tower's and the building's.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_EXECUTED, ACTION_FAILED, ACTION_INVALIDATED, ATTACK_ACTION,
    ATTACK_ORDER, BREAK, BUILD_ACTION, DAMAGED, DEAD, Effect, GO_ON, HUNT_ACTION,
    LION_ATTACKED_TIMEOUT, MISSILE_WARNING, MOVE_ASIDE_ACTION, NO_LONGER_ON_ME, Notify, OTHER_LOAD,
    PACKED_UNIT_CLASS, PASTURE_BUILT, REQUIRES_SEARCH, SENT_OFF, Surroundings, TARGET_MOVED,
    UnitAi, UnitSide, VILLAGER_NOTIFY, alert_owner, back_up, dodges, top_target_after,
};

impl UnitAi {
    /// The hunted animal's handler (`TribeHuntedAnimalUnitAIModule`, `FUN_140c26420`): its
    /// damage notify evades ([`UnitAi::hunted_damaged`]), 0x20f goes on; any other notify is the
    /// base handler's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hunted_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            DAMAGED => self.hunted_damaged(n, time, at, computer, ai_rng, side, fx),
            0x20f => Ok(GO_ON),
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The boar's handler (`FUN_140c25770`): its damage notify attacks
    /// ([`UnitAi::boar_damaged`]); a completion while it moves off a footprint (0x2bf) makes its
    /// idle timeout the adjusted one plus 1, drops the target (`FUN_140c5e1e0`) and sends it home
    /// on its next idle check (slot 0x1f0, `FUN_140c16d50`: `+0x1fd`), then goes to the base
    /// handler; 0x20f goes on.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn boar_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            DAMAGED => self.boar_damaged(n, time, at, ai_rng, side, fx),
            ACTION_COMPLETED => {
                if self.current_action == MOVE_ASIDE_ACTION {
                    self.idle_timeout = self.adjusted_idle_timeout.wrapping_add(1);
                    self.remove_target();
                    if let Some(home) = self.home.as_mut() {
                        home.go_home = true;
                    }
                }
                self.base_notify(n, time, at, computer, ai_rng, side, fx)
            }
            0x20f => Ok(GO_ON),
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The lion's handler (`FUN_140c266f0`): its completed attack sets its idle timeout
    /// (`FUN_140c41bc0` with 1000, then slot 0x1a8), drops the target and stops; a moved target
    /// is [`UnitAi::lion_target_moved`]'s. Both cases, and any other notify, go on to the base
    /// handler.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lion_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            ACTION_COMPLETED => {
                if n.p[0] == ATTACK_ACTION {
                    // FUN_140c41bc0 with 1000: the idle timeout, then slot 0x1a8
                    self.set_idle_timeout(LION_ATTACKED_TIMEOUT, ai_rng);
                    self.remove_target();
                    self.stop_current(true, at, fx)?;
                }
                self.base_notify(n, time, at, computer, ai_rng, side, fx)
            }
            TARGET_MOVED => self.lion_target_moved(n, time, at, computer, ai_rng, side, fx),
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The lion's moved target: a target the world holds within the lion's line of sight
    /// ("Target is still visible, tracking.", `FUN_14118f8c0` against its sight squared:
    /// `COMISS`, `JC` past) whose master has no flag 0x10 (`+0x654`) it attacks again (slot 0xb8:
    /// the target, forced, not any, its group) and the processing breaks. Else ("Target is
    /// invalid or no longer visible, stopping pursuit.") the target goes, the action stops (slot
    /// 0xb0 with 1) and slot 0x48 hands the notify to the owner (not held), the base handler's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn lion_target_moved(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        // "Target is still visible, tracking.": a target the world holds within the
        // lion's line of sight (FUN_14118f8c0 against its sight squared: COMISS, JC
        // past)
        let gap = at.target.filter(|_| self.current_target != -1).map(|t| {
            crate::movement::footprint_gap_squared(at.position, at.size, t.position, t.radius)
        });
        // a target within its sight whose master has no flag 0x10 (`+0x654`) it
        // attacks again (slot 0xb8: the target, forced, not any, its group) and the
        // processing breaks
        let flags = at.target.map_or(0, |t| t.flags);
        if let Some(gap) = gap
            && !crate::math::fmath::below_or_unordered(at.los * at.los, gap)
            && flags & 0x10 == 0
        {
            let _ = self.attack_object(self.current_target, true, false, at, side, fx)?;
            return Ok(BREAK);
        }
        // "Target is invalid or no longer visible, stopping pursuit.": the target
        // goes, the action stops (slot 0xb0 with 1) and slot 0x48 hands the notify
        // to the owner (not held)
        self.remove_target();
        self.stop_current(true, at, fx)?;
        self.base_notify(n, time, at, computer, ai_rng, side, fx)
    }

    /// The villager's handler (`FUN_140c22bf0`; the notifies it has no case for go to the base
    /// handler): a completion or failure of its action stops it, and slot 0x48 passes the notify
    /// to its owner's slot 0x2e8 (not held); a failed build (0x202, its
    /// Build waiting without work, action 0x25a) looks for another foundation to build
    /// (`FUN_140c1c7c0`, when the player's `+0x613` is 0: unsaved, 0 in this game): one the
    /// unit reaches within 0.1 (slot 0x3d0) it builds (slot 0xf0 with 1) and the processing
    /// breaks; finding none, or not reaching it, a human player's villager stops (slot
    /// 0x3c0) and the processing breaks. On the attack action (600) an invalidation, a
    /// moved target or a search (0x1fb, 0x1fc, 0x202) is the base handler's ("My attack
    /// target's moved, just doing normal target moved processing."), but a moved target
    /// with `+0x1a8` set: the unit's slot 0x328 posts the attack's failure to the AI
    /// (0x1f9 with 600, from and to the unit) and processing goes on. A missile's warning it
    /// does not dodge (the player's `+0xe10`) is its damage; the object its build or hunt made
    /// is [`UnitAi::work_on_made`]'s, a load of another resource [`UnitAi::drop_other_load`]'s.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn villager_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            ACTION_INVALIDATED | TARGET_MOVED | REQUIRES_SEARCH
                if self.current_action == ATTACK_ACTION =>
            {
                if n.kind == TARGET_MOVED && self.stop_after_kill {
                    let failed = Notify::own(at.id, ACTION_FAILED, [ATTACK_ACTION, 0, 0]);
                    self.post(failed, at.state)?;
                    return Ok(GO_ON);
                }
                self.base_notify(n, time, at, computer, ai_rng, side, fx)
            }
            ACTION_INVALIDATED | TARGET_MOVED | REQUIRES_SEARCH => {
                self.villager_search(n, at, computer, side, fx)
            }
            // a completion or a failure of its action (0x1fa, 0x1f9) stops it
            ACTION_COMPLETED | ACTION_FAILED => {
                if self.current_action == n.p[0] {
                    self.remove_target();
                    self.stop_current(true, at, fx)?;
                }
                Ok(GO_ON)
            }
            ACTION_EXECUTED if computer => Err(GenieError::unsupported(
                "a computer villager's executed action",
            )),
            ACTION_EXECUTED => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            // a missile's warning it does not dodge (the player's `+0xe10`) is its damage
            MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
            DAMAGED | MISSILE_WARNING => {
                self.villager_damaged(n, time, at, computer, ai_rng, side, fx)
            }
            // off the build or the hunt action, the object its action made (a pasture,
            // `p[1]`) which the world holds: the unit works it at its place (slot 0x7e8,
            // the caller's)
            PASTURE_BUILT => self.work_on_made(n, at, side, fx),
            // a load of another resource asked off (FUN_141658e20 from its place, its slot
            // 0x128 on the site, its slot 0x30 back to its action's target: the caller's)
            OTHER_LOAD => Self::drop_other_load(side, fx),
            // its own cases the slice does not model; any other is the base handler's
            0x217..=0x21a => {
                Err(VILLAGER_NOTIFY.with("notify", format!("{:#x} {:?}", n.kind, n.p)))
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The villager's made object (0x21d): off the build or the hunt action, the object its
    /// action made (a pasture, `p[1]`) which the world holds: the unit works it at its place
    /// (slot 0x7e8, the caller's).
    pub(super) fn work_on_made(
        &self,
        n: Notify,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if matches!(n.p[0], BUILD_ACTION | HUNT_ACTION)
            && let Some(made) = Self::world_object(n.p[1], at, side)?
        {
            fx.push(Effect::WorkOn {
                target: made.id,
                point: made.position,
            });
        }
        Ok(GO_ON)
    }

    /// The villager's load of another resource asked off (0x21c; `FUN_141658e20` from its place,
    /// its slot 0x128 on the site, its slot 0x30 back to its action's target: the caller's):
    /// slot 0x190 first, its top action's target, then the drop site the table names.
    pub(super) fn drop_other_load(
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let back = top_target_after(fx, side)?;
        if let Some(site) = side.table_drop_site()? {
            fx.push(Effect::DropElsewhere { site, back });
        }
        Ok(GO_ON)
    }

    /// The transport ship's handler (`FUN_140c2a580`) and the siege tower's (`FUN_140c28400`,
    /// the same cases; [`UnitAi::transport_damaged`] tells them apart: a human player's tower in a
    /// group that is not idle ignores the damage): its damage notify, and a missile's warning it
    /// takes as damage unless it dodges; any other notify is the base handler's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn transport_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
            DAMAGED | MISSILE_WARNING => {
                self.transport_damaged(n, time, at, computer, ai_rng, side, fx)
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The trade ship's and cart's handler (`FUN_140c29bf0`): its search when its action needs
    /// one, its damage notify, a missile's warning it takes as damage unless it dodges.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn trader_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            REQUIRES_SEARCH => self.trader_search(at, fx),
            MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
            DAMAGED | MISSILE_WARNING => {
                self.trade_damaged(n, time, at, computer, ai_rng, side, fx)
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The fishing ship's handler (`FUN_140c25ca0`): a builder's search for more to build, its
    /// damage notify, a missile's warning it takes as damage unless it dodges.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fishing_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            REQUIRES_SEARCH if self.current_action == BUILD_ACTION => {
                self.fishing_builder_search(at, computer, side, fx)
            }
            MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
            DAMAGED | MISSILE_WARNING => {
                self.fishing_damaged(n, time, at, computer, ai_rng, side, fx)
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The siege unit's handler (`FUN_140c287e0`): a computer player's packed unit's damage
    /// notify ([`UnitAi::packed_damaged`]); a target under its least range (0x1fe) makes it back
    /// up, as does a computer player's ranged unit's executed attack on a drawn chance (0x200);
    /// any other notify is the base handler's.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn siege_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            DAMAGED if at.class == PACKED_UNIT_CLASS && computer => {
                self.packed_damaged(n, time, at, computer, ai_rng, side, fx)
            }
            0x1fe => back_up(),
            ACTION_EXECUTED if computer && self.keeps_distance(at, ai_rng) => {
                if self.current_action == ATTACK_ACTION {
                    back_up()
                } else {
                    Ok(GO_ON)
                }
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// A computer player's packed siege unit's damage: dead (hit points below 1) it alerts its
    /// owner, its notifies end and it gives up; an attacker the world does not hold ends it
    /// (`FUN_140c54e30`); else it stops and runs away, and the processing breaks.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn packed_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if n.p[1] < 1 {
            alert_owner(computer)?;
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        if at.unit(n.caller).is_none() {
            return Ok(GO_ON);
        }
        self.stop_current(true, at, fx)?;
        let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
        Ok(BREAK)
    }

    /// The soldier's handler (`FUN_140c28bb0`): its own cases are the computer's, a backing-up
    /// and a healer's search.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn soldier_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            0x1fe => back_up(),
            ACTION_EXECUTED | 0x20f if computer => {
                Err(GenieError::unsupported("a computer soldier's notify"))
            }
            0x202 if self.current_action == 0x25b => {
                Err(GenieError::unsupported("a healer's search notify"))
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The tower's handler (`FUN_140c299f0`): its damage notify, alive or dead, and its
    /// destroyed target.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn tower_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            // "I'm dead, I give up.": the owner's alert (slot 0x2e8 with 0x209: a human
            // player's alert list and the old pather's hulled-area reset, not held; a
            // computer player's AI hears it), the notifies end and it gives up. Alive, the
            // other owners of the units inside it hear 0x201 and the base handler's damage
            // case follows
            DAMAGED if n.p[1] < 1 => {
                alert_owner(computer)?;
                self.end_notifies(time, at, fx)?;
                Ok(DEAD)
            }
            DAMAGED => {
                alert_owner(at.guest_computer)?;
                self.damaged(n, time, at, computer, ai_rng, side, fx)
            }
            // "I am a defensive building and my current target has been destroyed, purging
            // attack order.": a target gone or dead (above state 2) is forgotten
            // (`FUN_140c5e1e0`) and the current action stops (slot 0xb0 with 1)
            ACTION_COMPLETED if n.p[0] == 600 && self.current_order == ATTACK_ORDER => {
                let gone = self.current_target == -1 || at.target.is_none_or(|t| 2 < t.state);
                if gone {
                    self.remove_target();
                    self.stop_current(true, at, fx)?;
                }
                Ok(GO_ON)
            }
            _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
        }
    }

    /// The building's handler (`FUN_140c22610`): its damage notify, its attackers' coming and
    /// going; 0x202, 0x216 and 0x21b are the base handler's, any other notify ends there.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn building_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            DAMAGED => self.building_damaged(n, time, at, computer, side, fx),
            // "Unit #%d is attacking me": it joins the attackers and the unit is under attack
            600 => {
                let _ = self.add_attacker(n.caller);
                side.set_under_attack(true);
                Ok(GO_ON)
            }
            // "is no longer doing an action on me.": one in the set (FUN_140bf4cb0) leaves it
            NO_LONGER_ON_ME => {
                if let Ok(i) = self.attackers.binary_search(&n.caller) {
                    let _ = self.attackers.remove(i);
                }
                Ok(GO_ON)
            }
            0x202 | 0x216 | 0x21b => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            _ => Ok(GO_ON),
        }
    }

    /// The livestock's handler (`FUN_140c26ac0`): a unit sent off a footprint (0x216, with the
    /// new sending-off `DAT_1439e60e2` 1) is the base handler's; any notify but 0x202 drops the
    /// action value (`FUN_140c64b50`) and breaks off.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn livestock_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match n.kind {
            SENT_OFF => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            0x202 => Err(GenieError::unsupported(
                "a livestock's notify 0x202 (its settings' test)",
            )),
            _ => {
                self.set_action(-1);
                Ok(BREAK)
            }
        }
    }
}
