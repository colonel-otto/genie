//! Where the world's objects are, as movement reads them: each live object's position, radii and
//! class, and the obstruction record the game keeps for it in its obstruction manager
//! (`DAT_143f0a088`, queried by `FUN_141051900`).
//!
//! The game files its records in a five-level grid; the slice keeps them by object id. The grid
//! only orders a query's hits, and the slice stops a walker at the first possible hit
//! ([`Spatial::obstruction_candidates`]), so it needs no order yet.

use std::collections::BTreeMap;

/// Obstruction kinds the query tests (`FUN_141051e00`): boxes (2, 3, 11, 13) and circles (5).
const TESTED_KINDS: [u8; 5] = [2, 3, 5, 11, 13];

/// One object as movement sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    /// x, y, z on the map.
    pub position: [f32; 3],
    /// The master's radii on the ground (master `+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// The master's class (`+0x410`, decoded): the motion step treats gates (39) apart.
    pub class: i16,
    /// The obstruction record's kind (its byte `+1`, the master's `obstruction_type`), if the
    /// object has one: a foundation gets its record only when construction starts.
    pub obstruction: Option<u8>,
}

impl Placed {
    /// The record's box `x0, y0, x1, y1` (`+0x20`..`+0x2c`): the position plus and minus the radii.
    pub fn bounds(&self) -> [f32; 4] {
        let [x, y, _] = self.position;
        let [rx, ry] = self.radius;
        [x - rx, y - ry, x + rx, y + ry]
    }
}

/// The world's objects by id, and the map's size in tiles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spatial {
    pub map: [i32; 2],
    objects: BTreeMap<i32, Placed>,
}

impl Spatial {
    pub fn new(map: [i32; 2]) -> Self {
        Spatial {
            map,
            objects: BTreeMap::new(),
        }
    }

    pub fn get(&self, id: i32) -> Option<&Placed> {
        self.objects.get(&id)
    }

    pub fn insert(&mut self, id: i32, placed: Placed) {
        self.objects.insert(id, placed);
    }

    pub fn remove(&mut self, id: i32) {
        self.objects.remove(&id);
    }

    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, [f32; 4])> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?, p.bounds())))
    }

    /// The object moved: its record moves with it (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            p.position = position;
        }
    }

    /// The records a move query could report (`FUN_14114cc60` -> `FUN_141051900`): those of a
    /// tested kind whose box, grown by `grow`, meets `rect` (`x0, y0, x1, y1`, edges included),
    /// except the `ignore`d objects'. The game then tests each candidate against the move's
    /// segment and shrinks moving allies' boxes (master `+0x780`, at most 1); every hit is a
    /// candidate, so no candidate means no hit.
    pub fn obstruction_candidates(&self, rect: [f32; 4], grow: f32, ignore: &[i32]) -> Vec<i32> {
        self.objects
            .iter()
            .filter(|(id, p)| {
                p.obstruction.is_some_and(|k| TESTED_KINDS.contains(&k)) && !ignore.contains(id)
            })
            .filter(|(_, p)| {
                let [x0, y0, x1, y1] = p.bounds();
                let (x0, y0, x1, y1) = (x0 - grow, y0 - grow, grow + x1, grow + y1);
                rect[0] <= x1 && x0 <= rect[2] && rect[1] <= y1 && y0 <= rect[3]
            })
            .map(|(&id, _)| id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn villager(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            obstruction: Some(5),
        }
    }

    #[test]
    fn a_units_box_is_its_position_plus_and_minus_its_radii() {
        // MarvinBlue/426048731 tick 680: 9177's record box in the game's obstruction manager
        assert_eq!(villager(28.0, 89.0).bounds(), [27.8, 88.8, 28.2, 89.2]);
    }

    #[test]
    fn a_query_finds_the_boxes_its_rectangle_meets_but_not_the_ignored_or_unobstructing() {
        let mut s = Spatial::new([120, 120]);
        s.insert(9177, villager(28.0, 89.0));
        s.insert(9175, villager(30.0, 89.0));
        let mut foundation = villager(30.0, 88.0);
        foundation.radius = [1.0, 1.0];
        foundation.obstruction = None;
        s.insert(10028, foundation);
        // 9177's first step: from (28, 89) toward (28.0143, 88.9928), grown by its radius
        let step = [28.0, 88.99284, 28.01431, 89.0];
        assert_eq!(
            s.obstruction_candidates(step, 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(s.obstruction_candidates(step, 0.2, &[]), vec![9177]);
        // 9175's box grown by 0.2 starts at x 29.6
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.59, 89.0], 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.61, 89.0], 0.2, &[9177]),
            vec![9175]
        );
    }
}
