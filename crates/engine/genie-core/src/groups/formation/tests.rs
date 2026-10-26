use super::*;

/// `FUN_14102d580` copies a slot in use without its transform mark (`+0x44`).
#[test]
fn a_larger_slots_array_drops_the_transform_marks() {
    let mut f = Formation::new();
    f.resize_slots(1);
    f.use_slots(1);
    f.slots[0].unit_id = 7;
    f.slots[0].transformed = 1;
    f.resize_slots(4);
    assert_eq!((f.slots[0].unit_id, f.slots[0].transformed), (7, 0));
    assert_eq!(f.slot_capacity(), 4);
}

/// A 20 by 20 map of terrain class 1 but class 2 on `blocked`, masters 4 and 5 (category 1)
/// and 6 (category 3), and player 1's `units` (id, master, x, y) on it and on the
/// obstruction manager, all in group 1 at the origin facing x.
pub(crate) fn slot_world(units: &[(i32, i16, f32, f32)], blocked: &[(usize, usize)]) -> Turns {
    use crate::map::spatial::{Obstruction, Placed, Spatial, Tiles};
    use crate::map::terrain::{Terrain, TerrainKind};
    let objects: Vec<crate::objects::unit::Unit> = units
        .iter()
        .map(|&(id, master, x, y)| {
            let mut u = crate::objects::unit::Unit {
                id,
                position: [x, y, 0.0],
                ..crate::objects::unit::Unit::default()
            };
            u.life.master_id = master;
            u
        })
        .collect();
    let player = crate::world::Player {
        objects: objects.clone(),
        ..crate::world::Player::default()
    };
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0, 0],
        vec![crate::world::Player::default(), player],
    );
    let master = |id: i16, category: u8| crate::objects::masters::Master {
        id,
        kind: 70,
        formation_category: category,
        terrain_restriction: 0,
        radius: [0.2, 0.2],
        ..crate::objects::masters::Master::default()
    };
    let masters = crate::objects::masters::Masters::new([master(4, 1), master(5, 1), master(6, 3)]);
    t.masters = vec![masters.clone(), masters];
    let mut classes = vec![1u8; 400];
    for &(x, y) in blocked {
        classes[y * 20 + x] = 2;
    }
    t.spatial = Spatial::with_tiles([20, 20], Tiles::new(20, 20, classes));
    let kinds = vec![
        TerrainKind {
            shown: true,
            flags: 1,
            forest: false,
        },
        TerrainKind {
            shown: true,
            flags: 8,
            forest: false,
        },
    ];
    t.spatial.terrain = Some(Terrain::new(
        20,
        20,
        vec![(0, 255, -1); 400],
        kinds,
        vec![vec![1.0, 0.0]],
    ));
    for u in &objects {
        t.spatial.insert(
            u.id,
            Placed {
                position: u.position,
                radius: [0.2, 0.2],
                class: 6,
                player: 1,
                master: u.life.master_id,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(Obstruction { kind: 5, class: 2 }),
            },
        );
    }
    let mut g = UnitGroup::new(1);
    g.owner = 1;
    g.members = units
        .iter()
        .map(|&(id, ..)| crate::groups::Member {
            id,
            ..crate::groups::Member::default()
        })
        .collect();
    g.motion.position[1] = [1.0, 0.0, 0.0];
    g.motion.position[2] = [0.0, 1.0, 0.0];
    g.desired_spacing = 2.0;
    t.groups = crate::groups::UnitGroups {
        groups: vec![g],
        ..crate::groups::UnitGroups::default()
    };
    t
}

/// A slot at `at` (x, height, y) of category `category`, transformed and active.
pub(crate) fn slot_at(at: Vec3, category: i32) -> Slot {
    Slot {
        unit_id: -1,
        transform_position: at,
        desired_category: category,
        desired_sub_category: -1,
        active: 1,
        transformed: 1,
        ..Slot::default()
    }
}

/// `FUN_141016990`: along the stored positions on the ground from the front (the first
/// point): 0 is the front, 2.5 halfway along the first stretch (5 long), 8 halfway along the
/// second (6 long) at the second point's height, 11 its end; 12 is past the line, and one
/// point is no line.
#[test]
fn the_point_from_the_front_runs_along_the_stored_positions() {
    let point = |p: Vec3| crate::groups::LinePoint {
        point: p,
        ..crate::groups::LinePoint::default()
    };
    let line = crate::groups::PointLine {
        points: vec![
            point([0.0, 7.0, 0.0]),
            point([3.0, 1.0, 4.0]),
            point([3.0, 0.0, 10.0]),
        ],
        ..crate::groups::PointLine::default()
    };
    assert_eq!(find_point_from_front(&line, 0.0), Some([0.0, 7.0, 0.0]));
    assert_eq!(find_point_from_front(&line, -1.0), Some([0.0, 7.0, 0.0]));
    assert_eq!(find_point_from_front(&line, 2.5), Some([1.5, 7.0, 2.0]));
    assert_eq!(find_point_from_front(&line, 8.0), Some([3.0, 1.0, 7.0]));
    assert_eq!(find_point_from_front(&line, 11.0), Some([3.0, 1.0, 10.0]));
    assert_eq!(find_point_from_front(&line, 12.0), None);
    let one = crate::groups::PointLine {
        points: vec![point([0.0; 3])],
        ..crate::groups::PointLine::default()
    };
    assert_eq!(find_point_from_front(&one, 0.0), None);
}

/// `FUN_141011300`: a null object (none in the world's table) is -1; a ghost (its master 243
/// of type 25) is below 70, -1; a ram (class 13, master 35, of a human player) keeps its own
/// category (2), as the rams' rule never holds; a combat master with hero flag bit 4 is 5.
#[test]
fn a_unit_s_category_is_its_masters_and_none_for_a_null_object_or_a_ghost() {
    let mut t = slot_world(&[(10, 4, 1.0, 0.0), (35, 35, 2.0, 0.0)], &[]);
    for masters in &mut t.masters {
        *masters = crate::objects::masters::Masters::new([
            crate::objects::masters::Master {
                id: 4,
                kind: 70,
                formation_category: 1,
                hero_flag: 0x10,
                ..crate::objects::masters::Master::default()
            },
            crate::objects::masters::Master {
                id: 35,
                kind: 70,
                class: 13,
                formation_category: 2,
                ..crate::objects::masters::Master::default()
            },
            crate::objects::masters::Master {
                id: crate::vision::ghost::GHOST_MASTER,
                kind: crate::vision::ghost::GHOST,
                formation_category: 1,
                ..crate::objects::masters::Master::default()
            },
        ]);
    }
    t.players[1].kind = 1;
    t.players[1].ghosts = vec![crate::vision::ghost::Ghost {
        id: -1,
        original: None,
        original_id: -1,
        original_master: None,
        original_owner: 1,
        position: [3.0, 4.0, 0.0],
        tile: [3, 4],
        footprint: 0,
        bits: 0,
        cant_see: 0,
        seen: false,
        state: 2,
    }];
    assert_eq!(t.unit_category(None).unwrap(), -1);
    assert_eq!(t.unit_category(t.world_object(99)).unwrap(), -1);
    let ghost = t.world_object(-1);
    assert!(matches!(
        ghost,
        Some(crate::groups::WorldObject::Ghost(1, _))
    ));
    assert_eq!(t.unit_category(ghost).unwrap(), -1);
    assert_eq!(t.unit_category(t.world_object(35)).unwrap(), 2);
    assert_eq!(t.unit_category(t.world_object(10)).unwrap(), 5);
    // the world's table holds ghosts only in its negative half, and none for i32::MIN
    assert!(t.world_object(i32::MIN).is_none());
}

/// `FUN_14100e370` from the origin: of all, 12 (category 3) is nearest; of category 1, 11;
/// of master 4, 10; with 11 used, 10; of category 2, none.
#[test]
fn the_closest_unit_matches_its_slots_category_and_type() {
    let t = slot_world(
        &[(10, 4, 5.0, 0.0), (11, 5, 2.0, 0.0), (12, 6, 1.0, 0.0)],
        &[],
    );
    let g = t.groups.get(1).unwrap();
    let o = [0.0; 3];
    assert_eq!(t.closest_unit(g, o, -1, -1, &[]).unwrap(), 12);
    assert_eq!(t.closest_unit(g, o, 1, -1, &[]).unwrap(), 11);
    assert_eq!(t.closest_unit(g, o, 1, 4, &[]).unwrap(), 10);
    assert_eq!(t.closest_unit(g, o, 1, -1, &[11]).unwrap(), 10);
    assert_eq!(t.closest_unit(g, o, 2, -1, &[]).unwrap(), -1);
}

/// `FUN_141009760`: slots at 0 and 3 on x, units at 1 and -5. In order the first takes the
/// unit at 1 and the second the one at -5 (1 + 8); with the first unit barred from the first
/// slot, 5 + 2 is shorter, so that order wins. Units at 1 and 5 keep the plain order
/// (1 + 2 against 5 + 2).
#[test]
fn the_slots_take_their_units_in_the_shorter_of_two_orders() {
    let run = |second: f32| {
        let t = slot_world(&[(10, 4, 1.0, 0.0), (11, 4, second, 0.0)], &[]);
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
        t.populate_slots(&mut f, &g).unwrap();
        f.slots
            .iter()
            .map(|s| (s.unit_id, s.position_filled))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(-5.0), [(11, 1), (10, 1)]);
    assert_eq!(run(5.0), [(10, 1), (11, 1)]);
}

/// `FUN_14100a0c0`: slots at (0, 0) and (3, 0), member 10 at the first and 11 at
/// (1/3, 2.98), 3 from the first slot and 4 from the second. The trials of `FUN_141009760`
/// give 10 the first slot (4 in all against 6); the feature keeps the start whose squared
/// distances differ least, 11 in the first (9) and 10 in the second (9). A member the world
/// does not hold (12) keeps its row of costs (0 as the process starts) and takes a slot.
#[test]
fn the_feature_s_assignment_keeps_the_start_of_least_variance() {
    let y = (80.0f32 / 9.0).sqrt();
    let run = |feature: bool, gone: bool| {
        let mut t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 1.0 / 3.0, y)], &[]);
        if gone {
            t.groups.groups[0].members.push(crate::groups::Member {
                id: 12,
                ..crate::groups::Member::default()
            });
        }
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
        if feature {
            t.assign_slots_by_variance(&mut f, &g, &mut Vec::new())
                .unwrap();
        } else {
            t.populate_slots(&mut f, &g).unwrap();
        }
        f.slots
            .iter()
            .map(|s| (s.unit_id, s.position_filled))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(false, false), [(10, 1), (11, 1)]);
    assert_eq!(run(true, false), [(11, 1), (10, 1)]);
    assert_eq!(run(true, true), [(11, 1), (12, 1)]);
}

/// The feature's costs persist between calls (`DAT_143f161e0`): a member the world no
/// longer holds is placed by the row its last call wrote, and a cost row runs into the next
/// member's with more slots than members.
#[test]
fn the_feature_s_costs_of_a_gone_member_are_its_last() {
    let mut t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 3.0, 0.0)], &[]);
    let g = t.groups.get(1).unwrap().clone();
    let mut costs = Vec::new();
    let two = || vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
    let mut f = Formation::new();
    f.slots = two();
    t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
    assert_eq!(costs[..4], [0.0, 9.0, 9.0, 0.0]);
    // 11 leaves the world: its row (9, 0) stays and still gives it the second slot
    let _ = t.players[1].objects.remove(1);
    let mut f = Formation::new();
    f.slots = two();
    t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
    assert_eq!(f.slots[1].unit_id, 11);
    // three slots for two members: row 0 runs into row 1's first place, which row 1 writes
    let t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 3.0, 0.0)], &[]);
    let mut f = Formation::new();
    f.slots = vec![
        slot_at([0.0; 3], 1),
        slot_at([3.0, 0.0, 0.0], 1),
        slot_at([1.0, 0.0, 0.0], 1),
    ];
    let mut costs = Vec::new();
    t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
    assert_eq!(costs[..5], [0.0, 9.0, 9.0, 0.0, 4.0]);
}

/// `FUN_141009760` measures from the slot's `+0x1c`, which the load fills from the save's
/// `mTransformPosition` (0x141018637), not from `+0x10` (`mDesiredTransformPosition`).
#[test]
fn a_slot_takes_the_unit_nearest_its_transform_position() {
    let t = slot_world(&[(10, 4, 1.0, 0.0), (11, 4, 9.0, 0.0)], &[]);
    let g = t.groups.get(1).unwrap().clone();
    let mut f = Formation::new();
    let mut s = slot_at([10.0, 0.0, 0.0], 1);
    s.desired_transform_position = [0.0; 3];
    f.slots = vec![s];
    t.populate_slots(&mut f, &g).unwrap();
    assert_eq!(f.slots[0].unit_id, 11);
}

/// `FUN_141009060` after a column: a line of 3 takes its members (in order) into its places
/// from the middle out (1, 0, 2), one of 4 into (2, 0, 1, 3); with a line part-transformed,
/// or no column before, the slots take the closest units or none.
#[test]
fn a_line_after_a_column_fills_from_its_middle_out() {
    let run = |width: usize, transformed: i32, previous: i32| {
        let units: Vec<(i32, i16, f32, f32)> = (0..width)
            .map(|i| (10 + i as i32, 4, i as f32, 0.0))
            .collect();
        let mut t = slot_world(&units, &[]);
        let mut column = Formation::new();
        column.kind = previous;
        t.groups.groups[0].previous_formation = Some(column);
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.kind = LINE;
        f.maximum_width = width as i32;
        f.maximum_depth = 1;
        f.number_transformed_slots = transformed;
        f.slots = (0..width)
            .map(|i| slot_at([i as f32, 0.0, 0.0], -1))
            .collect();
        t.from_previous_column(&mut f, &g).unwrap();
        f.slots.iter().map(|s| s.unit_id).collect::<Vec<_>>()
    };
    assert_eq!(run(3, 3, COLUMN), [11, 10, 12]);
    assert_eq!(run(4, 4, COLUMN), [11, 12, 10, 13]);
    // part-transformed: each slot its closest
    assert_eq!(run(3, 2, COLUMN), [10, 11, 12]);
    assert_eq!(run(3, 3, LINE), [-1, -1, -1]);
}

/// `FUN_14103a1f0`'s group (1), target (2) and horde (3) transforms of every slot of the
/// array: the group's place plus the offset turned to the group's forward (for 1), as it is
/// (2 and 3),
/// the target's original place for 2; a turn of more than a right angle takes the units anew.
#[test]
fn the_group_target_and_horde_transforms_place_every_slot() {
    let mut t = slot_world(&[(10, 4, 3.0, 3.0)], &[]);
    t.groups.groups[0].motion.position[0] = [5.0, 0.0, 5.0];
    t.groups.groups[0].motion.position[1] = [0.0, 0.0, 1.0];
    t.groups.groups[0].current_target.original_position = [9.0, 0.0, 9.0];
    let g = t.groups.get(1).unwrap().clone();
    let run = |kind: i32| {
        let mut f = Formation::new();
        f.transform_type = kind;
        f.transform_orientation = [0.0, 0.0, 1.0];
        f.slots = vec![Slot {
            offset: [1.0, 0.0, 0.0],
            ..slot_at([0.0; 3], 1)
        }];
        f.spare_slots = vec![Slot::default()];
        let took = t.formation_transform(&mut f, &g, false).unwrap();
        (
            took,
            f.slots[0].transform_position,
            f.number_transformed_slots,
        )
    };
    let (_, turned, n) = run(1);
    assert_eq!(n, 2);
    // the offset ahead (x) turned to the group's forward (z)
    assert!(
        (turned[0] - 5.0).abs() < 1e-6 && (turned[2] - 6.0).abs() < 1e-6,
        "{turned:?}"
    );
    assert_eq!(run(3), (false, [6.0, 0.0, 5.0], 2));
    assert_eq!(run(2), (false, [10.0, 0.0, 9.0], 2));
    // facing back: the slot takes its (only) unit anew
    let mut f = Formation::new();
    f.transform_type = 3;
    f.transform_orientation = [0.0, 0.0, -1.0];
    f.slots = vec![slot_at([0.0; 3], 1)];
    assert!(t.formation_transform(&mut f, &g, false).unwrap());
    assert_eq!((f.slots[0].unit_id, f.slots[0].position_filled), (10, 1));
}

/// `FUN_14103a1f0`'s passability: slot 1 lies on a blocked tile (8, 5), behind which the
/// line from the group's place meets the tile's temporary record; its unit at (7.5, 5.5)
/// searches from its cell (x+1 blocked, then x-1) and finds slot 0's cell, so the slot takes
/// that cell's middle; below a spacing of 1 nudged by the cell's count (1: x + 0.25).
#[test]
fn an_unreachable_slot_moves_to_the_reachable_cell_its_unit_finds() {
    let run = |spacing: f32| {
        let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 5.5)], &[(8, 5)]);
        t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
        t.groups.groups[0].motion.lead = 10;
        t.groups.groups[0].desired_spacing = spacing;
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![
            Slot {
                unit_id: 10,
                ..slot_at([5.5, 0.0, 5.5], 1)
            },
            Slot {
                unit_id: 11,
                ..slot_at([8.5, 0.0, 5.5], 1)
            },
        ];
        t.passable_slots(&mut f, &g).unwrap();
        f.slots
            .iter()
            .map(|s| s.transform_position)
            .collect::<Vec<_>>()
    };
    assert_eq!(run(2.0), [[5.5, 0.0, 5.5], [5.5, 0.0, 5.5]]);
    assert_eq!(run(0.5), [[5.5, 0.0, 5.5], [5.75, 0.0, 5.5]]);
    // COMISS/JC at 0x14103cde3: a NaN spacing nudges as one below 1
    assert_eq!(run(f32::NAN), [[5.5, 0.0, 5.5], [5.75, 0.0, 5.5]]);
    // every slot standable: nothing moves
    let mut t = slot_world(&[(10, 4, 5.5, 5.5)], &[]);
    t.groups.groups[0].motion.lead = 10;
    let g = t.groups.get(1).unwrap().clone();
    let mut f = Formation::new();
    f.slots = vec![slot_at([12.5, 0.0, 5.5], 1)];
    t.passable_slots(&mut f, &g).unwrap();
    assert_eq!(f.slots[0].transform_position, [12.5, 0.0, 5.5]);
}

/// A ghost of id `id` at (`x`, `y`) in its player's doppelganger list.
pub(crate) fn ghost_at(id: i32, x: f32, y: f32) -> crate::vision::ghost::Ghost {
    crate::vision::ghost::Ghost {
        id,
        original: None,
        original_id: -1,
        original_master: None,
        original_owner: 1,
        position: [x, y, 0.0],
        tile: [x as i32, y as i32],
        footprint: 0,
        bits: 0,
        cant_see: 0,
        seen: false,
        state: 2,
    }
}

/// `FUN_1410465d0` for the constructor's lead -1 finds the world's object -1: a ghost there
/// (master 243, its restriction 1 standing on class 2 only) tests the slots, stands on none
/// of them nor on the group's place, so nothing moves; without it the first active slot's
/// unit tests them, and slot 1 moves as with that unit as the lead.
#[test]
fn the_passability_tests_with_the_world_s_object_of_the_lead_id() {
    use crate::map::terrain::{Terrain, TerrainKind};
    let run = |ghost: bool| {
        let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 5.5)], &[(8, 5)]);
        let kind = |flags: u8| TerrainKind {
            shown: true,
            flags,
            forest: false,
        };
        t.spatial.terrain = Some(Terrain::new(
            20,
            20,
            vec![(0, 255, -1); 400],
            vec![kind(1), kind(8)],
            vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        ));
        for masters in &mut t.masters {
            let ghost = crate::objects::masters::Master {
                id: crate::vision::ghost::GHOST_MASTER,
                kind: crate::vision::ghost::GHOST,
                terrain_restriction: 1,
                ..crate::objects::masters::Master::default()
            };
            *masters =
                crate::objects::masters::Masters::new(masters.iter().cloned().chain([ghost]));
        }
        if ghost {
            t.players[1].ghosts = vec![ghost_at(-1, 1.0, 1.0)];
        }
        t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
        assert_eq!(t.groups.groups[0].motion.lead, -1);
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![
            Slot {
                unit_id: 10,
                ..slot_at([5.5, 0.0, 5.5], 1)
            },
            Slot {
                unit_id: 11,
                ..slot_at([8.5, 0.0, 5.5], 1)
            },
        ];
        t.passable_slots(&mut f, &g).unwrap();
        f.slots
            .iter()
            .map(|s| s.transform_position)
            .collect::<Vec<_>>()
    };
    assert_eq!(run(false), [[5.5, 0.0, 5.5], [5.5, 0.0, 5.5]]);
    assert_eq!(run(true), [[5.5, 0.0, 5.5], [8.5, 0.0, 5.5]]);
}

/// The passability's extra step: slot 1's unit at (7.5, 7.5) finds slot 2's cell (6, 7)
/// first, which points toward the nearest slot (0, at (5.5, 5.5): -y), and the line to the
/// next cell's middle (6.5, 6.5) is clear, so the slot takes that cell.
#[test]
fn a_found_cell_steps_toward_the_nearest_slot_when_the_way_is_clear() {
    let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 7.5)], &[(8, 5)]);
    t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
    t.groups.groups[0].motion.lead = 10;
    let g = t.groups.get(1).unwrap().clone();
    let mut f = Formation::new();
    f.slots = vec![
        Slot {
            unit_id: 10,
            ..slot_at([5.5, 0.0, 5.5], 1)
        },
        Slot {
            unit_id: 11,
            ..slot_at([8.5, 0.0, 5.5], 1)
        },
        slot_at([6.5, 0.0, 7.5], 1),
    ];
    t.passable_slots(&mut f, &g).unwrap();
    assert_eq!(f.slots[1].transform_position, [6.5, 0.0, 6.5]);
    assert_eq!(f.slots[2].transform_position, [6.5, 0.0, 7.5]);
}

/// The passability's box and nearest slot as the assembly takes them: a NaN place keeps the
/// box's old bounds (`MINSS`/`MAXSS` of the new value first) and the earlier nearest slot
/// (`COMISS`/`CMOVBE`); an inactive slot counts for neither.
#[test]
fn the_passability_box_keeps_its_bounds_and_nearest_for_a_nan_place() {
    let nan = [f32::NAN, 0.0, f32::NAN];
    let mut idle = slot_at([50.0, 0.0, 50.0], 1);
    idle.active = 0;
    let slots = [
        slot_at([4.0, 0.0, 6.0], 1),
        slot_at(nan, 1),
        idle,
        slot_at([3.0, 0.0, 2.0], 1),
    ];
    let (bounds, nearest) = passability_box(&slots, [5.0, 0.0, 5.0], [3.0, 0.0, 3.0]);
    assert_eq!(bounds, [3.0, 2.0, 5.0, 6.0]);
    assert_eq!(nearest, Some(3));
    let (_, nearest) = passability_box(&slots[..2], [5.0, 0.0, 5.0], [3.0, 0.0, 3.0]);
    assert_eq!(nearest, Some(0));
    // a NaN tester place keeps the starting bounds for its part too
    let (bounds, _) = passability_box(&slots[..1], nan, [3.0, 0.0, 3.0]);
    assert_eq!(bounds, [4.0, 6.0, 4.0, 6.0]);
    assert_eq!(passability_box(&slots[1..2], nan, [0.0; 3]).1, None);
}

/// A reachable slot's cell mark: along y for `|dx| <= |dz|` and for a NaN (bit 7 for a `dz`
/// below 0 or NaN), else along x (bits 6 and 7 for a `dx` below 0 or NaN, bit 6 else); the
/// count goes up and bit 0 is set.
#[test]
fn a_cell_mark_points_toward_the_nearest_slot_and_a_nan_along_y() {
    assert_eq!(cell_mark(0, 0.0, 2.0), 3);
    assert_eq!(cell_mark(0, 0.0, -2.0), 0x83);
    assert_eq!(cell_mark(0, 3.0, 1.0), 0x43);
    assert_eq!(cell_mark(0, -3.0, 1.0), 0xc3);
    assert_eq!(cell_mark(0xc3, 0.0, 2.0), 5);
    assert_eq!(cell_mark(0, f32::NAN, 2.0), 3);
    assert_eq!(cell_mark(0, 3.0, f32::NAN), 0x83);
    assert_eq!(cell_mark(0, f32::NAN, f32::NAN), 0x83);
    // a slot's tile by CVTTSS2SI: a NaN place is off any map
    assert_eq!(slot_tile([f32::NAN, 0.0, 2.5]), (i32::MIN, 2));
}

/// The line transform's distance back (0x14103a850, `COMISS`/`JA`): a line's own when it is
/// above 0, else, a NaN too, the line's index times the spacing.
#[test]
fn a_nan_distance_back_takes_the_index_times_the_spacing() {
    let t = slot_world(&[(10, 4, 5.0, 5.0)], &[]);
    let mut g = t.groups.get(1).unwrap().clone();
    g.motion.position[0] = [9.0, 0.0, 5.0];
    g.desired_spacing = 2.0;
    let mut f = Formation::new();
    f.transform_type = 0;
    f.number_lines = 2;
    f.slots = vec![slot_at([0.0; 3], 1), slot_at([0.0; 3], 1)];
    f.lines = vec![
        FormationLine {
            indices: vec![0],
            ..FormationLine::default()
        },
        FormationLine {
            indices: vec![1],
            distance_back: f32::NAN,
            ..FormationLine::default()
        },
    ];
    let _ = t.formation_transform(&mut f, &g, false).unwrap();
    assert_eq!(f.slots[1].transform_position, [7.0, 0.0, 5.0]);
}

/// `FUN_141041810`: every slot of the array, spare ones too, loses its unit and filled mark.
#[test]
fn clearing_the_units_empties_every_slot_of_the_array() {
    let mut f = Formation::new();
    let taken = Slot {
        unit_id: 7,
        position_filled: 1,
        ..Slot::default()
    };
    f.slots = vec![taken];
    f.spare_slots = vec![taken];
    f.clear_units();
    assert!(
        f.slots
            .iter()
            .chain(&f.spare_slots)
            .all(|s| (s.unit_id, s.position_filled) == (-1, 0))
    );
}

/// The grid's count (bits 1 to 5) counts on and wraps, the other bits stay.
#[test]
fn a_grid_cell_counts_its_slots_in_bits_1_to_5() {
    assert_eq!(count_up(0), 2);
    assert_eq!(count_up(3), 5);
    assert_eq!(count_up(0xc1 | 0x3e), 0xc1);
}

/// One statistic of category 1 holding `units`, all of master 4, spacing 1.
fn one_statistic(t: &mut Turns, units: &[i32]) {
    let mut unit_types = vec![0u32; 50];
    unit_types[0] = 4;
    let mut counts = vec![0u32; 50];
    counts[0] = units.len() as u32;
    let g = &mut t.groups.groups[0];
    g.desired_spacing = 1.0;
    g.unit_types = vec![crate::groups::UnitTypeStatistic {
        formation_category: 1,
        units: units.to_vec(),
        desired_spacing: 1.0,
        number_unit_types: 1,
        unit_types,
        unit_type_counts: counts,
        ..crate::groups::UnitTypeStatistic::default()
    }];
}

/// `FUN_141012280` lays its slots, transforms them and takes the units, the column-to-line
/// flag set for a line after a column (0x141013d20..0x141013d3a): a line of three after a
/// column takes its members from the middle out (11, 10, 12), after a line the closest
/// (10, 11, 12); each as its slots and `lay_units` with that flag.
#[test]
fn a_line_after_a_column_takes_its_units_in_the_column_s_order() {
    let run = |previous: i32| {
        let mut t = slot_world(
            &[(10, 4, 5.5, 5.5), (11, 4, 5.5, 6.5), (12, 4, 5.5, 7.5)],
            &[],
        );
        one_statistic(&mut t, &[10, 11, 12]);
        t.groups.groups[0].motion.position[0] = [6.5, 0.0, 6.5];
        let mut before = Formation::new();
        before.kind = previous;
        t.groups.groups[0].previous_formation = Some(before);
        let g = t.groups.get(1).unwrap().clone();
        let mut whole = Formation::new();
        assert!(
            t.create_column_line(&mut whole, &g, false, (false, false))
                .unwrap()
        );
        let mut parts = Formation::new();
        assert!(column_line_slots(
            &mut parts,
            &g,
            false,
            (false, false),
            t.world_time
        ));
        t.lay_units(&mut parts, &g, previous == COLUMN).unwrap();
        assert_eq!(whole, parts);
        whole.slots.iter().map(|s| s.unit_id).collect::<Vec<_>>()
    };
    assert_eq!(run(COLUMN), [11, 10, 12]);
    assert_eq!(run(LINE), [10, 11, 12]);
}

/// A path level of `points` points at waypoint `waypoint` (none without points).
fn level(points: &[Vec3], waypoint: i32, flags: u32) -> PathData {
    PathData {
        level: 0,
        linked_type: 0,
        points: points.to_vec(),
        has_path: !points.is_empty(),
        path_flags: flags,
        waypoint,
        state: 0,
        range: 0.0,
        target: -1,
        pause_time: 0.0,
        flags: 0,
        length: 0.0,
        created: 0,
        continue_counter: 10,
        repath_time: 0.0,
        ignore: Vec::new(),
        ignored: Vec::new(),
    }
}

/// Player 1 (minimum column distance 10, column-to-line 5) with unit 7 of a soldier master
/// (type 70, category 1) in group 1 at the origin along `paths` (walked first).
fn type_world(paths: Vec<PathData>) -> Turns {
    let mut u = crate::objects::unit::Unit {
        id: 7,
        ..crate::objects::unit::Unit::default()
    };
    u.life.master_id = 4;
    let player = crate::world::Player {
        objects: vec![u],
        formations: crate::world::Formations {
            min_column_distance: 10,
            column_to_line_distance: 5,
            ..crate::world::Formations::default()
        },
        ..crate::world::Player::default()
    };
    let mut t = Turns::new(
        0,
        0,
        0,
        vec![0, 0],
        vec![crate::world::Player::default(), player],
    );
    let masters = crate::objects::masters::Masters::new([crate::objects::masters::Master {
        id: 4,
        kind: 70,
        formation_category: 1,
        ..crate::objects::masters::Master::default()
    }]);
    t.masters = vec![masters.clone(), masters];
    let mut g = UnitGroup::new(1);
    g.owner = 1;
    g.members = vec![crate::groups::Member {
        id: 7,
        ..crate::groups::Member::default()
    }];
    g.motion.paths = paths;
    t.groups = crate::groups::UnitGroups {
        groups: vec![g],
        ..crate::groups::UnitGroups::default()
    };
    t
}

fn own(t: &Turns) -> i32 {
    t.own_formation_type(t.groups.get(1).unwrap()).unwrap()
}

/// `FUN_141048180`: the top ancestor (the last level) on its last waypoint; a lone level has
/// no ancestor, a top without a path is not on it.
#[test]
fn the_last_high_level_waypoint_is_the_top_levels_last_point() {
    let top = level(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], 2, 0);
    let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
    assert!(on_last_high_level_waypoint(&[walked.clone(), top.clone()]));
    assert!(!on_last_high_level_waypoint(std::slice::from_ref(&top)));
    let mut early = top;
    early.waypoint = 1;
    assert!(!on_last_high_level_waypoint(&[walked.clone(), early]));
    assert!(!on_last_high_level_waypoint(&[walked, level(&[], 5, 0)]));
}

/// `UpdateDesiredPositions`'s own choice for a soldier: a line without a path, a column off
/// the last high waypoint or on a patrol, and on it a column only beyond both distances
/// (measured on the ground to the path's last point): 30 is beyond 10 and 5, 8 within the
/// minimum column distance, whatever the current type; a target's radius and range lengthen
/// the column-to-line distance.
#[test]
fn a_group_chooses_a_column_far_along_its_path_and_a_line_near_its_end() {
    assert_eq!(own(&type_world(Vec::new())), LINE);
    let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
    let path_to = |x: f32, waypoint: i32, flags: u32| {
        vec![
            walked.clone(),
            level(&[[0.0; 3], [x, 9.0, 0.0]], waypoint, flags),
        ]
    };
    assert_eq!(own(&type_world(path_to(30.0, 0, 0))), COLUMN);
    assert_eq!(own(&type_world(path_to(30.0, 1, 1))), COLUMN);
    assert_eq!(own(&type_world(path_to(30.0, 1, 0))), COLUMN);
    assert_eq!(own(&type_world(path_to(8.0, 1, 0))), LINE);
    // a column (whose type the game tests first) turns to a line within 10 too; at 12 it
    // stays one until a target's radius and range lengthen 5 past 12
    let mut t = type_world(path_to(8.0, 1, 0));
    let mut column = Formation::new();
    column.kind = COLUMN;
    t.groups.groups[0].current_formation = Some(column);
    assert_eq!(own(&t), LINE);
    t.groups.groups[0].motion.paths = path_to(12.0, 1, 0);
    assert_eq!(own(&t), COLUMN);
    let g = &mut t.groups.groups[0];
    g.current_target.target = 3;
    g.current_target.target_radius = 5.0;
    g.current_target.range = 2.5;
    assert_eq!(own(&t), LINE);
    // a lone level measures its own path; one point measures to the origin
    let mut t = type_world(vec![level(&[[30.0, 0.0, 0.0]], 0, 0)]);
    t.groups.groups[0].motion.position[0] = [40.0, 0.0, 0.0];
    assert_eq!(own(&t), COLUMN);
}

/// Flags bit 2 (`MinAutoDistanceLock`) choose a line, horde units only a horde; given a
/// type, or locked to one (not -1 or 0), the group takes it.
#[test]
fn a_lock_or_a_given_type_overrides_the_groups_own_choice() {
    let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
    let far = vec![walked, level(&[[0.0; 3], [30.0, 0.0, 0.0]], 1, 0)];
    let mut t = type_world(far);
    t.groups.groups[0].flags |= 4;
    assert_eq!(own(&t), LINE);
    t.masters[1] = crate::objects::masters::Masters::new([crate::objects::masters::Master {
        id: 4,
        kind: 70,
        ..crate::objects::masters::Master::default()
    }]);
    assert_eq!(own(&t), HORDE);
    let g = t.groups.get(1).unwrap().clone();
    assert_eq!(t.formation_type(&g, BOX).unwrap(), BOX);
    let mut locked = g;
    locked.formation_locked_type = STAGGERED;
    assert_eq!(t.formation_type(&locked, -1).unwrap(), STAGGERED);
    locked.formation_locked_type = 0;
    assert_eq!(t.formation_type(&locked, -1).unwrap(), HORDE);
}

/// createColumnLine's sizes: a line at least twice as wide as it is deep, a column the other
/// way round.
#[test]
fn a_line_is_twice_as_wide_as_deep_and_a_column_the_reverse() {
    assert_eq!(sizes(1, true), (1, 1));
    // root 4: 10 / 4 = 2 lines of 5
    assert_eq!(sizes(10, true), (2, 5));
    // root 3: 9 / 3 = 3 lines of 3, too deep: 2 lines of 5
    assert_eq!(sizes(9, true), (2, 5));
    assert_eq!(sizes(16, true), (3, 6));
    assert_eq!(sizes(10, false), (5, 2));
}

fn group_of(category: i32, count: usize, spacing: f32, types: &[(u32, u32)]) -> UnitGroup {
    let mut unit_types = vec![0u32; 50];
    let mut counts = vec![0u32; 50];
    for (i, &(t, n)) in types.iter().enumerate() {
        unit_types[i] = t;
        counts[i] = n;
    }
    UnitGroup {
        members: (0..count)
            .map(|i| crate::groups::Member {
                id: i as i32,
                ..crate::groups::Member::default()
            })
            .collect(),
        unit_types: vec![crate::groups::UnitTypeStatistic {
            formation_category: category,
            units: (0..count as i32).collect(),
            desired_spacing: spacing,
            number_unit_types: types.len() as i32,
            unit_types,
            unit_type_counts: counts,
            ..crate::groups::UnitTypeStatistic::default()
        }],
        ..UnitGroup::default()
    }
}

/// Ten units of two types in a line: two lines of five, centred (an odd line starts two
/// offsets forward), the types alternating with the second line starting on the type the
/// first ended with.
#[test]
fn a_line_of_two_unit_types_alternates_its_slots() {
    let g = group_of(3, 10, 1.0, &[(4, 5), (24, 5)]);
    let mut f = Formation::new();
    assert!(column_line_slots(&mut f, &g, false, (false, false), 77));
    assert_eq!((f.kind, f.maximum_width, f.maximum_depth), (LINE, 5, 2));
    assert_eq!(f.slots.len(), 10);
    assert_eq!(f.slots[0].offset, [0.0, 0.0, -2.0]);
    assert_eq!(f.slots[4].offset, [0.0, 0.0, 2.0]);
    let subs: Vec<i32> = f.slots.iter().map(|s| s.desired_sub_category).collect();
    assert_eq!(subs, [4, 24, 4, 24, 4, 4, 24, 4, 24, 24]);
    assert_eq!(f.lines[1].distance_back, 1.0);
    assert_eq!(f.lines[1].indices, [5, 6, 7, 8, 9]);
}
