//! What the unit AI's view asks of its unit: its picker, a helper's tests and a master switch.

use crate::GenieError;
use crate::movement::footprint_gap_squared;
use crate::objects::release::Releaser;
use crate::objects::unit::Unit;
use crate::unit_ai::UnitAi;
use crate::unit_ai::world_view::owner_view;
use crate::unit_ai::world_view::{UnitView, WorldPicker};
use crate::world::World;

impl UnitView<'_, '_> {
    /// The world as `bestUnitToAttack` and the attack tests read it for the unit
    /// ([`WorldPicker`]), with `seeker` and without a terrain restriction.
    pub(super) fn picker(&self, seeker: Option<crate::unit_ai::Seeker>) -> WorldPicker<'_> {
        WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: owner_view(&self.c.w.players, self.c.player, self.new_owner),
            foreign: owner_view(&self.c.w.players, self.c.player, self.new_owner).players(),
            spatial: &self.c.w.spatial,
            every: &self.c.w.masters,
            restriction: -1,
            unit: &*self.unit,
            attributes: &self.c.w.players[self.player].attributes,
            relations: &self.c.w.players[self.player].relations,
            ai_settings: self.ai_settings,
            now: self.c.w.world_time,
            mode_680: self.c.w.mode_680,
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
        let others = owner_view(&self.c.w.players, self.c.player, self.new_owner);
        let find = |id: i32| others.find(id);
        let Some(ai) = u.ai.as_ref() else {
            return Ok(false);
        };
        let patrolling = match self.c.w.groups.get(u.group.id) {
            None => {
                let busy = u.top_action_kind().is_some() || u.unmodelled_action.is_some();
                if ai.current_action.is_some()
                    || busy
                    || ai.current_order.is_some()
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
            let tm = t.master(&self.c.w.masters, tq)?;
            let gap = footprint_gap_squared(u.position(), m.radius, t.position(), tm.radius);
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
            owner_view(&self.c.w.players, self.c.player, self.new_owner)
                .players()
                .player(owner)
                .is_some_and(|q| q.computer)
        };
        let capture = self
            .c
            .w
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
                    .and_then(|(q, o)| o.master(&self.c.w.masters, q).ok())
                    .map(|tm| tm.kind)
            })
            .flatten();
        let picker = self.picker(None);
        let (target, _) = picker.target(attacker)?;
        crate::unit_ai::can_attack(&me, &ai.seeker_with(target_kind), &target)
    }
}

impl UnitView<'_, '_> {
    /// `f` on the world with the unit lent back to its slot ([`crate::world::step::Step::lend_at`]):
    /// a world routine on the unit while its AI runs, at once where the game's AI calls the
    /// unit's slot (its commands 0x388, 0x378, 0x3a0 and 0x3a8, its master change 0x90). The AI
    /// is out of the unit for its run: the routine finds a stand-in in its place (a default unit
    /// AI), and a routine that changed the stand-in fails the lend; a notify for the unit's own
    /// AI comes back in the routine's answer instead.
    ///
    /// # Errors
    ///
    /// `f`'s and the lend's, and a routine that reached the unit's AI.
    pub(crate) fn lend<R>(
        &mut self,
        f: impl FnOnce(&mut World, usize, usize) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        self.unit.ai = Some(UnitAi::default());
        let answer = self.c.lend_at(self.unit, f);
        let stand_in = self.unit.ai.take();
        let answer = answer?;
        if stand_in != Some(UnitAi::default()) {
            return Err(GenieError::unreachable(
                "a world routine the unit AI's side called reached the unit's AI",
            )
            .with("object", self.unit.id));
        }
        Ok(answer)
    }

    /// Slot 0x90 with 0 from `own` to master `next` (a task's switch, a villager's swap): the
    /// action list clears (the old actions' teardowns), then the master change at once, the
    /// unit lent back to its slot ([`World::change_master`]: the master's unit part, what it
    /// gives and takes of the owner's attributes and the tracker's promotion).
    ///
    /// # Errors
    ///
    /// What the teardowns and the switch do not model.
    pub(crate) fn switch_unit_master(
        &mut self,
        own: &crate::objects::masters::Master,
        next: i16,
    ) -> Result<(), GenieError> {
        let by = Releaser {
            id: self.unit.id,
            class: own.class,
            player: self.player,
        };
        let (mut releases, mut leaves) = (Vec::new(), Vec::new());
        self.unit
            .tear_down_actions(&mut releases, &mut leaves, by)?;
        self.torn_down((leaves, releases))?;
        self.lend(|w, p, i| w.change_master(p, i, next))
    }
}

impl UnitView<'_, '_> {
    /// Slot 0x178 on object `id`, which the unit's action of `by` takes
    /// ([`crate::world::lets::Lets::take`]); the unit itself, out of its slot, is taken where it
    /// is.
    pub(crate) fn take(&mut self, id: i32, by: Releaser) {
        let (p, unit, own) = (self.c.player, &mut *self.unit, &mut self.own_notifies);
        self.c
            .w
            .with_lets_for(p, own, |lets, others| lets.take(others, Some(unit), id, by));
    }

    /// Slot 0x180 on object `id`, which the unit's action of `by` lets go
    /// ([`crate::world::lets::Lets::release`]); the unit itself, out of its slot, is let go
    /// where it is, its AI told when the side's call returns.
    ///
    /// # Errors
    ///
    /// [`crate::world::lets::Lets::release`]'s.
    pub(crate) fn release(&mut self, id: i32, by: Releaser) -> Result<(), GenieError> {
        let (p, unit, own) = (self.c.player, &mut *self.unit, &mut self.own_notifies);
        self.c.w.with_lets_for(p, own, |lets, others| {
            lets.release(others, Some(unit), id, by)
        })
    }

    /// What the unit's action list's teardown left the world, at once
    /// ([`crate::world::lets::Lets::torn_down`]): the buildings its counted builders leave, then
    /// the targets it lets go.
    ///
    /// # Errors
    ///
    /// [`crate::world::lets::Lets::torn_down`]'s.
    pub(crate) fn torn_down(
        &mut self,
        (leaves, releases): (Vec<i32>, Vec<(i32, Releaser)>),
    ) -> Result<(), GenieError> {
        let (p, unit, own) = (self.c.player, &mut *self.unit, &mut self.own_notifies);
        self.c.w.with_lets_for(p, own, |lets, others| {
            lets.torn_down(others, Some(unit), (leaves, releases))
        })
    }
}
