//! A unit entering a building or a transport, and the transport's sail.

use crate::GenieError;
use crate::gathering::gather::unit::give_load;
use crate::map::spatial::Spatial;
use crate::math::rng::{Stream, Well512};
use crate::movement::Me;
use crate::movement::move_to::{MoveDefaults, MoveTo};
use crate::objects::masters::{Master, Masters};
use crate::objects::sprite::Graphic;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::objects::unit::object_master;
use crate::pathing::Pathing;
use crate::unit_ai::ActionKind;
use crate::unit_ai::task_finder::{Actor, DropSite};
use crate::world::others::OthersRef;
use crate::world::step::Step;

/// The units inside object `id`, any player's (its slot 0x498).
fn units_inside(others: OthersRef<'_>, id: i32) -> usize {
    others.every().filter(|u| u.life.inside == Some(id)).count()
}

impl Unit {
    /// The Enter's update ([`crate::garrison::EnterAction::update`]) with what it reads as it
    /// starts ([`Unit::enter_reads`]), on the Enter copied out of the action list and put back
    /// after it; its code (1, an Enter that got the unit in and the unit out again before this
    /// update, or a failed one's 3 delete it, its destructor letting its building go). Nothing
    /// without an Enter.
    ///
    /// # Errors
    ///
    /// The reads' and the update's.
    pub(crate) fn enter_part(&mut self, c: &mut Step<'_>, me: Me) -> Result<HeadCode, GenieError> {
        let Some(mut enter) = self.actions.enter().copied() else {
            return Ok(HeadCode::Keeps);
        };
        let reads = self.enter_reads(c, &enter)?;
        let updated = enter.update(c, self, me, &reads);
        if let Some(e) = self.actions.enter_mut() {
            *e = enter;
        }
        updated
    }

    /// What the Enter's update reads as it starts: its target, one of its own player's or an
    /// ally's (another player's is not modelled), with room for the unit (its count, the units
    /// of the player inside it, below its master's capacity) and the rest of
    /// [`crate::garrison::enter_view`]; the `MoveTo`'s give-up (a human player's villager waits,
    /// by its unit AI's stance) and the sprites its constructors take for none
    /// (`FUN_140aa0d30`, `FUN_140aa0df0`).
    ///
    /// # Errors
    ///
    /// The target's master's and [`crate::garrison::enter_view`]'s.
    fn enter_reads(
        &self,
        c: &Step<'_>,
        enter: &crate::garrison::EnterAction,
    ) -> Result<crate::garrison::EnterReads, GenieError> {
        let others = OthersRef::of(&c.w.players, c.player);
        let (target, start) = match others.find(enter.target) {
            Some((tp, t)) => {
                let master = t.master(&c.w.masters, tp)?;
                let room = units_inside(others, t.id) < usize::from(master.garrison);
                let (mut view, start) = crate::garrison::enter_view(
                    &c.w.spatial,
                    &c.w.masters,
                    self,
                    (t, master, room),
                    enter.range,
                )?;
                // FUN_141663dc0 with allies, or Gaia's
                view.allied = tp == 0
                    || crate::unit_ai::task_finder::allied(
                        &c.w.players[c.player].relations,
                        c.player,
                        tp,
                        true,
                    );
                (Some(view), start)
            }
            None => (
                None,
                crate::garrison::WalkStart {
                    gap: 0.0,
                    speed: 0.0,
                },
            ),
        };
        let master = self.master(&c.w.masters, c.player);
        let villager = master.as_ref().is_ok_and(|m| m.class == 4);
        Ok(crate::garrison::EnterReads {
            target,
            start,
            waits: villager && !c.w.players[c.player].computer,
            stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            defaults: master.map_or_else(
                |_| MoveDefaults::default(),
                |m| MoveDefaults::of(m, self.abilities.look.as_ref()),
            ),
        })
    }

    /// The Enter's enter in its update (`FUN_140a867b0` into `target`, [`Unit::enter_building`],
    /// then the update's own `set_state(1)`): the load the building took goes to the owner, the
    /// building gains the unit and its firepower at once, and the unit group it left is the
    /// caller's ([`Step::left_group`]).
    pub(crate) fn enter_in_update(
        &mut self,
        c: &mut Step<'_>,
        target: i32,
    ) -> Result<(), GenieError> {
        let (tp, building) =
            OthersRef::of(&c.w.players, c.player)
                .find(target)
                .ok_or(GenieError::unreachable(
                    "an Enter's enter without the target its update held",
                ))?;
        let inside = units_inside(OthersRef::of(&c.w.players, c.player), target);
        let mut adds = Vec::new();
        let mut w = EnterWorld {
            spatial: &mut c.w.spatial,
            pathing: &mut c.w.pathing,
            masters: c.w.masters.get(c.player),
            target_masters: c.w.masters.get(tp),
            graphics: &c.w.graphics,
            sprite_rng: c.w.streams.get(Stream::Sprite),
            drop_sites: &c.w.drop_sites,
            attributes: &c.w.players[c.player].attributes,
            civ: c.w.players[c.player].civ,
            sharing: c.w.players[c.player].sharing,
            player: c.player,
            attribute_adds: &mut adds,
            firepower: 0.0,
            left_group: None,
        };
        if !self.enter_building(building, inside, &mut w)? {
            return Err(GenieError::unsupported(
                "an Enter that did not get in (state 2)",
            ));
        }
        // the update's own set_state(1) after the enter's
        self.entered(&mut w)?;
        let firepower = w.firepower;
        c.left_group = w.left_group.map(|g| (g, target));
        // the load the building took, the owner's within the enter (nothing after it in the
        // enter reads the attributes)
        c.w.players[c.player].add_attributes(adds)?;
        // its building, any player's, gains the unit at once (the enter's slot 0x1c0)
        if let Some((_, b)) = c.w.object_mut(target) {
            b.firepower += firepower;
            b.garrison.push(self.id);
        }
        Ok(())
    }

    /// `FUN_140a867b0` on a unit inside its target already (the update's walk again put it in):
    /// notify 0x1fa with the enter action value and its `set_state(1)`, then the update's own
    /// `set_state(1)` ([`Unit::set_entered`] twice).
    ///
    /// # Errors
    ///
    /// The notify's and the `set_state(1)`'s.
    pub(crate) fn entered_again(&mut self, c: &mut Step<'_>) -> Result<(), GenieError> {
        self.post_own(
            crate::unit_ai::ai_kinds::NOTIFY_ACTION_COMPLETED,
            [ActionKind::Enter.value(), 0, 0],
        )?;
        for _ in 0..2 {
            self.set_entered(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )?;
        }
        Ok(())
    }

    /// The Transport's update ([`crate::garrison::transport::TransportAction::update`] on the
    /// step, the action copied out of its list and written back after): what it reads of the
    /// unit's master ([`crate::garrison::transport::TransportReads`]: the sprite its `MoveTo`'s
    /// start shows, `FUN_140aa0d30`'s default for none); its code, whose 1 or 3 deletes it.
    pub(crate) fn transport_update(
        &mut self,
        c: &mut Step<'_>,
        me: Me,
    ) -> Result<HeadCode, GenieError> {
        let mut t = *self
            .actions
            .transport()
            .ok_or(crate::garrison::transport::NO_TRANSPORT)?;
        let master = object_master(
            self.own_master.as_deref(),
            (self.id, self.life.master_id),
            c.w.masters.get(c.player),
        )?;
        let move_sprite = match t.move_to {
            Some(MoveTo::Start) => {
                let defaults = MoveDefaults::of(master, self.abilities.look.as_ref());
                t.move_shown(master, defaults)?
            }
            _ => None,
        };
        let reads = crate::garrison::transport::TransportReads {
            radius: master.radius,
            restriction: master.terrain_restriction,
            move_sprite,
            standing: c.w.graphics.get(&master.standing_sprite).copied(),
        };
        let code = t.update(c, self, me, &reads)?;
        if let Some(own) = self.actions.transport_mut() {
            *own = t;
        }
        Ok(HeadCode::of(code))
    }

    /// The Transport's slot 8 on a move to `point` ([`crate::garrison::transport::TransportAction::move_to`])
    /// with the map about the unit, then what its state change leaves: the unit stops (its
    /// `MoveTo`'s teardown) and takes its standing sprite.
    pub(crate) fn transport_sail(
        &mut self,
        point: [f32; 3],
        (spatial, zones, pathing): (&Spatial, &mut crate::pathing::zones::Zones, &mut Pathing),
        (masters, graphics, sprite_rng): (
            Option<&Masters>,
            &std::collections::BTreeMap<i16, Graphic>,
            &mut Well512,
        ),
    ) -> Result<(), GenieError> {
        let master = self
            .master_in(masters)
            .ok()
            .ok_or(crate::garrison::transport::NO_MASTER)?;
        let ship = crate::garrison::transport::Ship {
            position: self.position(),
            radius: master.radius,
            loaded: !self.garrison.is_empty(),
            map: spatial.map,
            moves: self.body.mover().is_some_and(|m| m.stats.speed != 0.0),
        };
        let restriction = master.terrain_restriction;
        let mut shore = crate::garrison::transport::MapShore::new(
            spatial,
            zones,
            pathing,
            restriction,
            self.position(),
        )?;
        let t = self
            .actions
            .transport_mut()
            .ok_or(GenieError::unreachable("a sail without its Transport"))?;
        let changed = t.move_to(point, &ship, &mut shore);
        if changed.stopped
            && let Some(m) = self.body.mover_mut()
        {
            m.stop()?;
        }
        if changed.stands {
            self.show_standing(masters, graphics, sprite_rng)?;
        }
        Ok(())
    }

    /// The unit's new Enter: its own action, or with `sub` the sub-action of its Artifact on top.
    pub(crate) fn place_enter(&mut self, action: crate::garrison::EnterAction, sub: bool) {
        match self.actions.artifact_mut().filter(|_| sub) {
            Some(art) => art.enter = Some(action),
            None => self.actions.set_enter(Some(action)),
        }
    }

    /// The Enter's `set_state(1)` (`FUN_140a87320` case 1): its sub-actions go (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, slot 0x5d0), the unit shows its master's standing
    /// sprite ([`Unit::shows_standing_sprite`]) and, the Enter not being quiet, its unit AI gets
    /// notify 0x1f9 with the enter action value.
    pub(crate) fn entered(&mut self, w: &mut EnterWorld<'_>) -> Result<(), GenieError> {
        self.set_entered(w.masters, w.graphics, w.sprite_rng)
    }

    /// [`Unit::entered`] with the owner's masters, the sprites' table and the Sprite stream its
    /// standing sprite reads.
    ///
    /// # Errors
    ///
    /// A unit without its Enter, and the stop's, the sprite's and the notify's.
    pub(crate) fn set_entered(
        &mut self,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite_rng: &mut Well512,
    ) -> Result<(), GenieError> {
        let enter = self
            .actions
            .enter_mut()
            .ok_or(GenieError::unsupported("an entered unit without its Enter"))?;
        if enter.move_to.take().is_some()
            && let Some(m) = self.body.mover_mut()
        {
            m.stop()?;
        }
        enter.state = crate::garrison::ENTERED;
        self.shows_standing_sprite(masters, graphics, sprite_rng)?;
        self.post_own(
            crate::unit_ai::ai_kinds::NOTIFY_ACTION_FAILED,
            [ActionKind::Enter.value(), 0, 0],
        )
    }

    /// The enter (`FUN_140a867b0`) into `target`, one of the unit's player's buildings: a
    /// building that takes what the unit carries (`FUN_140fdd190`, as a drop site) takes its load
    /// first (the resource for kinds 15 to 17 is food, 0 to 3 and 56 their own, others none:
    /// [`give_load`]; the unit carries nothing after, `FUN_14119d310` with −1). A building at no
    /// more than a fifth of its master's hit points (in floats) and a master with a transform
    /// task (0x88) on the building's are not modelled. Then slot 0x1a0 (`FUN_141212e10` ->
    /// `FUN_141084b30` -> `FUN_1411921d0`): with room in the building, the unit's sight goes from
    /// its tile (slot 0x1d8), it leaves its tile's list (`FUN_140f5f560`), the obstruction
    /// manager (`FUN_141055b40`) and the mesh (`+0x660`, `FUN_141053840`), a member of a unit
    /// group leaves it (`FUN_14102c160`, the caller's: [`EnterWorld::left_group`]) and stands
    /// (slot 0x560), and it joins the
    /// building's (slot 0x1c0, `FUN_1411cb850` -> `FUN_14118c890`: `+0x2f0`, at the building's
    /// x and y); its records leave the registry's lists (`FUN_14128a1d0` from its mask to 0);
    /// the building's sight takes a circle of 5 (slot 0x1d0 with 5) and the villager's town-bell
    /// flag follows the garrison (`FUN_1412067f0` with 3). Then notify 0x1fa with the enter
    /// action value and `set_state(1)` ([`Unit::entered`]). The building's firepower (`+0xa00`)
    /// and villager count (`+0xba8`) follow from the units inside; the sound, the tracker and
    /// a Gaia building's capture (slot 0x70) are not held.
    pub(crate) fn enter_building(
        &mut self,
        building: &Self,
        inside: usize,
        w: &mut EnterWorld<'_>,
    ) -> Result<bool, GenieError> {
        let masters = w.masters.ok_or(crate::garrison::NO_MASTERS)?;
        let bm = building.master_in(w.target_masters.or(Some(masters)))?;
        // a copy: going inside changes the unit, which may hold it
        let me = self.master_in(Some(masters))?.clone();
        let me = &me;
        let actor = Actor {
            id: self.id,
            master: me,
            player: w.player,
            held: (self.life.attribute_type, self.life.amount),
            attribute_95: w.attributes.get(95).copied().unwrap_or(0.0),
            civ: w.civ,
        };
        if crate::unit_ai::task_finder::drops_at(w.drop_sites, bm.id, &actor)
            && 0.0 < self.life.amount
            && bm.kind == crate::orders::BUILDING
        {
            let kind = self.life.attribute_type;
            let resource = match kind {
                15..=17 => Some(0),
                0..=3 | 56 => Some(kind),
                _ => None,
            };
            if let Some(resource) = resource {
                give_load(
                    (&w.spatial.relations, w.sharing),
                    w.player,
                    w.attribute_adds,
                    resource,
                    self.life.amount,
                )?;
                self.life.attribute_type = -1;
                self.life.amount = 0.0;
            }
        }
        if bm.kind == crate::orders::BUILDING
            && building.life.hp
                <= f32::from(bm.hit_points) * crate::math::image::garrison::ENTER_HP_SHARE
        {
            return Err(GenieError::unsupported(
                "an enter into a building at a fifth of its hit points (state 13)",
            ));
        }
        if me
            .tasks
            .iter()
            .find(|t| t.action_type == 0x88)
            .is_some_and(|t| t.object_id == bm.id)
        {
            return Err(GenieError::unsupported(
                "an enter of a master that transforms (task 0x88)",
            ));
        }
        if usize::from(bm.garrison) <= inside {
            return Ok(false);
        }
        self.go_inside(building, (masters, me, bm), w)?;
        self.post_own(
            crate::unit_ai::ai_kinds::NOTIFY_ACTION_COMPLETED,
            [ActionKind::Enter.value(), 0, 0],
        )?;
        self.entered(w)?;
        Ok(true)
    }

    /// The unit's slot 0x1a0 into `building` (`FUN_141212e10`; the room test, slot 0x198, is the
    /// caller's): its sight goes from its tile, it leaves its tile's list, the obstruction
    /// manager and the mesh, a member of a unit group leaves it and stands, it joins the
    /// building's, its records leave the registry's lists; the building's sight takes a circle
    /// of 5 and the villager's town-bell flag follows the garrison (`FUN_1412067f0` with 3).
    /// The building's `+0x9a8` (the units inside the unit), and a siege building's slot 0x1b0
    /// are not modelled.
    pub(crate) fn go_inside(
        &mut self,
        building: &Self,
        (masters, me, bm): (&Masters, &Master, &Master),
        w: &mut EnterWorld<'_>,
    ) -> Result<(), GenieError> {
        let target = building.id;
        if self.life.inside.is_some() {
            return Err(GenieError::unsupported("an enter of a unit inside another"));
        }
        // the building's `+0x9a8` takes the units inside the unit (slot 0x498)
        if !self.garrison.is_empty() {
            return Err(GenieError::unsupported(
                "an enter of a unit with units inside it",
            ));
        }
        let placed = w
            .spatial
            .get(self.id)
            .copied()
            .ok_or(GenieError::unsupported("an entering unit without a place"))?;
        if let Some(v) = w.spatial.sight.as_mut() {
            v.sight_off(self.id, self.position(), placed.radius, false, false);
        }
        let at = building.position();
        let id = self.id;
        self.body.base_mut().go_inside(id, at, w.spatial);
        w.pathing.removed(self.id);
        // a member of a unit group (+0x6d0) leaves the group the world holds (removeUnit with the
        // building and 0, FUN_14102c160: the caller's, `left_group`), then stands (slot 0x560)
        if self.group.id != -1 {
            w.left_group = Some(self.group.id);
            if let Some(m) = self.body.mover_mut() {
                m.stand()?;
            }
        }
        let building_radius = w.spatial.get(target).map_or(bm.radius, |p| p.radius);
        self.life.inside = Some(target);
        // the building's slot 0x1c0 (FUN_1411cb850 -> FUN_14120b790): a unit (type 70) adds its
        // share to the building's volleys
        if me.kind == 70 {
            let own = w.target_masters.unwrap_or(masters);
            let rate = crate::garrison::attack_rate(bm, own, building.reload_powerup);
            w.firepower = crate::garrison::share(me, masters, self.reload_powerup, rate)?;
        }
        if let Some(v) = w.spatial.sight.as_mut() {
            if let Some(r) = v.registry.as_mut().filter(|_| 0 <= self.id) {
                let mask = r.entry(self.id).map_or(0, |e| e.mask);
                r.update_unit(
                    self.id,
                    w.player as u8,
                    [self.position()[0], self.position()[1]],
                    placed.radius,
                    placed.class,
                    mask & 0xffff_ffff,
                    0,
                )?;
                // its mask (`+0x8f8`) is none after
                r.entries.entry(self.id).or_default().mask = 0;
            }
            // a building's sight takes a circle of 5 (FUN_141212e10, type 80 only)
            if bm.kind == crate::orders::BUILDING {
                let _ = v.sight_on_los(target, at, building_radius, false, 5, false);
            }
        }
        // FUN_1412067f0 with 3, for a villager or master 1808: the town-bell record of its work
        // (FUN_141206a20 with the flag clear: the Enter's target at its place, no target class,
        // the unit AI's action, no second target), then the flag set for a building in state 2
        // (its own player's) that takes villagers; with 3 it is never cleared
        if me.class == 4 || me.id == 1808 {
            if !self.town_bell.flag {
                let action =
                    self.ai
                        .as_ref()
                        .map(|ai| ai.current_action)
                        .ok_or(GenieError::unsupported(
                            "a town-bell record of a unit without a unit AI",
                        ))?;
                self.town_bell.record(crate::orders::town_bell::BackToWork {
                    target,
                    target_type: -1,
                    action,
                    target2: -1,
                    point: [at[0], at[1]],
                });
            }
            if bm.kind == crate::orders::BUILDING
                && building.life.state == 2
                && bm.building.is_some_and(|b| b.garrison_type & 1 != 0)
            {
                self.town_bell.flag = true;
            }
        }
        Ok(())
    }
}

/// What the enter into a building ([`Unit::enter_building`]) reads and changes of the world
/// besides the unit and the building.
pub(crate) struct EnterWorld<'a> {
    /// Every object's place, the sight and the registry.
    pub spatial: &'a mut Spatial,
    /// The mesh the unit's shape leaves.
    pub pathing: &'a mut Pathing,
    /// The owner's masters.
    pub masters: Option<&'a Masters>,
    /// The target's owner's masters, which its master is (the owner's when None).
    pub target_masters: Option<&'a Masters>,
    /// The graphics, for the standing sprite.
    pub graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The Sprite stream, which a new sprite may draw from.
    pub sprite_rng: &'a mut Well512,
    /// The drop-site table.
    pub drop_sites: &'a [DropSite],
    /// The owner's attributes and civ, as the drop-site test's actor.
    pub attributes: &'a [f32],
    /// The owner's civ.
    pub civ: u8,
    /// The owner shares its gathered resources (`+0x1426`).
    pub sharing: bool,
    /// The owner.
    pub player: usize,
    /// The attributes the owner gains (a load), which the caller adds.
    pub attribute_adds: &'a mut Vec<(i16, f32)>,
    /// What the building's volleys gain (`+0xa00`, [`crate::garrison::share`]), which the caller
    /// adds.
    pub firepower: f32,
    /// The unit group the unit was a member of (`+0x6d0`): the caller runs the group's removeUnit
    /// (`FUN_14102c160` with the building and 0, [`crate::world::World::remove_unit`]) when the
    /// world holds it (`FUN_14152ff40`).
    pub left_group: Option<i32>,
}

impl Unit {
    /// The inside update of an object inside another (`FUN_140a98790`), which runs in place of
    /// its actions' update (`RGE_Action_Object::update`): the head's idle update
    /// (`FUN_140a78480` -> `FUN_140a784a0`: a target or second target past state 6 let go, its
    /// sub-actions' idle update; the head's deletion right after lets them go alike), the idle
    /// update of each action below it, and the head's deletion ([`Unit::end_head_update`] with
    /// its 1). A unit its actions got in this update waits for the next. An empty list takes the
    /// default task instead: livestock inside makes its Artifact in one update and loses it in
    /// the next.
    pub(crate) fn inside_update(&mut self, c: &mut Step<'_>) -> Result<(), GenieError> {
        if !self.actions.is_empty() {
            return self.end_head_update(c, HeadCode::Deletes);
        }
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "the inside update of an action the slice does not hold",
            ));
        }
        Ok(())
    }
}
