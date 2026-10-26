//! An order's processing: the combat and work orders, the top target and the attack slots.

use crate::GenieError;
use crate::rng::Well512;
use crate::unit_ai::{
    ATTACK_ACTION, ATTACK_MOVE, ATTACK_ORDER, AUTO_SCOUT_ACTION, AUTO_SCOUT_ORDER,
    BOAR_LURING_ACTION, BUILD_ORDER, CIVILIAN, CLASS_18, CONVERT_ACTION, DEFEND_ORDER,
    DEFENDLESS_MASTER, Effect, FOLLOW_ORDER, FOUNDATION_ORDER, HEAL_ACTION, HUNT_ORDER, LiveTop,
    MOVE_ORDER, ORDER_ON_TARGET, ORDER_ON_TARGET_2, OUTSIDE_ORDER, PendingTop, QueuedOrder,
    REPAIR_ACTION, REPAIR_ORDER, RUN_AWAY_ORDER, SCOUT_ACTION, SOLDIER_AI, STOP_ORDER,
    Surroundings, TREBUCHET_CLASS, TREBUCHET_PACKED_CLASS, UnitAi, UnitCommand, UnitSeen, UnitSide,
    WAIT_ACTION, WAIT_SOLIDIFY_ACTION, order_move,
};

impl UnitAi {
    /// Slot 0x230 (`FUN_140c5c170`, every type's) on the order `o`: one not ready returns 0xe.
    /// Each case runs its slot, forced (its force argument 1); a slot that does not take the
    /// order returns 0 (the queue drops it), one that does goes to the tail
    /// ([`UnitAi::order_tail`]), which makes it the AI's order with its priority, and 1:
    /// - 700, the attack: `AttackObject` by the type ([`UnitAi::attack_slot`], any target, the
    ///   group flag set).
    /// - `0x2bd`, the defend: a unit of master 128 takes none; without a target slot 0x178 at the
    ///   order's point ([`UnitAi::defend_point`]), else slot 0x170 on the target
    ///   ([`UnitAi::defend_object`]), within the order's range.
    /// - `0x2be`, the build: slot 0xf0 on the foundation and the tail, which the caller runs
    ///   ([`Effect::OrderOnTarget`], `Turns::build_order`).
    /// - `0x2bf`, `0x2c0`, `0x2ce`, the heal, conversion and repair: slots 0xe0, 0xd8 and 0xe8
    ///   ([`UnitAi::work_object`]) through the unit's commands ([`UnitCommand`]).
    /// - `0x2c1`, the explore, `0x2da`, the scout, and `0x321`, the auto scout: slot 0x100 to the
    ///   order's point cut to tiles (`CVTTSS2SI`, [`UnitAi::explore_at`]); whatever it gave, the
    ///   scout then holds the action value `0x276` and the auto scout `0x2bd`, the one before the
    ///   last.
    /// - `0x2c2`, the stop: the target goes (`FUN_140c5e1e0`), the action stops (slot 0xb0 with
    ///   1), and the tail ends the order.
    /// - `0x2c3`, run away: slot 0x160 with 1 and no unit ([`UnitAi::run_away`]).
    /// - `0x2c5`, the gather: slot 0xd0 ([`UnitAi::gather_order`]).
    /// - `0x2c6`, the move, and `0x2d4`, the group gather: slot 0x130 to the order's point within
    ///   its range, forced and walking (a move's unit then shows its points, slot 0x2d8, not
    ///   held).
    /// - `0x2c7`, the patrol ("I don't know what to do with patrol orders"), and an order no case
    ///   names: 0.
    /// - `0x2c8`, the follow: slot 0x168 on the target within the order's range
    ///   ([`UnitAi::follow_object`]).
    /// - `0x2c9`, the hunt, and `0x322`, the auto boar luring: slot 0xc8 ([`UnitAi::hunt_order`]);
    ///   whatever it gave, the luring then holds the action value `0x2be`, the one before the
    ///   last.
    /// - `0x2ca`, the transport: slot 0x128 to the order's point ([`UnitAi::transport_to`]).
    /// - `0x2cd`, the enter: slot 0x110 on the target ([`UnitAi::enter_object`]).
    /// - `0x2d1` and 800, the unloads: slots 0x118 and 0x120 at the order's point
    ///   ([`UnitAi::unload_at`]).
    /// - `0x2d2`, seek and destroy: slot 0x180 on the target at the order's point cut to tiles
    ///   ([`UnitAi::seek_and_destroy`]).
    /// - `0x2d3`, explore and destroy: slot 0x188 to the order's point cut to tiles
    ///   ([`UnitAi::explore_and_destroy`]).
    /// - `0x2d8`, the wait: the action value `0x274` and the order `0x2d8` (leaving a patrol
    ///   order as the tail does).
    /// - `0x2d9`, the attack roundup: slot 0xc0 on the target ([`UnitAi::attack_roundup`]); the
    ///   tail makes the order 700.
    /// - `0x2db` and `0x325`, the order on a target ([`UnitAi::order_on_target`]).
    /// - `0x2dd`, the attack move: a unit of class 51 or 54 with a target runs slot 0xb8 on it (the
    ///   tail makes the order 700), any other slot 0x130 to the order's point.
    /// - `0x2de`, the foundation's: slot 0x108 on the target
    ///   ([`UnitAi::wait_for_foundation`]); whatever it gave, the action value `0x27a`, the one
    ///   before the last.
    ///
    /// `world` is the unit's surroundings, its side and its owner's type, which the full update
    /// and the queue between turns pass; [`UnitAi::take_order_now`] passes none but the unit's
    /// `place` (its x and y and its master's class), as its own order (`AUTO_SCOUT`'s stop or
    /// auto scout order) is the one that runs.
    pub(crate) fn process_order(
        &mut self,
        o: &QueuedOrder,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        place: Option<([f32; 2], i16)>,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if !o.ready {
            return Ok(0xe);
        }
        let place = world
            .as_ref()
            .map(|(at, _, _)| ([at.position[0], at.position[1]], at.class))
            .or(place);
        let class = place.map_or(-1, |(_, class)| class);
        let tile = |v: f32| crate::checkpoint::cvttss2si(v);
        // `bVar2`: the attack move's attack, whose tail makes the order 700
        let mut attack_move = false;
        let taken = match o.order {
            ATTACK_ORDER
            | DEFEND_ORDER
            | RUN_AWAY_ORDER
            | crate::ai_kinds::ORDER_SEEK_AND_DESTROY
            | crate::ai_kinds::ORDER_ATTACK_ROUNDUP
            | ATTACK_MOVE => self.combat_order(o, world, ai, fx, &mut attack_move)?,
            BUILD_ORDER
            | crate::ai_kinds::ORDER_HEAL
            | crate::ai_kinds::ORDER_CONVERT
            | REPAIR_ORDER
            | crate::ai_kinds::ORDER_GATHER
            | HUNT_ORDER
            | crate::ai_kinds::ORDER_AUTO_BOAR_LURING
            | FOUNDATION_ORDER => self.work_order(o, world, fx)?,
            FOLLOW_ORDER
            | crate::ai_kinds::ORDER_TRANSPORT
            | crate::garrison::ENTER_ORDER
            | crate::ai_kinds::ORDER_UNLOAD
            | crate::ai_kinds::ORDER_UNLOAD_OVER_WALL => self.object_move_order(o, world, fx)?,
            crate::ai_kinds::ORDER_EXPLORE => {
                self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx)
            }
            STOP_ORDER => {
                self.remove_target();
                self.stop_with(true, place, fx)?;
                true
            }
            MOVE_ORDER | crate::ai_kinds::ORDER_GROUP_GATHER => {
                let m = self.move_to(order_move(o));
                fx.extend(m.map(Effect::Move));
                m.is_some()
            }
            crate::ai_kinds::ORDER_EXPLORE_AND_DESTROY => {
                self.explore_and_destroy([tile(o.point[0]), tile(o.point[1])], true, fx)
            }
            crate::ai_kinds::ORDER_WAIT => {
                // case 0x2d8: the wait's action value, leaving a patrol order, then the order
                // itself; the tail finds it the AI's already
                self.current_action = WAIT_ACTION;
                self.leave_patrol_timers(ai);
                self.current_order = crate::ai_kinds::ORDER_WAIT;
                true
            }
            crate::ai_kinds::ORDER_SCOUT => {
                let explored = self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx);
                self.set_action(SCOUT_ACTION);
                explored
            }
            ORDER_ON_TARGET | ORDER_ON_TARGET_2 => {
                let (at, side, _) = world.ok_or(OUTSIDE_ORDER)?;
                return self.order_on_target(o, class, at, side, ai, fx);
            }
            AUTO_SCOUT_ORDER => {
                let explored = self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx);
                self.set_action(AUTO_SCOUT_ACTION);
                explored
            }
            // the patrol order (0x2c7) and the orders the switch has no case for
            _ => false,
        };
        if !taken {
            return Ok(0);
        }
        Ok(self.order_tail(o.order, o.priority, class, attack_move, ai))
    }

    /// [`UnitAi::process_order`]'s combat cases: the attack, the defence (a master that cannot
    /// defend takes none: the switch's `return 0`, which is the tail's "not taken"), the run
    /// away, seek and destroy, the attack roundup and the attack move (`attack_move` its attack).
    pub(super) fn combat_order(
        &mut self,
        o: &QueuedOrder,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
        attack_move: &mut bool,
    ) -> Result<bool, GenieError> {
        let (at, side, computer) = world.ok_or(OUTSIDE_ORDER)?;
        let tile = |v: f32| crate::checkpoint::cvttss2si(v);
        Ok(match o.order {
            ATTACK_ORDER => self.attack_slot(o.target, true, at, side, fx)?,
            DEFEND_ORDER => {
                if at.master == DEFENDLESS_MASTER {
                    return Ok(false);
                }
                if o.target == -1 {
                    self.defend_point([o.point[0], o.point[1]], o.range, true, at)
                } else {
                    self.defend_object(o.target, o.range, true, at, side)?
                }
            }
            RUN_AWAY_ORDER => self.run_away(true, at, computer, ai, side, fx)?,
            crate::ai_kinds::ORDER_SEEK_AND_DESTROY => {
                let tiles = [tile(o.point[0]), tile(o.point[1])];
                self.seek_and_destroy(o.target, tiles, true, at, side, fx)?
            }
            crate::ai_kinds::ORDER_ATTACK_ROUNDUP => self.attack_roundup(o.target, at, side, fx)?,
            ATTACK_MOVE => {
                if o.target != -1 && matches!(at.class, TREBUCHET_CLASS | TREBUCHET_PACKED_CLASS) {
                    *attack_move = true;
                    self.attack_slot(o.target, true, at, side, fx)?
                } else {
                    let m = self.move_to(order_move(o));
                    fx.extend(m.map(Effect::Move));
                    m.is_some()
                }
            }
            _ => false,
        })
    }

    /// [`UnitAi::process_order`]'s work cases: the build (slot 0xf0, `FUN_140c47d80`, with 1 on
    /// the target; the tail makes the order the AI's with its priority), the heal, convert and
    /// repair, the gather, the hunt and auto boar luring (the luring then holds the action value
    /// `0x2be`) and the foundation's wait (then the action value `0x27a`).
    pub(super) fn work_order(
        &mut self,
        o: &QueuedOrder,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let (at, side, _) = world.ok_or(OUTSIDE_ORDER)?;
        Ok(match o.order {
            BUILD_ORDER => self.build_object(o.target, true, at, side, fx)?,
            crate::ai_kinds::ORDER_HEAL | crate::ai_kinds::ORDER_CONVERT | REPAIR_ORDER => {
                let (action, command) = match o.order {
                    crate::ai_kinds::ORDER_HEAL => (HEAL_ACTION, UnitCommand::Heal(o.target)),
                    crate::ai_kinds::ORDER_CONVERT => {
                        (CONVERT_ACTION, UnitCommand::Convert(o.target))
                    }
                    _ => (REPAIR_ACTION, UnitCommand::Repair(o.target)),
                };
                self.work_object(o.target, true, action, command, at, side)?
            }
            crate::ai_kinds::ORDER_GATHER => self.gather_order(o.target, at, side, fx)?,
            HUNT_ORDER | crate::ai_kinds::ORDER_AUTO_BOAR_LURING => {
                let hunts = self.hunt_order(o.target, at, side, fx)?;
                if o.order == crate::ai_kinds::ORDER_AUTO_BOAR_LURING {
                    self.set_action(BOAR_LURING_ACTION);
                }
                hunts
            }
            FOUNDATION_ORDER => {
                let waits = self.wait_for_foundation(o.target, true, side)?;
                self.set_action(WAIT_SOLIDIFY_ACTION);
                waits
            }
            _ => false,
        })
    }

    /// [`UnitAi::process_order`]'s moves onto an object or a point of one: the follow, the
    /// transport, the enter and the two unloads.
    pub(super) fn object_move_order(
        &mut self,
        o: &QueuedOrder,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let (at, side, _) = world.ok_or(OUTSIDE_ORDER)?;
        Ok(match o.order {
            FOLLOW_ORDER => self.follow_object(o.target, o.range, true, at, side, fx)?,
            crate::ai_kinds::ORDER_TRANSPORT => self.transport_to(o.point, true, side)?,
            crate::garrison::ENTER_ORDER => self.enter_object(o.target, true, at, side)?,
            crate::ai_kinds::ORDER_UNLOAD | crate::ai_kinds::ORDER_UNLOAD_OVER_WALL => {
                let over_wall = o.order == crate::ai_kinds::ORDER_UNLOAD_OVER_WALL;
                self.unload_at([o.point[0], o.point[1]], true, over_wall, at, side)?
            }
            _ => false,
        })
    }

    /// The order's tail (`LAB_140c5d432`) once its case took order value `order` of priority
    /// `priority`: the stop (`0x2c2`) leaves a patrol order ([`UnitAi::leave_patrol_timers`]) and
    /// ends the order and its priority (−1). The attack roundup (`0x2d9`) and the attack move's
    /// attack (`attack_move`) leave a patrol order and become the attack order (700); the attack
    /// order of a unit of class 18 (`class`, its master's) leaves a patrol order, its target goes
    /// (`+0x48` −1, its class and point kept) and the order is −1. Any other becomes the order,
    /// leaving a patrol order for another (`LAB_140c5d611`). The priority is the order's but
    /// for the stop. Returns 1.
    pub(crate) fn order_tail(
        &mut self,
        order: i32,
        priority: i32,
        class: i16,
        attack_move: bool,
        ai: &mut Well512,
    ) -> u32 {
        if order == STOP_ORDER {
            self.leave_patrol_timers(ai);
            self.current_order = -1;
            self.order_priority = -1;
            return 1;
        }
        let new = if order == crate::ai_kinds::ORDER_ATTACK_ROUNDUP || attack_move {
            self.leave_patrol_timers(ai);
            ATTACK_ORDER
        } else if order == ATTACK_ORDER && class == CLASS_18 {
            self.leave_patrol_timers(ai);
            self.current_target = -1;
            -1
        } else {
            if self.current_order != order {
                self.leave_patrol_timers(ai);
            }
            order
        };
        self.current_order = new;
        self.order_priority = priority;
        1
    }

    /// Slot 400 as the full update stands: the target of the unit's top action, its type and the
    /// footprints' squared gap (`FUN_14118f8c0`). The game's stop (slot 0x3c0) empties the
    /// action list at once and a command fills it, so a read later in the same update sees
    /// either: an [`Effect`] pending in `fx` that replaces the actions decides
    /// ([`top_target_after`]), else a command the update gave ([`UnitAi::commanded`],
    /// [`UnitSide::top_target_now`]), its object as the world holds it
    /// ([`UnitAi::world_object`]); with neither, what the update read at its start
    /// ([`Surroundings::top_target`]).
    ///
    /// # Errors
    ///
    /// An object the side does not hold.
    ///
    /// [`top_target_after`]: crate::unit_ai::top_target_after
    pub(super) fn top_target_live(
        &self,
        at: &Surroundings,
        fx: &[Effect],
        side: &mut dyn UnitSide,
    ) -> Result<Option<LiveTop>, GenieError> {
        let start = at.top_target.map(|t| LiveTop {
            id: t.id,
            kind: t.kind,
            gap_squared: crate::movement::footprint_gap_squared(
                at.position,
                at.size,
                t.position,
                t.radius,
            ),
        });
        let id = match PendingTop::of(fx) {
            PendingTop::Replaced(id) => id,
            PendingTop::Unchanged if self.commanded => side.top_target_now()?,
            PendingTop::Unchanged => return Ok(start),
        };
        let Some(id) = id else {
            return Ok(None);
        };
        if let Some(t) = start.filter(|t| t.id == id) {
            return Ok(Some(t));
        }
        Ok(Self::world_object(id, at, side)?.map(|s| LiveTop {
            id,
            kind: s.kind,
            gap_squared: s.gap_squared,
        }))
    }

    /// Slot 400 within the unit's range (slot 0x1e8, `FUN_141086330`): its top action's target
    /// as the update stands ([`UnitAi::top_target_live`]) when the footprints' squared gap is at
    /// most the weapon range squared and at least the least range squared (`COMISS`/`JC` each:
    /// a NaN is not in range).
    ///
    /// # Errors
    ///
    /// An object the side does not hold.
    pub(super) fn top_target_in_range(
        &self,
        at: &Surroundings,
        fx: &[Effect],
        side: &mut dyn UnitSide,
    ) -> Result<Option<i32>, GenieError> {
        let Some(t) = self.top_target_live(at, fx, side)? else {
            return Ok(None);
        };
        let gap = t.gap_squared;
        let in_range =
            gap <= at.weapon_range * at.weapon_range && at.min_range * at.min_range <= gap;
        Ok(in_range.then_some(t.id))
    }

    /// The object `id` as the world holds it (`FUN_1415417a0`): the unit itself (`at`, its own
    /// target of no one), among the objects the update names ([`Surroundings::unit`]), else as
    /// the unit's side finds it ([`UnitSide::seen_unit`]); None when the world does not hold it.
    pub(super) fn world_object(
        id: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<Option<UnitSeen>, GenieError> {
        if id == at.id {
            return Ok(Some(UnitSeen {
                id,
                class: at.class,
                owner: at.player,
                combat_level: at.combat_level,
                state: at.state,
                target: None,
                gap_squared: crate::movement::footprint_gap_squared(
                    at.position,
                    at.size,
                    at.position,
                    at.size,
                ),
                position: at.position,
                hp: at.hp,
                master: at.master,
                kind: at.kind,
                main: None,
                allied: true,
            }));
        }
        at.unit(id)
            .map_or_else(|| side.seen_unit(id), |u| Ok(Some(*u)))
    }

    /// `AttackObject` (slot 0xb8) forced, by the unit AI's type, on `target`, `any` target or
    /// not: the civilian's (`FUN_140c17370`, [`UnitAi::civilian_attack`]: an animal or a tree it
    /// hunts, any other it attacks, not `any`); the soldier's (`FUN_140c17550`) the base's, then
    /// a member of a group the world holds stops moving ([`UnitSide::group_attack_stop`]); the
    /// others' the base's ([`UnitAi::attack_object`]). The priest's (`FUN_140c17490`) asks its
    /// slot 0x1b8 (`FUN_140c1af00`) whether to convert the target first: not modelled, the base's
    /// stops at the priest's slot 0xa0, which is not either.
    pub(super) fn attack_slot(
        &mut self,
        target: i32,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        match self.ai_type {
            CIVILIAN => self.civilian_attack(target, at, side, fx),
            SOLDIER_AI => {
                if !self.attack_object(target, true, any, at, side, fx)? {
                    return Ok(false);
                }
                if at.group.is_some() {
                    side.group_attack_stop()?;
                }
                Ok(true)
            }
            _ => self.attack_object(target, true, any, at, side, fx),
        }
    }

    /// `attackRoundupObject` (slot 0xc0, `FUN_140c43aa0`) on `target`: the AI's own target
    /// ("I'm already attacking that target") or one the world does not hold takes nothing; then
    /// slot 0xa0 not `any` ([`UnitAi::attack_check`]) must let it attack the target; with an
    /// action value the unit must reach the target within its range (slot 0x3d0,
    /// [`UnitSide::reaches`]: "can't path to new target"). Then the unit's attack command, forced
    /// ([`UnitAi::attack_command`]), and on success the action is the attack (600, the last action
    /// keeping the old) and the target its class and place (`FUN_140c64c10`). Unlike
    /// `AttackObject`, `+0x1d5` stays.
    pub(super) fn attack_roundup(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if self.current_target == target {
            return Ok(false);
        }
        let Some((class, position)) = self.attack_check(target, false, at, side)? else {
            return Ok(false);
        };
        if self.current_action != -1 && !side.reaches(target, at.weapon_range)? {
            return Ok(false);
        }
        if !self.attack_command(target, true, at, side, fx)? {
            return Ok(false);
        }
        self.set_action(ATTACK_ACTION);
        self.target_at(target, Some(i32::from(class)), position);
        Ok(true)
    }
}
