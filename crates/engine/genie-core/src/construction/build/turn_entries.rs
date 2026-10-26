//! The construction's entries outside a builder's update ([`crate::turn::Turns`]): a
//! building's work and completion, its storage's leave, the built object's stacking and its
//! new objects.

use crate::construction::build::{
    BUILDING_TYPE, Built, CLASS_39, Completion, Construction, Site, StackUnit, WorkChain,
    show_built,
};
use crate::masters::Master;
use crate::order::Place;
use crate::unit_ai::GenieError;

impl crate::turn::Turns {
    /// What a built building's storage gave its owner leaves (`FUN_1411e63f0`, then
    /// `FUN_1411e6580`, [`crate::turn::storage_leaves`]) outside its update.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, its own store, and an attribute the owner lacks.
    pub(crate) fn storage_leaves(
        &mut self,
        owner: usize,
        id: i32,
        master: &Master,
    ) -> Result<(), GenieError> {
        let (_, u) = self.object_mut(id).ok_or(NOT_HELD)?;
        for (kind, amount) in crate::turn::storage_leaves(u, master) {
            self.players[owner].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// The completion's first part (`FUN_1411c6bd0`, [`Completion::first_part`]) of the
    /// foundation `id` of `owner` outside its update: its owner gains its attributes, a pasture
    /// fills ([`crate::turn::Turns::setup_pasture`]), then a mule cart's search runs from state 0
    /// ([`crate::turn::Turns::cart_search`]).
    ///
    /// # Errors
    ///
    /// What the part does not model.
    pub(crate) fn completion_part(
        &mut self,
        owner: usize,
        id: i32,
        master: &Master,
    ) -> Result<(), GenieError> {
        let (_, u) = self.object(id).ok_or(NOT_HELD)?;
        let construction = u.construction.ok_or(NO_CONSTRUCTION)?;
        let completion = Completion {
            master: Some(master),
            annexed: false,
            attribute_204: false,
            mode_680: self.mode_680,
            farm_bonus: crate::turn::farm_bonus(&self.players[owner].attributes),
            masters: self.masters.get(owner),
            computer: self.players[owner].computer,
            ai: u.ai.is_some(),
            main: u.annex_of.is_some(),
            now: self.world_time,
            rice: u.rice,
        };
        let part = completion.first_part(&construction)?;
        // its tracker part (FUN_14104eac0): the foundation leaves the pending lists
        self.track_complete(owner, id, master.id);
        for (kind, amount) in part.attributes {
            self.players[owner].add_attribute(kind, amount)?;
        }
        if part.pasture {
            self.setup_pasture(owner, id)?;
        }
        if part.cart {
            self.cart_search(owner, id, 0)?;
        }
        Ok(())
    }

    /// The construction's start (`FUN_1411e5670`, [`Construction::begin`]) of the foundation
    /// `id` of `owner` outside its update: a desolid one starts, its missiles per volley are its
    /// master's, and its owner watches it ([`crate::turn::Turns::watch_started`]).
    ///
    /// # Errors
    ///
    /// What the start does not model.
    pub(crate) fn construction_starts(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let place = self.place(id).ok_or(NOT_HELD)?;
        let (Place::Live(p, _) | Place::Sleeping(p, _)) = place;
        let master = held(&mut self.players, place).life.master_id;
        let volley = self
            .masters
            .get(p)
            .and_then(|m| m.get(master))
            .map_or(0.0, |m| m.volley_fire);
        let now = self.world_time;
        let u = held(&mut self.players, place);
        let Some(c) = u.construction.as_mut() else {
            return Ok(());
        };
        if Construction::begin(
            id,
            c,
            u.mover.as_mut(),
            &mut self.spatial,
            &mut self.pathing,
            now,
        )? {
            c.started = false;
            u.firepower = volley;
            self.watch_started(owner, id)?;
        }
        Ok(())
    }
}

/// The seconds of work a new object's building gets at once from its owner's slot 0x1c0 with its
/// flag (`FUN_14162ce00`'s `FUN_1411cc430` with `DAT_142eea788`).
pub(super) const STACK_WORK: f32 = crate::math::image::build::STACK_WORK;

/// The guard of an object the routines here reach through the object itself (their callers
/// hold it): the slice's lists hold every object the world does.
pub(crate) const NOT_HELD: GenieError =
    GenieError::unsupported("an object the slice does not hold");

/// The guard of a building without its construction: the slice gives every building one.
pub(crate) const NO_CONSTRUCTION: GenieError =
    GenieError::unsupported("a building without its construction");

/// The object at `place`, in its player's live or sleeping list (the world's table,
/// `FUN_1415417a0`, holds both).
pub(crate) fn held(players: &mut [crate::turn::Player], place: Place) -> &mut crate::turn::Unit {
    match place {
        Place::Live(p, i) => &mut players[p].objects[i],
        Place::Sleeping(p, i) => &mut players[p].sleeping[i],
    }
}

impl crate::turn::Turns {
    /// `FUN_1411cc430` outside a builder's update, `amount` seconds on the building `id`: first
    /// each annex (`+0xbc0`..`+0xbd8`) of a building type that is desolid starts
    /// (`FUN_1411cd0d0`, [`crate::turn::Turns::clear_foundations_under`], then `FUN_1411e5670`,
    /// [`crate::turn::Turns::construction_starts`]), then while the building is an annex of a
    /// main building in state 0 with at least 1 hit point (COMISS: a NaN too) the work goes to
    /// the main building, whose annexes start alike. A building in state 0 with at least 1 hit
    /// point (a NaN not) starts if desolid and takes the work ([`Construction::add_work`]); one
    /// that reaches its build time completes (`set_state(2)`, [`crate::turn::Turns::set_state`]).
    ///
    /// # Errors
    ///
    /// What the starts and the completion do not model.
    pub(crate) fn work_building(&mut self, id: i32, amount: f32) -> Result<(), GenieError> {
        let mut at = id;
        loop {
            for annex in self.spatial.annexes_of(at) {
                let Some((q, a)) = self.object(annex) else {
                    continue;
                };
                let desolid = a.construction.is_some_and(|c| c.desolid);
                if self.master_of(q, a.life.master_id)?.kind < 80 || !desolid {
                    continue;
                }
                self.clear_foundations_under(annex)?;
                self.construction_starts(q, annex)?;
            }
            let Some(main) = self.object(at).and_then(|(_, u)| u.annex_of) else {
                break;
            };
            match self.object(main) {
                Some((_, m))
                    if m.life.state == 0
                        && crate::math::fmath::at_most_or_unordered(1.0, m.life.hp) =>
                {
                    at = main;
                }
                _ => break,
            }
        }
        let (q, u) = self.object(at).ok_or(NOT_HELD)?;
        if u.life.state != 0 || crate::math::fmath::below_or_unordered(u.life.hp, 1.0) {
            return Ok(());
        }
        if u.construction.is_some_and(|c| c.desolid) {
            self.clear_foundations_under(at)?;
            self.construction_starts(q, at)?;
        }
        let (q, u) = self.object_mut(at).ok_or(NOT_HELD)?;
        let c = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
        if c.add_work(&mut u.life.hp, amount) {
            self.set_state(q, at, 2)?;
        }
        Ok(())
    }

    /// The building `id`'s `set_state(2)` from state 0 outside a builder's update, after its
    /// annexes' (`FUN_1411dbb70`; [`crate::turn::Turns::set_state`] runs them first): the
    /// completion ([`Construction::complete`]), then what it leaves its owner
    /// ([`crate::turn::Turns::built`]).
    ///
    /// # Errors
    ///
    /// What the completion does not model.
    pub(crate) fn complete_building(&mut self, id: i32) -> Result<(), GenieError> {
        let attribute_204 = self
            .players
            .iter()
            .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0));
        let place = self.place(id).ok_or(NOT_HELD)?;
        let (Place::Live(p, _) | Place::Sleeping(p, _)) = place;
        let farm_bonus = crate::turn::farm_bonus(&self.players[p].attributes);
        let computer = self.players[p].computer;
        let annexed = !self.spatial.annexes_of(id).is_empty();
        let (mode_680, now) = (self.mode_680, self.world_time);
        let masters = self.masters.get(p);
        let u = held(&mut self.players, place);
        let (ai, main, rice) = (u.ai.is_some(), u.annex_of.is_some(), u.rice);
        let master = masters.and_then(|m| m.get(u.life.master_id));
        let construction = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
        let mut site = Site {
            id,
            life: &mut u.life,
            construction,
            mover: u.mover.as_mut(),
            firepower: Some(&mut u.firepower),
            roo_called: Some(&mut u.roo_called),
            store: Some(&mut u.store),
            completion: Completion {
                master,
                annexed,
                attribute_204,
                mode_680,
                farm_bonus,
                masters,
                computer,
                ai,
                main,
                now,
                rice,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let built = Construction::complete(&mut site, &mut self.spatial, &mut self.pathing)?;
        let u = held(&mut self.players, place);
        if let Some(m) = self.masters.get(p).and_then(|m| m.get(u.life.master_id)) {
            show_built(
                &mut u.sprite,
                m,
                u.build_frame,
                &self.graphics,
                self.streams.get(crate::math::rng::Stream::Sprite),
            );
        }
        // the completion's first part's tracker part (FUN_1411c6bd0 -> FUN_14104eac0)
        let master_id = held(&mut self.players, place).life.master_id;
        self.track_complete(p, id, master_id);
        self.built(p, id, built)
    }

    /// What a completion leaves the owner `p` of the building `id` ([`Built`]), in the game's
    /// order: its history event (the owner's id, `+0x718`, as `data1`; the game records it
    /// between the storage's attributes and the counts, which it does not read), the attributes
    /// it gains, a pasture's livestock ([`crate::turn::Turns::setup_pasture`]), a mule cart's
    /// search (`FUN_1411c6bd0`'s tail), the watch of
    /// a construction that started (`FUN_1411e5670`), the stack unit
    /// ([`crate::turn::Turns::stack_unit`]), the technology (`FUN_1413cdcd0`,
    /// [`crate::turn::Turns::research_built`]) and the destroy of a building that disappears
    /// when built (slot 0xd0, [`crate::turn::Turns::destroy`]).
    ///
    /// # Errors
    ///
    /// What the parts do not model.
    pub(crate) fn built(&mut self, p: usize, id: i32, built: Built) -> Result<(), GenieError> {
        if let Some(kind) = built.history {
            let now = self.world_time;
            self.players[p]
                .history
                .add_event(kind, p as f32, 0.0, 0, now);
        }
        for (kind, amount) in built.attributes {
            self.players[p].add_attribute(kind, amount)?;
        }
        if built.pasture {
            self.setup_pasture(p, id)?;
        }
        if let Some(state) = built.cart {
            self.cart_search(p, id, state)?;
        }
        if built.started {
            if let Some((_, u)) = self.object_mut(id)
                && let Some(c) = u.construction.as_mut()
            {
                c.started = false;
            }
            self.watch_started(p, id)?;
        }
        if let Some(stack) = built.stack {
            self.stack_unit(p, id, stack)?;
        }
        if let Some(t) = built.tech {
            self.research_built(p, t)?;
        }
        if built.vanishes {
            self.destroy(p, id)?;
        }
        Ok(())
    }

    /// The stack unit (`+0x986`) of the completed building `building` of player `p`
    /// (`FUN_1411dbb70`'s block): its owner's slot 0x1c0 makes it at the building's place with
    /// its flag ([`crate::turn::Turns::make_new_object`]); a new building takes the old one's
    /// command queue (the old unit AI's slot 0x218, its orders not empty, then slot 0x1f8: the
    /// new one's slot 0x208 per order, `FUN_140c15d00`) and its gather point (`+0xc30`..: a
    /// target that is the old building names the new one; a target the world no longer holds
    /// keeps the point alone, and with x or y −1 none); then the new object loses the hit points
    /// the old one lacked. Not modelled: the attacking units of the foundation the world still
    /// holds (`+0xce0`, [`crate::turn::Unit::attacking_units`]: slot 0x140 on each with the new
    /// object, a repeat of the last skipped). A gate's stack gate takes the old one's lock (slot
    /// 0x670 with `+0xc7c`, [`crate::order::lock_gate`]). The old building keeps the new
    /// object's id (`+0xcb0`, [`crate::turn::Unit::stack_made`]). Not held: the local player's
    /// tracker (`FUN_14165c2a0`, `FUN_14165c470`).
    ///
    /// # Errors
    ///
    /// What the parts do not model.
    pub(crate) fn stack_unit(
        &mut self,
        p: usize,
        building: i32,
        stack: StackUnit,
    ) -> Result<(), GenieError> {
        let (_, old) = self.object(building).ok_or(NOT_HELD)?;
        let (orders, gather) = (
            old.ai
                .as_ref()
                .map(|ai| ai.building_orders.clone())
                .unwrap_or_default(),
            old.gather_point,
        );
        let old_class = self.master_of(p, old.life.master_id)?.class;
        let old_lock = old.gate.map(|g| g.locked);
        // its attacking units (+0xce0) the world still holds each hear of the new object (their
        // slot 0x140), which is not modelled
        let enemies = old
            .attacking_units
            .iter()
            .any(|&id| self.object(id).is_some());
        if enemies {
            return Err(GenieError::unsupported(
                "a stack unit's completion with other players' units on it (+0xce0)",
            ));
        }
        let Some(new) = self.make_new_object(p, stack.master, stack.at, true)? else {
            return Ok(());
        };
        // the old building keeps the new object's id (`+0xcb0`, FUN_1411c5f70)
        if let Some((_, b)) = self.object_mut(building) {
            b.stack_made = Some(new);
        }
        let (q, nu) = self.object(new).ok_or(NOT_HELD)?;
        let new_master = self.master_of(q, nu.life.master_id)?;
        let mut point = None;
        let building_made = new_master.kind == BUILDING_TYPE;
        let gate_lock = (building_made && old_class == CLASS_39 && new_master.class == CLASS_39)
            .then_some(old_lock);
        if building_made {
            // a gather point on the old building names the new one; one whose target the world
            // no longer holds keeps its point, none at x or y −1 (compared as floats)
            point = gather.map(|g| {
                let target = if g.target == building { new } else { g.target };
                match self.object(target) {
                    Some((_, t)) => Some(crate::turn::GatherPoint {
                        point: [t.position[0], t.position[1], 0.0],
                        target,
                        target_master: t.life.master_id,
                    }),
                    None => (g.point[0] != -1.0 && g.point[1] != -1.0).then_some(
                        crate::turn::GatherPoint {
                            point: [g.point[0], g.point[1], 0.0],
                            target: -1,
                            target_master: -1,
                        },
                    ),
                }
            });
        }
        let now = self.world_time;
        let (_, nu) = self.object_mut(new).ok_or(NOT_HELD)?;
        if building_made {
            if let Some(ai) = nu.ai.as_mut().filter(|_| !orders.is_empty()) {
                ai.building_orders.extend(orders);
            }
            if let Some(g) = point {
                nu.gather_point = g;
            }
        }
        // a gate's stack gate takes the old one's lock (its slot 0x670 with `+0xc7c`)
        if let Some(lock) = gate_lock {
            let locked = lock.ok_or(GenieError::unreachable(
                "a building without its gate state (every building holds one)",
            ))?;
            crate::order::lock_gate(nu, locked, now)?;
        }
        nu.life.hp -= stack.hp_lost;
        // the walks of this update read the last move time the lock set
        if let Some(t) = nu.mover.as_ref().map(|m| m.last_move) {
            self.spatial.set_last_move(new, t);
        }
        Ok(())
    }

    /// The player's slot 0x1c0 (`FUN_14162ce00`) for `master` at `at`: the master's slot 0x40
    /// makes the object (none for a master the player has not got, `FUN_141666a60`); with its
    /// flag its footprint is cleared with `param_6` ([`crate::turn::Turns::clear_footprint`]) and
    /// a building is worked 10000 seconds at once ([`crate::turn::Turns::work_building`]: a stack
    /// unit completes there); a master that gathers by itself joins the visible-resource lists
    /// ([`crate::turn::Turns::made_new_object`]). The editor's part (mode 4) never runs in a game.
    /// Its id.
    ///
    /// # Errors
    ///
    /// What the parts do not model ([`crate::turn::Turns::combat_object`],
    /// [`crate::turn::Turns::plain_object`], [`crate::turn::Turns::make_building`]).
    pub(crate) fn make_new_object(
        &mut self,
        p: usize,
        master: i16,
        at: [f32; 3],
        flagged: bool,
    ) -> Result<Option<i32>, GenieError> {
        let Some(made) = self.masters.get(p).and_then(|m| m.get(master)).cloned() else {
            return Ok(None);
        };
        // a combat unit (type 70, an effect's spawn): the master's slot 0x40 there, its footprint
        // cleared with the flag, its visible resources for one that gathers by itself
        if made.kind == crate::garrison::COMBAT_KIND {
            let idx = self.combat_object(p, master, &made, at)?;
            let id = self.players[p].objects[idx].id;
            if flagged {
                self.clear_footprint(p, master, [at[0], at[1]], true)?;
            }
            if made.auto_gather {
                self.made_new_object(p)?;
            }
            return Ok(Some(id));
        }
        // a plain object (types 10 and 20 to 39: a pasture's livestock): the master's slot 0x40
        // there, facing its drawn angle
        if made.kind != BUILDING_TYPE {
            let idx = self.plain_object(p, master, &made, at, crate::spawn::Facing::Drawn)?;
            let id = self.players[p].objects[idx].id;
            if flagged {
                self.clear_footprint(p, master, [at[0], at[1]], true)?;
            }
            if made.auto_gather {
                self.made_new_object(p)?;
            }
            return Ok(Some(id));
        }
        let ai = Ok(crate::unit_ai::factory_type(
            made.class,
            master,
            made.volley_missile.unwrap_or(-1),
            made.combat_level,
        ));
        let (id, _) = self.make_building(p, master, ai, at)?;
        if flagged {
            self.clear_footprint(p, master, [at[0], at[1]], true)?;
            self.work_building(id, STACK_WORK)?;
        }
        if made.auto_gather {
            if self.players[p].objects.last().map(|u| u.id) != Some(id) {
                return Err(GenieError::unsupported(
                    "an auto-gathered new building with annexes (its visible resources)",
                ));
            }
            self.made_new_object(p)?;
        }
        Ok(Some(id))
    }
}
