//! The placement test's terrains (`FUN_141123f80`): the hill mode, the placement and side
//! terrains and the tiles beside a box.

use crate::spawn::PlaceMap;
use crate::unit_ai::GenieError;

/// What BUILD's placement test reads of the building master and the game.
#[derive(Clone, Copy, Debug)]
pub struct BuildSite<'a> {
    /// The player who places it.
    pub player: usize,
    /// A human player (`p6`: the explored test).
    pub human: bool,
    /// The master's clearance radii (`+0x5fc`, `+0x600`; `p9`).
    pub clearance: [f32; 2],
    /// The master's class.
    pub class: i16,
    /// The master's `obstruction_type` is neither 0 nor 12 (`FUN_1411205c0`).
    pub obstructs: bool,
    /// The master's terrain restriction's value per terrain.
    pub restriction: &'a [f32],
    /// `hill_mode` (`+0x604`, `p7`).
    pub hill_mode: u8,
    /// The placement terrains (`+0x5f2`..`+0x5f8`, [`placement_terrains`]).
    pub placement: [i16; 4],
    /// The world's zone mode (`+0x820`, `FUN_1415e8e10`).
    pub zones: bool,
    /// The objects the scan passes over (`p12` and, for a building, its annexes, `FUN_14123d090`):
    /// none for BUILD, a reseeding gatherer's old target ([`crate::turn::Turns::reseed`]).
    pub own: &'a [i32],
}

/// The hill mode's test of a tile of the box (`FUN_141123f80`'s `p7` part, the master's
/// `hill_mode` `+0x604`): mode 1 takes flat ground and the four side slopes (shapes 0 and 5 to
/// 8), mode 2 flat ground alone, mode 3 a tile of the box's first column or last row within one
/// step of the `base` elevation; the other modes take any. True when the tile fails (code 3).
pub(super) fn hill_fails(
    mode: u8,
    map: &dyn PlaceMap,
    (tx, ty): (i32, i32),
    (x0, y1): (i32, i32),
    base: Option<u8>,
) -> Result<bool, GenieError> {
    let shape = || {
        map.shape(tx, ty).ok_or(GenieError::unsupported(
            "a hill mode where the tiles' shapes are not known",
        ))
    };
    Ok(match mode {
        1 => !matches!(shape()?, 0 | 5..=8),
        2 => shape()? != 0,
        3 if tx == x0 || ty == y1 => {
            let e = i32::from(map.elevation(tx, ty).ok_or(GenieError::unsupported(
                "a hill mode where the elevations are not known",
            ))?);
            let b = i32::from(base.unwrap_or(0));
            e < b - 1 || b + 1 < e
        }
        _ => false,
    })
}

/// `FUN_141123f80`'s placement terrains (`placement`: the master's side terrains `+0x5f2`,
/// `+0x5f4`, then its terrains `+0x5f6`, `+0x5f8`; −1 for none) for a box of tiles `b` (x0, y0,
/// x1, y1) around point `p`: false where they refuse it (the test's code 1). With a terrain,
/// the point's tile must be of one of the two, or share the flag 4 or 8 (the terrains' `.dat`
/// flags, [`PlaceMap::terrain_flags`]) with them, and the box shrinks to that tile. With a side
/// terrain, one of the tiles beside the box (the row below, the column right, the row above and
/// the column left) must be of a side terrain, or of a terrain with the flag 0x40 or 0x10 when a
/// side terrain has it. A terrain of −1 beside one of 0 or more (its flags read past the table)
/// is not modelled.
///
/// # Errors
///
/// Where the map cannot tell.
pub fn placement_terrains(
    placement: [i16; 4],
    map: &dyn PlaceMap,
    p: [f32; 2],
    b: [i32; 4],
) -> Result<bool, GenieError> {
    let [side1, side2, t1, t2] = placement;
    let mut b = b;
    if 0 <= t1 || 0 <= t2 {
        let (x, y) = (p[0] as i32, p[1] as i32);
        if !point_terrain_allows(map, [t1, t2], x, y)? {
            return Ok(false);
        }
        b = [x, y, x, y];
    }
    if side1 < 0 && side2 < 0 {
        return Ok(true);
    }
    let sides = side_terrains(map, [side1, side2])?;
    for (x, y) in tiles_beside(b, map.size()) {
        if sides.contains(&i16::from(map.terrain(x, y)?)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// [`placement_terrains`]'s flags of terrain `t` (the `.dat`'s, [`PlaceMap::terrain_flags`]).
///
/// # Errors
///
/// A terrain of −1 or beyond the terrains (its flags read past the table), not modelled.
pub(super) fn placement_flags(map: &dyn PlaceMap, t: i16) -> Result<u8, GenieError> {
    usize::try_from(t)
        .ok()
        .and_then(|t| map.terrain_flags(t))
        .ok_or(GenieError::unsupported(
            "placement terrains with one of -1 or beyond the terrains",
        ))
}

/// [`placement_terrains`]'s test of the point's tile (`x`, `y`) against the master's terrains
/// `terrains`: of one of the two, or sharing the flag 4 or 8 with them.
///
/// # Errors
///
/// Where the map cannot tell, and a terrain whose flags it does not hold.
pub(super) fn point_terrain_allows(
    map: &dyn PlaceMap,
    [t1, t2]: [i16; 2],
    x: i32,
    y: i32,
) -> Result<bool, GenieError> {
    let tile = i16::from(map.terrain(x, y)?);
    if tile == t1 || tile == t2 {
        return Ok(true);
    }
    let (f1, f2, ft) = (
        placement_flags(map, t1)?,
        placement_flags(map, t2)?,
        placement_flags(map, tile)?,
    );
    let water = (f1 & 4 != 0 || f2 & 4 != 0) && ft & 4 != 0;
    let shallow = (f1 & 8 != 0 || f2 & 8 != 0) && ft & 8 != 0;
    Ok(water || shallow)
}

/// [`placement_terrains`]'s side terrains: the master's own (those of 0 or more), then for the
/// flag 0x40 and then 0x10, when a side terrain has it, every other terrain with it in the
/// terrains' order.
///
/// # Errors
///
/// A terrain whose flags the map does not hold.
pub(super) fn side_terrains(
    map: &dyn PlaceMap,
    [side1, side2]: [i16; 2],
) -> Result<Vec<i16>, GenieError> {
    let (f1, f2) = (placement_flags(map, side1)?, placement_flags(map, side2)?);
    let mut sides: Vec<i16> = [side1, side2].into_iter().filter(|&t| 0 <= t).collect();
    for bit in [0x40, 0x10] {
        if f1 & bit != 0 || f2 & bit != 0 {
            for t in 0..map.terrain_count() {
                let t = i16::try_from(t).unwrap_or(i16::MAX);
                if placement_flags(map, t)? & bit != 0 && t != side1 && t != side2 {
                    sides.push(t);
                }
            }
        }
    }
    Ok(sides)
}

/// [`placement_terrains`]'s tiles beside the box `b` (x0, y0, x1, y1) on a map of `size`: the
/// row below, the column right, the row above (right to left) and the column left (top to
/// bottom), each where the map holds it.
pub(super) fn tiles_beside([x0, y0, x1, y1]: [i32; 4], [w, h]: [i32; 2]) -> Vec<(i32, i32)> {
    let mut beside = Vec::new();
    if 0 < y0 {
        beside.extend((x0..=x1).map(|x| (x, y0 - 1)));
    }
    if x1 + 1 < w {
        beside.extend((y0..=y1).map(|y| (x1 + 1, y)));
    }
    if y1 + 1 < h {
        beside.extend((x0..=x1).rev().map(|x| (x, y1 + 1)));
    }
    if 0 < x0 {
        beside.extend((y0..=y1).rev().map(|y| (x0 - 1, y)));
    }
    beside
}
