//! What the unit AI's view asks of its unit: its picker, a helper's tests, a forced attack,
//! its victim and readiness, a master switch, its target's visibility, the villager swap and
//! the hunt plan.

use crate::combat::damage::Victim;
use crate::construction::build::Walk;
use crate::map::spatial::Placed;
use crate::movement::{Me, footprint_gap_squared};
use crate::objects::unit::Unit;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::world_view::{
    HuntPlan, NOT_HELD, UnitView, VILLAGER_FEMALE, VILLAGER_MALE, WorldPicker,
};
use crate::unit_ai::{GenieError, UnitAi};
use crate::world::lookups::{
    Releaser, SideWorld, task_search, task_target_with, victim_in, victim_of,
};

impl UnitView<'_> {
    /// The world as `bestUnitToAttack` and the attack tests read it for the unit
    /// ([`WorldPicker`]), with `seeker` and without a terrain restriction.
    pub(super) const fn picker(&self, seeker: Option<crate::unit_ai::Seeker>) -> WorldPicker<'_> {
        WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: -1,
            unit: &*self.unit,
            attributes: self.attributes,
            relations: self.relations,
            ai_settings: self.ai_settings,
            now: self.world_time,
            mode_680: self.mode_680,
        }
    }

    /// `AskForHelp`'s tests of a helper `u` (of player `owner`, master `m`) that passed its
    /// class, owner and reach tests, against `attacker`, all reads (the game asks its attack
    /// check before its owner's test; neither changes what the other reads): a helper in no group
    /// helps only when idle (no action value, no action, no order, an empty queue and no target:
    /// "He has a `UnitAIaction`"), one in a group only when the group patrols (`FUN_141017c30` or
    /// `FUN_141017c40`); then its top action's target within its range (slot 400, slot 0x1e8:
    /// the footprints' squared gap between the least and the weapon range squared) keeps it, but
    /// a building whose master has no attack for a patrolling group's member; then its unit AI's
    /// attack check (slot 0xa0 with 0, [`crate::unit_ai::can_attack`]) on the attacker. True for
    /// a helper that answers.
    pub(crate) fn helper_free(
        &self,
        owner: usize,
        u: &Unit,
        m: &crate::objects::masters::Master,
        attacker: i32,
    ) -> Result<bool, GenieError> {
        let (others, foreign, player) = (self.others, self.foreign, self.player);
        let find = |id: i32| {
            others
                .iter()
                .find(|o| o.id == id)
                .map(|o| (player, o))
                .or_else(|| foreign.find(id))
        };
        let master_of =
            |q: usize, o: &Unit| self.every.get(q).and_then(|ms| ms.get(o.life.master_id));
        let Some(ai) = u.ai.as_ref() else {
            return Ok(false);
        };
        let patrolling = match self.groups.get(u.group.id) {
            None => {
                let busy = u.top_action_kind().is_some() || u.unmodelled_action.is_some();
                if ai.current_action != -1
                    || busy
                    || ai.current_order != -1
                    || !ai.orders.is_empty()
                    || ai.current_target != -1
                {
                    return Ok(false);
                }
                false
            }
            Some(g) if g.patrols() => true,
            Some(_) => return Ok(false),
        };
        if let Some((tq, t)) = u.top_target().filter(|&t| t != -1).and_then(find) {
            let tm = master_of(tq, t).ok_or(NOT_HELD)?;
            let gap = footprint_gap_squared(u.position, m.radius, t.position, tm.radius);
            let in_range =
                gap <= m.weapon_range * m.weapon_range && m.min_range * m.min_range <= gap;
            let spared = patrolling && tm.kind == crate::orders::BUILDING && tm.weapons.is_empty();
            if in_range && !spared {
                return Ok(false);
            }
        }
        let computer = if owner == self.player {
            self.computer
        } else {
            self.foreign.player(owner).is_some_and(|q| q.computer)
        };
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(u.id))
            .map_or(0, |s| s.capture);
        let me = u.attacker_of(
            ai.ai_type,
            Some(m),
            (owner, computer),
            self.attacker.scenario,
            capture,
        );
        let target_kind = (ai.current_target != -1)
            .then(|| {
                find(ai.current_target)
                    .and_then(|(q, o)| master_of(q, o))
                    .map(|tm| tm.kind)
            })
            .flatten();
        let picker = self.picker(None);
        let (target, _) = picker.target(attacker)?;
        crate::unit_ai::can_attack(&me, &ai.seeker_with(target_kind), &target)
    }

    /// The forced attack command on the target of the unit's own Attack (`FUN_141081a80`): the
    /// top Attack's slot 0x48 (`FUN_140a841d0`, [`crate::combat::attack::Attack::work`]) on the target
    /// and its place, with the main building of an annex for a unit of an area effect, the
    /// unit's top speed and canMoveToAttack ([`crate::unit_ai::Seeker::can_move`]); what it takes
    /// and lets go is the caller's ([`UnitView::takes`], [`UnitView::releases`]), a new target
    /// clearing the unit's attack count (`+0x900`). Its answer is not read.
    pub(super) fn attack_again(
        &mut self,
        target: i32,
        t: &Placed,
        master: &crate::objects::masters::Master,
        ai: &UnitAi,
    ) -> Result<(), GenieError> {
        let mark = |id: i32, p: &Placed| -> Result<crate::combat::attack::Mark, GenieError> {
            let (_, u) = self
                .foreign
                .find(id)
                .or_else(|| self.others.iter().find(|u| u.id == id).map(|u| (0, u)))
                .ok_or(NOT_HELD)?;
            let m = self
                .every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(NOT_HELD)?;
            Ok(crate::combat::attack::Mark {
                id,
                kind: m.kind,
                class: m.class,
                state: u.life.state,
                position: p.position,
                radius: p.radius,
            })
        };
        let target_mark = mark(target, t)?;
        let main = match self
            .foreign
            .find(target)
            .map(|(_, u)| u.annex_of)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == target)
                    .map(|u| u.annex_of)
            })
            .flatten()
        {
            Some(m) => match self.spatial.get(m).copied() {
                Some(p) => Some(mark(m, &p)?),
                None => None,
            },
            None => None,
        };
        let unit = &mut *self.unit;
        let wielder = crate::combat::attack::Wielder {
            area_effect: master.area_effect,
            speed: unit.mover.as_ref().map_or(0.0, |m| m.stats.speed),
            position: unit.position,
            radius: master.radius,
        };
        let can_move = ai.current_order != -1 || ai.stance <= 1;
        let attack = unit.attack.as_mut().expect("the caller's own Attack");
        let w = attack.work(Some(target_mark), main, &wielder, |_| can_move);
        let by = Releaser {
            id: unit.id,
            class: master.class,
            player: self.player,
        };
        if w.clears_count {
            unit.attack_count = 0;
        }
        if let Some(old) = w.lets_go {
            self.releases.push((old, by));
        }
        if let Some(new) = w.takes {
            self.takes.push((new, by));
        }
        Ok(())
    }

    /// What a hit of the unit reads of the object `id` ([`victim_in`]): one of the owner's live
    /// objects or another player's; None for an object the view does not hold (the owner's
    /// sleeping objects, or the old owner's of a unit taken over in its update, whose attributes
    /// it does not hold).
    pub(super) fn victim_of_target(&self, id: i32) -> Option<Victim> {
        let (q, t, attributes) = if let Some(t) = self.others.iter().find(|u| u.id == id) {
            (self.player, t, self.attributes)
        } else {
            let (q, t) = self.foreign.find(id)?;
            (q, t, &self.foreign.player(q)?.attributes[..])
        };
        let master = self.every.get(q)?.get(t.life.master_id)?;
        let dip = SideWorld {
            player: self.player,
            relations: self.relations,
            foreign: self.foreign,
            drop_sites: self.drop_sites,
        };
        Some(victim_in(
            t,
            (q, master, attributes),
            (self.player, &dip),
            self.spatial,
            self.mode_680,
        ))
    }

    /// The start's `set_state(6)` of the unit's new Attack ([`crate::combat::attack::Attack::enter_ready`])
    /// with what its readyToAttack and its swing read: the target as a hit reads it
    /// ([`UnitView::victim_of_target`]), the unit as a combat object, its unit AI's stance, leash
    /// and queue. Returns the notifies it posted. Its swing lands no hit and fires nothing before
    /// its update, nor does readyToAttack let the target go or send the unit home there.
    pub(super) fn ready_at_start(
        &mut self,
        attack: &mut crate::combat::attack::Attack,
        master: &crate::objects::masters::Master,
        ai: &UnitAi,
    ) -> Result<Vec<(i32, [i32; 3])>, GenieError> {
        let mut victim = self
            .victim_of_target(attack.target)
            .ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
        let unit = &mut *self.unit;
        let own = victim_of(
            unit,
            master,
            self.attributes,
            self.player,
            self.spatial.terrain.as_ref(),
            self.mode_680,
        );
        let me = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: crate::combat::attack::READY,
        };
        let walk = Walk {
            mover: unit.mover.as_mut(),
            me,
            dt: 0.0,
            world: self.spatial,
            plan: None,
        };
        let mut blasted = Vec::new();
        let mut strike = crate::combat::attack::Strike {
            sprite: &mut unit.sprite,
            anim_speed: &mut unit.anim_speed,
            standing: self.graphics.get(&master.standing_sprite).copied(),
            volleys: self
                .masters
                .is_some_and(|m| crate::combat::attack::volleys(master, m)),
            attack_timer: &mut unit.attack_timer,
            reload_worldtime: &mut unit.reload_worldtime,
            reload: unit.reload_stinger + (master.reload - unit.reload_powerup),
            hit_chance: master.hit_chance,
            victim: &mut victim,
            blasted: &mut blasted,
            sprite_rng: self.sprite,
            own,
            executed: None,
            fire: None,
            can_move: ai.current_order != -1 || ai.stance <= 1,
            stance: ai.stance,
            state_position: ai.state_position,
            posted: Vec::new(),
            queue_empty: ai.orders.is_empty(),
            world_time: self.world_time,
            ai_order: Some(ai.current_order),
            ai_type: Some(ai.ai_type),
            computer: self.computer,
            armed: crate::objects::unit::armed(master, unit.firepower),
            volley_under_way: unit.next_volley != 0,
            let_go: None,
            forgets: false,
            sends_home: false,
            swing_sprites: crate::combat::attack::swing_sprites(
                unit.abilities.look.as_ref(),
                master,
                self.graphics,
            ),
        };
        attack.enter_ready(walk, &mut strike)?;
        // set_state 6 and 7 hit nothing, fire nothing and let nothing go: the update does
        debug_assert!(strike.executed.is_none() && strike.fire.is_none());
        debug_assert!(strike.let_go.is_none() && !strike.forgets && !strike.sends_home);
        Ok(std::mem::take(&mut strike.posted))
    }
}

impl<'u> UnitView<'u> {
    /// Slot 0x90 with 0 from `own` to `next` (a task's switch, a villager's swap): the action list
    /// clears (the old actions' teardowns), then the master's unit part
    /// ([`crate::orders::switch_master`]); what it gives and takes of the owner's attributes and
    /// its promotion are the owner's part after the update.
    ///
    /// # Errors
    ///
    /// What the teardowns and the switch do not model.
    pub(super) fn switch_unit_master(
        &mut self,
        own: &crate::objects::masters::Master,
        next: &crate::objects::masters::Master,
    ) -> Result<(), GenieError> {
        let unit = &mut *self.unit;
        let by = Releaser {
            id: unit.id,
            class: own.class,
            player: self.player,
        };
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        let world = crate::orders::SwitchWorld {
            spatial: self.spatial,
            pathing: self.pathing,
            graphics: self.graphics,
            sprite: self.sprite,
        };
        let switched = crate::orders::switch_master(unit, own, next, world)?;
        self.attribute_adds.extend(switched.leaving);
        if switched.promoted {
            self.promotions.push((unit.id, own.id, next.id));
        }
        self.attribute_adds.extend(switched.coming);
        Ok(())
    }

    /// The attack command's sight of its target `t` (of master `t_master`): its tile seen by the
    /// unit's player, or explored when the master shows in the fog (its fog flag).
    ///
    /// # Errors
    ///
    /// A tile the map's sight does not hold.
    pub(super) fn target_visible(
        &self,
        t: &Placed,
        t_master: &crate::objects::masters::Master,
    ) -> Result<bool, GenieError> {
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        Ok(self.spatial.sees_tile(self.player, tx, ty)
            || (t_master.fog_visibility != 0 && self.spatial.explored(self.player, tx, ty)?))
    }

    /// The attack command's villager swap: a villager (class 4) switches to its player's villager
    /// of its task swap group (`+0x7b8` 1: the master table's `+0x298`, 83; else `+0x928`, 293)
    /// when the player has it.
    pub(super) fn villager_swap(
        &self,
        master: &crate::objects::masters::Master,
    ) -> Option<&'u crate::objects::masters::Master> {
        if master.class != 4 {
            return None;
        }
        let swap = if master.task_by_group == 1 {
            VILLAGER_MALE
        } else {
            VILLAGER_FEMALE
        };
        self.masters.and_then(|m| m.get(swap))
    }

    /// [`UnitView::hunt_now`]'s plan: the target as the unit's player finds it (an annex
    /// stands for its main building's master among the hunter's drop sites), slot 0x820's task
    /// with any filter and the master it switches to ([`task_search`]), the unit's own master,
    /// the quarry the Hunt starts on and the target's place.
    ///
    /// # Errors
    ///
    /// A target gone or of a master the slice does not have, no hunt task, and a target without
    /// its place.
    ///
    /// [`UnitView::hunt_now`]: crate::unit_ai::world_view::UnitView::hunt_now
    pub(super) fn hunt_plan(&self, target: i32) -> Result<HuntPlan<'u>, GenieError> {
        use crate::gathering::hunt::Quarry;
        use crate::unit_ai::task_finder::Diplomacy;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let unit = &*self.unit;
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (tp, t) = find(target).ok_or(crate::orders::HUNT_TARGET_GONE)?;
        let t_master = self
            .every
            .get(tp)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(crate::world::OBJECT_MASTER)?;
        let main = if t_master.kind == crate::orders::BUILDING {
            t.annex_of.unwrap_or(target)
        } else {
            target
        };
        let main_master = match find(main) {
            Some((mp, m)) => {
                self.every
                    .get(mp)
                    .and_then(|ms| ms.get(m.life.master_id))
                    .ok_or(crate::world::OBJECT_MASTER)?
                    .id
            }
            None => t_master.id,
        };
        let (Some(task_t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Err(crate::orders::HUNT_TARGET_GONE);
        };
        let masters = self.masters.ok_or(crate::world::HUNTER_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::world::HUNTER_MASTER)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let (task, switch) = task_search(masters, &actor, &task_t, &general, -1, &dip)?
            .ok_or(crate::orders::NO_HUNT_TASK)?;
        let hunter = switch.unwrap_or(own);
        let quarry = Quarry {
            id: target,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: dip.allied(player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: hunter.drop_sites.contains(&main_master),
        };
        let them = *self
            .spatial
            .get_known(target)?
            .ok_or(crate::world::TARGET_UNPLACED)?;
        Ok(HuntPlan {
            task,
            switch,
            own,
            quarry,
            them,
        })
    }
}
