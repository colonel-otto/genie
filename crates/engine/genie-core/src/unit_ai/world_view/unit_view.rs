//! What the unit AI's view asks of its unit: its picker, a helper's tests and a master switch.

use crate::GenieError;
use crate::movement::footprint_gap_squared;
use crate::objects::release::Releaser;
use crate::objects::unit::Unit;
use crate::unit_ai::world_view::owner_view;
use crate::unit_ai::world_view::{NOT_HELD, UnitView, WorldPicker};

impl UnitView<'_, '_> {
    /// The world as `bestUnitToAttack` and the attack tests read it for the unit
    /// ([`WorldPicker`]), with `seeker` and without a terrain restriction.
    pub(super) fn picker(&self, seeker: Option<crate::unit_ai::Seeker>) -> WorldPicker<'_> {
        WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: owner_view(self.others, self.new_owner),
            foreign: owner_view(self.others, self.new_owner).players(),
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
        let others = owner_view(self.others, self.new_owner);
        let find = |id: i32| others.find(id);
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
            owner_view(self.others, self.new_owner)
                .players()
                .player(owner)
                .is_some_and(|q| q.computer)
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
}

impl UnitView<'_, '_> {
    /// Slot 0x90 with 0 from `own` to `next` (a task's switch, a villager's swap): the action list
    /// clears (the old actions' teardowns), then the master's unit part
    /// ([`crate::orders::switch_master`]); what it gives and takes of the owner's attributes and
    /// its promotion are the owner's part after the update.
    ///
    /// # Errors
    ///
    /// What the teardowns and the switch do not model.
    pub(crate) fn switch_unit_master(
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
        let (mut releases, mut leaves) = (Vec::new(), Vec::new());
        unit.tear_down_actions(&mut releases, &mut leaves, by)?;
        self.lets.torn_down(self.others, (leaves, releases))?;
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
}
