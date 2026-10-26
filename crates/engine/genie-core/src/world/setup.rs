//! The world as the header save holds it ([`World::new`]): the next object id, the
//! planners and the shape mesh the game's load builds.

use crate::map::spatial::Spatial;
use crate::math::rng::Streams;
use crate::movement::Mover;
use crate::pathing::Pathing;
use crate::world::player::Player;
use crate::world::{HISTORY_PERIOD, PLANNED_MASTERS, Recycling, World};

impl World {
    /// The world's next object id (`FUN_14152fd70`) for a new object of `master` for `player`: it
    /// counts on, and the id goes to the trace ([`crate::trace::new_object`]).
    #[track_caller]
    pub(crate) fn take_object_id(&mut self, player: usize, master: i16) -> i32 {
        let id = self.next_object_id;
        self.next_object_id += 1;
        crate::trace::new_object(self.ticks, player, id, master);
        id
    }

    /// The world as the header save holds it: its time, frame seed, turn order and players.
    #[must_use]
    pub fn new(
        world_time: u32,
        frame_seed: u32,
        player_turn: usize,
        player_time: Vec<u32>,
        players: Vec<Player>,
    ) -> Self {
        Self {
            world_time,
            frame_seed,
            player_turn,
            player_time,
            players,
            ticks: 0,
            next_object_id: 0,
            masters: Vec::new(),
            spatial: Spatial::default(),
            pathing: Pathing::default(),
            graphics: std::collections::BTreeMap::default(),
            drop_sites: Vec::new(),
            lines: crate::objects::lines::UnitLines::default(),
            linked_techs: std::collections::BTreeMap::new(),
            settings: crate::world::settings::Settings::default(),
            start_work_due: false,
            mode_680: 0,
            spawned_relics: 0,
            game_state: 0,
            victory_conquest: false,
            zones: crate::pathing::zones::Zones::default(),
            recycling: Recycling::default(),
            streams: Streams::default(),
            techs: Vec::new(),
            effects: Vec::new(),
            market: crate::players::market::Market::default(),
            groups: crate::groups::UnitGroups::default(),
            dt_seconds: 0.0,
            dt: 0,
            history_period: HISTORY_PERIOD,
            allow_gaia_capture: Vec::new(),
            local_player: 0,
            tech_tree: crate::players::world_player::TechTree::default(),
            alliance_locked: 0,
            teams: [1; 8],
            instant_work: 0,
        }
    }
}

impl World {
    /// `World::load_world`'s planners (`FUN_140f7d310`): the restrictions of Gaia's masters 83
    /// (the villager's) and 535 (the galley's), each unless one of the same values has one; any
    /// other restriction finds none (its units walk without the tile pathfinder).
    pub fn register_planners(&mut self) {
        for master in PLANNED_MASTERS {
            if let Some(r) = self
                .masters
                .first()
                .and_then(|m| m.get(master))
                .and_then(|m| usize::try_from(m.terrain_restriction).ok())
            {
                self.pathing.register(r);
            }
        }
    }

    /// The shape mesh the game's load builds, which no save holds: an empty mesh of the map's
    /// width, then the load's rehook of each object (`RGE_Static_Object::rehook`, slot 9, through
    /// `FUN_141665650` and `FUN_1411689f0`: each player's live list, then its sleeping list):
    /// its position set again (slot 0x58 -> `FUN_14118c540`: below state 7 and tall, then
    /// `FUN_141056460`'s shape by its master's obstruction kind, not 0 or 12, and class, not 0,
    /// for an object that does not move, [`Pathing::place_shape`]), then with an obstruction
    /// record the refresh an update ends with (`FUN_141054fe0`, [`Pathing::refresh`]). An object
    /// inside another has none (it left the obstruction manager); `mROOAlreadyCalled`
    /// (`+0x686`) is not held (0 at a game's start), and a desolid foundation in a save is left
    /// without one, as the game's own placement leaves it (unchecked at a load).
    pub fn build_mesh(&mut self) {
        let width = u32::try_from(self.spatial.map[0]).unwrap_or(0);
        self.pathing.start_mesh(width);
        for (q, p) in self.players.iter().enumerate() {
            for u in p.objects.iter().chain(&p.sleeping) {
                if u.life.inside.is_some() || 7 <= u.life.state {
                    continue;
                }
                let (Some(placed), Some(m)) = (
                    self.spatial.get(u.id),
                    self.masters.get(q).and_then(|m| m.get(u.life.master_id)),
                ) else {
                    continue;
                };
                let moving = u.mover.as_ref().is_some_and(Mover::is_moving);
                let desolid = u.construction.is_some_and(|c| c.desolid);
                let kind = m.obstruction_type;
                if placed.tall
                    && kind != 0
                    && kind != 12
                    && m.obstruction_class != 0
                    && !moving
                    && !desolid
                {
                    self.pathing.place_shape(u.id, placed);
                }
                self.pathing.refresh(u.id, placed, moving, m.outline);
            }
        }
    }
}
