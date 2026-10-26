use super::*;

/// records/425906646's update 1883: a TC's gather point on a tree that is gone; its new
/// villager takes the closest tree of Gaia's (by centre) within its sight of the point,
/// measured from the tree's footprint, else none (it walks to the point).
#[test]
fn a_gather_point_whose_tree_is_gone_takes_the_closest_tree_within_sight() {
    let unit = |id, master, x: f32, y: f32| Unit {
        id,
        position: [x, y, 0.0],
        life: Lifecycle {
            master_id: master,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut t = Turns::new(
        0,
        1,
        0,
        vec![0, 0],
        vec![
            crate::turn::Player {
                sleeping: vec![unit(10, 349, 20.5, 20.5), unit(11, 349, 22.5, 20.5)],
                ..crate::turn::Player::default()
            },
            crate::turn::Player {
                objects: vec![unit(99, 83, 18.0, 18.0)],
                ..crate::turn::Player::default()
            },
        ],
    );
    let tree = crate::objects::masters::Master {
        id: 349,
        class: 15,
        radius: [0.5, 0.5],
        ..crate::objects::masters::Master::default()
    };
    let villager = crate::objects::masters::Master {
        id: 83,
        class: 4,
        line_of_sight: 4.0,
        ..crate::objects::masters::Master::default()
    };
    t.masters = vec![
        crate::objects::masters::Masters::new([tree.clone()]),
        crate::objects::masters::Masters::new([villager, tree]),
    ];
    let g = GatherPoint {
        point: [21.0, 21.0, 0.0],
        target: 5,
        target_master: 349,
    };
    assert_eq!(t.lost_gather_target(1, 0, g).unwrap(), Some(10));
    // 7 tiles from the closest tree's footprint, beyond the villager's sight of 4
    let far = GatherPoint {
        point: [30.0, 21.0, 0.0],
        ..g
    };
    assert_eq!(t.lost_gather_target(1, 0, far).unwrap(), None);
}

/// MarvinBlue/426048731's TC 9149 at (104, 47), radii 2: its gather point (107.2451,
/// 46.075863) lies right and below, so the right side's lower half (6) comes first among the
/// sides the first pass tries; the villager (radius 0.2) steps by 0.5 from (106.5, 47.5).
/// The sheep 9520 at (106.68, 47.15) blocks the first two candidates and stays (livestock in
/// state 2), so each ends its spot; (106.5, 46.5) is free.
/// A map of one tile's shape and elevation for the hill test.
struct Hill(u8, u8);

impl PlaceMap for Hill {
    fn size(&self) -> [i32; 2] {
        [10, 10]
    }
    fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
        Ok(0)
    }
    fn explored(&self, _: usize, _: i32, _: i32) -> bool {
        true
    }
    fn visible(&self, _: usize, _: i32, _: i32) -> bool {
        true
    }
    fn vision(&self, _: usize) -> [u64; 2] {
        [0; 2]
    }
    fn elevation(&self, _: i32, _: i32) -> Option<u8> {
        Some(self.1)
    }
    fn shape(&self, _: i32, _: i32) -> Option<u8> {
        Some(self.0)
    }
    fn allied(&self, a: usize, b: usize, _: bool) -> bool {
        a == b
    }
    fn objects(&self, _: i32, _: i32) -> Result<Vec<Near>, GenieError> {
        Ok(Vec::new())
    }
}

#[test]
fn a_hill_mode_takes_flat_ground_side_slopes_or_an_edge_within_one_step() {
    let fails = |mode, shape, elevation, at| {
        hill_fails(mode, &Hill(shape, elevation), at, (2, 5), Some(3)).unwrap()
    };
    // mode 1: flat and the four side slopes (5 to 8) pass, corners and saddles fail
    assert!(!fails(1, 0, 3, (3, 3)) && !fails(1, 5, 3, (3, 3)) && !fails(1, 8, 3, (3, 3)));
    assert!(fails(1, 2, 3, (3, 3)) && fails(1, 9, 3, (3, 3)));
    // mode 2: flat alone
    assert!(!fails(2, 0, 3, (3, 3)) && fails(2, 6, 3, (3, 3)));
    // mode 3: the first column and the last row within one step of the corner's elevation
    assert!(!fails(3, 9, 4, (2, 3)) && fails(3, 0, 5, (2, 3)) && fails(3, 0, 1, (4, 5)));
    assert!(!fails(3, 0, 7, (4, 4)));
    // the other modes take any tile
    assert!(!fails(0, 12, 9, (3, 3)));
}

#[test]
fn the_tc_spawns_its_villager_below_the_sheep_on_its_right() {
    let site = Site {
        position: [104.0, 47.0, 0.0],
        radius: [2.0, 2.0],
        gather: Some([107.2451, 46.075_863]),
        walls: [false; 4],
    };
    let mut tried = Vec::new();
    let p = spawn_point(&site, [0.2, 0.2], &mut |c| {
        tried.push(c);
        Ok(if c[1] > 46.9 {
            Tested::Blocked { stays: true }
        } else {
            Tested::Free
        })
    })
    .unwrap();
    assert_eq!(p, [106.5, 46.5]);
    assert_eq!(tried, [[106.5, 47.5], [106.5, 47.0], [106.5, 46.5]]);
}

#[test]
fn a_blocker_that_moves_lets_the_spot_try_its_next_candidate() {
    let spot = Spot {
        at: [1.0, 1.0],
        step: [0.5, 0.0],
        dir: [1, 0],
        count: 1,
    };
    let mut n = 0;
    let p = try_spot(&spot, &mut |_| {
        n += 1;
        Ok(if n == 1 {
            Tested::Blocked { stays: false }
        } else {
            Tested::Free
        })
    })
    .unwrap();
    assert_eq!(p, Some([1.5, 1.0]));
}

fn sheep(state: u8) -> Near {
    Near {
        id: 9520,
        position: [106.68481, 47.14652, 0.0],
        radius: [0.35, 0.35],
        tall: true,
        kind: 70,
        class: 58,
        master: 705,
        buildable_on: 0,
        obstruction_type: 5,
        auto_gather: false,
        state,
        desolid: false,
        player: 0,
        moving: true,
        fog_visibility: 0,
        fog_mask: None,
        holds: false,
        hp: 7.0,
        forest_tile: false,
    }
}

#[test]
fn livestock_stays_alive_a_tree_only_once_dead() {
    assert!(sheep(2).stays());
    assert!(!sheep(1).stays());
    let tree = Near {
        class: TREE,
        kind: 10,
        ..sheep(2)
    };
    assert!(!tree.stays());
    assert!(Near { state: 3, ..tree }.stays());
}

/// A map the player explored but does not see now, with its masks.
struct Fogged(Vec<Near>, [u64; 2]);

impl PlaceMap for Fogged {
    fn size(&self) -> [i32; 2] {
        [120, 120]
    }
    fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
        Ok(0)
    }
    fn explored(&self, _: usize, _: i32, _: i32) -> bool {
        true
    }
    fn visible(&self, _: usize, _: i32, _: i32) -> bool {
        false
    }
    fn vision(&self, _: usize) -> [u64; 2] {
        self.1
    }
    fn elevation(&self, _: i32, _: i32) -> Option<u8> {
        Some(0)
    }
    fn shape(&self, _: i32, _: i32) -> Option<u8> {
        Some(0)
    }
    fn allied(&self, a: usize, b: usize, _: bool) -> bool {
        a == b
    }
    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
        Ok(self
            .0
            .iter()
            .filter(|o| o.position[0] as i32 == x && o.position[1] as i32 == y)
            .copied()
            .collect())
    }
}

/// In the fog, of the objects without a fog visibility only the player's own block a BUILD,
/// and another player's building whose visibility mask meets both the player's masks.
#[test]
fn a_fogged_building_blocks_a_build_by_its_visibility_mask() {
    let house = Near {
        id: 500,
        position: [20.5, 20.5, 0.0],
        radius: [1.0, 1.0],
        kind: 80,
        class: 3,
        master: 70,
        obstruction_type: 2,
        moving: false,
        player: 2,
        state: 2,
        fog_mask: Some((1 << 1) | (1 << 33)),
        ..sheep(2)
    };
    let site = BuildSite {
        player: 1,
        human: true,
        clearance: [1.0, 1.0],
        class: 3,
        obstructs: true,
        restriction: &[1.0],
        hill_mode: 0,
        placement: [-1; 4],
        zones: false,
        own: &[],
    };
    let test = |o: Near, masks: [u64; 2]| {
        build_placement(&site, &Fogged(vec![o], masks), [21.0, 21.0]).unwrap()
    };
    // the house's mask meets what player 1 sees and explored: it blocks
    assert_eq!(test(house, [1 << 1, 1 << 33]), 6);
    // not what it sees: another player's house in the fog passes
    assert_eq!(test(house, [1 << 2, 1 << 33]), 0);
    // without a UMV entry neither, but its own does
    let unmasked = Near {
        fog_mask: None,
        ..house
    };
    assert_eq!(test(unmasked, [1 << 1, 1 << 33]), 0);
    assert_eq!(
        test(
            Near {
                player: 1,
                ..unmasked
            },
            [0, 0]
        ),
        6
    );
}

struct Flat(Vec<Near>);

impl PlaceMap for Flat {
    fn size(&self) -> [i32; 2] {
        [120, 120]
    }
    fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
        Ok(0)
    }
    fn explored(&self, _: usize, _: i32, _: i32) -> bool {
        true
    }
    fn visible(&self, _: usize, _: i32, _: i32) -> bool {
        true
    }
    fn vision(&self, _: usize) -> [u64; 2] {
        [0; 2]
    }
    fn elevation(&self, _: i32, _: i32) -> Option<u8> {
        Some(0)
    }
    fn shape(&self, _: i32, _: i32) -> Option<u8> {
        Some(0)
    }
    fn allied(&self, a: usize, b: usize, _: bool) -> bool {
        a == b
    }
    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
        Ok(self
            .0
            .iter()
            .filter(|o| o.position[0] as i32 == x && o.position[1] as i32 == y)
            .copied()
            .collect())
    }
}

#[test]
fn the_sheep_blocks_the_villager_where_their_boxes_overlap() {
    let villager = Placing {
        player: 1,
        radius: [0.2, 0.2],
        tall: true,
        class: 4,
        restriction: &[1.0],
        hill_mode: 0,
        placement: [-1; 4],
    };
    let map = Flat(vec![sheep(2)]);
    let at = |x, y| placement(&villager, &map, [x, y], false).unwrap();
    assert_eq!(at(106.5, 47.5), Tested::Blocked { stays: true });
    assert_eq!(at(106.5, 47.0), Tested::Blocked { stays: true });
    assert_eq!(at(106.5, 46.5), Tested::Free);
}

/// Two objects on one tile that block the point differently: the first of the tile's list
/// reports (the game's scan, `FUN_141264db0`), whichever stays.
#[test]
fn the_first_object_of_a_tile_s_list_reports_the_block() {
    let villager = Placing {
        player: 1,
        radius: [0.2, 0.2],
        tall: true,
        class: 4,
        restriction: &[1.0],
        hill_mode: 0,
        placement: [-1; 4],
    };
    // a sheep that stays (state 2) and a soldier that does not, on the same tile
    let soldier = Near {
        id: 9521,
        class: 6,
        master: 4,
        ..sheep(2)
    };
    let at = |list: Vec<Near>| placement(&villager, &Flat(list), [106.5, 47.5], false).unwrap();
    assert_eq!(at(vec![sheep(2), soldier]), Tested::Blocked { stays: true });
    assert_eq!(
        at(vec![soldier, sheep(2)]),
        Tested::Blocked { stays: false }
    );
}
