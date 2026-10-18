//! The map's terrain per tile (`RGE_Map`'s tiles, 0x40 bytes each: `+0x9` the terrain, `+0xa` the
//! terrain to restore, `+0xc` the layer) and what the `.dat` says of each terrain, as far as the
//! slice reads them: a completed building's snow check and its foundation terrain
//! (`FUN_1411dbb70`, `FUN_1411ddd90`).
//!
//! The obstruction manager keeps its own copy of the terrain classes ([`crate::spatial::Tiles`]),
//! which a terrain change does not update; the map's render caches (`FUN_140f741b0`'s blends and
//! minimap) are not held.

use crate::unit_ai::Unsupported;

/// What the `.dat` says of a terrain (its record at map `+0x6238`, 0x1f0 bytes each).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainKind {
    /// The record's first byte (`enabled`) or its `terrain_to_draw` (`+0xf8`) of 0 or more: the
    /// map's terrain reads (`FUN_140f690e0`, `FUN_140f69000`) see the terrain, else terrain 0.
    pub shown: bool,
    /// The `.dat`'s `is_water` byte, which the map's load copies to the terrain table
    /// (`FUN_1413fc340`, `DAT_143cc7b40 + 0x2c`): bit 7 marks snow (`FUN_1413fc280`).
    pub flags: u8,
}

/// The terrain a completed building leaves on snow (`FUN_1411ddd90`): its foundation 27 becomes 6
/// with the layer 32, and it skips tiles of 36.
const FOUNDATION: u8 = 27;
const SNOW_FOUNDATION: u8 = 6;
const SNOW_LAYER: i16 = 32;
const SNOW_SKIPPED: u8 = 36;

/// Terrains a foundation never covers (`FUN_1411ddd90`: 47 and 69).
const UNCOVERED: [u8; 2] = [47, 69];

/// The terrains foundation 27 covers, and the layers it covers them under (the bits of
/// `0x500001209`; the layer test lists their complement, `0x2ffffedf6` and above 34).
fn plain(t: i16) -> bool {
    matches!(t, 0 | 3 | 9 | 12 | 32 | 34)
}

/// `FUN_1413fc120`: the terrain table entry's first byte, the terrain's id, is a farm's: 7, 8,
/// 29 to 31, 63 to 67 and 117 to 121.
pub fn is_farm_terrain(t: u8) -> bool {
    matches!(t, 7 | 8 | 29..=31 | 63..=67 | 117..=121)
}

/// The map's terrain and the `.dat`'s terrain records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Terrain {
    width: i32,
    height: i32,
    /// Per tile, row by row: the terrain (`+0x9`), the terrain to restore (`+0xa`, 255 for none)
    /// and the layer (`+0xc`, −1 for none).
    pub kinds: Vec<u8>,
    pub restore: Vec<u8>,
    pub layers: Vec<i16>,
    /// Per terrain id, the `.dat`'s records.
    pub terrains: Vec<TerrainKind>,
    /// Each terrain restriction's value per terrain (`world+0x788`, the `.dat`'s
    /// `passable_buildable_dmg_multiplier`).
    pub restrictions: Vec<Vec<f32>>,
    /// Per tile, its elevation (`+0x14`, the save's `t->m_elevation`); empty when not known.
    pub elevations: Vec<u8>,
}

/// What a completed building's foundation reads of it (`FUN_1411ddd90`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Foundation {
    pub position: [f32; 3],
    /// Its master's `clearance_size` (`+0x5fc`).
    pub clearance: [f32; 2],
    /// The foundation terrain, the master's `+0x988` byte.
    pub terrain: u8,
    /// The building's `snow_flag` (`+0xc61`).
    pub snow: bool,
    /// The master's id and class, its `defense_terrain_bonus` (`+0x820`).
    pub master: i16,
    pub class: i16,
    pub terrain_bonus: i16,
    /// The game's setting `FUN_140ec0cd0` reads at `+0x680` ([`crate::turn::Turns::mode_680`]):
    /// at 1 a foundation without a snow layer goes into the layer, not the terrain.
    pub mode_680: i32,
}

impl Terrain {
    pub fn new(
        width: i32,
        height: i32,
        tiles: Vec<(u8, u8, i16)>,
        terrains: Vec<TerrainKind>,
        restrictions: Vec<Vec<f32>>,
    ) -> Self {
        Terrain {
            width,
            height,
            kinds: tiles.iter().map(|t| t.0).collect(),
            restore: tiles.iter().map(|t| t.1).collect(),
            layers: tiles.iter().map(|t| t.2).collect(),
            terrains,
            restrictions,
            elevations: Vec::new(),
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (0 <= x && x < self.width && 0 <= y && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    /// A tile's terrain (the map tile's `+0x9`), None off the map.
    pub fn kind(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|i| self.kinds[i])
    }

    /// The ground's height under (`x`, `y`) on the map (the static new_position's,
    /// `0x1411a58b0`, which slot 0x60 takes): the tile's elevation plus its slope's share by the
    /// tile's shape (`+0x8`, made at the map's load from the elevations about it). A tile whose
    /// eight neighbours on the map share its elevation is flat; a slope's share is not modelled.
    pub fn ground(&self, x: f32, y: f32) -> Result<f32, Unsupported> {
        let (tx, ty) = (x as i32, y as i32);
        let i = self
            .index(tx, ty)
            .filter(|_| !self.elevations.is_empty())
            .ok_or(Unsupported(
                "the ground's height where the elevations are not known",
            ))?;
        let e = self.elevations[i];
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(j) = self.index(tx + dx, ty + dy)
                    && self.elevations[j] != e
                {
                    return Err(Unsupported(
                        "the ground's height on a slope (the tile's shape)",
                    ));
                }
            }
        }
        Ok(f32::from(e))
    }

    /// The terrain defense of an object on tile (`x`, `y`) whose master has `defense_terrain_bonus`
    /// `bonus` (`FUN_141082530`): the restriction's value (`world+0x788`) for the tile's terrain
    /// to restore, else its terrain (the tile's `+0xa`, else `+0x9`); 0 counts as 1, and an
    /// object off the tiles has 1. None for a restriction or terrain the data has not got.
    pub fn defense(&self, bonus: i16, x: i32, y: i32) -> Option<f32> {
        let Some(i) = self.index(x, y) else {
            return Some(1.0);
        };
        let t = match self.restore[i] {
            255 => self.kinds[i],
            t => t,
        };
        let v = *self
            .restrictions
            .get(usize::try_from(bonus).ok()?)?
            .get(usize::from(t))?;
        Some(if v == 0.0 { 1.0 } else { v })
    }

    /// The terrain a map read sees of `t` (`FUN_140f690e0`): `t` when its record exists and it is
    /// shown, else 0.
    fn seen(&self, t: u8) -> u8 {
        match self.terrains.get(usize::from(t)) {
            Some(k) if k.shown => t,
            _ => 0,
        }
    }

    /// `FUN_1413fc280`: the terrain table's snow bit; a terrain beyond the records has none.
    fn snowy(&self, t: u8) -> bool {
        self.terrains
            .get(usize::from(t))
            .is_some_and(|k| k.flags & 0x80 != 0)
    }

    /// The snow check of a completed building on tile (`x`, `y`) (`FUN_1411dbb70`'s "pick snow"):
    /// the tile's terrain (`FUN_1413fbee0`) or its layer read as a terrain (`FUN_1413fbf90`, the
    /// layer's low byte) is snow. Its decal (`FUN_1413b7bf0`, which may draw from the Object
    /// stream between a push and a pop of its state, `FUN_140811270`/`FUN_140811210`) is not
    /// held.
    pub fn snow_under(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        let i = self
            .index(x, y)
            .ok_or(Unsupported("a building off the map's tiles"))?;
        let kind = self.seen(self.kinds[i]);
        let layer = self.seen(self.layers[i] as u8);
        Ok(self.snowy(kind) || self.snowy(layer))
    }

    /// A completed building's foundation terrain (`FUN_1411ddd90`) over its position plus and
    /// minus its clearance (clamped to the map, `FUN_1409a47d0`). A tile of a terrain its
    /// `defense_terrain_bonus` scales (neither 0 nor 1) keeps it to restore; then, but on 47 and
    /// 69, foundation 27 covers the tiles of terrains 0, 3, 9, 12, 32, 34 or a farm's whose
    /// layer is none, one of those or a farm's; on snow it covers every tile but 36's with
    /// terrain 6 and layer 32; any other foundation covers the tiles of other terrains. A covered
    /// tile's layer goes to the snow layer (or none, `FUN_140f640f0`), then its terrain to the
    /// foundation's (`FUN_140f64170`), or with no snow layer and setting `+0x680` at 1 its layer.
    /// The deferred covering under `DAT_143f69454` (0 when probed), farms, master 109's gaps,
    /// masters 357 and 1188 and class 14 are not modelled.
    pub fn cover(&mut self, f: &Foundation) -> Result<(), Unsupported> {
        /// `DAT_142eb1e9c`.
        const INSIDE: f32 = crate::image::terrain::INSIDE;
        if f.class == 49 || f.class == 14 {
            return Err(Unsupported("a foundation terrain of a farm or of class 14"));
        }
        if matches!(f.master, 109 | 357 | 1188) {
            return Err(Unsupported(
                "a foundation terrain of master 109, 357 or 1188",
            ));
        }
        let [x, y, _] = f.position;
        let [cx, cy] = f.clearance;
        let (mut x0, mut y0) = ((x - cx) as i32, (y - cy) as i32);
        let (mut x1, mut y1) = (((x + cx) - INSIDE) as i32, ((y + cy) - INSIDE) as i32);
        if x1 < x0 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if y1 < y0 {
            std::mem::swap(&mut y0, &mut y1);
        }
        let (w, h) = (self.width - 1, self.height - 1);
        if x0 > w || y0 > h || x1 < 0 || y1 < 0 {
            return Err(Unsupported("a foundation terrain off the map"));
        }
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(w), y1.min(h));
        let (mut wanted, mut kind, mut layer) = (f.terrain, f.terrain, -1i16);
        if f.terrain == FOUNDATION && f.snow {
            (wanted, kind, layer) = (SNOW_SKIPPED, SNOW_FOUNDATION, SNOW_LAYER);
        }
        let bonus = usize::try_from(f.terrain_bonus)
            .ok()
            .map(|r| {
                self.restrictions
                    .get(r)
                    .ok_or(Unsupported("a terrain bonus the data file does not have"))
            })
            .transpose()?
            .cloned();
        if is_farm_terrain(wanted) {
            return Err(Unsupported("a foundation of a farm's terrain"));
        }
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                let i = self.index(tx, ty).expect("clamped to the map");
                let t = self.kinds[i];
                if let Some(values) = bonus.as_ref() {
                    let v = values.get(usize::from(t)).copied().unwrap_or(0.0);
                    if v != 0.0 && v != 1.0 {
                        self.restore[i] = t;
                    }
                }
                if UNCOVERED.contains(&t) {
                    continue;
                }
                let covered = if wanted == FOUNDATION {
                    let l = self.layers[i];
                    (plain(i16::from(t)) || (t < 200 && is_farm_terrain(t)))
                        && (l < 0 || plain(l) || (l < 200 && is_farm_terrain(l as u8)))
                } else {
                    wanted != t
                };
                if !covered {
                    continue;
                }
                self.layers[i] = layer;
                if layer != -1 || f.mode_680 != 1 {
                    self.kinds[i] = kind;
                } else {
                    self.layers[i] = i16::from(kind);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(kinds: Vec<u8>, layers: Vec<i16>) -> Terrain {
        let n = kinds.len();
        let mut terrains = vec![
            TerrainKind {
                shown: true,
                flags: 0x20
            };
            40
        ];
        terrains[32].flags = 0xa0;
        Terrain {
            width: 2,
            height: n as i32 / 2,
            kinds,
            restore: vec![255; n],
            layers,
            terrains,
            restrictions: vec![vec![0.0; 40]; 5],
            elevations: vec![0; n],
        }
    }

    #[test]
    fn the_ground_is_a_flat_tile_s_elevation_and_a_slope_is_not_modelled() {
        let mut t = map(vec![0; 8], vec![-1; 8]);
        t.elevations = vec![1, 1, 1, 1, 1, 1, 2, 1];
        assert_eq!(t.ground(0.5, 0.5), Ok(1.0));
        assert!(t.ground(1.5, 2.5).is_err());
    }

    /// MarvinBlue/426048731's house 10026 at (104, 38): its tiles 103..104 x 37..38 were snow (32)
    /// and grass 9 with a snow layer; the save after 1115 holds terrain 6 with layer 32 on each.
    #[test]
    fn a_house_completed_on_snow_leaves_terrain_6_under_a_snow_layer() {
        let mut t = map(vec![32, 32, 32, 9], vec![-1, -1, -1, 32]);
        assert!(t.snow_under(0, 0).unwrap());
        assert!(t.snow_under(1, 1).unwrap());
        t.cover(&Foundation {
            position: [1.0, 1.0, 0.0],
            clearance: [1.0, 1.0],
            terrain: FOUNDATION,
            snow: true,
            master: 70,
            class: 3,
            terrain_bonus: 4,
            mode_680: 0,
        })
        .unwrap();
        assert_eq!(t.kinds, [6, 6, 6, 6]);
        assert_eq!(t.layers, [32, 32, 32, 32]);
        assert_eq!(t.restore, [255; 4]);
    }

    #[test]
    fn foundation_27_covers_grass_but_not_a_terrain_outside_its_list() {
        let mut t = map(vec![0, 2, 9, 47], vec![-1, -1, -1, -1]);
        assert!(!t.snow_under(0, 0).unwrap());
        t.cover(&Foundation {
            position: [1.0, 1.0, 0.0],
            clearance: [1.0, 1.0],
            terrain: FOUNDATION,
            snow: false,
            master: 70,
            class: 3,
            terrain_bonus: -1,
            mode_680: 0,
        })
        .unwrap();
        assert_eq!(t.kinds, [27, 2, 27, 47]);
    }
}
