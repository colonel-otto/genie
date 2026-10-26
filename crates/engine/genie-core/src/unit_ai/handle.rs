//! The notifies' handling, the artifact's and the base's.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_EXECUTED, ACTION_FAILED, ACTION_INVALIDATED, ARTIFACT_AI,
    ARTIFACT_CARED, ATTACK_ACTION, ATTACK_ORDER, BREAK, BUILD_ACTION, DAMAGED, DEAD, Effect,
    FISHING_AI, GO_ON, GROUP_ASIDE_ACTION, HUNT_ACTION, IN_THE_WAY, LION_ATTACKED_TIMEOUT,
    MISSILE_WARNING, MOVE_ASIDE_ACTION, Memory, NO_LONGER_ON_ME, Notify, OTHER_LOAD, OWNER_CHANGED,
    PACKED_UNIT_CLASS, PASTURE_BUILT, Pick, REQUIRES_SEARCH, SENT_OFF, SENT_OFF_GROUP, SIEGE_AI,
    SIEGE_TOWER_AI, Search, Surroundings, TARGET_MOVED, TRADE_AI, UnitAi, UnitSide,
    VILLAGER_NOTIFY, alert_owner, back_up, dodges, search_radius, top_target_after,
};

impl UnitAi {
    /// Slot 0x238 per factory type: each type's handler, most of which pass the notify on to the
    /// base handler ([`UnitAi::base_notify`]).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match self.ai_type {
            // TribeHuntedAnimalUnitAIModule (FUN_140c26420): its damage notify evades
            2 => match n.kind {
                DAMAGED => self.hunted_damaged(n, time, at, computer, ai_rng, side, fx),
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the boar (FUN_140c25770): its damage notify attacks; a completion while it moves
            // off a footprint (0x2bf) makes its idle timeout the adjusted one plus 1, drops the
            // target (FUN_140c5e1e0) and sends it home on its next idle check (slot 0x1f0,
            // FUN_140c16d50: +0x1fd), then goes to the base handler
            4 => match n.kind {
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
            },
            // the lion (FUN_140c266f0); both its cases go on to the base handler
            5 => match n.kind {
                ACTION_COMPLETED => {
                    if n.p[0] == ATTACK_ACTION {
                        // FUN_140c41bc0 with 1000: the idle timeout, then slot 0x1a8
                        self.set_idle_timeout(LION_ATTACKED_TIMEOUT, ai_rng);
                        self.remove_target();
                        self.stop_current(true, at, fx)?;
                    }
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                TARGET_MOVED => {
                    // "Target is still visible, tracking.": a target the world holds within the
                    // lion's line of sight (FUN_14118f8c0 against its sight squared: COMISS, JC
                    // past)
                    let gap = at.target.filter(|_| self.current_target != -1).map(|t| {
                        crate::movement::footprint_gap_squared(
                            at.position,
                            at.size,
                            t.position,
                            t.radius,
                        )
                    });
                    // a target within its sight whose master has no flag 0x10 (`+0x654`) it
                    // attacks again (slot 0xb8: the target, forced, not any, its group) and the
                    // processing breaks
                    let flags = at.target.map_or(0, |t| t.flags);
                    if let Some(gap) = gap
                        && !crate::math::fmath::below_or_unordered(at.los * at.los, gap)
                        && flags & 0x10 == 0
                    {
                        let _ =
                            self.attack_object(self.current_target, true, false, at, side, fx)?;
                        return Ok(BREAK);
                    }
                    // "Target is invalid or no longer visible, stopping pursuit.": the target
                    // goes, the action stops (slot 0xb0 with 1) and slot 0x48 hands the notify
                    // to the owner (not held)
                    self.remove_target();
                    self.stop_current(true, at, fx)?;
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the villager (FUN_140c22bf0; the notifies it has no case for go to the base handler):
            // a completion or failure of its action stops it, and slot 0x48
            // passes the notify to its owner's slot 0x2e8 (not held); a failed build (0x202, its
            // Build waiting without work, action 0x25a) looks for another foundation to build
            // (FUN_140c1c7c0, when the player's +0x613 is 0: unsaved, 0 in this game): one the
            // unit reaches within 0.1 (slot 0x3d0) it builds (slot 0xf0 with 1) and the processing
            // breaks; finding none, or not reaching it, a human player's villager stops (slot
            // 0x3c0) and the processing breaks. On the attack action (600) an invalidation, a
            // moved target or a search (0x1fb, 0x1fc, 0x202) is the base handler's ("My attack
            // target's moved, just doing normal target moved processing."), but a moved target
            // with `+0x1a8` set: the unit's slot 0x328 posts the attack's failure to the AI
            // (0x1f9 with 600, from and to the unit) and processing goes on
            8 => match n.kind {
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
                PASTURE_BUILT => {
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
                // a load of another resource asked off (FUN_141658e20 from its place, its slot
                // 0x128 on the site, its slot 0x30 back to its action's target: the caller's)
                OTHER_LOAD => {
                    // slot 0x190 first: its top action's target
                    let back = top_target_after(fx, side)?;
                    if let Some(site) = side.table_drop_site()? {
                        fx.push(Effect::DropElsewhere { site, back });
                    }
                    Ok(GO_ON)
                }
                // its own cases the slice does not model; any other is the base handler's
                0x217..=0x21a => {
                    Err(VILLAGER_NOTIFY.with("notify", format!("{:#x} {:?}", n.kind, n.p)))
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the transport ship (FUN_140c2a580): its damage notify, and a missile's warning it
            // takes as damage unless it dodges; any other notify is the base handler's
            11 => match n.kind {
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.transport_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the trade ship and cart (FUN_140c29bf0): its search when its action needs one, its
            // damage notify, a missile's warning it takes as damage unless it dodges
            TRADE_AI => match n.kind {
                REQUIRES_SEARCH => self.trader_search(at, fx),
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.trade_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the fishing ship (FUN_140c25ca0): a builder's search for more to build, its damage
            // notify, a missile's warning it takes as damage unless it dodges
            FISHING_AI => match n.kind {
                REQUIRES_SEARCH if self.current_action == BUILD_ACTION => {
                    self.fishing_builder_search(at, computer, side, fx)
                }
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.fishing_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the siege unit (FUN_140c287e0): a computer player's packed unit's damage notify; a
            // target under its least range (0x1fe) makes it back up, as does a computer
            // player's ranged unit's executed attack on a drawn chance (0x200); any other notify
            // is the base handler's
            SIEGE_AI => match n.kind {
                DAMAGED if at.class == PACKED_UNIT_CLASS && computer => {
                    if n.p[1] < 1 {
                        alert_owner(computer)?;
                        self.end_notifies(time, at, fx)?;
                        return Ok(DEAD);
                    }
                    // FUN_140c54e30: an attacker the world does not hold ends it
                    if at.unit(n.caller).is_none() {
                        return Ok(GO_ON);
                    }
                    self.stop_current(true, at, fx)?;
                    let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
                    Ok(BREAK)
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
            },
            // the siege tower (FUN_140c28400): as the transport ship's, but a human player's
            // tower in a group that is not idle ignores the damage
            SIEGE_TOWER_AI => match n.kind {
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.transport_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the soldier (FUN_140c28bb0): its own cases are the computer's, a backing-up and a
            // healer's search
            13 => match n.kind {
                0x1fe => back_up(),
                ACTION_EXECUTED | 0x20f if computer => {
                    Err(GenieError::unsupported("a computer soldier's notify"))
                }
                0x202 if self.current_action == 0x25b => {
                    Err(GenieError::unsupported("a healer's search notify"))
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the tower (FUN_140c299f0)
            15 => match n.kind {
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
            },
            // the building (FUN_140c22610): most notifies end there
            17 => match n.kind {
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
            },
            // livestock (FUN_140c26ac0): a unit sent off a footprint (0x216, with the new
            // sending-off `DAT_1439e60e2` 1) is the base handler's; any notify but 0x202 drops
            // the action value (FUN_140c64b50) and breaks off
            19 => match n.kind {
                SENT_OFF => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                0x202 => Err(GenieError::unsupported(
                    "a livestock's notify 0x202 (its settings' test)",
                )),
                _ => {
                    self.set_action(-1);
                    Ok(BREAK)
                }
            },
            ARTIFACT_AI => self.artifact_notify(n, time, at, computer, ai_rng, side, fx),
            _ => Err(GenieError::unsupported(
                "the notify handler of a unit-AI type not modelled",
            )),
        }
    }

    /// The artifact's notify handler (slot 0x238, `FUN_140c21f70`): the kinds it lets be (500,
    /// 0x1f6 to 0x1f8, 0x1fd to 0x200, 0x202, 0x203, 0x20b, 0x20f, 0x259 to 0x262, 0x265 to
    /// 0x269, 0x278, 699 and 0x2be) go on; a unit attacking it (600) joins its attackers
    /// (`FUN_140b43f30`) and puts its unit under attack (`FUN_14119d2d0` with 1); a unit sent off
    /// a footprint (0x216, 0x21b) is the base handler's. A completion, failure or invalidation
    /// of its action (without an order its action value, with one the order's action: action +
    /// 100 = order) and a moved target the world no longer holds or that its owner no longer
    /// sees (`FUN_1416635e0` not 0xf) end in its stop, as any other kind does ("Unprocessed
    /// notify"): the target goes (`FUN_140c5e1e0`), the action stops (slot 0xb0 with 1) and slot
    /// 0x48 hands the notify to the owner (its alerts, not held). Each answers 2.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn artifact_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let stops = match n.kind {
            500
            | 0x1f6..=0x1f8
            | 0x1fd..=0x200
            | 0x202
            | 0x203
            | OWNER_CHANGED
            | 0x20f
            | 0x259..=0x262
            | 0x265..=0x269
            | 0x278
            | 699
            | 0x2be => false,
            ACTION_FAILED | ACTION_COMPLETED | ACTION_INVALIDATED => {
                if self.current_order == -1 {
                    self.current_action == n.p[0]
                } else {
                    self.current_action + 100 == self.current_order
                }
            }
            // "Target is still visible.": the world holds the target, which its owner sees
            TARGET_MOVED => !at
                .target
                .is_some_and(|t| self.current_target != -1 && t.sight == 0xf),
            SENT_OFF | SENT_OFF_GROUP => {
                return self.base_notify(n, time, at, computer, ai_rng, side, fx);
            }
            600 => {
                let _ = self.add_attacker(n.caller);
                side.set_under_attack(true);
                false
            }
            _ => true,
        };
        if stops {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// The artifact's processIdle (slot 72, `FUN_140c1e410`, factory type 14): `ClosestObject`
    /// (`FUN_140c488b0`) of any class it cares about, relation, state, master and workers from
    /// its place, the reach test only for a unit that moves (slot 0x208 above 0), which sets the
    /// alert mode ([`UnitAi::alert`]). While the lists hold the objects of at most one player
    /// but Gaia (`DAT_143e6f92c` below 2, [`Found::players`]), the object found, when the world
    /// holds it and another player owns it, takes the artifact over: the unit's slot 0x70 to its
    /// owner ([`Effect::ChangeOwner`]), and processIdle produced (6, true); else 5.
    ///
    /// [`Found::players`]: crate::unit_ai::Found::players
    pub(super) fn artifact_idle(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: search_radius(at.los, at.search_radius),
            classes: &ARTIFACT_CARED,
            size: at.size,
            pick: Pick::Closest {
                group: -1,
                relation: -1,
                state: -1,
                master: -1,
                workers: -1,
                spared: -1,
                reach: 0.0 < at.speed,
            },
            memory: Memory::default(),
        })?;
        self.alert(found, at.dt_seconds);
        if found.players < 2
            && found.id != -1
            && let Some(u) = side.seen_unit(found.id)?
            && u.owner != at.player
        {
            fx.push(Effect::ChangeOwner(u.owner));
            return Ok(true);
        }
        Ok(false)
    }

    /// The artifact's slot 73 (`FUN_140c21d40`): its processIdle (slot 0x240 with 1,
    /// [`UnitAi::artifact_idle`]); once another player took it over (6) its target goes
    /// (`FUN_140c5e1e0`), its action stops (slot 0xb0 with 1) and its idle timer is 0 (it
    /// answers 10), else 9.
    pub(super) fn artifact_misc(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if self.artifact_idle(at, side, fx)? {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            self.idle_timer = 0;
        }
        Ok(())
    }

    /// The base handler (`FUN_140c589c0`) for the kinds the slice posts: a completed action
    /// ([`UnitAi::action_completed`]), an ownership change (stopAction, `FUN_140c5e1e0`, slot 0xb0
    /// with 1, `FUN_140c5dc10` with the handler's time, the order queue emptied), the kinds it
    /// ignores; anything else is not modelled.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn base_notify(
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
            ACTION_COMPLETED => self.action_completed(n, at, computer, ai_rng, side, fx),
            ACTION_FAILED => self.action_failed(n, at, computer, ai_rng, side, fx),
            ACTION_INVALIDATED => self.action_invalidated(n, at, computer, ai_rng, side, fx),
            crate::movement::explore::WANTS_POINT if self.explorer() => {
                self.seek_unexplored(at, computer, ai_rng, side, fx)
            }
            OWNER_CHANGED => {
                fx.push(Effect::Stop);
                self.remove_target();
                self.stop_current(true, at, fx)?;
                self.end_notifies(time, at, fx)?;
                self.orders.clear();
                Ok(GO_ON)
            }
            // a MoveTo's moved target: with a saved patrol processIdle runs (slot 0x240 with 1),
            // its result unread
            TARGET_MOVED if self.patrol_path.is_some() => {
                let _ = self.process_idle(at, computer, ai_rng, side, fx)?;
                Ok(GO_ON)
            }
            // a moved target, then the kinds "I'm ignoring that." and the action values
            TARGET_MOVED
            | 0x1f6
            | 0x1f7
            | 0x1f8
            | 0x1ff
            | ACTION_EXECUTED
            | 0x203
            | 0x259..=0x262
            | 0x265..=0x269
            | 0x2be => Ok(GO_ON),
            // readyToAttack's target out of reach (0x1fd): a unit on the attack order that cannot
            // move (FUN_140fdd080: a master of type 30 and up with speed) gives the order up
            0x1fd => {
                if self.current_order == ATTACK_ORDER && !(29 < at.kind && 0.0 < at.speed) {
                    self.current_order = -1;
                }
                Ok(GO_ON)
            }
            DAMAGED | MISSILE_WARNING => self.damaged(n, time, at, computer, ai_rng, side, fx),
            IN_THE_WAY => self.in_the_way(n, at, side, fx),
            SENT_OFF => self.sent_off(n, side, fx),
            // a unit of a group sent off a footprint: the action value 0x2c0
            SENT_OFF_GROUP => {
                self.set_action(GROUP_ASIDE_ACTION);
                Ok(GO_ON)
            }
            // "Unit #%d is no longer doing an action on me.": an attacker leaves the set
            NO_LONGER_ON_ME => {
                if let Ok(i) = self.attackers.binary_search(&n.caller) {
                    let _ = self.attackers.remove(i);
                }
                Ok(GO_ON)
            }
            _ => Err(
                GenieError::unsupported("a notify kind the base handler does not model")
                    .with("notify", format_args!("{:#x}", n.kind))
                    .with("caller", n.caller),
            ),
        }
    }
}
