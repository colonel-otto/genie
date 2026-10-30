use super::*;
use crate::objects::masters::class;
use crate::vision::registry::UnitUpdate;

fn world(players: usize) -> Visibility {
    let n = 120 * 120;
    Visibility::new(
        120,
        120,
        vec![
            VisibleMap {
                counts: vec![-1; n],
                explored: 0,
            };
            players
        ],
        vec![0; n],
        Vec::new(),
        0,
        vec![false; players],
    )
}

fn unit(player: usize, los: f32) -> Seer {
    Seer {
        player,
        master: 83,
        los,
        state: 2,
        explored: true,
        building: None,
        umv: 0,
        capture: 0,
        kind: 70,
    }
}

fn resources(lists: usize) -> VisibleResources {
    VisibleResources {
        capacities: vec![2; lists],
        lists: vec![Vec::new(); lists],
    }
}

fn rec(id: i32, x: f32, y: f32) -> VisibleResource {
    VisibleResource {
        id,
        x,
        y,
        ..VisibleResource::default()
    }
}

/// `RGE_Combat_Object::update`'s takeover by sight: the first player from 1 on but the owner
/// whose bit the mask holds; outside the standard modes a computer player takes over only
/// when it allows taking Gaia's units.
#[test]
fn the_first_other_player_that_sees_the_object_takes_it_over() {
    use crate::world::player::Player;
    let players = |computers: [bool; 4]| {
        computers.map(|computer| Player {
            computer,
            ..Player::default()
        })
    };
    let humans = players([false; 4]);
    let none = [false; 4];
    assert_eq!(sight_taker(0b0111, 0, true, &humans, &none), Some(1));
    assert_eq!(sight_taker(0b0110, 1, true, &humans, &none), Some(2));
    assert_eq!(sight_taker(0b0001, 1, true, &humans, &none), None);
    assert_eq!(sight_taker(1 << 33 | 0b10, 1, true, &humans, &none), None);
    let ai = players([false, true, false, true]);
    let allows = [false, false, false, true];
    assert_eq!(sight_taker(0b1110, 0, false, &ai, &allows), Some(2));
    assert_eq!(sight_taker(0b1010, 0, false, &ai, &allows), Some(3));
    assert_eq!(sight_taker(0b0010, 0, false, &ai, &allows), None);
    assert_eq!(sight_taker(0b0010, 0, true, &ai, &allows), Some(1));
}

/// The combat object's sight answers a changed mask only for an object others take over by
/// seeing it (`+0x924` 1), once the registry took it.
#[test]
fn a_changed_mask_is_the_answer_for_an_object_taken_over_by_sight() {
    let mut v = world(3);
    v.registry = Some(Registry::new(3, 65));
    let mut gaia = unit(0, 2.0);
    gaia.capture = 1;
    v.add_seer(7, gaia);
    v.add_seer(8, unit(0, 2.0));
    let i = 10 * 120 + 10;
    v.unified[i] = 0b011;
    let at = [10.5, 10.5, 0.0];
    assert_eq!(
        v.combat_sight(7, 0, at, [0.2, 0.2], class::CLASS_6, None),
        Ok(Some(0b011))
    );
    assert_eq!(
        v.combat_sight(7, 0, at, [0.2, 0.2], class::CLASS_6, None),
        Ok(None)
    );
    assert_eq!(
        v.combat_sight(8, 0, at, [0.2, 0.2], class::CLASS_6, None),
        Ok(None)
    );
    let r = v.registry.as_ref().unwrap();
    assert_eq!(r.entry(8).map(|e| e.mask), Some(0b011));
    assert_eq!(r.lists[1][6].len(), 2);
}

/// `FUN_14120f800`: an object with an id writes the new owner into its records in every
/// list (`+0x8b0`..); one without an id has none to write.
#[test]
fn an_ownership_change_writes_the_new_owner_into_the_object_s_records() {
    let mut v = world(3);
    let mut r = Registry::new(3, 65);
    r.entries.entry(7).or_default().mask = 0b11;
    r.update_unit(UnitUpdate {
        id: 7,
        owner: 0,
        position: [10.5, 10.5],
        radii: [0.2, 0.2],
        class: class::CLASS_6,
        old: 0,
        new: 0b11,
    })
    .unwrap();
    v.registry = Some(r);
    v.records_owner(7, 2).unwrap();
    v.records_owner(-5, 2).unwrap();
    let r = v.registry.as_ref().unwrap();
    assert_eq!([r.lists[0][6][0].owner, r.lists[1][6][0].owner], [2, 2]);
    assert_eq!(r.lists[2][6].len(), 0);
}

/// `FUN_14120f800`'s sight: an object that sees loses its sight on the old owner's map
/// (slot 0x1d8) and gains it, with its new master's line of sight, on the new one's (slot
/// 0x1d0); one that does not see (inside another, or not in state 2) changes owner and line
/// of sight alone.
#[test]
fn an_ownership_change_moves_the_sight_of_an_object_that_sees() {
    let at = [10.5, 10.5, 0.0];
    let mut v = world(3);
    v.add_seer(7, unit(0, 2.0));
    let _ = v.sight_on(7, at, [0.2, 0.2], false, false).unwrap();
    let tile = |x: i32, y: i32| (y * 120 + x) as usize;
    assert_eq!(v.maps[0].counts[tile(12, 10)], 1);
    v.owner_changed(7, at, [0.2, 0.2], (1, 3.0), true).unwrap();
    assert_eq!(v.maps[0].counts[tile(12, 10)], 0);
    assert_eq!(v.maps[1].counts[tile(13, 10)], 1);
    assert_eq!(v.maps[1].counts[tile(14, 10)], -1);
    let s = v.seer(7).copied().unwrap();
    assert_eq!((s.player, s.los), (1, 3.0));
    v.owner_changed(7, at, [0.2, 0.2], (2, 4.0), false).unwrap();
    assert_eq!(v.maps[1].counts[tile(13, 10)], 1);
    assert_eq!(v.maps[2].counts[tile(10, 10)], -1);
    let s = v.seer(7).copied().unwrap();
    assert_eq!((s.player, s.los), (2, 4.0));
}

/// `AddResource` (`FUN_141288830`): the record goes last in its group's list at the object's
/// place, its distance and byte 0; a full list doubles its capacity. A manager without lists
/// adds nothing.
#[test]
fn a_resource_goes_last_in_its_group_s_list_and_a_full_list_doubles() {
    let mut r = resources(5);
    r.add(10, 4, 3.5, 7.5);
    r.add(11, 4, 1.5, 2.5);
    assert_eq!(r.capacities[4], 2);
    r.add(12, 4, 0.5, 0.5);
    assert_eq!(r.capacities, [2, 2, 2, 2, 4]);
    assert_eq!(
        r.lists[4],
        [rec(10, 3.5, 7.5), rec(11, 1.5, 2.5), rec(12, 0.5, 0.5)]
    );
    assert!(r.lists[..4].iter().all(Vec::is_empty));
    let mut none = VisibleResources::default();
    none.add(10, 0, 1.0, 1.0);
    assert_eq!(none, VisibleResources::default());
}

/// `Remove_Resource` (`FUN_141289b80`): the first record of the object takes the last one's
/// record and the list shortens; its index, 0 for an empty list, -1 when not found.
#[test]
fn a_removed_resource_s_place_takes_the_last_record() {
    let mut r = resources(5);
    for id in [1, 2, 3] {
        r.add(id, 0, id as f32, 0.0);
    }
    assert_eq!(r.remove(1, 0), 0);
    assert_eq!(r.lists[0], [rec(3, 3.0, 0.0), rec(2, 2.0, 0.0)]);
    assert_eq!(r.remove(2, 0), 1);
    assert_eq!(r.lists[0], [rec(3, 3.0, 0.0)]);
    assert_eq!(r.remove(9, 0), -1);
    assert_eq!(r.remove(9, 1), 0);
    assert_eq!(VisibleResources::default().remove(3, 0), -1);
}

/// `Get_Closest_Resource` (`FUN_141289000`): the group's record nearest the point (squared,
/// the first of equals), past the ids to skip, takes its distance; none for a group out of
/// range or an empty list.
#[test]
fn the_closest_resource_skips_the_ids_given_and_keeps_its_distance() {
    let mut r = resources(5);
    r.add(1, 2, 13.0, 10.0);
    r.add(2, 2, 10.0, 12.0);
    r.add(3, 2, 7.0, 10.0);
    assert_eq!(r.closest(10.0, 10.0, 2, &[]), Some(1));
    assert_eq!(r.lists[2][1].distance, 2.0);
    assert_eq!(r.closest(10.0, 10.0, 2, &[2]), Some(0));
    assert_eq!(r.lists[2][0].distance, 3.0);
    assert_eq!(r.closest(10.0, 10.0, 2, &[2, 1]), Some(2));
    assert_eq!(r.closest(10.0, 10.0, 2, &[1, 2, 3]), None);
    assert_eq!(r.closest(10.0, 10.0, 1, &[]), None);
    assert_eq!(r.closest(10.0, 10.0, 5, &[]), None);
    assert_eq!(r.closest(10.0, 10.0, -1, &[]), None);
}

/// The tiles a player's map explores go to its tile list (player `+0x180`, `FUN_14165d500`)
/// in order: the circle's (`explore_terrain`) and a rectangle's (`explore_terrain_sq`) newly
/// explored tiles, a revealed rectangle's never explored ones (`reveal_terrain_sq`); the
/// player's update ends with the list cleared (`FUN_141661030`).
#[test]
fn newly_explored_tiles_join_the_player_s_tile_list_until_its_update_ends() {
    let mut v = world(2);
    let _ = v.explore(1, 10, 10, 0, false).unwrap();
    let _ = v.explore(1, 10, 10, 1, false).unwrap();
    assert_eq!(
        v.new_tiles[1],
        [
            [10, 10],
            [9, 9],
            [10, 9],
            [11, 9],
            [9, 10],
            [11, 10],
            [9, 11],
            [10, 11],
            [11, 11]
        ]
    );
    let _ = v.explore_rect(1, 11, 9, 12, 9).unwrap();
    let _ = v.reveal_rect(1, 11, 8, 12, 8).unwrap();
    assert_eq!(v.new_tiles[1][9..], [[12, 9], [11, 8], [12, 8]]);
    assert_eq!(v.new_tiles[0], Vec::<[i32; 2]>::new());
    v.clear_new_tiles(1);
    assert_eq!(v.new_tiles[1], Vec::<[i32; 2]>::new());
}

/// `FUN_1411980b0`'s lists: each player whose map explored the object's tile (its bit 32 + p
/// of the unified map, from 0 while the player exists) loses the object from its group's list.
#[test]
fn a_resource_leaves_the_lists_of_the_players_that_explored_its_tile() {
    let mut v = world(3);
    v.resources = vec![resources(5), resources(5), resources(5)];
    for r in &mut v.resources {
        r.add(7, 1, 20.5, 30.5);
    }
    let _ = v.explore(2, 20, 30, 0, false).unwrap();
    v.forget_resource(7, [20.5, 30.5, 0.0], 1).unwrap();
    assert_eq!(v.resources[2].lists[1], []);
    assert_eq!(v.resources[0].lists[1], [rec(7, 20.5, 30.5)]);
    assert_eq!(v.resources[1].lists[1], [rec(7, 20.5, 30.5)]);
}

#[test]
fn a_sight_counts_its_circle_explored_then_visible_and_leaves_it_explored() {
    let mut v = world(3);
    assert_eq!(v.explore(2, 10, 10, 1, false).unwrap(), 9);
    assert_eq!(v.maps[2].explored, 9);
    let i = v.index(10, 10);
    assert_eq!(v.maps[2].counts[i], 1);
    assert_eq!(v.unified[i], 1 << 2 | 1 << 34);
    assert_eq!(v.explore(2, 11, 10, 1, false).unwrap(), 3);
    assert_eq!(v.maps[2].counts[i], 2);
    v.unexplore(2, 10, 10, 1, false).unwrap();
    assert_eq!(v.maps[2].counts[i], 1);
    v.unexplore(2, 11, 10, 1, false).unwrap();
    assert_eq!(v.maps[2].counts[i], 0);
    assert_eq!(v.unified[i], 1 << 34, "explored, no longer visible");
    assert_eq!(v.maps[2].explored, 12);
    // a tile at 0 stays 0; one never explored goes to -2
    v.unexplore(2, 10, 10, 0, false).unwrap();
    assert_eq!(v.maps[2].counts[i], 0);
    v.unexplore(2, 20, 20, 0, false).unwrap();
    assert_eq!(v.maps[2].counts[v.index(20, 20)], -2);
}

#[test]
fn a_unit_s_sight_moves_with_its_tile_and_shrinks_to_its_footprint_when_it_dies() {
    let mut v = world(3);
    v.add_seer(7, unit(1, 2.0));
    let r = [0.5, 0.5];
    let _ = v.sight_on(7, [5.5, 5.5, 0.0], r, false, false).unwrap();
    let before = v.maps[1].counts.clone();
    v.moved(7, [5.5, 5.5, 0.0], [5.9, 5.2, 0.0], r).unwrap();
    assert_eq!(v.maps[1].counts, before, "the same tile");
    v.moved(7, [5.9, 5.2, 0.0], [6.1, 5.2, 0.0], r).unwrap();
    assert_eq!(v.maps[1].counts[v.index(8, 5)], 1);
    assert_eq!(v.maps[1].counts[v.index(3, 5)], 0);
    v.state_changed(7, [6.1, 5.2, 0.0], r, 3).unwrap();
    let seen: Vec<_> = (0..120 * 120)
        .filter(|&i| 0 < v.maps[1].counts[i])
        .collect();
    assert_eq!(
        seen,
        vec![v.index(5, 4), v.index(6, 4), v.index(5, 5), v.index(6, 5)]
    );
    v.state_changed(7, [6.1, 5.2, 0.0], r, 7).unwrap();
    assert!(v.maps[1].counts.iter().all(|&c| c <= 0));
}

#[test]
fn the_extent_list_matches_the_saved_entries_of_a_town_centre_and_a_house() {
    let mut v = world(1);
    assert_eq!(v.extent_entry(29, 94, 27, 92, 30, 95), 1);
    assert_eq!(v.extents[0].guid, 0x0101_0202);
    assert_eq!(
        v.extents[0].offsets,
        vec![[-2, -2], [1, -2], [-2, 1], [1, 1]]
    );
    assert_eq!(v.extent_entry(49, 87, 48, 86, 49, 87), 2);
    assert_eq!(v.extents[1].guid, 257);
    assert_eq!(v.extents[1].offsets, vec![[-1, -1], [0, -1], [-1, 0]]);
    assert_eq!(v.extent_entry(10, 10, 9, 10, 10, 10), 3);
    assert_eq!(v.extents[2].offsets, vec![[-1, 0]]);
    assert_eq!(v.extent_entry(30, 95, 28, 93, 31, 96), 1, "found again");
    assert_eq!(v.extents_capacity, 8);
    // 5 × 5 about its centre: the corners, then a step of 2 along the rows and the columns
    assert_eq!(v.extent_entry(10, 10, 8, 8, 12, 12), 4);
    assert_eq!(v.extents[3].guid, 0x0202_0202);
    assert_eq!(
        v.extents[3].offsets,
        vec![
            [-2, -2],
            [2, -2],
            [-2, 2],
            [2, 2],
            [0, -2],
            [0, 2],
            [-2, 0],
            [2, 0]
        ]
    );
    assert_eq!(v.extents[3].capacity, 9);
    // one tile wide and three high
    assert_eq!(v.extent_entry(10, 10, 10, 9, 10, 11), 5);
    assert_eq!(v.extents[4].offsets, vec![[0, -1], [0, 1]]);
    // off the map, or one tile: none
    assert_eq!(v.extent_entry(200, 10, 200, 10, 201, 11), 0);
    assert_eq!(v.extent_entry(10, 200, 10, 200, 11, 201), 0);
    assert_eq!(v.extent_entry(-5, 10, -5, 10, -4, 11), 0);
    assert_eq!(v.extent_entry(10, -5, 10, -5, 11, -4), 0);
    assert_eq!(v.extent_entry(10, 10, 10, 10, 10, 10), 0);
    // the list grows by 8 once full
    for w in 1..5 {
        let _ = v.extent_entry(60, 60, 60, 60, 60 + w, 61);
    }
    assert_eq!((v.extents.len(), v.extents_capacity), (9, 16));
}

#[test]
fn a_first_sight_of_a_building_s_tile_explores_the_building() {
    let mut v = world(3);
    assert_eq!(v.link_building([29.0, 94.0, 0.0], [2.0, 2.0]).unwrap(), 1);
    assert_eq!(v.link(29, 94).unwrap(), (ANCHOR, 1));
    assert_eq!(v.link(28, 93).unwrap(), (LINKED, 0xff));
    assert_eq!(v.link(30, 95).unwrap(), (LINKED, 0x11));
    assert_eq!(v.anchor_of(27, 92), Some([29, 94]));
    assert_eq!(v.extent_of(30, 92), Some([27, 92, 30, 95]));
    // a sight of LOS 0 on the building's corner tile
    assert_eq!(v.explore(1, 27, 92, 0, false).unwrap(), 1);
    assert_eq!(v.maps[1].counts[v.index(27, 92)], 1);
    assert_eq!(v.maps[1].counts[v.index(30, 95)], 0);
    assert_eq!(v.maps[1].explored, 16);
    v.unlink_building([29.0, 94.0, 0.0], [2.0, 2.0]).unwrap();
    assert_eq!(v.link(29, 94).unwrap(), (0, 0));
}

#[test]
fn a_building_sees_its_footprint_while_built_then_its_circle_from_a_tile_corner() {
    let mut v = world(3);
    let site = Seer {
        master: 70,
        explored: false,
        state: 0,
        building: Some(BuildingSight::default()),
        ..unit(2, 2.0)
    };
    v.add_seer(9, site);
    let (at, r) = ([10.0, 10.0, 0.0], [1.0, 1.0]);
    assert_eq!(
        v.sight_on(9, at, r, false, false).unwrap(),
        0,
        "not started"
    );
    v.sight_off(9, at, r, true, false).unwrap();
    assert!(v.maps[2].counts.iter().all(|&c| c == -1));
    v.construction_started(9, at, r).unwrap();
    let seen = |v: &Visibility| (0..120 * 120).filter(|&i| 0 < v.maps[2].counts[i]).count();
    assert_eq!(seen(&v), 4);
    v.seer_mut(9).unwrap().building.as_mut().unwrap().built = true;
    v.sight_off(9, at, r, true, false).unwrap();
    let _ = v.sight_on(9, at, r, false, false).unwrap();
    // the table of a tile corner: at LOS 2 a 4 by 4 square around the corner
    assert_eq!(seen(&v), 16);
    assert_eq!(v.maps[2].counts[v.index(8, 9)], 1);
    assert_eq!(v.maps[2].counts[v.index(11, 9)], 1);
}

/// A computer player's first sight of a tile queues it for its AI module (slot 0x88), once per
/// tile; a human player's sight queues nothing.
#[test]
fn a_computer_players_newly_explored_tiles_are_queued_for_its_ai_module() {
    let mut v = world(3);
    v.computer = vec![false, true, false];
    assert_eq!(v.explore(1, 10, 10, 0, false), Ok(1));
    assert_eq!(v.explore(1, 10, 10, 0, false), Ok(0));
    assert_eq!(v.explore(2, 20, 20, 0, false), Ok(1));
    assert_eq!(v.take_ai_tiles(), vec![(1, [10, 10])]);
    assert_eq!(v.take_ai_tiles(), vec![]);
}
