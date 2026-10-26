//! A foundation's construction ([`Construction`]): its work, its start and its completion,
//! with the completion's storage, ground and counts.

use crate::construction::build::{
    Built, CASTLE_WONDER_COST, CASTLES_BUILT, Construction, MASTER_1889, MainWork, RICE_PASTURE,
    SITE_GONE, STARTED, Site, StackUnit, WONDER, WONDERS_BUILT, Worked,
};
use crate::map::spatial::Spatial;
use crate::masters::Master;
use crate::pathing::Pathing;
use crate::turn::{CASTLE, CASTLES, MONASTERIES, MONASTERY, WONDERS};
use crate::unit_ai::GenieError;

/// [`Construction::complete`]'s storage: its master's entries of modes 4, 8 and 64 go to its
/// owner (slot 0xf8, into `attributes`), those of mode 16 to its own `store` (`FUN_1411abad0`: an
/// entry from 0); an entry of no resource or no amount gives nothing.
pub(super) fn hand_storage(
    master: &Master,
    attributes: &mut Vec<(i16, f32)>,
    mut store: Option<&mut Vec<(i16, f32)>>,
) {
    for &(kind, amount, flag) in &master.storage {
        if kind < 0 || amount <= 0.0 {
            continue;
        }
        if matches!(flag, 4 | 8 | 64) {
            attributes.push((kind, amount));
        }
    }
    // the entries of mode 16 go to its own store (FUN_1411abad0: an entry from 0)
    for &(kind, amount, flag) in &master.storage {
        if kind < 0 || amount <= 0.0 || flag & 0x10 == 0 {
            continue;
        }
        if let Some(store) = store.as_deref_mut() {
            match store.iter_mut().find(|e| e.0 == kind) {
                Some(e) => e.1 += amount,
                None => store.push((kind, amount)),
            }
        }
    }
}

/// [`Construction::complete`]'s ground: on snow the building gets its snow flag; its foundation
/// terrain covers its clearance (terrain 0x3f under master 1889's pasture on mangrove or rice,
/// `stack_master` [`RICE_PASTURE`]; [`crate::map::terrain::Terrain::cover`]).
///
/// # Errors
///
/// A map without its terrain, and a tile the terrain does not hold.
pub(super) fn lay_foundation(
    c: &mut Construction,
    world: &mut Spatial,
    placed: &crate::map::spatial::Placed,
    master: &Master,
    building: &crate::masters::BuildingMaster,
    stack_master: i16,
    mode_680: i32,
) -> Result<(), GenieError> {
    /// The terrain the pasture master 1889's stack unit on mangrove or rice terrain lays
    /// (`0x3f`).
    const RICE_PASTURE_TERRAIN: u8 = 0x3f;
    let terrain = world.terrain.as_mut().ok_or(GenieError::unsupported(
        "a completion without the map's terrain",
    ))?;
    let [x, y, _] = placed.position;
    if !c.snow && 0 <= x as i32 && 0 <= y as i32 && terrain.snow_under(x as i32, y as i32)? {
        c.snow = true;
    }
    if 0 <= building.foundation_terrain {
        let laid = if stack_master == RICE_PASTURE {
            RICE_PASTURE_TERRAIN
        } else {
            building.foundation_terrain as u8
        };
        terrain.cover(&crate::map::terrain::Foundation {
            position: placed.position,
            clearance: master.clearance,
            terrain: laid,
            snow: c.snow,
            master: master.id,
            class: master.class,
            terrain_bonus: master.terrain_bonus,
            mode_680,
        })?;
    }
    Ok(())
}

/// [`Construction::complete`]'s counts into `attributes`: the castle and the wonder count in
/// attributes 134 and 173, 42 and 174 and add their paid cost to 184; the monastery counts in
/// 52; any other building's paid cost grows 98 and 165 unless it is an annex (`main`) or has a
/// stack unit. The paid cost sums the cost entries of a resource flagged paid (`+0x880`) as
/// shorts. The castle's and the wonder's history event (0x1411dd094), returned.
pub(super) fn completion_counts(
    master: &Master,
    building: &crate::masters::BuildingMaster,
    main: bool,
    attributes: &mut Vec<(i16, f32)>,
) -> Option<u8> {
    // the castle's and the wonder's history event (0x1411dd094), the counts (slot 0xf8 with
    // 1.0, 0x14330eb8c)
    let history = match master.id {
        CASTLE => Some(crate::players::history::CASTLE_BUILT),
        WONDER => Some(crate::players::history::WONDER_BUILT),
        _ => None,
    };
    match master.id {
        MONASTERY => attributes.push((MONASTERIES, 1.0)),
        WONDER => attributes.extend([(WONDERS, 1.0), (WONDERS_BUILT, 1.0)]),
        CASTLE => attributes.extend([(CASTLES, 1.0), (CASTLES_BUILT, 1.0)]),
        _ => {}
    }
    // its paid cost: the cost entries of a resource flagged paid (`+0x880`), summed as shorts
    let paid: i32 = master
        .cost
        .iter()
        .filter(|&&(resource, _, paid)| 0 <= resource && paid)
        .map(|&(_, amount, _)| i32::from(amount))
        .sum();
    if matches!(master.id, CASTLE | WONDER) {
        attributes.push((CASTLE_WONDER_COST, paid as f32));
    } else if !main && building.stack_unit == -1 {
        attributes.push((98, paid as f32));
        attributes.push((165, paid as f32));
    }
    history
}

impl Construction {
    /// `FUN_1411cc430`: `amount` seconds of work on a foundation. An annex whose main building
    /// takes the work leaves it all to the caller ([`WorkChain::main`], [`Worked::Main`]: the
    /// main building's result, its start's build points first). Otherwise its annexes of a
    /// building type that are desolid start first ([`AnnexStart`]); then a foundation in state 0
    /// with at least 1 hit point (else the work is done) starts with the first work
    /// ([`Construction::begin`]), each adds its share of the master's hit points (all of them
    /// without a build time) up to the build time, and the work that reaches it completes the
    /// building ([`Construction::complete`]).
    ///
    /// [`AnnexStart`]: crate::construction::build::AnnexStart
    /// [`WorkChain::main`]: crate::construction::build::WorkChain::main
    pub fn work(
        site: &mut Site<'_>,
        amount: f32,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<Worked, GenieError> {
        if let Some(main) = site.chain.main {
            let MainWork {
                mut construction,
                mut hp,
            } = main;
            if construction.desolid {
                construction.progress = f32::from_bits(STARTED);
            }
            let going = !construction.add_work(&mut hp, amount);
            return Ok(Worked::Main { amount, going });
        }
        for a in &mut site.chain.annexes {
            if a.construction.desolid {
                world.started(a.id);
                if Self::begin(
                    a.id,
                    a.construction,
                    a.mover.as_deref_mut(),
                    world,
                    pathing,
                    now,
                )? {
                    *a.firepower = a.volley;
                }
            }
        }
        // COMISS: a NaN hit point count is not worked on either
        if site.life.state != 0 || crate::math::fmath::below_or_unordered(site.life.hp, 1.0) {
            return Ok(Worked::Done);
        }
        let c = &mut *site.construction;
        if c.desolid {
            // FUN_1411cd0d0: the desolid foundations it overlaps go after the builder's update
            // (Turns::clear_foundations_under), then FUN_1411e5670
            world.started(site.id);
            if Self::begin(site.id, c, site.mover.as_deref_mut(), world, pathing, now)?
                && let (Some(f), Some(m)) = (site.firepower.as_deref_mut(), site.completion.master)
            {
                *f = m.volley_fire;
            }
        }
        if !c.add_work(&mut site.life.hp, amount) {
            return Ok(Worked::Going);
        }
        Self::complete(site, world, pathing).map(Worked::Completed)
    }

    /// `FUN_1411cc430`'s work of `amount` seconds on a foundation in state 0 with at least 1 hit
    /// point: at most up to its build time, it adds its share of its master's hit points `hp`
    /// (all of them without a build time of at least 1), never past them; true when its build
    /// points reach its build time.
    pub(crate) fn add_work(&mut self, hp: &mut f32, amount: f32) -> bool {
        let mut amount = amount;
        let build_time = f32::from(self.build_time);
        let hit_points = f32::from(self.hit_points);
        if build_time < self.progress + amount {
            amount = build_time - self.progress;
        }
        let new = if 1.0 <= build_time {
            (amount * hit_points) / build_time + *hp
        } else {
            hit_points
        };
        *hp = if hit_points < new { hit_points } else { new };
        let before = self.progress;
        self.progress = amount + before;
        crate::math::fmath::at_most_or_unordered(build_time, amount + before)
    }

    /// The construction completes: the building's `set_state(2)` (`FUN_1411dbb70` from state 0;
    /// its annexes' come first, the caller's), then `FUN_14152bcb0`, whose part for players with
    /// attribute 204 is not modelled. The completion's first part (`FUN_1411c6bd0`,
    /// [`Completion::first_part`]) and a desolid one's construction start (`FUN_1411e5670`,
    /// [`Construction::begin`]: [`Built::started`]); then the block for state 2: its build points
    /// hold its build time; its master's storage of modes 4, 8 and 64 goes to its owner (slot
    /// 0xf8, returned); its sight swaps from its footprint to its line of sight (slots 0x1d8,
    /// 0x1d0; not for classes 51 and 54, whose `+0xca1` the constructor set); it takes its
    /// standing sprite (not held); a stack unit (`+0x986`) drops its record and mesh shape
    /// (`FUN_141055b40`), lets its tiles go (`FUN_140f65580`, a building on a tile that is no
    /// annex) and sets `+0x686`; its owner makes the stack unit (the caller's, [`Built::stack`]);
    /// on snow it gets its snow flag; its foundation terrain covers its clearance (terrain 0x3f
    /// under master 1889's pasture, [`crate::map::terrain::Terrain::cover`]; the tile it stands on,
    /// `+0x2e8`, keeps no terrain to restore in the slice); its owner's attributes 98 and 165
    /// grow by its paid cost (`0x1411dd3ee`: the sum of its cost entries flagged paid) unless it
    /// is an annex or has a stack unit; the castle (82) and the wonder (276) instead record
    /// their building in their owner's history ([`Built::history`]), count in attributes 134
    /// and 173, 42 and 174, and add their paid cost to 184 (an annex too); the monastery (104)
    /// counts in 52; then the combat object's and the base `set_state`
    /// (`FUN_14121cc40`, `FUN_14119d710`) put it in state 2. The local player's sound (slot
    /// 0x4c0), the event 0x69 (`FUN_140c898a0`), the owner's AI (slot 0x388) and the snow decal
    /// are not held. Last, the master's technology is researched (`FUN_1413cdcd0` with `param_3`
    /// 0 and no building; [`Built::tech`], the caller's), and a master that disappears when built
    /// is destroyed (slot 0xd0, the caller's, [`Built::vanishes`]; its hit points are 0 already,
    /// as the builder's test after the work reads them). A master with object flag 0x40 and a
    /// foundation terrain (its tiles' classes again, `FUN_141055790`) is not modelled; a wall's (`adjacent_mode`) facets, its own and its neighbours' (`FUN_1411ce890`), are not
    /// held.
    ///
    /// # Errors
    ///
    /// What it does not model, and a building without its master or place.
    ///
    /// [`Completion::first_part`]: crate::construction::build::Completion::first_part
    pub fn complete(
        site: &mut Site<'_>,
        world: &mut Spatial,
        pathing: &mut Pathing,
    ) -> Result<Built, GenieError> {
        let id = site.id;
        let done = site.completion;
        let master = done.master.ok_or(GenieError::unsupported(
            "a completion of a master the slice does not have",
        ))?;
        let building = master.building.ok_or(GenieError::unsupported(
            "a completion of a master that is no building",
        ))?;
        if done.attribute_204 {
            return Err(crate::players::diplomacy::REVEAL_UNITS);
        }
        // FUN_1411c6bd0 (its build points are its build time already: FUN_1411cc430 wrote them)
        let first = done.first_part(site.construction)?;
        let mut attributes = first.attributes;
        // FUN_1411e5670 while still in state 0 (2 lies within 1 to 6)
        let started = Self::begin(
            id,
            site.construction,
            site.mover.as_deref_mut(),
            world,
            pathing,
            done.now,
        )?;
        if started && let Some(f) = site.firepower.as_deref_mut() {
            *f = master.volley_fire;
        }
        // a wall's (adjacent mode) facets and its neighbours' (FUN_1411ce890) are not held
        if 0 <= building.foundation_terrain && master.object_flags & 0x40 != 0 {
            return Err(GenieError::unsupported(
                "a completion of a master of object flag 0x40 (its tiles' classes, FUN_141055790)",
            ));
        }
        let c = &mut *site.construction;
        c.progress = f32::from(c.build_time);
        hand_storage(master, &mut attributes, site.store.as_deref_mut());
        let placed = *world.get(id).ok_or(GenieError::unsupported(
            "a completed building without a place",
        ))?;
        if !matches!(master.class, 51 | 54)
            && let Some(v) = world.sight.as_mut()
        {
            v.construction_completed(id, placed.position, placed.radius);
        }
        let stack_master = if master.id == MASTER_1889 && 0 <= building.stack_unit && done.rice {
            RICE_PASTURE
        } else {
            building.stack_unit
        };
        let mut stack = None;
        if 0 <= building.stack_unit {
            pathing.removed(id);
            world.drop_record(id);
            pathing.tiles_changed(world.take_changes());
            if !done.main
                && let Some(v) = world.sight.as_mut()
            {
                v.unlink_building(placed.position, master.radius);
            }
            stack = Some(StackUnit {
                master: stack_master,
                at: placed.position,
                hp_lost: f32::from(master.hit_points) - site.life.hp,
            });
            if let Some(r) = site.roo_called.as_deref_mut() {
                *r = true;
            }
        }
        lay_foundation(
            c,
            world,
            &placed,
            master,
            &building,
            stack_master,
            done.mode_680,
        )?;
        let history = completion_counts(master, &building, done.main, &mut attributes);
        let before = site.life.state;
        site.life.state = 2;
        world.sight_state(id, 2);
        let vanishes = building.disappears_when_built;
        if vanishes {
            site.life.hp = 0.0;
        }
        Ok(Built {
            attributes,
            history,
            tech: (0 <= building.tech).then_some(building.tech),
            cart: first.cart.then_some(before),
            stack,
            vanishes,
            started,
            pasture: first.pasture,
        })
    }

    /// The construction starts (`FUN_1411e5670`) on a desolid foundation (one not desolid, or
    /// without its own master copy `+0x6c0`, which a desolid one always holds, returns at once,
    /// false): its master is its owner's again (the same in the slice), its missiles per volley
    /// its master's (`+0xa00`, the caller's: true); it is no longer desolid, holds a sliver of
    /// progress, and gets its obstruction record (`FUN_141056460`: the tiles it marks, which the
    /// planners see, and its shape in the mesh); its footprint comes into its player's sight and
    /// its tiles link to its anchor again (`FUN_140f73900`); its owner watches it
    /// (`FUN_141090430` when `FUN_1411cc980`, the caller's through [`Construction::started`]);
    /// its last move time becomes the world's time plus 1. Not held: its damage sprites (made
    /// for a player not allied with the local one), the tall count of its tile (`FUN_140f64540`)
    /// and `+0xca4`.
    ///
    /// # Errors
    ///
    /// A foundation the world does not hold.
    pub(crate) fn begin(
        id: i32,
        c: &mut Self,
        mover: Option<&mut crate::movement::Mover>,
        world: &mut Spatial,
        pathing: &mut Pathing,
        now: u32,
    ) -> Result<bool, GenieError> {
        if !c.desolid {
            return Ok(false);
        }
        let placed = *world.get(id).ok_or(SITE_GONE)?;
        c.desolid = false;
        c.started = true;
        c.progress = f32::from_bits(STARTED);
        if let Some(v) = world.sight.as_mut() {
            v.construction_started(id, placed.position, placed.radius);
        }
        // a master of obstruction kind 0 or 12, or of class 0, gets no record (FUN_141056460)
        if c.record.recorded() {
            world.add_record(id, c.record);
        }
        pathing.tiles_changed(world.take_changes());
        if let Some(p) = world.get(id) {
            pathing.moved(id, p, false);
        }
        if let Some(m) = mover {
            m.last_move = now.wrapping_add(1);
            world.set_last_move(id, m.last_move);
        }
        Ok(true)
    }
}
