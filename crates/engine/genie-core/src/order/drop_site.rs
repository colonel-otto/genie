//! The drop-offs: an unasked one, the ordered drop site, the table's, one now and elsewhere,
//! and the way back.

use crate::gathering::hunt::HUNT;
use crate::math::rng::Stream;
use crate::order::{BUILDING, HUNTED_LOADS, NO_POINT, Work, nearest_table_site};
use crate::turn::Turns;
use crate::unit_ai::GenieError;

impl Turns {
    /// `UNIT_ORDER`'s drop-off of a carrying unit without a gather action on top
    /// (`FUN_14161e370`'s case 8, past the Gather, Hunt and Build): a master with drop sites
    /// (`FUN_14109e210`: one not below 0) searches them from the unit ([`Turns::ordered_drop_site`]);
    /// with none found, or without drop sites, the drop-site table's nearest building for the
    /// resource it holds from its class ([`Turns::table_drop_site`]); with none, nothing. A unit
    /// (type 70 and up) then works on it (slot 0x7e8 at (−1, −1, −1): no carried-resource test,
    /// the task filter 110 for a load of meat, wood or food, kinds 15, 1 and 0, else 5;
    /// [`Turns::order_work`]) and, for the drop-off 8 (`way_back`), queues its way back to work
    /// ([`Turns::queue_way_back`]).
    pub(super) fn drop_off_unasked(
        &mut self,
        p: usize,
        idx: usize,
        way_back: bool,
    ) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        let (kind, class, at) = (
            master.kind,
            master.class,
            [unit.position[0], unit.position[1]],
        );
        let held = unit.life.attribute_type;
        let mut site = None;
        if master.drop_sites.iter().any(|&d| 0 <= d) {
            site = self.ordered_drop_site(p, idx)?;
        }
        if site.is_none() {
            site = self.table_drop_site(p, at, held, class);
        }
        let Some(site) = site else {
            return Ok(());
        };
        if kind < crate::garrison::COMBAT_KIND {
            return Ok(());
        }
        let filter = if HUNTED_LOADS.contains(&held) {
            HUNT
        } else {
            crate::gathering::gather::GATHER
        };
        let work = Work {
            target: site,
            point: [NO_POINT; 3],
            carry_test: false,
            filter,
            notified: false,
        };
        self.order_work(p, idx, &work)?;
        if way_back {
            self.queue_way_back(p, idx)?;
        }
        Ok(())
    }

    /// `FUN_1411671e0` as `UNIT_ORDER`'s drop-off calls it for the unit at (`p`, `idx`): over its
    /// master's drop sites, of its player's live objects in state 2, inside others too, at work
    /// or not, by the squared distance (not by path); the unit the seeker (its zone, its
    /// planner) where its slot 0x2c0 answers 1: any unit below type 70, else one whose master
    /// drops at neither 45 nor 2172 and of a class other than 21 and 2 (`FUN_141226580`).
    pub(super) fn ordered_drop_site(
        &mut self,
        p: usize,
        idx: usize,
    ) -> Result<Option<i32>, GenieError> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        let seeks = master.kind < crate::garrison::COMBAT_KIND
            || (!master.drop_sites.iter().any(|&d| d == 45 || d == 2172)
                && !matches!(master.class, 21 | 2));
        let masters = self.masters.get(p).ok_or(GenieError::unsupported(
            "a drop-off of a player without masters",
        ))?;
        if !seeks {
            return crate::turn::drop_site_search(
                &self.players[p].objects,
                masters,
                &master.drop_sites,
                unit.position,
                None,
                (false, -1),
            );
        }
        let state = unit.top_action().map_or(0, |a| a.state);
        let player = &self.players[p];
        let mut world = crate::turn::DropWorld {
            masters: self.masters.get(p),
            spatial: &self.spatial,
            zones: &mut self.zones,
            pathing: &mut self.pathing,
            object: self.streams.get(Stream::Object),
            world_time: self.world_time,
            computer: player.computer,
            construction_rate: player.construction_rate,
            groups: &self.groups,
        };
        player.objects[idx].drop_site_with(&mut world, &player.objects, state, (false, false, -1))
    }

    /// `FUN_141658e20` for player `p` at `at`: of the buildings the drop-site table gives for the
    /// resource `held` from workers of `class` ([`crate::tasks::table_drop_buildings`] for the
    /// player's civ), the objects of each master's done list in the player's tracker in turn
    /// (`FUN_14104f600`, the master itself the key) that the world holds in state 2, the nearest
    /// by the squared distance of the centres (the first of equals; none from
    /// [`TABLE_DROP_FAR`] on).
    ///
    /// [`TABLE_DROP_FAR`]: crate::order::TABLE_DROP_FAR
    pub(crate) fn table_drop_site(
        &self,
        p: usize,
        at: [f32; 2],
        held: i16,
        class: i16,
    ) -> Option<i32> {
        let player = &self.players[p];
        let look = |id: i32| self.object(id).map(|(_, o)| (o.life.state, o.position));
        nearest_table_site(
            (&self.drop_sites, player.civ, &player.tracker),
            (at, held, class),
            look,
        )
    }

    /// The Build's `set_state(8)` as `UNIT_ORDER`'s drop-off (slot 0x78) gives it to the builder
    /// at (`p`, `idx`): a counted builder leaves its foundation's builders first
    /// ([`Turns::leave_builders`]); a builder that carries something (`+0x310` above 0, COMISS:
    /// not a NaN) searches the drop-site table ([`Turns::table_drop_site`] for what it carries
    /// and its class) and its slot 0x820 the task on the site found (any action type, no switch,
    /// [`Turns::find_work_task`]: its range, else the Build task's); then
    /// [`crate::construction::build::BuildAction::set_drop`], whose second targets the world releases and
    /// takes and whose notify 0x203 goes to the unit AI.
    pub(crate) fn build_drop_off_now(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        use crate::construction::build::DropSearch;
        let by = self.releaser(p, idx);
        let unit = &self.players[p].objects[idx];
        let build = unit.build.as_ref().expect("the caller's Build");
        let (target, counted, task_range) = (build.target, build.added, build.task.range);
        let (amount, held, at) = (
            unit.life.amount,
            unit.life.attribute_type,
            [unit.position[0], unit.position[1]],
        );
        let class = self.master_of(p, unit.life.master_id)?.class;
        let search = if 0.0 < amount {
            match self.table_drop_site(p, at, held, class) {
                None => DropSearch::Nothing,
                Some(id) => {
                    let range = self
                        .find_work_task(p, idx, id, -1, false)?
                        .map_or(task_range, |t| t.work_range);
                    DropSearch::Site { id, range }
                }
            }
        } else {
            DropSearch::Empty
        };
        if counted {
            self.leave_builders(target)?;
        }
        let unit = &mut self.players[p].objects[idx];
        let me = crate::movement::Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: crate::construction::build::DROP,
        };
        let build = unit.build.as_mut().expect("the caller's Build");
        build.added = false;
        let mut walk = crate::movement::move_to::Walk {
            mover: unit.mover.as_mut(),
            me,
            dt: 0.0,
            world: &mut self.spatial,
            plan: None,
        };
        let out = build.set_drop(&mut walk, None, search)?;
        if out.notified {
            unit.post_own(
                crate::unit_ai::NO_DROP_SITE,
                [crate::unit_ai::BUILD_ACTION, 0, 0],
            )?;
        }
        if out.let_go != -1 {
            self.release(out.let_go, by)?;
        }
        if out.took != -1 {
            self.take(out.took, by);
        }
        Ok(())
    }

    /// Slot 0x78's answer to a load of another resource than its task's (`FUN_140a78060`): the
    /// unit AI's slot 0x38 takes notify 0x21c ([`crate::unit_ai::OTHER_LOAD`]) from and to the
    /// unit at (`p`, `idx`); a unit without a unit AI, none.
    pub(super) fn other_load(&mut self, p: usize, idx: usize) {
        let unit = &mut self.players[p].objects[idx];
        let (id, state) = (unit.id, unit.life.state);
        if let Some(ai) = unit.ai.as_mut() {
            let n = crate::unit_ai::Notify::own(id, crate::unit_ai::OTHER_LOAD, [0; 3]);
            let _ = ai.accept(n, state);
        }
    }

    /// The villager's case 0x21c ([`crate::unit_ai::Effect::DropElsewhere`]) for the unit at
    /// (`p`, `idx`) with the drop-site table's building `site` (`FUN_141658e20`) and the target
    /// its top action had (`back`, slot 0x190: the action's target id, `+0x30`, that the world
    /// holds): it works on the site at its place (slot 0x128: the carried-resource test, any task,
    /// `+0xa14` 0, [`Turns::order_work`]); with a target its unit AI then queues the order on it
    /// (slot 0x30: `0x2db` issued by the unit, the target's master as its type and its owner's
    /// number, its place, range 0, priority 100, ready, neither at once nor in front).
    ///
    /// # Errors
    ///
    /// What the work and the queue do not model.
    pub(crate) fn drop_elsewhere(
        &mut self,
        p: usize,
        idx: usize,
        site: i32,
        back: Option<i32>,
    ) -> Result<(), GenieError> {
        let back = back.filter(|&t| 0 <= t).and_then(|t| {
            let (q, o) = self.object(t)?;
            Some((t, o.life.master_id, q, o.position))
        });
        let point = self
            .object(site)
            .map(|(_, o)| o.position)
            .ok_or(GenieError::unreachable("the drop site found"))?;
        let work = Work {
            target: site,
            point,
            carry_test: true,
            filter: -1,
            notified: false,
        };
        self.order_work(p, idx, &work)?;
        let Some((target, master, owner, place)) = back else {
            return Ok(());
        };
        let unit = &mut self.players[p].objects[idx];
        let (id, state) = (unit.id, unit.life.state);
        if let Some(ai) = unit.ai.as_mut() {
            let order = crate::unit_ai::QueuedOrder {
                issuer: id,
                order: crate::unit_ai::ORDER_ON_TARGET,
                priority: 100,
                target,
                target_owner: owner as i32,
                point: place,
                range: 0.0,
                shown: [0.0; 2],
                ready: true,
                target_type: i32::from(master),
            };
            let _ = ai.queue_order(state, order, false, false)?;
        }
        Ok(())
    }

    /// `FUN_141205ac0`: the unit at (`p`, `idx`) queues its way back to work: the work its
    /// town-bell records keep ([`crate::town_bell::TownBell::back_to_work`]: the object the world
    /// holds, the second record moved up when it holds the object; else the record as it is)
    /// goes in front of its unit AI's queue (`FUN_140c41ab0` -> `FUN_140c42640`): the order on a
    /// target (`0x325`) issued by its owner on the object (its master the target type, its owner
    /// the target's, its place the point) or on the record's target (its target type, owner −1,
    /// its point); height 0, range 0, priority 100, shown at the point, ready. A unit without a
    /// unit AI only looks the records up.
    pub(crate) fn queue_way_back(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        use crate::town_bell::RecordTarget;
        let mut bell = self.players[p].objects[idx].town_bell;
        let lookup = |t: i32| -> Option<RecordTarget> {
            let (op, o) = self.object(t)?;
            let m = self.master_of(op, o.life.master_id).ok()?;
            Some(RecordTarget {
                id: t,
                building: m.kind == BUILDING,
                class: m.class,
                built: o
                    .construction
                    .is_some_and(|c| f32::from(m.build_time) <= c.progress),
            })
        };
        let (found, record) = bell.back_to_work(&lookup);
        let target = match found {
            Some(f) => {
                let (op, o) = self.object(f.id).expect("the lookup found it");
                let master = self.master_of(op, o.life.master_id)?.id;
                (
                    f.id,
                    i32::from(master),
                    op as i32,
                    [o.position[0], o.position[1]],
                )
            }
            None => (record.target, record.target_type, -1, record.point),
        };
        let unit = &mut self.players[p].objects[idx];
        unit.town_bell = bell;
        let Some(ai) = unit.ai.as_mut() else {
            return Ok(());
        };
        let (id, target_type, owner, [x, y]) = target;
        ai.insert_order(
            crate::unit_ai::QueuedOrder {
                issuer: p as i32,
                order: crate::unit_ai::ORDER_ON_TARGET_2,
                priority: 100,
                target: id,
                target_owner: owner,
                point: [x, y, 0.0],
                range: 0.0,
                shown: [x, y],
                ready: true,
                target_type,
            },
            true,
        );
        Ok(())
    }
}
