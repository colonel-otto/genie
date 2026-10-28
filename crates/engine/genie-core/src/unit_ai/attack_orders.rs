//! The orders a unit AI takes at once and its attack commands.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::unit_ai::{
    ARTIFACT_AI, CIVILIAN, Effect, Notify, PENDING_BEFORE_ATTACK, QueuedOrder, SIEGE_AI,
    SIEGE_TOWER_AI, Surroundings, UnitAi, UnitSide, civilian_may_attack, lion_may_attack,
};

impl UnitAi {
    /// `FUN_140c55c90`'s immediate branch ("Adding as an immediate order."), for a unit in state
    /// 2 or below and an order other than −1. With `keep_from` (inFront 1, the unit's id) a
    /// current order goes in front of the queue first ("Keeping old order, putting it onto the
    /// queue.", [`UnitAi::insert_order`]): the issuer the unit, the current order, priority
    /// (`+0x40`), target and its class, owner −1, the target point, the range `+0x5c`, and the
    /// new order's shown point and ready flag. Then the order goes in front. But for an attack
    /// roundup (`0x2d9`, "so I'm not stopping the object") the target goes (`FUN_140c5e1e0`) and
    /// the action stops (slot 0xb0 with 1, which ends an order of its action and reads the
    /// unit's `place` for it). The notifies end (`FUN_140c5dc10` with 0; outside the full update
    /// without `world`, where a completion it would stop on is not modelled), the queue's first
    /// order runs at once (slot 0x230, [`UnitAi::process_order`] with `world`, the surroundings
    /// and the unit) and, unless it was not ready, leaves the queue (`FUN_140c5e280`).
    pub(super) fn order_now(
        &mut self,
        order: QueuedOrder,
        keep_from: Option<i32>,
        place: Option<([f32; 2], i16)>,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if let Some(unit) = keep_from
            && let Some(current) = self.current_order
        {
            let kept = QueuedOrder {
                issuer: unit,
                order: current,
                priority: self.order_priority,
                target: self.current_target,
                target_owner: -1,
                point: self.target_point,
                range: self.desired_distance,
                shown: order.shown,
                ready: order.ready,
                target_type: self.target_type,
            };
            self.insert_order(kept, true);
        }
        self.insert_order(order, true);
        if order.order != OrderKind::AttackRoundup {
            self.remove_target();
            self.stop_with(true, place, fx)?;
        }
        match &world {
            Some((at, _, _)) => self.end_notifies(0, at, fx)?,
            None => self.clear_notifies(0)?,
        }
        let front = self.orders[0];
        if self.process_order(&front, world, place, ai, fx)? != 0xe {
            let _ = self.orders.remove(0);
        }
        Ok(())
    }

    /// `FUN_140c42640`: an order joins the queue, in front of the first when `in_front` and the
    /// queue holds one, else after the last. A full queue doubles its capacity first, its new
    /// records filled with a stop order (`0x2c2`) to nothing (Vec's own growth here); a failed
    /// allocation, the routine's only other way out, cannot happen (`operator new` throws).
    pub(crate) fn insert_order(&mut self, order: QueuedOrder, in_front: bool) {
        if in_front && !self.orders.is_empty() {
            self.orders.insert(0, order);
        } else {
            self.orders.push(order);
        }
    }

    /// `AttackObject` (slot 0xb8, `FUN_140c43720`): `+0x1d5` clears; not `force`d, an AI with an
    /// action fails (false). The target must pass [`UnitAi::attack_check`] with `any`; then the
    /// unit's attack command ([`UnitAi::attack_command`]), on success the action is the attack
    /// (600, the last action keeping the old) and the target its point and class
    /// (`FUN_140c64c10`). The group flag's attack by the unit's group (behind the byte
    /// `DAT_143f01835`, which nothing writes) cannot run.
    pub(super) fn attack_object(
        &mut self,
        target: i32,
        force: bool,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action.is_some() {
            return Ok(false);
        }
        let Some((class, position)) = self.attack_check(target, any, at, side)? else {
            return Ok(false);
        };
        if !self.attack_command(target, force, at, side, fx)? {
            return Ok(false);
        }
        self.set_action(Some(ActionKind::Attack));
        self.target_at(target, Some(i32::from(class)), position);
        Ok(true)
    }

    /// `AttackObject`'s test of `target` (`FUN_140c43720`, and `attackRoundupObject`'s): the world
    /// must hold it and its slot 0xa0 with `any` must let it attack the target: the base's
    /// (`FUN_140c48400`: the boar's, the ranged units', the ships', the artifact's, the siege
    /// tower's and the livestock's) anything, the building's (`FUN_140c18b60`) nothing, the
    /// lion's ([`lion_may_attack`]), the civilian's ([`civilian_may_attack`]), the soldier's, the
    /// tower's and the siege unit's ([`UnitSide::may_attack`]); the hunted animal's
    /// (`FUN_140c18d00`) reads its action list (slot 0x538) and the priest's (`FUN_140c19020`) its
    /// conversions: not modelled. Its master's class and its place then.
    pub(super) fn attack_check(
        &self,
        target: i32,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        if matches!(self.ai_type, 13 | 15 | SIEGE_AI) {
            return side.may_attack(target, &self.seeker(at));
        }
        let Some(t) = Self::world_object(target, at, side)? else {
            return Ok(None);
        };
        let may = match self.ai_type {
            // FUN_140c48400, the base's: the boar's, the ranged units' (6, 7), the trade
            // ship's, the transport ship's, the fishing ship's, the artifact's, the siege
            // tower's and the livestock's
            4 | 6 | 7 | 10 | 11 | 12 | ARTIFACT_AI | SIEGE_TOWER_AI | 19 => true,
            5 => any || lion_may_attack(at.attack_priority, t.class),
            CIVILIAN => civilian_may_attack(at.attack_priority, t.class),
            // FUN_140c18b60
            17 => false,
            _ => {
                return Err(GenieError::unsupported(
                    "AttackObject's slot 0xa0 of a unit AI type not modelled",
                ));
            }
        };
        Ok(may.then_some((t.class, t.position)))
    }

    /// The unit's attack command (slot 0x338, [`UnitSide::attack`]) on `target`, `force`d or not,
    /// as `AttackObject`, `attackRoundupObject` and seek and destroy give it: a stop the
    /// processing made before (slot 0x3c0, a pending [`Effect::Stop`]) emptied the action list
    /// before the command fills it, so it runs first ([`UnitSide::stop`]), with the caps on the
    /// last human order time before it ([`UnitSide::cap_human_order`]) in their order. The notifies the
    /// command posts (the unit's slot 0x328 -> the AI's slot 0x38) go into the queue, or the
    /// buffer while it is full. True when the unit attacks: it then holds an action of its own
    /// ([`UnitAi::commanded`]).
    pub(super) fn attack_command(
        &mut self,
        target: i32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if fx.contains(&Effect::Stop) {
            if fx.iter().any(|e| {
                !matches!(
                    e,
                    Effect::Stop
                        | Effect::CapHumanOrder(_)
                        | Effect::BuildObject(_)
                        | Effect::HuntObject(_)
                        | Effect::Move(_)
                )
            }) {
                return Err(PENDING_BEFORE_ATTACK.with("effects", format!("{fx:?}")));
            }
            // the effects before it ran before it: in order (a requeued build or hunt order's
            // command too, whose action the attack command replaces)
            for e in std::mem::take(fx) {
                match e {
                    Effect::CapHumanOrder(cap) => side.cap_human_order(cap)?,
                    Effect::BuildObject(foundation) => side.build_now(foundation)?,
                    Effect::HuntObject(quarry) => side.hunt_now(quarry, self)?,
                    Effect::Move(m) => side.ai_move_now(m)?,
                    _ => side.stop()?,
                }
            }
        }
        let attacked = side.attack(target, force, self)?;
        for (kind, p) in attacked.posted {
            self.post(Notify::own(at.id, kind, p), at.state)?;
        }
        if attacked.attacks {
            self.commanded = true;
        }
        Ok(attacked.attacks)
    }

    /// The civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`) with its force flag 1, as every
    /// caller in the slice passes it (the damage notify and the unit in the way): an object the
    /// world does not hold (`FUN_1415417a0`) gets nothing (false); an animal or a tree (classes 9,
    /// 10 and 15) is hunted (slot 0xc8, [`UnitAi::hunt_object`]); any other goes to the base
    /// `AttackObject` with `any` 0 and the group flag 1 ([`UnitAi::attack_object`]). True when
    /// it hunts or attacks.
    pub(super) fn civilian_attack(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let t = match at.unit(target) {
            Some(t) => *t,
            None => match side.seen_unit(target)? {
                Some(t) => t,
                None => return Ok(false),
            },
        };
        if matches!(t.class, 9 | 10 | 15) {
            self.hunt_object(target, i32::from(t.class), t.position, at, fx)?;
            return Ok(true);
        }
        self.attack_object(target, true, false, at, side, fx)
    }

    /// The attackers' upkeep of the full update (`0x140c6741b`): with none, a unit under attack
    /// no longer is (`FUN_14119d2d0` with 0); else the first attacker that the world does not
    /// hold, that has no unit AI, is dead (state above 2) or whose unit AI's target is another
    /// leaves the set (one an update).
    pub(super) fn keep_attackers(&mut self, at: &Surroundings, side: &mut dyn UnitSide) {
        if self.attackers.is_empty() {
            if at.under_attack {
                side.set_under_attack(false);
            }
            return;
        }
        let gone = self.attackers.iter().position(|&id| {
            at.unit(id)
                .is_none_or(|u| 2 < u.state || u.target != Some(at.id))
        });
        if let Some(i) = gone {
            let _ = self.attackers.remove(i);
        }
    }
}
