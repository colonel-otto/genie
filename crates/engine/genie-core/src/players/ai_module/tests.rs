use crate::movement::explore::InfluenceMap;
use crate::players::ai_module::{AiModule, AiModules, InfoModule, SLOTS, TacticalModule};

fn module() -> AiModule {
    AiModule {
        info: InfoModule {
            width: 4,
            height: 3,
            path_map: InfluenceMap::new([4, 3], 0xff),
            explorer: None,
            search: Vec::new(),
        },
        tactical: TacticalModule {
            slot_times: vec![7, 8],
        },
    }
}

#[test]
fn an_explored_tile_on_the_map_writes_zero_into_the_path_map() {
    let mut m = module();
    m.explored_tile(2, 1);
    assert_eq!(m.info.path_map.get(2, 1), Some(0));
    assert_eq!(m.info.path_map.get(1, 1), Some(0xff));
}

#[test]
fn an_explored_tile_off_the_map_changes_nothing() {
    let mut m = module();
    let before = m.clone();
    for (x, y) in [(-1, 0), (0, -1), (4, 0), (0, 3)] {
        m.explored_tile(x, y);
    }
    assert_eq!(m, before);
}

#[test]
fn the_hooks_bound_is_the_modules_size_not_the_path_maps() {
    // FUN_140b8ab60 tests +0x10518/+0x1051c, then the grid lets a tile off its own size be
    let mut m = module();
    m.info.width = 10;
    m.info.height = 10;
    let before = m.info.path_map.clone();
    m.explored_tile(8, 8);
    assert_eq!(m.info.path_map, before);
}

#[test]
fn a_slot_past_the_saved_words_or_the_slots_is_minus_one() {
    let t = module().tactical;
    assert_eq!(t.slot(0), 7);
    assert_eq!(t.slot(1), 8);
    assert_eq!(t.slot(2), -1);
    assert_eq!(t.slot(SLOTS as u32), -1);
    assert_eq!(t.slot(u32::MAX), -1);
}

#[test]
fn the_table_holds_a_module_by_player() {
    let mut t = AiModules::default();
    assert!(t.get(3).is_none());
    t.set(3, module());
    assert!(t.get(3).is_some());
    assert!(t.get(2).is_none());
    t.get_mut(3).unwrap().info.explorer = None;
}

#[test]
fn the_worlds_sight_hands_its_explored_tiles_to_the_players_module() {
    use crate::vision::visibility::{Visibility, VisibleMap};
    let mut t = crate::world::World::new(0, 1, 0, vec![0; 3], Vec::new());
    let n = 16 * 16;
    let mut v = Visibility::new(
        16,
        16,
        vec![
            VisibleMap {
                counts: vec![-1; n],
                explored: 0,
            };
            3
        ],
        vec![0; n],
        Vec::new(),
        0,
        vec![false, true, false],
    );
    assert_eq!(v.explore(1, 3, 2, 0, false), Ok(1));
    assert_eq!(v.explore(2, 5, 5, 0, false), Ok(1));
    t.spatial.sight = Some(v);
    let mut m = module();
    m.info.width = 16;
    m.info.height = 16;
    m.info.path_map = InfluenceMap::new([16, 16], 0xff);
    t.ai.set(1, m);
    assert!(t.take_sight_unsupported().is_none());
    assert_eq!(t.ai.get(1).unwrap().info.path_map.get(3, 2), Some(0));
    assert_eq!(t.ai.get(1).unwrap().info.path_map.get(5, 5), Some(0xff));
}
