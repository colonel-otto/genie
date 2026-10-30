use super::*;
use crate::map::Footprint;
use crate::movement::move_to::{MoveDefaults, MoveTo, MoveToReads, SpriteReads, WalkTo};
use crate::objects::ObjectId;
use crate::objects::lifecycle::Lifecycle;
use crate::objects::masters::ClassId;
use crate::objects::masters::class;
use crate::objects::masters::master_type;
use crate::objects::unit::Unit;
use crate::players::PlayerId;
use crate::test_ids::master_id;
use crate::test_ids::object_id;
use crate::world::{Player, World};
use std::collections::BTreeMap;

/// The two tiles beside a builder on a footprint's diagonal (`LAB_140aafc09`'s arguments), by
/// the quarter its step points into: up-left, down-left, and the rest.
#[test]
fn the_tiles_beside_a_diagonal_follow_its_quarter() {
    assert_eq!(beside_diagonal(5, 5, -1, -1), ([6, 5], [5, 6]));
    assert_eq!(beside_diagonal(5, 5, -1, 1), ([5, 4], [6, 5]));
    assert_eq!(beside_diagonal(5, 5, 0, 1), ([4, 5], [5, 6]));
    assert_eq!(beside_diagonal(5, 5, 1, -1), ([4, 5], [5, 6]));
    assert_eq!(beside_diagonal(5, 5, 1, 1), ([5, 4], [4, 5]));
}

/// An object of `master` (class `class`, radius `r`, `buildable` on) of player 1 at `x`, `y`.
fn on_map(
    x: f32,
    y: f32,
    master: MasterId,
    class: ClassId,
    r: f32,
    buildable: u8,
) -> crate::map::spatial::Placed {
    crate::map::spatial::Placed {
        position: Position::new(x, y, 0.0),
        radius: Footprint::new(r, r),
        class,
        player: PlayerId::new(1),
        master,
        obstruction_type: 0,
        buildable_on: buildable,
        tall: false,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    }
}

/// What `FUN_1415c3620` reads besides the place: hit points `hp` and height `height` of a
/// live static object.
const fn seen(hp: f32, height: f32) -> ClearLook {
    ClearLook {
        hp,
        height,
        roo_called: false,
        kind: master_type::STATIC,
        state: 2,
        desolid: false,
    }
}

/// `FUN_1415c3620` for a house (master 70, radius 1) of player 1 at (10.5, 10.5): rows, then
/// columns, then each tile's list: two decorations on one tile go in the list's order; a
/// wall of the owner only under a gate (class 39); a stump (a tree of at most 1 hit point)
/// only for the builder's call; a flag master (274) of height 0 stays, one with height goes;
/// what does not overlap, a live tree and the house's own master stay.
#[test]
fn the_footprint_clearing_takes_buildable_objects_in_the_tiles_order() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    let objects = [
        (
            5,
            on_map(10.5, 10.5, master_id(1358), class::CLASS_5, 0.5, 1),
            seen(1.0, 0.0),
        ),
        (
            3,
            on_map(10.2, 10.2, master_id(1351), class::CLASS_5, 0.5, 1),
            seen(1.0, 0.0),
        ),
        (
            8,
            on_map(11.2, 10.5, master_id(349), class::TREE, 0.5, 0),
            seen(0.5, 1.0),
        ),
        (
            9,
            on_map(11.5, 9.5, master_id(350), class::TREE, 0.5, 0),
            seen(100.0, 1.0),
        ),
        (
            11,
            on_map(11.7, 9.7, master_id(350), class::TREE, 0.5, 0),
            seen(f32::NAN, 1.0),
        ),
        (
            4,
            on_map(9.5, 10.5, master_id(72), class::WALL, 0.5, 0),
            seen(100.0, 1.0),
        ),
        (
            6,
            on_map(10.2, 11.5, master_id(0x112), class::CLASS_11, 0.5, 1),
            seen(1.0, 0.0),
        ),
        (
            7,
            on_map(11.5, 11.5, master_id(0x112), class::CLASS_11, 0.5, 1),
            seen(1.0, 2.0),
        ),
        (
            10,
            on_map(13.6, 10.5, master_id(1358), class::CLASS_5, 0.5, 1),
            seen(1.0, 0.0),
        ),
        (
            12,
            on_map(10.6, 10.6, ids::HOUSE, class::BUILDING, 1.0, 1),
            seen(1.0, 0.0),
        ),
    ];
    for (id, p, _) in &objects {
        world.insert(object_id(*id), *p);
    }
    let look: BTreeMap<i32, ClearLook> = objects.iter().map(|o| (o.0, o.2)).collect();
    let site = |class: ClassId, trees: bool| ClearingSite {
        player: PlayerId::new(1),
        master: ids::HOUSE,
        class,
        radius: Footprint::new(1.0, 1.0),
        at: [10.5, 10.5],
        trees,
    };
    let ids = |world: &Spatial, s: ClearingSite| -> Vec<i32> {
        footprint_clearing(world, &s, &look)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(ids(&world, site(class::BUILDING, false)), [5, 3, 7]);
    assert_eq!(ids(&world, site(class::BUILDING, true)), [5, 3, 8, 7]);
    assert_eq!(ids(&world, site(class::GATE, false)), [4, 5, 3, 7]);
    // a wall of another player stays under a gate
    world.insert(
        object_id(4),
        crate::map::spatial::Placed {
            player: PlayerId::new(2),
            ..objects[5].1
        },
    );
    assert_eq!(ids(&world, site(class::GATE, false)), [5, 3, 7]);
    // a master without radii clears nothing; a NaN radius is not 0 or less
    let mut none = site(class::BUILDING, true);
    none.radius = Footprint::new(0.0, 0.0);
    assert_eq!(ids(&world, none), Vec::<i32>::new());
    none.radius = Footprint::new(f32::NAN, 0.0);
    assert_eq!(ids(&world, none), Vec::<i32>::new());
}

/// The objects whose looks the clearing about (10.5, 10.5) reads ([`clearing_objects`]): those
/// on the tiles from 4 before to 4 past the place each way (6 to 14), rows first, then columns,
/// then each tile's list; one a tile further stays out, and so does every object off the map.
#[test]
fn the_clearing_reads_the_objects_on_its_tiles() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    for (id, x, y) in [
        (1, 14.9, 6.0),
        (2, 6.0, 6.2),
        (3, 6.5, 6.5),
        (4, 15.0, 10.5),
        (5, 10.5, 5.9),
        (6, 10.5, 14.5),
    ] {
        world.insert(
            object_id(id),
            on_map(x, y, master_id(1358), class::CLASS_5, 0.5, 1),
        );
    }
    assert_eq!(clearing_objects(&world, [10.5, 10.5]), [2, 3, 1, 6]);
    // at the map's corner the tiles start at 0 and end at its last
    assert_eq!(clearing_objects(&world, [17.0, 2.0]), [1]);
    assert_eq!(clearing_objects(&world, [40.0, 40.0]), Vec::<i32>::new());
}

/// The record each cleared object loses in the clearing ([`Cleared::record_goes`]): with
/// `param_6`, a tall master's or a recorded object's (`FUN_141189670`, a NaN height too), not
/// one whose record went (`+0x686`); a unit below state 3 loses it in its destroy's death
/// part whatever the call; a building only when it is tall and not desolid.
#[test]
fn a_cleared_object_loses_its_record_by_its_type_and_the_call() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    let looks = [
        (1, seen(1.0, 1.0)),
        (2, seen(1.0, f32::NAN)),
        (
            3,
            ClearLook {
                roo_called: true,
                ..seen(1.0, 1.0)
            },
        ),
        (4, seen(1.0, 0.0)),
        (
            5,
            ClearLook {
                kind: master_type::COMBAT,
                ..seen(1.0, 0.0)
            },
        ),
        (
            6,
            ClearLook {
                kind: master_type::BUILDING,
                desolid: true,
                ..seen(1.0, 1.0)
            },
        ),
        (
            7,
            ClearLook {
                kind: master_type::BUILDING,
                ..seen(1.0, 1.0)
            },
        ),
        (
            8,
            ClearLook {
                kind: master_type::COMBAT,
                state: 5,
                ..seen(1.0, 0.0)
            },
        ),
    ];
    for (k, (id, _)) in looks.iter().enumerate() {
        let x = 9.5 + 0.1 * k as f32;
        world.insert(
            object_id(*id),
            on_map(x, 10.5, master_id(1358), class::CLASS_5, 0.5, 1),
        );
    }
    let look: BTreeMap<i32, ClearLook> = looks.iter().copied().collect();
    let goes = |trees: bool| -> Vec<i32> {
        let s = ClearingSite {
            player: PlayerId::new(1),
            master: ids::HOUSE,
            class: class::BUILDING,
            radius: Footprint::new(1.0, 1.0),
            at: [10.5, 10.5],
            trees,
        };
        footprint_clearing(&world, &s, &look)
            .unwrap()
            .iter()
            .filter(|c| c.record_goes)
            .map(|c| c.id)
            .collect()
    };
    assert_eq!(goes(true), [1, 2, 5, 7]);
    assert_eq!(goes(false), [5, 7]);
}

/// The completion's first part (`FUN_1411c6bd0`) of a farm whose build points reached its
/// build time: attribute 237's amount for the resource of 238 only when 239 is 0 or more too
/// and names a master the owner does not have; a pasture is not modelled; a mule cart's
/// search follows with a unit AI only.
#[test]
fn a_farm_s_bonus_wants_both_attributes_238_and_239() {
    let farm = Master {
        id: ids::FARM,
        class: class::FARM,
        ..Master::default()
    };
    let masters = crate::objects::masters::Masters::new([Master {
        id: master_id(68),
        ..Master::default()
    }]);
    let built = Construction {
        progress: 15.0,
        desolid: false,
        builders: 1,
        hit_points: 480,
        build_time: 15,
        record: Obstruction { kind: 0, class: 0 },
        snow: false,
        sprite_override: false,
    };
    let part = |bonus: [f32; 3], c: &Construction| {
        Completion {
            master: Some(&farm),
            farm_bonus: bonus,
            masters: Some(&masters),
            computer: false,
            ai: false,
        }
        .first_part(c)
    };
    assert_eq!(
        part([45.0, 0.0, -1.0], &built).unwrap().attributes,
        Vec::new()
    );
    assert_eq!(
        part([45.0, -1.0, 2.0], &built).unwrap().attributes,
        Vec::new()
    );
    assert_eq!(
        part([45.0, 0.0, 2.0], &built).unwrap().attributes,
        [(0, 45.0)]
    );
    assert_eq!(
        part([45.0, 0.0, 68.0], &built),
        Err(GenieError::unsupported(
            "a farm's bonus near a master (FUN_1411d3cd0)"
        ))
    );
    // not built yet: nothing
    let going = Construction {
        progress: 14.5,
        ..built
    };
    assert_eq!(
        part([45.0, 0.0, 2.0], &going).unwrap().attributes,
        Vec::new()
    );
    // a cart (a task of action type 0x99) with a unit AI searches
    let cart = Master {
        tasks: vec![Task {
            action_type: CART_TASK,
            ..Task::default()
        }],
        ..Master::default()
    };
    let searches = |ai: bool| {
        Completion {
            master: Some(&cart),
            farm_bonus: [0.0; 3],
            masters: None,
            computer: false,
            ai,
        }
        .first_part(&built)
        .unwrap()
        .cart
    };
    assert_eq!((searches(true), searches(false)), (true, false));
}

/// A villager's Build task: range 0.1, work value 1, work rate 1.
pub(super) const TASK: BuildTask = BuildTask {
    range: 0.1,
    work_value: 1.0,
    work_rate: 1.0,
    resource: -1,
    id: TaskId::of_task(-1),
    moving: None,
};

/// [`TASK`] with moving sprite 3.
pub(super) const MOVING_TASK: BuildTask = BuildTask {
    moving: Some(3),
    ..TASK
};

/// The sprites' table of the Build's tests: 3 (the moving sprite of [`MOVING_TASK`]), 7, 20 and
/// 21, each looped, 10 frames of 16 facets.
pub(crate) fn graphics() -> BTreeMap<i16, crate::objects::sprite::Graphic> {
    [3, 7, 20, 21]
        .map(|id| {
            let g = crate::objects::sprite::Graphic {
                id,
                frames: 10,
                facets: 16,
                speed_multiplier: 1.0,
                frame_duration: 0.1,
                replay_delay: 0.0,
                flags: 1,
            };
            (id, g)
        })
        .into()
}

/// A Build started on the foundation `id` (state 0) at `at`, `gap` from its builder (state 2,
/// speed 1, no default sprites), with `task` ([`BuildAction::start`]).
pub(crate) fn started(id: i32, at: Position, gap: f32, task: BuildTask) -> BuildAction {
    let graphics = graphics();
    let move_reads = MoveToReads {
        sprites: SpriteReads {
            defaults: MoveDefaults::default(),
            graphics: &graphics,
        },
        state: 2,
        speed: 1.0,
        at: Position::new(at.x() - gap, at.y(), 0.0),
        ground: Footprint::new(0.2, 0.2),
    };
    let to = WalkTo {
        id,
        at,
        gap,
        state: Some(0),
    };
    BuildAction::start(to, task, &move_reads).unwrap()
}

/// A Build's start (`0x140aadd80`, `set_state(4)`) takes the task's moving sprite for its
/// `MoveTo` ([`BuildTask::walk`]).
#[test]
fn a_build_s_move_to_takes_its_task_s_moving_sprite() {
    let moving = started(10028, Position::new(10.5, 10.5, 0.0), 5.0, MOVING_TASK);
    assert_eq!(moving.move_to.and_then(|m| m.sprite).map(|g| g.id), Some(3));
}

/// MarvinBlue/426048731's house 10028 as BUILD placed it: a foundation of master 70.
fn house() -> crate::map::spatial::Placed {
    crate::map::spatial::Placed {
        position: Position::new(30.0, 88.0, 0.0),
        radius: Footprint::new(1.0, 1.0),
        class: class::BUILDING,
        player: PlayerId::new(2),
        master: ids::HOUSE,
        obstruction_type: 2,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    }
}

/// records/425906646's update 4804: builders of players 4 and 5 told to build another
/// foundation: even one at work (state 6) starts over on the new one (state 4), no longer
/// among builders, its `MoveTo` arrived when the footprints touch.
/// Slot 0x38 (`FUN_140ab0350`, `set_state(2)`): the builder no longer counts (the caller
/// takes it out of the foundation's builders when it did), its `MoveTo` goes and stops the
/// unit, the foundation goes (the caller releases it) and the point; state 2.
#[test]
fn a_stop_halts_the_build_and_lets_its_foundation_go() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut b = BuildAction {
        state: MOVE,
        move_to: Some(MoveToAction::sub_in(MoveTo::Moving)),
        added: true,
        ..started(31148, Position::ORIGIN, 0.0, TASK)
    };
    assert_eq!(b.halt(Some(&mut mover)), Ok((true, [31148, -1])));
    assert_eq!(
        (b.state, b.added, b.move_to, b.target),
        (HALTED, false, None, -1)
    );
    assert_eq!(b.point, Position::new(-1.0, -1.0, -1.0));
    assert_eq!(mover.velocity.unwrap().velocity.y(), 0.0);
    // one not counted among the builders, standing: nothing to stop
    let mut idle = BuildAction {
        state: WORK,
        move_to: None,
        ..started(31150, Position::ORIGIN, 0.0, TASK)
    };
    assert_eq!(idle.halt(None), Ok((false, [31150, -1])));
}

/// The Build's constructor (`FUN_140aad4e0`) and `set_state(4)` take the foundation's place
/// as the action's point (`+0x38`..`+0x40`) and its `MoveTo` the foundation as its target.
#[test]
fn a_build_s_point_is_its_foundation_s_place() {
    let b = started(10913, Position::new(61.5, 92.5, 1.0), 0.8, TASK);
    let walk = b.move_to.unwrap();
    assert_eq!(
        (
            b.point,
            ObjectId::field(walk.target),
            walk.point,
            walk.state
        ),
        (
            Position::new(61.5, 92.5, 1.0),
            10913,
            Position::new(61.5, 92.5, 1.0),
            MoveTo::Start
        )
    );
}

#[test]
fn a_builder_sent_to_another_foundation_starts_over() {
    let mut b = BuildAction {
        state: WORK,
        move_to: None,
        added: true,
        timer: 0.5,
        ..started(31148, Position::ORIGIN, 0.0, TASK)
    };
    let walk = started(31150, Position::ORIGIN, 0.0, TASK).move_to;
    b.retarget(31150, Position::ORIGIN, walk, None).unwrap();
    // set_state(4) keeps the timer
    assert_eq!(
        b,
        BuildAction {
            timer: 0.5,
            ..started(31150, Position::ORIGIN, 0.0, TASK)
        }
    );
    assert_eq!(
        (b.state, b.added, b.move_to.map(|m| m.state)),
        (MOVE, false, Some(MoveTo::Arrived))
    );
}

/// records/425906646's update 673: builders 27232 and 27234, at work on house 31011 (state
/// 7, counted among its builders), are told to build it again: each leaves the builders and
/// starts over (state 4, its `MoveTo` arrived beside the house); one at work (state 6) stays.
#[test]
fn a_builder_told_to_build_its_foundation_again_starts_over() {
    let mut c = Construction {
        progress: 3.0,
        desolid: false,
        builders: 2,
        hit_points: 25,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let mut b = BuildAction {
        state: PLACE,
        move_to: None,
        added: true,
        ..started(31011, Position::ORIGIN, 0.0, TASK)
    };
    let walk = started(31011, Position::ORIGIN, 0.0, TASK).move_to;
    b.retake(Position::ORIGIN, walk, None, Some(&mut c.builders), false)
        .unwrap();
    assert_eq!(c.builders, 1);
    assert_eq!(b, started(31011, Position::ORIGIN, 0.0, TASK));
    assert_eq!(b.move_to.map(|m| m.state), Some(MoveTo::Arrived));
    let mut working = BuildAction {
        state: WORK,
        move_to: None,
        ..started(31011, Position::ORIGIN, 0.0, TASK)
    };
    let before = working;
    working
        .retake(Position::ORIGIN, walk, None, Some(&mut c.builders), false)
        .unwrap();
    assert_eq!(working, before);
    assert_eq!(c.builders, 1);
}

/// A world of player 1 holding the foundations `units` on `spatial`, `masters` its masters'
/// table (and Gaia's): the work and the completion on the world.
fn site_world(spatial: Spatial, masters: Vec<Master>, units: Vec<Unit>) -> World {
    let ms = crate::objects::masters::Masters::new(masters);
    let mut t = World::new(
        0,
        1,
        PlayerId::new(0),
        vec![0; 2],
        vec![
            Player::default(),
            Player {
                objects: units.into(),
                attributes: vec![0.0; 600],
                ..Player::default()
            },
        ],
    );
    t.spatial = spatial;
    t.masters = vec![ms.clone(), ms];
    t
}

/// The foundation `id` at `at` with its life cycle and construction.
fn foundation_at(id: i32, at: Position, life: Lifecycle, construction: Construction) -> Unit {
    Unit {
        id: object_id(id),
        body: crate::objects::layers::Body::at(at),
        life,
        construction: Some(construction),
        pasture: -1,
        my_pasture: -1,
        ..Unit::default()
    }
}

/// A building master `id` of class 3 without a stack unit, a foundation terrain or a
/// technology, costing `cost`.
fn built_master(id: MasterId, cost: Vec<(i16, i16, bool)>) -> Master {
    Master {
        id,
        kind: master_type::BUILDING,
        class: class::BUILDING,
        cost,
        building: Some(crate::objects::masters::BuildingMaster {
            adjacent_mode: 0,
            disappears_when_built: false,
            stack_unit: None,
            foundation_terrain: -1,
            tech: None,
            garrison_type: 0,
            garrison_heal_rate: 0.0,
            garrison_repair_rate: 0.0,
            annexes: [None; 4],
            annex_offsets: [[0.0; 2]; 4],
            head_unit: None,
            transform_unit: None,
            destruction_sprite: -1,
            construction_sprite: -1,
        }),
        ..Master::default()
    }
}

/// A 4 by 4 map of buildable tiles with its terrain (terrain 0, no layer).
fn small_map() -> Spatial {
    use crate::map::spatial::Tiles;
    use crate::map::terrain::{Terrain, TerrainKind};
    let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
    let kind = TerrainKind {
        shown: true,
        flags: 0x20,
        forest: false,
    };
    world.terrain = Some(Terrain::new(
        4,
        4,
        vec![(0, 255, -1); 16],
        vec![kind; 40],
        vec![vec![0.0; 40]; 5],
    ));
    world
}

/// MarvinBlue/426048731's house 10026 in update 1115 (player 1): the work that reaches 25 s
/// completes it on the world, each part at once: its build points hold the build time, its
/// storage (population headroom, 4, of mode 4) and attributes 98 and 165 go to its owner, it
/// stands in state 2 and its foundation (27) covers its snow with terrain 6 under layer 32.
/// The work's answer: its construction no longer goes on.
#[test]
fn the_work_that_reaches_the_build_time_completes_the_building() {
    use crate::map::spatial::Tiles;
    use crate::map::terrain::{Terrain, TerrainKind};
    let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
    world.insert(
        object_id(10026),
        crate::map::spatial::Placed {
            position: Position::new(2.0, 2.0, 0.0),
            ..house()
        },
    );
    let mut kinds = vec![
        TerrainKind {
            shown: true,
            flags: 0x20,
            forest: false,
        };
        40
    ];
    kinds[32].flags = 0xa0;
    world.terrain = Some(Terrain::new(
        4,
        4,
        vec![(32, 255, -1); 16],
        kinds,
        vec![vec![0.0; 40]; 5],
    ));
    let mut master = built_master(ids::HOUSE, vec![(1, 25, true), (4, 5, false)]);
    // a house's cost: 25 wood paid; the unpaid entry does not count
    master.clearance = Footprint::new(1.0, 1.0);
    master.terrain_bonus = 4;
    master.storage = vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)];
    if let Some(b) = master.building.as_mut() {
        b.foundation_terrain = 27;
    }
    let life = Lifecycle {
        state: 0,
        hp: 548.0,
        master_id: ids::HOUSE,
        ..Lifecycle::default()
    };
    let c = Construction {
        progress: 24.99,
        desolid: false,
        builders: 2,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let house = foundation_at(10026, Position::new(2.0, 2.0, 0.0), life, c);
    let mut t = site_world(world, vec![master], vec![house]);
    assert_eq!(t.work_building(10026, 0.02), Ok(false));
    let h = t.players[1].objects.by_id(10026).unwrap();
    let c = h.construction.unwrap();
    assert_eq!((h.life.state, c.progress, c.snow), (2, 25.0, true));
    let a = &t.players[1].attributes;
    assert_eq!((a[4], a[98], a[165]), (5.0, 25.0, 25.0));
    let terrain = t.spatial.terrain.as_ref().unwrap();
    assert_eq!(
        (terrain.kinds[5], terrain.layers[5], terrain.kinds[0]),
        (6, 32, 32)
    );
}

/// `FUN_1411dbb70`'s tail (`0x1411dd094`..): the castle (82) and the wonder (276) record
/// their building in their owner's history (events 6 and 4), count in attributes 134 and 173,
/// 42 and 174, and add their paid cost to 184 rather than to 98 and 165, an annex's too; the
/// monastery (104) counts in 52 and adds its paid cost to 98 and 165 as other buildings do,
/// but as an annex.
#[test]
fn the_castle_the_wonder_and_the_monastery_count_their_completion() {
    use crate::players::history::{CASTLE_BUILT, WONDER_BUILT};
    /// The attributes the completion of master `id` gave, by index, and its history events.
    fn complete(
        id: MasterId,
        cost: Vec<(i16, i16, bool)>,
        main: bool,
    ) -> (Vec<(usize, f32)>, Vec<u8>) {
        let mut world = small_map();
        world.insert(
            object_id(10026),
            crate::map::spatial::Placed {
                position: Position::new(2.0, 2.0, 0.0),
                master: id,
                ..house()
            },
        );
        let life = Lifecycle {
            state: 0,
            hp: 100.0,
            master_id: id,
            ..Lifecycle::default()
        };
        let c = Construction {
            progress: 100.0,
            desolid: false,
            builders: 1,
            hit_points: 100,
            build_time: 100,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
        };
        let mut h = foundation_at(10026, Position::new(2.0, 2.0, 0.0), life, c);
        h.annex_of = main.then_some(10027);
        let mut t = site_world(world, vec![built_master(id, cost)], vec![h]);
        // a history records events from its first time slice on
        t.players[1].history.add_entry(0, 3.0, 0.0);
        t.complete_building(10026).unwrap();
        assert_eq!(t.players[1].objects.by_id(10026).unwrap().life.state, 2);
        let gained = t.players[1]
            .attributes
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v != 0.0)
            .map(|(k, &v)| (k, v))
            .collect();
        let events = t.players[1].history.events.iter().map(|e| e.kind).collect();
        (gained, events)
    }
    let castle = || vec![(2, 650, true)];
    let wonder = || vec![(0, 1000, true), (1, 1000, true), (2, 1000, false)];
    for main in [false, true] {
        assert_eq!(
            complete(ids::CASTLE, castle(), main),
            (
                vec![(134, 1.0), (173, 1.0), (184, 650.0)],
                vec![CASTLE_BUILT]
            )
        );
        assert_eq!(
            complete(ids::WONDER, wonder(), main),
            (
                vec![(42, 1.0), (174, 1.0), (184, 2000.0)],
                vec![WONDER_BUILT]
            )
        );
    }
    assert_eq!(
        complete(ids::MONASTERY, vec![(1, 175, true)], false),
        (vec![(52, 1.0), (98, 175.0), (165, 175.0)], vec![])
    );
    assert_eq!(
        complete(ids::MONASTERY, vec![(1, 175, true)], true),
        (vec![(52, 1.0)], vec![])
    );
}

/// MarvinBlue/426048731's house 10028 in update 731: 9177's first work starts it (its record
/// marks its tiles) and adds 0.021 s of work: 550 hit points over 25 s, so 1.462 hit points.
/// Its construction goes on.
#[test]
fn the_first_work_starts_the_foundation_and_adds_its_share_of_hit_points() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    world.insert(object_id(10028), house());
    let life = Lifecycle {
        state: 0,
        hp: 1.0,
        master_id: ids::HOUSE,
        ..Lifecycle::default()
    };
    let c = Construction {
        progress: 0.0,
        desolid: true,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let site = foundation_at(10028, house().position, life, c);
    let mut t = site_world(
        world,
        vec![built_master(ids::HOUSE, Vec::new())],
        vec![site],
    );
    let rate = 1.0f32 * 1.0 * 0.021;
    assert_eq!(t.work_building(10028, rate), Ok(true));
    let h = t.players[1].objects.by_id(10028).unwrap();
    let c = h.construction.unwrap();
    assert_eq!(h.life.hp, 1.462);
    assert!(!c.desolid);
    assert_eq!(c.progress, f32::from_bits(STARTED) + rate);
    assert!(t.spatial.get(10028).unwrap().obstruction.is_some());
    assert!(t.spatial.tiles().word(29, 87).unwrap() & crate::map::spatial::BLOCKED != 0);
}

/// A gate's foundation (master 665) with its two posts (1192, annexes of a building type):
/// `FUN_1411cc430` starts each desolid post first (its record, its missiles per volley), a post
/// already started stays as it is, then the gate's own first work starts it.
#[test]
fn the_work_starts_the_desolid_annexes_first() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    let at = |x: f32, id: MasterId| crate::map::spatial::Placed {
        position: Position::new(x, 88.0, 0.0),
        master: id,
        ..house()
    };
    world.insert(object_id(10028), at(30.0, master_id(665)));
    world.insert(object_id(10029), at(28.5, master_id(1192)));
    world.insert(object_id(10030), at(31.5, master_id(1192)));
    world.set_annex(10029, Some(10028));
    world.set_annex(10030, Some(10028));
    let foundation = |desolid| Construction {
        progress: 0.0,
        desolid,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let life = |master_id| Lifecycle {
        state: 0,
        hp: 1.0,
        master_id,
        ..Lifecycle::default()
    };
    let unit = |id, x, master, desolid| {
        let mut u = foundation_at(
            id,
            Position::new(x, 88.0, 0.0),
            life(master),
            foundation(desolid),
        );
        u.annex_of = (master == master_id(1192)).then_some(10028);
        u
    };
    let mut post = built_master(master_id(1192), Vec::new());
    post.volley_fire = 2.0;
    let mut t = site_world(
        world,
        vec![built_master(master_id(665), Vec::new()), post],
        vec![
            unit(10028, 30.0, master_id(665), true),
            unit(10029, 28.5, master_id(1192), true),
            unit(10030, 31.5, master_id(1192), false),
        ],
    );
    assert_eq!(t.work_building(10028, 0.021), Ok(true));
    let u = |id| t.players[1].objects.by_id(id).unwrap();
    let post = u(10029).construction.unwrap();
    assert!(!post.desolid);
    assert_eq!(post.progress, f32::from_bits(STARTED));
    assert_eq!(u(10029).firepower, 2.0);
    assert!(t.spatial.get(10029).unwrap().obstruction.is_some());
    assert_eq!(
        (u(10030).construction.unwrap(), u(10030).firepower),
        (foundation(false), 0.0)
    );
    assert!(!u(10028).construction.unwrap().desolid);
}

/// The construction's start (`FUN_1411e5670`) sets the foundation's last move time to the
/// world's time plus 1, which a walker later in the same update reads (slot 0x5e8) when it
/// meets the new record: it outranks a path made before (429513284's builder 30830 meeting
/// the house 33439 that 30828 started in update 199).
#[test]
fn a_construction_s_start_moves_its_foundation_for_the_walks_of_the_update() {
    use crate::map::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    world.insert(
        object_id(33439),
        crate::map::spatial::Placed {
            position: Position::new(194.0, 64.0, 0.0),
            ..house()
        },
    );
    world.set_gait(
        33439,
        Some(crate::map::spatial::Gait {
            last_move: 7000,
            ..crate::map::spatial::Gait::default()
        }),
    );
    let mut c = Construction {
        progress: 0.0,
        desolid: true,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let mut mover = crate::gathering::hunt::tests::walker();
    let started = Construction::begin(
        33439,
        &mut c,
        Some(&mut mover),
        &mut world,
        &mut Pathing::default(),
        8280,
    );
    assert_eq!(started, Ok(true));
    assert_eq!(mover.last_move, 8281);
    assert_eq!(world.gait(33439).map(|g| g.last_move), Some(8281));
}

/// An annex (a gate's post) whose main building is in state 0 with a hit point gives it the
/// work (`FUN_1411cc430`'s loop): the post itself is not worked, and the answer is the main
/// building's (its start's build points, then the work): its construction goes on, and no
/// longer once the work reaches its build time and completes it.
#[test]
fn an_annex_s_work_goes_to_its_main_building() {
    let foundation = Construction {
        progress: 0.0,
        desolid: true,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
    };
    let run = |main: Construction, amount: f32| {
        let mut world = small_map();
        world.insert(
            object_id(10028),
            crate::map::spatial::Placed {
                position: Position::new(2.0, 2.0, 0.0),
                master: master_id(665),
                ..house()
            },
        );
        let life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: master_id(1192),
            ..Lifecycle::default()
        };
        let mut post = foundation_at(10029, Position::new(1.0, 2.0, 0.0), life, foundation);
        post.annex_of = Some(10028);
        let gate = foundation_at(
            10028,
            Position::new(2.0, 2.0, 0.0),
            Lifecycle {
                master_id: master_id(665),
                ..life
            },
            main,
        );
        let masters = vec![
            built_master(master_id(665), Vec::new()),
            built_master(master_id(1192), Vec::new()),
        ];
        let mut t = site_world(world, masters, vec![gate, post]);
        let going = t.work_building(10029, amount);
        let post = t.players[1].objects.by_id(10029).unwrap();
        assert_eq!(
            (post.construction, post.life.hp),
            (Some(foundation), 1.0),
            "the post itself is not worked"
        );
        let state = t.players[1].objects.by_id(10028).unwrap().life.state;
        (going, state)
    };
    assert_eq!(run(foundation, 0.021), (Ok(true), 0));
    let nearly = Construction {
        progress: 24.99,
        desolid: false,
        ..foundation
    };
    assert_eq!(run(nearly, 0.021), (Ok(false), 2));
}

use crate::map::spatial::Spatial;
