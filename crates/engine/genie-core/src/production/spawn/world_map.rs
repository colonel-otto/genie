//! The map as the placement test reads it from the world ([`WorldMap`]).

use crate::production::spawn::{Near, PlaceMap};
use crate::unit_ai::GenieError;
use crate::world::Turns;

/// The map as the spawn's placement test reads it: the terrain, the players' sight and the
/// objects with what their units and masters say.
pub(crate) struct WorldMap<'a> {
    pub(crate) turns: &'a Turns,
}

impl PlaceMap for WorldMap<'_> {
    fn terrain_flags(&self, t: usize) -> Option<u8> {
        self.turns
            .spatial
            .terrain
            .as_ref()
            .and_then(|m| m.terrains.get(t))
            .map(|k| k.flags)
    }

    fn terrain_count(&self) -> usize {
        self.turns
            .spatial
            .terrain
            .as_ref()
            .map_or(0, |m| m.terrains.len())
    }

    fn size(&self) -> [i32; 2] {
        self.turns.spatial.map
    }

    fn terrain(&self, x: i32, y: i32) -> Result<u8, GenieError> {
        let t = self
            .turns
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a placement test without the map's terrain",
            ))?;
        t.kind(x, y).ok_or(GenieError::unsupported(
            "a placement test off the map's tiles",
        ))
    }

    fn explored(&self, player: usize, x: i32, y: i32) -> bool {
        // without the sight the spawn stops before (Turns::spawn)
        self.turns.spatial.explored(player, x, y).unwrap_or(false)
    }

    fn visible(&self, player: usize, x: i32, y: i32) -> bool {
        self.turns.spatial.visible(player, x, y)
    }

    fn vision(&self, player: usize) -> [u64; 2] {
        self.turns.spatial.vision(player)
    }

    fn elevation(&self, x: i32, y: i32) -> Option<u8> {
        let t = self.turns.spatial.terrain.as_ref()?;
        let [w, h] = self.turns.spatial.map;
        (0 <= x && 0 <= y && x < w && y < h)
            .then(|| t.elevations.get((y * w + x) as usize).copied())
            .flatten()
    }

    fn shape(&self, x: i32, y: i32) -> Option<u8> {
        let t = self.turns.spatial.terrain.as_ref()?;
        let [w, h] = self.turns.spatial.map;
        (0 <= x && 0 <= y && x < w && y < h)
            .then(|| t.shapes.get((y * w + x) as usize).copied())
            .flatten()
    }

    fn allied(&self, a: usize, b: usize, with_4: bool) -> bool {
        a == b
            || (b != 0
                && self.turns.players.get(a).is_some_and(|p| {
                    matches!(p.relations.get(b), Some(0))
                        || (with_4 && p.relations.get(b) == Some(&4))
                }))
    }

    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
        let t = self.turns;
        t.spatial
            .objects_on(x, y)
            .map(|(id, placed)| {
                let (owner, unit) = t.object(id).ok_or(GenieError::unsupported(
                    "an object near a spawn point the slice does not update",
                ))?;
                let master = t.master_of(owner, unit.life.master_id)?;
                Ok(Near {
                    id,
                    position: placed.position,
                    radius: placed.radius,
                    tall: placed.tall,
                    kind: master.kind,
                    class: placed.class,
                    master: placed.master,
                    buildable_on: placed.buildable_on,
                    obstruction_type: placed.obstruction_type,
                    auto_gather: master.auto_gather,
                    state: unit.life.state,
                    desolid: unit.construction.is_some_and(|c| c.desolid),
                    player: owner,
                    moving: master.moving.as_ref().is_some_and(|m| 0.0 < m.speed),
                    fog_visibility: master.fog_visibility,
                    fog_mask: t.spatial.sight.as_ref().and_then(|v| {
                        let umv = v.seer(id).map_or(0, |s| s.umv);
                        (50 <= master.kind && umv != 0).then(|| {
                            v.registry
                                .as_ref()
                                .and_then(|r| r.entry(id))
                                .map_or(0, |e| e.mask)
                        })
                    }),
                    holds: 0.0 < unit.life.amount
                        && matches!(unit.life.attribute_type, 0..=3 | 15..=17),
                    hp: unit.life.hp,
                    forest_tile: t.spatial.terrain.as_ref().is_some_and(|ter| {
                        ter.kind(placed.position[0] as i32, placed.position[1] as i32)
                            .and_then(|k| ter.terrains.get(usize::from(k)))
                            .is_some_and(|k| k.forest)
                    }),
                })
            })
            .collect()
    }
}
