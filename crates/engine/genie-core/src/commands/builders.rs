//! BUILD's builders: their task, the queue and the direct path, the wait for the foundation
//! and the build command.

use crate::GenieError;
use crate::commands::{FOUNDATION_GONE, NO_BUILD_TASK};
use crate::construction::build::{BUILD, BuildAction, BuildTask};
use crate::math::rng::Stream;
use crate::movement::footprint_gap;
use crate::unit_ai::task_finder::world::{TaskWorld, task_search, task_target_with};
use crate::unit_ai::task_finder::{Actor, TaskTarget};
use crate::unit_ai::world_view::Wait;
use crate::unit_ai::{BUILD_ACTION, BUILD_ORDER};
use crate::world::{STATIC_TYPES_END, World};

impl World {
    /// BUILD's builders (the cost was paid with the foundation, [`World::found`]; the pending
    /// debit came before, [`World::debit_listed`]), the handler's last loop: each unit of the
    /// packet the world holds (`FUN_1415417a0`: any player's, asleep or not), in the packet's
    /// order, of a master of type 30 and up that moves (its slot 0xe8; COMISS: a NaN speed too),
    /// or any unit of a queued BUILD: its ignore list empties (`FUN_141197da0`); with a unit AI,
    /// a queued BUILD, or one that `commits` while the AI holds an order not ready
    /// (`FUN_140c4dfa0`), goes into its queue as the build order (slot 0x30 into slot 0x28:
    /// [`World::queue_build`], issued by the unit's own player); else its slot 0x128 on the
    /// foundation with 1: a static object's (types below 40, `FUN_141071bd0`) does nothing; a
    /// combat object's empties its unit AI's queue first ([`World::empty_queue_for_work`]); a unit's
    /// takes the Build ([`World::build_task`], [`World::build_directly`]), another player's
    /// whatever task its slot 0x7e8 finds on the foundation ([`World::order_work`]). A unit the world no
    /// longer holds is passed over. A committing BUILD's queue then takes the readiness pass with
    /// 0 (`FUN_140c42b60`, [`crate::orders::readiness`]): every queued order is ready.
    pub(crate) fn task_builders(
        &mut self,
        player: usize,
        foundation: i32,
        master: i16,
        position: [f32; 3],
        builders: &[i32],
        (queued, commits): (bool, bool),
    ) -> Result<(), GenieError> {
        // the builders go through the order splitter, which counts the command
        self.groups.count_command();
        let site = (player, foundation, master);
        for &id in builders {
            let Some((q, unit)) = self.object(id) else {
                continue;
            };
            let m = self.master_of(q, unit.life.master_id)?;
            let speed = m.moving.map_or(0.0, |s| s.speed);
            let kind = m.kind;
            let moves = 0x1d < kind && crate::math::fmath::below_or_unordered(0.0, speed);
            if !moves && !queued {
                continue;
            }
            let to_queue = unit
                .ai
                .as_ref()
                .is_some_and(|ai| queued || (commits && ai.has_unready_order()));
            let Some(crate::orders::Place::Live(q, idx)) = self.place(id) else {
                // a sleeping object is a static one without a unit AI: its slot 0x128 does
                // nothing
                continue;
            };
            if to_queue {
                self.queue_build(q, idx, site, position)?;
                if commits && let Some(ai) = self.players[q].objects[idx].ai.as_mut() {
                    // the readiness pass with 0: every queued order is ready
                    crate::orders::readiness(&mut ai.orders, false);
                }
            } else if STATIC_TYPES_END <= kind && q == player {
                let task = self.build_task(q, id, site, position)?;
                self.empty_queue_for_work(q, idx, kind);
                self.build_directly(q, idx, task, site, position)?;
            } else if STATIC_TYPES_END <= kind {
                self.empty_queue_for_work(q, idx, kind);
                // another player's unit: slot 0x128's slot 0x7e8 finds whatever task it has on
                // the foundation (a Build on an ally's), the work command's general path
                let work = crate::orders::Work {
                    target: foundation,
                    point: [position[0], position[1], 0.0],
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(q, idx, &work)?;
            } else if let Some(m) = self.players[q].objects[idx].mover.as_mut() {
                m.ignore_units.clear();
            }
        }
        Ok(())
    }

    /// Slot 0x128's start with 1 on the unit at `idx` of player `q`, of master type `kind`: a
    /// combat object's and a building's (`FUN_141211b80`, types 70 and 80) set `+0xa14` and
    /// empty their unit AI's order queue (`FUN_140c5e010`); the action object's
    /// (`FUN_14106c590`, types 40 to 60) goes straight to the work.
    pub(super) fn empty_queue_for_work(&mut self, q: usize, idx: usize, kind: u8) {
        if crate::objects::masters::master_type::COMBAT <= kind
            && let Some(ai) = self.players[q].objects[idx].ai.as_mut()
        {
            ai.orders.clear();
        }
    }

    /// What slot 0x128 on `foundation` (of `master`, player `fp`'s, at `position`) takes for the
    /// builder `id` of player `q`: the master it builds with ([`Masters::work_master`], with
    /// `q`'s masters and attributes) and the Build action of that master's task. A builder
    /// whose action would be replaced, without a Build task for it or with one not enabled, is
    /// not modelled.
    ///
    /// [`Masters::work_master`]: crate::objects::masters::Masters::work_master
    pub(crate) fn build_task(
        &self,
        q: usize,
        id: i32,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(i16, BuildAction), GenieError> {
        let masters = self.masters.get(q).ok_or(GenieError::unsupported(
            "BUILD tasks the builders: no master table",
        ))?;
        let site =
            self.masters
                .get(fp)
                .and_then(|m| m.get(master))
                .ok_or(GenieError::unsupported(
                    "BUILD of a master the player does not have",
                ))?;
        let target = TaskTarget {
            id: foundation,
            master: site,
            player: fp,
            state: 0,
            held: (-1, 0.0),
            hp: 1.0,
            max_hp: f32::from(site.hit_points),
            attackable: true,
            pasture: -1,
            top_action: None,
        };
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let owner = &world.players[q];
        let unit = owner
            .objects
            .iter()
            .find(|o| o.id == id)
            .ok_or(GenieError::unsupported("a builder the slice does not have"))?;
        if unit.artifact.is_some() || unit.production.is_some() || unit.fly.is_some() {
            return Err(GenieError::unsupported(
                "a builder with an action (its replacement)",
            ));
        }
        let actor = Actor {
            id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(GenieError::unsupported(
                    "a unit of a master the player does not have",
                ))?,
            player: q,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let (with, task) = masters
            .work_master(&actor, &target, &target, -1, &world)?
            .ok_or(GenieError::unsupported(
                "a builder with no task for its foundation",
            ))?;
        if task.action_type != BUILD {
            return Err(GenieError::unsupported(
                "a builder whose task for its foundation is not Build",
            ));
        }
        if !crate::unit_ai::task_finder::enabled(&owner.attributes, task.enablement) {
            return Err(GenieError::unsupported(
                "a builder whose Build task is not enabled",
            ));
        }
        let builder = masters
            .get(with)
            .expect("work_master returns a master the player has");
        let gap = footprint_gap(unit.position, builder.radius, position, site.radius);
        let task = BuildTask {
            range: task.work_range,
            work_value: task.work_val1,
            work_rate: builder.work_rate + unit.work_rate_powerup,
            resource: task.attribute_type,
            id: task.id,
        };
        Ok((with, BuildAction::start(foundation, position, gap, task)))
    }

    /// The queue path of a builder at `idx` of player `q` (slot 0x30 into slot 0x28): its
    /// ignore list empties (`FUN_141197da0`) and its unit AI's queue takes the build order on
    /// `foundation` (of `master`, player `fp`'s) at `position`, issued by `q`, priority 100, the
    /// point shown, ready, as the game's saves hold it ([`World::build_order`] runs it).
    ///
    /// [`World::build_order`]: crate::world::World::build_order
    pub(crate) fn queue_build(
        &mut self,
        q: usize,
        idx: usize,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), GenieError> {
        let unit = &mut self.players[q].objects[idx];
        if let Some(m) = unit.mover.as_mut() {
            m.ignore_units.clear();
        }
        let state = unit.life.state;
        let order = crate::unit_ai::QueuedOrder {
            issuer: q as i32,
            order: BUILD_ORDER,
            priority: 100,
            target: foundation,
            target_owner: fp as i32,
            point: [position[0], position[1], 0.0],
            range: 0.0,
            shown: [position[0], position[1]],
            ready: true,
            target_type: i32::from(master),
        };
        let ai = unit.ai.as_mut().ok_or(GenieError::unsupported(
            "a queued build order without a unit AI",
        ))?;
        let _ = ai.queue_order(state, order, false, false)?;
        Ok(())
    }

    /// The direct path of a builder at `idx` of player `q`, slot 0x128 on `foundation` (player
    /// `fp`'s) at `position` with what [`World::build_task`] found: its ignore list empties
    /// (`FUN_141197da0`), the town-bell record (slot 0x7e8 with task filter −1), then the work
    /// command (`FUN_14106ff70`): the master changes, the Build takes its foundation, the unit
    /// AI's notify, the action list clears and holds the Build.
    pub(crate) fn build_directly(
        &mut self,
        player: usize,
        idx: usize,
        (with, build): (i16, BuildAction),
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), GenieError> {
        let site_class = i32::from(self.master_of(fp, master)?.class);
        if let Some(m) = self.players[player].objects[idx].mover.as_mut() {
            m.ignore_units.clear();
        }
        // slot 0x128 -> slot 0x7e8 (task filter -1): the villager's town-bell record, its
        // flag cleared (FUN_1412067f0)
        self.town_bell_order(player, idx, foundation, -1)?;
        // the work command (FUN_14106ff70): the master changes (slot 0x90, its action list
        // cleared); the Build action takes its foundation (FUN_140a7a230, slot 0x178)
        if self.players[player].objects[idx].life.master_id != with {
            self.change_master(player, idx, with)?;
        }
        self.take(foundation, self.releaser(player, idx));
        // the work command's notify (slot 0x828): the build order and the foundation, with
        // the point slot 0x128 passes (BUILD's x and y, on the ground)
        let unit = &mut self.players[player].objects[idx];
        if let Some(ai) = unit.ai.as_mut() {
            ai.notify_work(
                BUILD_ORDER,
                BUILD_ACTION,
                Some((foundation, site_class)),
                [position[0], position[1], 0.0],
                true,
                self.streams.get(Stream::Ai),
            )?;
            self.remember_order(player, idx, (BUILD_ORDER, BUILD_ACTION))?;
        }
        // the install (slot 0x808): the action list clears (its actions' teardowns), then
        // holds the Build
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build = Some(build);
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) after its unit AI's update, with what it found
    /// there ([`Wait`]): slot 0x820's switch to the master its swap group gave (slot 0x90 with 0,
    /// [`World::change_master`]; a villager's candidates are of its own type, never a
    /// building's), the action list cleared (`FUN_140a98620`, [`World::clear_actions`]; a wait
    /// it held goes too, its teardown the base one's) and the new wait in it (slot 0x808,
    /// `FUN_14106e990`: the wait's slot 0x70 with no action and its start both do nothing).
    pub(crate) fn wait_for_foundation(
        &mut self,
        player: usize,
        idx: usize,
        wait: Wait,
    ) -> Result<(), GenieError> {
        if let Some(master) = wait.switch {
            self.change_master(player, idx, master)?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].wait_solidify = Some(
            crate::construction::wait_solidify::WaitSolidify::new(wait.task, wait.target),
        );
        Ok(())
    }

    /// The unit's build command (slot 0x380, `FUN_14120d410`, forced) on `foundation`: a Build on
    /// top takes the foundation (its slot 0x48, [`World::build_take`]); without one its slot
    /// 0x820 with any action type and the switch ([`task_search`] on the foundation and its main
    /// building; the switch, slot 0x90 with 0, [`World::change_master`]) finds a task, and a
    /// Build action on it (`FUN_140aad4e0` with that task, whatever its type) replaces the
    /// unit's actions. The foundation's master's class and its place, for the unit AI. A
    /// foundation that is gone or without a task (slot 0x380's 0) is not modelled.
    pub(crate) fn build_command(
        &mut self,
        player: usize,
        idx: usize,
        foundation: i32,
    ) -> Result<(i32, [f32; 3]), GenieError> {
        if self.players[player].objects[idx].top_action_kind() == Some(BUILD) {
            let class = self.build_take(player, idx, foundation)?;
            let f_at =
                self.object(foundation)
                    .map(|(_, f)| f.position)
                    .ok_or(GenieError::unsupported(
                        "a BuildObject on a foundation that is gone",
                    ))?;
            return Ok((class, f_at));
        }
        let gone = FOUNDATION_GONE;
        let (fp, f) = self.object(foundation).ok_or_else(|| gone.clone())?;
        let (site, f_at) = (self.master_of(fp, f.life.master_id)?, f.position);
        let find = |id: i32| self.object(id);
        let t =
            task_target_with(foundation, 1, &find, &self.masters)?.ok_or_else(|| gone.clone())?;
        let general =
            task_target_with(foundation, 2, &find, &self.masters)?.ok_or_else(|| gone.clone())?;
        let masters = self
            .masters
            .get(player)
            .ok_or(GenieError::unsupported("a build order: no master table"))?;
        let world = TaskWorld {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let owner = &world.players[player];
        let unit = &owner.objects[idx];
        let actor = Actor {
            id: unit.id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(GenieError::unsupported(
                    "a unit of a master the player does not have",
                ))?,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        // slot 0x820 with any action type (0xffff): whatever task it finds, the Build takes
        // (FUN_140aad4e0 on it), its type and its enablement unread
        let (task, switch) =
            task_search(masters, &actor, &t, &general, -1, &world)?.ok_or(NO_BUILD_TASK)?;
        let builder = switch.unwrap_or(actor.master);
        let gap = footprint_gap(unit.position, builder.radius, f_at, site.radius);
        let build = BuildAction::start(
            foundation,
            f_at,
            gap,
            BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
                resource: task.attribute_type,
                id: task.id,
            },
        );
        let class = i32::from(site.class);
        if let Some(m) = switch.map(|m| m.id) {
            self.change_master(player, idx, m)?;
        }
        self.take(foundation, self.releaser(player, idx));
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build = Some(build);
        Ok((class, f_at))
    }
}
