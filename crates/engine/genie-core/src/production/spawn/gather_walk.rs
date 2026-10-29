//! A new unit's walk to its building's gather point (`FUN_140ace9c0`).

use crate::GenieError;
use crate::commands::GatherPoint;
use crate::math::rng::Stream;
use crate::objects::unit::Unit;
use crate::orders::Work;
use crate::production::spawn::{BUILDING, VILLAGER};
use crate::unit_ai::ActionKind;
use crate::unit_ai::OrderKind;
use crate::world::World;

impl World {
    /// `FUN_140ace9c0` with a gather point without a target, and an unloaded unit's move
    /// (`FUN_1411e7220`): the unit waits to move (slot 0x2a0 with 1) and takes slot 0x128
    /// (`FUN_141211b80`) with no target at the point (z 0; its `+0xa14` `notified`: 1 for a new
    /// unit, whose unit AI's order queue empties, `FUN_140c5e010`; 0 for an unloaded one); slot
    /// 0x7e8 (`FUN_141227190`) runs the work command (`FUN_14106ff70`), which finds no task: a
    /// unit with speed in state 2 notifies its unit AI with the `MoveTo`'s order and action, no
    /// target and the point, its notifies cleared with `+0xa14` ([`UnitAi::notify_work`], slot
    /// 0x828), and moves (slot 0x138, `FUN_141216f10`: a villager is no longer under the town bell,
    /// `+0x9dc`; with `+0xa14` the order queue empties again; `FUN_14106d340`: the notifies
    /// cleared with the world time, then, the unit holding no action, the `MoveTo` to the point,
    /// range 0, installed). The command count (`+0x888`) is not held.
    ///
    /// [`UnitAi::notify_work`]: crate::unit_ai::UnitAi::notify_work
    pub(crate) fn gather_move(
        &mut self,
        p: usize,
        new: usize,
        point: [f32; 3],
        notified: bool,
    ) -> Result<(), GenieError> {
        let world_time = self.world_time;
        let sprite = self.point_move_sprite(p, new)?;
        let villager = self.players[p].objects[new].master(&self.masters, p)?.class == VILLAGER;
        let unit = &mut self.players[p].objects[new];
        if let Some(m) = unit.body.mover_mut() {
            m.waiting = 1;
        }
        if unit.top_action_kind().is_some() {
            return Err(GenieError::unsupported(
                "a gather point's move of a unit with an action",
            ));
        }
        // a unit without speed takes nothing; one not in state 2 would notify without moving
        if unit.body.mover().map_or(0.0, |m| m.stats.speed) <= 0.0 {
            return Ok(());
        }
        if unit.life.state != 2 {
            return Err(GenieError::unsupported(
                "a gather point's move of a unit not in state 2",
            ));
        }
        if let Some(ai) = unit.ai.as_mut() {
            if notified {
                ai.orders.clear();
            }
            ai.notify_work(
                Some(OrderKind::Move),
                Some(ActionKind::Move),
                None,
                point,
                notified,
                self.streams.get(Stream::Ai),
            )?;
            if notified {
                ai.orders.clear();
            }
            ai.clear_notifies(world_time)?;
        }
        if villager {
            unit.town_bell.flag = false;
        }
        unit.actions
            .set_move_to(Some(crate::movement::move_to::MoveToAction {
                sprite,
                state: crate::movement::move_to::MoveTo::Start,
                goal: Some(point),
                target: -1,
                range: 0.0,
                quiet: false,
                wait: crate::movement::move_to::MoveWait::default(),
            }));
        Ok(())
    }

    /// `FUN_140ace9c0` with a gather point (`+0xc30` 1) and a building whose unit AI holds no
    /// command queue location (its slot 0x218): the new unit waits to move (slot 0x2a0 with 1,
    /// `+0x7f9`) and takes the work command on the target at the point (slot 0x128: z 0, its
    /// `+0xa14` 1, no carry test, any task). A target that is gone is looked for again
    /// ([`World::lost_gather_target`]); one on the building itself (a garrison) and a building
    /// target (slot 0x7e8) are not modelled.
    /// `FUN_140ace9c0`'s search for a gather point's target that is gone: the closest (by its
    /// centre, the first of equals) of Gaia's live objects of the class of Gaia's master the
    /// point names (`FUN_141166670` over `+0x30`), unless it lies farther than the new unit's
    /// line of sight (squared) from the point, measured from its footprint (`FUN_140a847c0`);
    /// then, but for the classes 5, 29 and 33 (the unit AI's slot 0x80, not modelled), the same
    /// over Gaia's sleeping objects (`+0x38`). None found: the unit moves to the point.
    pub(super) fn lost_gather_target(
        &self,
        p: usize,
        new: usize,
        g: GatherPoint,
    ) -> Result<Option<i32>, GenieError> {
        let class = self
            .masters
            .first()
            .and_then(|m| m.get(g.target_master))
            .map_or(-1, |m| m.class);
        let los = self.players[p].objects[new]
            .master(&self.masters, p)?
            .line_of_sight;
        let [x, y, _] = g.point;
        let gaia = self
            .players
            .first()
            .ok_or(GenieError::unsupported("a world without Gaia"))?;
        let closest = |list: &[Unit]| -> Result<Option<i32>, GenieError> {
            let mut best: Option<(&Unit, f32)> = None;
            for u in list {
                if u.master(&self.masters, 0)?.class != class {
                    continue;
                }
                let (dx, dy) = (x - u.position()[0], y - u.position()[1]);
                let d = dy * dy + dx * dx;
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((u, d));
                }
            }
            let Some((u, _)) = best else {
                return Ok(None);
            };
            let [rx, ry] = u.master(&self.masters, 0)?.radius;
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let (gx, gy) = (gap(x - u.position()[0], rx), gap(y - u.position()[1], ry));
            Ok((gy * gy + gx * gx <= los * los).then_some(u.id))
        };
        if let Some(id) = closest(&gaia.objects)? {
            return Ok(Some(id));
        }
        if matches!(class, 5 | 29 | 33) {
            return Err(GenieError::unsupported(
                "a gather point's lost target of class 5, 29 or 33 (the unit AI's slot 0x80)",
            ));
        }
        closest(&gaia.sleeping)
    }

    /// The new unit's walk to the gather point of its building `building` (`FUN_140ace9c0` with a
    /// point or a target): a point is a move; a target that is gone is looked for again
    /// ([`World::lost_gather_target`]). The unit waits to move (slot 0x2a0 with 1); on a building
    /// its own task list's garrison finder (filter 3, [`World::garrison_task_found`]) decides: a
    /// task found is worked at once (slot 0x7e8 with filter 3, outside slot 0x128: `+0xa14` 1);
    /// else, and for any other target, slot 0x128 works it (`notified`, any task). An unloaded
    /// unit's caller goes to slot 0x128 at once (`finder` clear). A gather point on its own
    /// building takes the unit in at once ([`World::enter_forced`]) while the building holds
    /// fewer than 10 or fewer than its capacity.
    pub(crate) fn gather(
        &mut self,
        p: usize,
        new: usize,
        g: GatherPoint,
        building: i32,
        notified: bool,
        finder: bool,
    ) -> Result<(), GenieError> {
        if g.target == -1 {
            return self.gather_move(p, new, [g.point[0], g.point[1], 0.0], notified);
        }
        // its own building takes it at once (slot 0x1a0 forced) while it holds fewer than 10 or
        // fewer than its master's capacity (its slot 0x498 against `+0x5d4`); else the building
        // is a target as any other
        if g.target == building
            && let Some((bp, b)) = self.object(building)
        {
            let room = i32::from(b.master(&self.masters, bp)?.garrison);
            let held = self.garrisoned(building)?;
            if held < 10 || held < room {
                return self.enter_forced(p, new, building);
            }
        }
        let target = if self.object(g.target).is_some() {
            g.target
        } else {
            match self.lost_gather_target(p, new, g)? {
                Some(id) => id,
                None => {
                    return self.gather_move(p, new, [g.point[0], g.point[1], 0.0], notified);
                }
            }
        };
        let (owner, t) = self.object(target).ok_or(GenieError::unreachable(
            "unreachable: a gather point's target found",
        ))?;
        let on_building = t.master(&self.masters, owner)?.kind == BUILDING;
        if let Some(m) = self.players[p].objects[new].body.mover_mut() {
            m.waiting = 1;
        }
        let garrison = finder
            && on_building
            && self.garrison_task_found(p, new, target, [g.point[0], g.point[1], 0.0])?;
        let work = Work {
            target,
            point: [g.point[0], g.point[1], 0.0],
            carry_test: false,
            filter: if garrison { 3 } else { -1 },
            notified: garrison || notified,
        };
        self.order_work(p, new, &work)
    }
}
