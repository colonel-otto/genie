//! A unit entering a building or a transport, and the transport's sail.

use crate::GenieError;
use crate::gathering::gather::unit::give_load;
use crate::map::spatial::Spatial;
use crate::math::rng::{Stream, Well512};
use crate::movement::Me;
use crate::movement::move_to::MoveTo;
use crate::objects::masters::{Master, Masters};
use crate::objects::sprite::Graphic;
use crate::objects::unit::Unit;
use crate::pathing::Pathing;
use crate::unit_ai::UnitAi;
use crate::unit_ai::task_finder::{Actor, DropSite};
use crate::world::context::Context;

impl Unit {
    /// The Enter's update ([`crate::garrison::EnterAction::update`]) with what it reads of its
    /// target, one of its own player's (another player's is not modelled): room for the unit (its
    /// count, the units of the player inside it, plus 1 within its master's capacity) and the
    /// rest of [`crate::garrison::enter_view`]; a transport's reach test is
    /// [`crate::garrison::ship_reaches`]. Then its step: the enter (`FUN_140a867b0`) and the
    /// update's `set_state(1)`, with an enter again ([`crate::garrison::EnterStep::again`]); a
    /// wait's standing sprite; the transport's move, the caller's after the update
    /// ([`Context::transport_moves`]).
    pub(crate) fn enter_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
        let enter = self.enter.expect("checked by the caller");
        let found = others
            .iter()
            .find(|u| u.id == enter.target)
            .map(|t| (c.player, t))
            .or_else(|| c.foreign.find(enter.target));
        // the units inside the target, any player's (its slot 0x498)
        let foreign = c.foreign;
        let inside_of = |others: &[Self], id: i32| {
            let inside = |u: &&Self| u.life.inside == Some(id);
            let theirs: usize = (0..foreign.before.len() + 1 + foreign.after.len())
                .filter_map(|q| foreign.player(q))
                .map(|p| p.objects.iter().chain(&p.sleeping).filter(inside).count())
                .sum();
            others.iter().filter(inside).count() + theirs
        };
        let (building, walking, meeting) = match found {
            Some((tp, t)) => {
                let master = c
                    .every_masters
                    .get(tp)
                    .and_then(|m| m.get(t.life.master_id))
                    .ok_or(GenieError::unsupported(
                        "an Enter's building of a master the slice lacks",
                    ))?;
                let room = inside_of(others, t.id) < usize::from(master.garrison);
                let (mut view, walking, meeting) = crate::garrison::enter_view(
                    c.spatial,
                    c.zones,
                    c.every_masters,
                    self,
                    (t, master, room),
                    enter.range,
                )?;
                // FUN_141663dc0 with allies, or Gaia's
                view.allied =
                    tp == 0 || crate::unit_ai::task_finder::allied(c.relations, c.player, tp, true);
                (Some(view), walking, meeting)
            }
            None => (None, (0.0, 0.0), Ok(([0.0; 2], 0))),
        };
        let ship = found.map(|(_, t)| t);
        // the MoveTo's give-up waits for a human player's villager (by its unit AI's stance)
        let villager = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .is_some_and(|m| m.class == 4);
        let gives_up = (
            villager && !c.computer,
            self.ai.as_ref().map_or(0, |ai| ai.stance),
        );
        let walk = c.walk(self.body.mover_mut(), me);
        let step = self.enter.as_mut().expect("checked above").update(
            walk,
            building,
            walking,
            || meeting.clone(),
            |w, to| {
                let (Some(ship), Some(plan)) = (ship, w.plan.as_mut()) else {
                    return Ok(false);
                };
                crate::garrison::ship_reaches(ship, w.world, plan, to)
            },
            gives_up,
        )?;
        if let Some(to) = step.order {
            c.transport_moves.push((enter.target, to));
        }
        if step.waits {
            self.shows_standing_sprite(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        if step.enter {
            let (tp, building) = others
                .iter()
                .find(|u| u.id == enter.target)
                .map(|t| (c.player, t))
                .or_else(|| c.foreign.find(enter.target))
                .expect("the update found it");
            let inside = inside_of(others, enter.target);
            let mut w = EnterWorld {
                spatial: c.spatial,
                pathing: c.pathing,
                masters: c.masters,
                target_masters: c.every_masters.get(tp),
                graphics: c.graphics,
                sprite_rng: c.streams.get(Stream::Sprite),
                drop_sites: c.drop_sites,
                attributes: c.attributes,
                civ: c.civ,
                sharing: c.sharing,
                player: c.player,
                attribute_adds: &mut c.attribute_adds,
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
            if step.again {
                // FUN_140a867b0 on the unit inside: notify 0x1fa and its set_state(1), then the
                // update's
                self.post_own(
                    crate::unit_ai::ai_kinds::NOTIFY_ACTION_COMPLETED,
                    [crate::garrison::ENTER_ACTION, 0, 0],
                )?;
                self.entered(&mut w)?;
                self.entered(&mut w)?;
            }
            let firepower = w.firepower;
            c.left_group = w.left_group.map(|g| (g, enter.target));
            if let Some(b) = others.iter_mut().find(|u| u.id == enter.target) {
                b.firepower += firepower;
                b.garrison.push(self.id);
            } else {
                // another player's: the caller's after the update
                c.entered = Some((enter.target, firepower));
            }
        }
        if step.done {
            // its 1 (an Enter that got the unit in, the unit out again before this update) or a
            // failed one's 3: the action list removes it, its teardown letting its building go
            // (slot 0x180)
            if let Some(e) = self.enter.take() {
                c.releases.push((e.target, self.releaser(c)));
            }
        }
        Ok(())
    }

    /// The Transport's update ([`crate::garrison::transport::TransportAction::update`]): its `MoveTo` first
    /// ([`crate::garrison::transport::TransportAction::walk`], by what the unit's AI asks of the path in
    /// its state 3, `0x140aa2a70`), then its state with the map about the unit where the walk
    /// left it ([`crate::garrison::transport::MapShore`]). Then its step: the unit stops and takes its
    /// standing sprite ([`Unit::show_standing`]), a failure notifies its unit AI (0x1f9 with
    /// the transport's action value), the owner hears that it cannot unload
    /// ([`crate::unit_ai::alert_owner`]), the unload is the caller's before the unit AI part
    /// ([`Context::transport_unload`]), and a 1 or 3 removes it from the list (its teardown: no
    /// `MoveTo` is left, no target).
    pub(crate) fn transport_update(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
    ) -> Result<(), GenieError> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::garrison::transport::NO_MASTER)?;
        let (radius, restriction) = (master.radius, master.terrain_restriction);
        let t = self.transport.as_mut().expect("checked by the caller");
        let ask = match t.move_to {
            Some(MoveTo::Start) => {
                let (at, to) = ([me.position[0], me.position[1]], [t.point[0], t.point[1]]);
                UnitAi::path_ask(self.ai.as_mut(), at, to)
            }
            _ => crate::movement::PathAsk::default(),
        };
        let walked = t.walk(Some(c.walk(self.body.mover_mut(), me)), ask)?;
        let position = c.spatial.get(self.id).map_or(self.position, |p| p.position);
        let ship = crate::garrison::transport::Ship {
            position,
            radius,
            loaded: !self.garrison.is_empty(),
            map: c.spatial.map,
            moves: self.body.mover().is_some_and(|m| m.stats.speed != 0.0),
        };
        let mut shore = crate::garrison::transport::MapShore::new(
            c.spatial,
            c.zones,
            c.pathing,
            restriction,
            position,
        )?;
        let step = t.update(walked, &ship, &mut shore)?;
        if step.changed.stopped
            && let Some(m) = self.body.mover_mut()
        {
            m.stop()?;
        }
        if step.changed.stands {
            self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        if step.failed {
            self.post_own(
                crate::unit_ai::ACTION_FAILED,
                [crate::garrison::transport::TRANSPORT_ACTION, 0, 0],
            )?;
        }
        if step.cannot_unload {
            crate::unit_ai::alert_owner(c.computer)?;
        }
        if matches!(step.code, 1 | 3) {
            self.transport = None;
        }
        c.transport_unload = step.unload;
        Ok(())
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
        let master = masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::garrison::transport::NO_MASTER)?;
        let ship = crate::garrison::transport::Ship {
            position: self.position,
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
            self.position,
        )?;
        let t = self
            .transport
            .as_mut()
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
        match self.artifact.as_mut().filter(|_| sub) {
            Some(art) => art.enter = Some(action),
            None => self.enter = Some(action),
        }
    }

    /// The Enter's `set_state(1)` (`FUN_140a87320` case 1): its sub-actions go (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, slot 0x5d0), the unit shows its master's standing
    /// sprite ([`Unit::shows_standing_sprite`]) and, the Enter not being quiet, its unit AI gets
    /// notify 0x1f9 with the enter action value.
    pub(crate) fn entered(&mut self, w: &mut EnterWorld<'_>) -> Result<(), GenieError> {
        let enter = self
            .enter
            .as_mut()
            .ok_or(GenieError::unsupported("an entered unit without its Enter"))?;
        if enter.move_to.take().is_some()
            && let Some(m) = self.body.mover_mut()
        {
            m.stop()?;
        }
        enter.state = crate::garrison::ENTERED;
        self.shows_standing_sprite(w.masters, w.graphics, w.sprite_rng)?;
        self.post_own(
            crate::unit_ai::ai_kinds::NOTIFY_ACTION_FAILED,
            [crate::garrison::ENTER_ACTION, 0, 0],
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
        let bm = w
            .target_masters
            .unwrap_or(masters)
            .get(building.life.master_id)
            .ok_or(GenieError::unsupported(
                "an enter into a building of a master the slice lacks",
            ))?;
        let me = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "an entering unit of a master the slice lacks",
            ))?;
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
            [crate::garrison::ENTER_ACTION, 0, 0],
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
            v.sight_off(self.id, self.position, placed.radius, false, false);
        }
        let at = building.position;
        w.spatial.go_inside(self.id, at);
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
        self.position = [at[0], at[1], self.position[2]];
        if let Some(v) = w.spatial.sight.as_mut() {
            if let Some(r) = v.registry.as_mut().filter(|_| 0 <= self.id) {
                let mask = r.entry(self.id).map_or(0, |e| e.mask);
                r.update_unit(
                    self.id,
                    w.player as u8,
                    [self.position[0], self.position[1]],
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
    /// The garrison's part of the update: the inside update of a unit that started the update
    /// inside another (`0x140a98790`), else its Enter's or its transport's update. A unit that
    /// started a default task in this update (`default_made`) takes the other branch.
    pub(crate) fn garrison_part(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
        inside_at_start: bool,
        default_made: bool,
    ) -> Result<(), GenieError> {
        // RGE_Action_Object::update: the actions of an object inside another do not update; its
        // inside update (FUN_140a98790) deletes its top action (FUN_140a78480 -> FUN_140a784a0:
        // a dead target let go, the sub-actions' end; then the action's teardown, which lets its
        // targets go, slot 0x180, and a MoveTo's stops the unit, slot 0x5d0); the actions below
        // it (FUN_140a78470) and the next one's slot 0x28 need a second action, which the slice
        // never holds; a unit its actions got in this update waits for the next. An empty list
        // takes the default task instead (its branch of the three): livestock inside makes its
        // Artifact in one update and loses it in the next
        if inside_at_start && !default_made {
            if let Some(e) = self.enter.take() {
                if e.move_to.is_some()
                    && let Some(m) = self.body.mover_mut()
                {
                    m.stop()?;
                }
                c.releases.push((e.target, self.releaser(c)));
            } else if self.top_action_kind() == Some(crate::ownership::artifact::ARTIFACT_TYPE) {
                // the Artifact goes (FUN_140a73fc0): its target is let go, then its sub-actions
                // go (an Enter's teardown lets its building go; a walking MoveTo's stops the
                // unit)
                let mut art = self.artifact.take().expect("its top action");
                let by = self.releaser(c);
                let (released, gone) = art.tear_down();
                for t in [released, gone.released].into_iter().flatten() {
                    c.releases.push((t, by));
                }
                if gone.stops
                    && let Some(m) = self.body.mover_mut()
                {
                    m.stop()?;
                }
            } else if self.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "the inside update of an action the slice does not hold",
                ));
            } else if self.top_action_kind().is_some() {
                // any other action's destructor, as the list's deletion runs it
                let by = self.releaser(c);
                self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
            }
        } else if self.enter.is_some() {
            self.enter_update(c, others, me)?;
        } else if self.transport.is_some() {
            self.transport_update(c, me)?;
        }
        Ok(())
    }
}
