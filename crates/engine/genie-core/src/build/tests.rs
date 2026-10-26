use super::*;

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
    master: i16,
    class: i16,
    r: f32,
    buildable: u8,
) -> crate::spatial::Placed {
    crate::spatial::Placed {
        position: [x, y, 0.0],
        radius: [r, r],
        class,
        player: 1,
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
        kind: 10,
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
    use crate::spatial::Tiles;
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    let objects = [
        (5, on_map(10.5, 10.5, 1358, 5, 0.5, 1), seen(1.0, 0.0)),
        (3, on_map(10.2, 10.2, 1351, 5, 0.5, 1), seen(1.0, 0.0)),
        (8, on_map(11.2, 10.5, 349, TREE, 0.5, 0), seen(0.5, 1.0)),
        (9, on_map(11.5, 9.5, 350, TREE, 0.5, 0), seen(100.0, 1.0)),
        (
            11,
            on_map(11.7, 9.7, 350, TREE, 0.5, 0),
            seen(f32::NAN, 1.0),
        ),
        (4, on_map(9.5, 10.5, 72, WALL, 0.5, 0), seen(100.0, 1.0)),
        (6, on_map(10.2, 11.5, 0x112, 11, 0.5, 1), seen(1.0, 0.0)),
        (7, on_map(11.5, 11.5, 0x112, 11, 0.5, 1), seen(1.0, 2.0)),
        (10, on_map(13.6, 10.5, 1358, 5, 0.5, 1), seen(1.0, 0.0)),
        (12, on_map(10.6, 10.6, 70, 3, 1.0, 1), seen(1.0, 0.0)),
    ];
    for (id, p, _) in &objects {
        world.insert(*id, *p);
    }
    let look = |id: i32| objects.iter().find(|o| o.0 == id).map(|o| o.2);
    let site = |class: i16, trees: bool| ClearingSite {
        player: 1,
        master: 70,
        class,
        radius: [1.0, 1.0],
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
    assert_eq!(ids(&world, site(3, false)), [5, 3, 7]);
    assert_eq!(ids(&world, site(3, true)), [5, 3, 8, 7]);
    assert_eq!(ids(&world, site(GATE, false)), [4, 5, 3, 7]);
    // a wall of another player stays under a gate
    world.insert(
        4,
        crate::spatial::Placed {
            player: 2,
            ..objects[5].1
        },
    );
    assert_eq!(ids(&world, site(GATE, false)), [5, 3, 7]);
    // a master without radii clears nothing; a NaN radius is not 0 or less
    let mut none = site(3, true);
    none.radius = [0.0, 0.0];
    assert_eq!(ids(&world, none), Vec::<i32>::new());
    none.radius = [f32::NAN, 0.0];
    assert_eq!(ids(&world, none), Vec::<i32>::new());
}

/// The record each cleared object loses in the clearing ([`Cleared::record_goes`]): with
/// `param_6`, a tall master's or a recorded object's (`FUN_141189670`, a NaN height too), not
/// one whose record went (`+0x686`); a unit below state 3 loses it in its destroy's death
/// part whatever the call; a building only when it is tall and not desolid.
#[test]
fn a_cleared_object_loses_its_record_by_its_type_and_the_call() {
    use crate::spatial::Tiles;
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
                kind: 70,
                ..seen(1.0, 0.0)
            },
        ),
        (
            6,
            ClearLook {
                kind: 80,
                desolid: true,
                ..seen(1.0, 1.0)
            },
        ),
        (
            7,
            ClearLook {
                kind: 80,
                ..seen(1.0, 1.0)
            },
        ),
        (
            8,
            ClearLook {
                kind: 70,
                state: 5,
                ..seen(1.0, 0.0)
            },
        ),
    ];
    for (k, (id, _)) in looks.iter().enumerate() {
        let x = 9.5 + 0.1 * k as f32;
        world.insert(*id, on_map(x, 10.5, 1358, 5, 0.5, 1));
    }
    let look = |id: i32| looks.iter().find(|o| o.0 == id).map(|o| o.1);
    let goes = |trees: bool| -> Vec<i32> {
        let s = ClearingSite {
            player: 1,
            master: 70,
            class: 3,
            radius: [1.0, 1.0],
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
        id: 50,
        class: FARM,
        ..Master::default()
    };
    let masters = crate::masters::Masters::new([Master {
        id: 68,
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
        started: false,
    };
    let part = |bonus: [f32; 3], c: &Construction| {
        Completion {
            master: Some(&farm),
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: bonus,
            masters: Some(&masters),
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
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
        tasks: vec![crate::masters::Task {
            action_type: CART_TASK,
            ..crate::masters::Task::default()
        }],
        ..Master::default()
    };
    let searches = |ai: bool| {
        Completion {
            master: Some(&cart),
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0; 3],
            masters: None,
            computer: false,
            ai,
            main: false,
            now: 0,
            rice: false,
        }
        .first_part(&built)
        .unwrap()
        .cart
    };
    assert_eq!((searches(true), searches(false)), (true, false));
}

/// A villager's Build task: range 0.1, work value 1, work rate 1.
const TASK: BuildTask = BuildTask {
    range: 0.1,
    work_value: 1.0,
    work_rate: 1.0,
    resource: -1,
    id: -1,
};

/// MarvinBlue/426048731's house 10028 as BUILD placed it: a foundation of master 70.
fn house() -> crate::spatial::Placed {
    crate::spatial::Placed {
        position: [30.0, 88.0, 0.0],
        radius: [1.0, 1.0],
        class: 3,
        player: 2,
        master: 70,
        obstruction_type: 2,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    }
}

/// `FUN_140aaddb0`: a Build on a building annex (`+0xbb8`) counts its builder in the main
/// building. The join (`0x140aae200` case 7) raises the main building's count and leaves the
/// annex's; the work's extra-builder factor (`0x140ab0370` case 6) reads the main building's
/// two builders; the prelude of the `set_state(4)` of slot 0x48 on the annex again
/// (`FUN_140ab1b30`, [`BuildAction::retake`]) lowers the main building's count.
#[test]
fn a_builder_on_an_annex_counts_among_its_main_building_s_builders() {
    fn update(
        build: &mut BuildAction,
        (life, annex, main): (&mut Lifecycle, &mut Construction, &mut u8),
        world: &mut Spatial,
    ) {
        let (mut pathing, mut object) = (Pathing::default(), crate::math::rng::Well512::default());
        let groups = crate::group::UnitGroups::default();
        let site = Site {
            id: 10029,
            life,
            construction: annex,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: true,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: Some(main),
        };
        let walk = Walk {
            mover: None,
            me: Me {
                id: 5,
                position: [32.0, 88.0, 0.0],
                group: -1,
                action_state: build.state,
            },
            dt: 0.03,
            world,
            plan: Some(crate::movement::Planning {
                pathing: &mut pathing,
                now: 0,
                computer: false,
                object: &mut object,
                construction_rate: 0.0,
                groups: &groups,
            }),
        };
        let builder = Builder {
            class: VILLAGER,
            master: 83,
            held: 0.0,
            held_kind: -1,
            drop_site: false,
            ai: None,
            waits: &mut 0,
            build_targets: &[],
            look: Looks(&|_| None),
            walk_target_state: Some(0),
            deposit: None,
            attributes: 4,
            gone_orders: &[],
            stack_made: None,
        };
        let _ = build.update(walk, Some(site), builder).unwrap();
    }
    let mut world = Spatial::default();
    world.insert(
        10029,
        crate::spatial::Placed {
            player: 1,
            ..house()
        },
    );
    let mut life = Lifecycle {
        state: 0,
        hp: 1.0,
        master_id: 1192,
        ..Lifecycle::default()
    };
    let mut annex = Construction {
        progress: 1.0,
        desolid: false,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    // another builder works on the main building
    let mut main = 1u8;
    let mut build = BuildAction {
        state: WAIT,
        move_to: None,
        timer: 0.01,
        ..BuildAction::start(10029, [0.0; 3], 0.0, TASK)
    };
    update(&mut build, (&mut life, &mut annex, &mut main), &mut world);
    assert_eq!(
        (build.state, build.added, main, annex.builders),
        (PLACE, true, 2, 0)
    );
    update(&mut build, (&mut life, &mut annex, &mut main), &mut world);
    let rate = 1.0f32 * 1.0 * 0.03;
    let n = f32::from(main);
    assert_eq!(
        annex.progress,
        1.0 + rate * (((n - 1.0) * EXTRA_BUILDER + 1.0) / n)
    );
    build
        .retake(([0.0; 3], 0.0), None, Some(&mut main), false)
        .unwrap();
    assert_eq!((build.state, build.added, main), (MOVE, false, 1));
}

/// records/425906646's update 3145: a builder carrying wood when its lumber camp completes
/// drops it there (`FUN_140aadc50`); food kinds become food; at a house it keeps its load.
#[test]
fn a_builder_drops_its_load_at_a_drop_site_it_completes() {
    let mut world = Spatial::default();
    let mut finish = |kind: i16, drop_site: bool| {
        let mut b = BuildAction {
            state: PLACE,
            move_to: None,
            ..BuildAction::start(31500, [0.0; 3], 0.0, TASK)
        };
        let mut walk = Walk {
            mover: None,
            me: Me {
                id: 1,
                position: [0.0; 3],
                group: -1,
                action_state: 0,
            },
            dt: 0.02,
            world: &mut world,
            plan: None,
        };
        let load = Load {
            amount: 6.881_128,
            kind,
            drop_site,
        };
        let dropped = b.finish(&mut walk, None, load).unwrap();
        assert_eq!(b.state, FINISHED);
        dropped
    };
    assert_eq!(finish(1, true), Some((1, 6.881_128)));
    assert_eq!(finish(16, true), Some((0, 6.881_128)));
    assert_eq!(finish(1, false), None);
    assert_eq!(finish(-1, true), None);
}

/// records/425906646's update 4804: builders of players 4 and 5 told to build another
/// foundation: even one at work (state 6) starts over on the new one (state 4), no longer
/// among builders, its `MoveTo` arrived when the footprints touch.
/// Slot 0x38 (`FUN_140ab0350`, `set_state(2)`): the builder no longer counts (the caller
/// takes it out of the foundation's builders when it did), its `MoveTo` goes and stops the
/// unit, the foundation goes (the caller releases it) and the point; state 2.
#[test]
fn a_stop_halts_the_build_and_lets_its_foundation_go() {
    let mut mover = crate::hunt::tests::walker();
    let mut b = BuildAction {
        state: MOVE,
        move_to: Some(MoveTo::Moving),
        goal: Some([3.0, 4.0, 0.0]),
        added: true,
        ..BuildAction::start(31148, [0.0; 3], 0.0, TASK)
    };
    assert_eq!(b.halt(Some(&mut mover)), Ok((true, [31148, -1])));
    assert_eq!(
        (b.state, b.added, b.move_to, b.goal, b.target),
        (HALTED, false, None, None, -1)
    );
    assert_eq!((b.point, b.walk_target), ([-1.0; 3], -1));
    assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
    // one not counted among the builders, standing: nothing to stop
    let mut idle = BuildAction {
        state: WORK,
        move_to: None,
        ..BuildAction::start(31150, [0.0; 3], 0.0, TASK)
    };
    assert_eq!(idle.halt(None), Ok((false, [31150, -1])));
}

/// The Build's constructor (`FUN_140aad4e0`) and `set_state(4)` take the foundation's place
/// as the action's point (`+0x38`..`+0x40`) and its `MoveTo` the foundation as its target.
#[test]
fn a_build_s_point_is_its_foundation_s_place() {
    let b = BuildAction::start(10913, [61.5, 92.5, 1.0], 0.8, TASK);
    assert_eq!(
        (b.point, b.walk_target, b.move_to),
        ([61.5, 92.5, 1.0], 10913, Some(MoveTo::Start))
    );
}

/// 428326873's builders 8774 and 8775 (player 6) walking to the foundation 10913 that its
/// owner deletes in update 4626 (state 7, still in the world): the `MoveTo`'s update finds
/// its target above state 6 and returns 5 (quiet); the Build lets the foundation go (slot
/// 0x68 with none: its builder count, the release) and walks to its point (`set_state(11)`:
/// the old `MoveTo` stops the unit, a quiet `MoveTo` to the point starts in state 3). At
/// the point (its `MoveTo` arrived, 1) or failed (3), or without one (1), it waits
/// (`set_state(3)`: notify 0x202 and the standing sprite without queued orders).
#[test]
fn a_builder_whose_foundation_dies_walks_to_its_point_then_waits() {
    fn builder(waits: &mut u8, state: Option<u8>) -> Builder<'_> {
        Builder {
            class: VILLAGER,
            master: 83,
            held: 0.0,
            held_kind: -1,
            drop_site: false,
            ai: None,
            waits,
            build_targets: &[],
            look: Looks(&|_| None),
            walk_target_state: state,
            deposit: None,
            attributes: 4,
            gone_orders: &[],
            stack_made: None,
        }
    }
    fn walk<'a>(mover: &'a mut crate::movement::Mover, world: &'a mut Spatial, me: Me) -> Walk<'a> {
        Walk {
            mover: Some(mover),
            me,
            dt: 0.02,
            world,
            plan: None,
        }
    }
    use crate::spatial::Placed;
    let mut world = Spatial::default();
    world.insert(
        10913,
        Placed {
            position: [61.5, 92.5, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 6,
            master: 70,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let mut mover = crate::hunt::tests::walker();
    let mut c = Construction {
        progress: 3.0,
        desolid: false,
        builders: 1,
        hit_points: 25,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let mut life = Lifecycle {
        state: 7,
        ..Lifecycle::default()
    };
    let mut b = BuildAction {
        move_to: Some(MoveTo::Moving),
        added: true,
        ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
    };
    let site = Site {
        id: 10913,
        life: &mut life,
        construction: &mut c,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let me = Me {
        id: 8774,
        position: [58.0, 90.0, 0.0],
        group: -1,
        action_state: MOVE,
    };
    let mut waits = 0;
    let step = b
        .update(
            walk(&mut mover, &mut world, me),
            Some(site),
            builder(&mut waits, Some(7)),
        )
        .unwrap();
    assert!(step.released && !step.notified);
    assert_eq!(c.builders, 0);
    assert_eq!(
        (b.state, b.target, b.walk_target, b.added),
        (WALK_POINT, -1, -1, false)
    );
    assert_eq!(
        (b.move_to, b.goal, b.point),
        (
            Some(MoveTo::Start),
            Some([61.5, 92.5, 0.0]),
            [61.5, 92.5, 0.0]
        )
    );
    assert_eq!(mover.velocity.unwrap().velocity[2], 0.0);
    // a foundation the world removed (425984813's 4045, built and replaced by its stack
    // unit) is no longer the MoveTo's target, which walks on: no walk to the point
    let mut gone = BuildAction {
        target: -1,
        move_to: Some(MoveTo::Arrived),
        ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
    };
    let _ = gone
        .update(
            walk(&mut mover, &mut world, me),
            None,
            builder(&mut waits, None),
        )
        .unwrap();
    assert_eq!(gone.walk_target, -1);
    assert_ne!(gone.state, WALK_POINT);
    // a unit without speed: its MoveTo fails at the start (13)
    let mut still = crate::hunt::tests::walker();
    still.stats.speed = 0.0;
    let mut held = BuildAction {
        target: -1,
        move_to: Some(MoveTo::Moving),
        ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
    };
    let _ = held
        .update(
            walk(&mut still, &mut world, me),
            None,
            builder(&mut waits, Some(7)),
        )
        .unwrap();
    assert_eq!(held.move_to, Some(MoveTo::Failed));
    // at the point, failed, or without a MoveTo: set_state(3)
    for end in [Some(MoveTo::Arrived), Some(MoveTo::Failed), None] {
        let mut w = BuildAction {
            state: WALK_POINT,
            target: -1,
            walk_target: -1,
            move_to: end,
            ..BuildAction::start(10913, [61.5, 92.5, 0.0], 0.8, TASK)
        };
        let step = w
            .update(
                walk(&mut mover, &mut world, me),
                None,
                builder(&mut waits, None),
            )
            .unwrap();
        assert_eq!((w.state, w.move_to), (IDLE, None));
        assert!(step.notified && step.stands);
    }
}

/// 428857364's builder 8676 (player 4) waiting for foundation 10645's footprint (state 222)
/// when its owner deletes the foundation in update 367 (state 8, still in the world): the
/// Build walks back to it (`set_state(4)`, then slot 0x68 with none), but the new `MoveTo`
/// refuses a target above state 6 (`FUN_140a7a230`) and walks to the foundation's place in
/// the same update, with the task's range and no target (the game's builder, 2.3 tiles off,
/// plans its path there).
#[test]
fn a_waiting_builder_whose_foundation_dies_walks_to_its_place() {
    use crate::spatial::Placed;
    let mut world = Spatial::default();
    world.insert(
        10645,
        Placed {
            position: [67.0, 112.0, 0.0],
            radius: [1.5, 1.5],
            class: 3,
            player: 4,
            master: 621,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        },
    );
    let mut mover = crate::hunt::tests::walker();
    let mut c = Construction {
        progress: 0.0,
        desolid: false,
        builders: 0,
        hit_points: 0,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let mut life = Lifecycle {
        state: 8,
        ..Lifecycle::default()
    };
    let mut b = BuildAction {
        state: WAIT,
        move_to: None,
        timer: 0.916,
        ..BuildAction::start(10645, [67.0, 112.0, 0.0], 0.8, TASK)
    };
    let site = Site {
        id: 10645,
        life: &mut life,
        construction: &mut c,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    // within the task's range of the place, so the walk ends without a path
    let me = Me {
        id: 8676,
        position: [67.05, 112.0, 0.0],
        group: -1,
        action_state: WAIT,
    };
    let mut waits = 0;
    let step = b
        .update(
            Walk {
                mover: Some(&mut mover),
                me,
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 118,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut waits,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: None,
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert!(step.released);
    assert_eq!(
        (b.state, b.target, b.walk_target, b.goal),
        (MOVE, -1, -1, Some([67.0, 112.0, 0.0]))
    );
    // the MoveTo ran in the same update
    assert_eq!(b.move_to, Some(MoveTo::Arrived));
    // a foundation the MoveTo takes (state 0) stays its target
    let mut taken = BuildAction::start(10645, [67.0, 112.0, 0.0], 0.8, TASK);
    let mut walk = Walk {
        mover: Some(&mut mover),
        me,
        dt: 0.02,
        world: &mut world,
        plan: None,
    };
    taken.restart(&mut walk, None, Some(0)).unwrap();
    assert_eq!((taken.walk_target, taken.goal), (10645, None));
    taken.restart(&mut walk, None, Some(7)).unwrap();
    assert_eq!(
        (taken.walk_target, taken.goal, taken.move_to),
        (-1, Some([67.0, 112.0, 0.0]), Some(MoveTo::Start))
    );
}

#[test]
fn a_builder_sent_to_another_foundation_starts_over() {
    let mut b = BuildAction {
        state: WORK,
        move_to: None,
        added: true,
        timer: 0.5,
        ..BuildAction::start(31148, [0.0; 3], 0.0, TASK)
    };
    b.retarget((31150, [0.0; 3]), 0.0, None).unwrap();
    assert_eq!(b, BuildAction::start(31150, [0.0; 3], 0.0, TASK));
    assert_eq!(
        (b.state, b.added, b.move_to),
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
        started: false,
    };
    let mut b = BuildAction {
        state: PLACE,
        move_to: None,
        added: true,
        ..BuildAction::start(31011, [0.0; 3], 0.0, TASK)
    };
    b.retake(([0.0; 3], 0.0), None, Some(&mut c.builders), false)
        .unwrap();
    assert_eq!(c.builders, 1);
    assert_eq!(b, BuildAction::start(31011, [0.0; 3], 0.0, TASK));
    assert_eq!(b.move_to, Some(MoveTo::Arrived));
    let mut working = BuildAction {
        state: WORK,
        move_to: None,
        ..BuildAction::start(31011, [0.0; 3], 0.0, TASK)
    };
    let before = working;
    working
        .retake(([0.0; 3], 0.0), None, Some(&mut c.builders), false)
        .unwrap();
    assert_eq!(working, before);
    assert_eq!(c.builders, 1);
}

/// MarvinBlue/426048731's house 10026 in update 1115 (player 1): the work that reaches 25 s
/// completes it: its build points hold the build time, its storage (population headroom, 4,
/// of mode 4) and attributes 98 and 165 go to its owner, it stands in state 2 and its
/// foundation (27) covers its snow with terrain 6 under layer 32.
#[test]
fn the_work_that_reaches_the_build_time_completes_the_building() {
    use crate::spatial::Tiles;
    use crate::terrain::{Terrain, TerrainKind};
    let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
    world.insert(
        10026,
        crate::spatial::Placed {
            position: [2.0, 2.0, 0.0],
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
    let master = Master {
        id: 70,
        kind: 80,
        class: 3,
        clearance: [1.0, 1.0],
        terrain_bonus: 4,
        storage: vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)],
        // a house's cost: 25 wood paid; the unpaid entry does not count
        cost: vec![(1, 25, true), (4, 5, false)],
        building: Some(crate::masters::BuildingMaster {
            adjacent_mode: 0,
            disappears_when_built: false,
            stack_unit: -1,
            foundation_terrain: 27,
            tech: -1,
            garrison_type: 0,
            garrison_heal_rate: 0.0,
            garrison_repair_rate: 0.0,
            annexes: [-1; 4],
            annex_offsets: [[0.0; 2]; 4],
            head_unit: -1,
            transform_unit: -1,
            destruction_sprite: -1,
            construction_sprite: -1,
        }),
        ..Master::default()
    };
    let mut life = Lifecycle {
        state: 0,
        hp: 548.0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut c = Construction {
        progress: 24.99,
        desolid: false,
        builders: 2,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let mut site = Site {
        id: 10026,
        life: &mut life,
        construction: &mut c,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: Some(&master),
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let worked = Construction::work(&mut site, 0.02, &mut world, &mut Pathing::default(), 0);
    assert_eq!(
        worked,
        Ok(Worked::Completed(Built {
            attributes: vec![(4, 5.0), (98, 25.0), (165, 25.0)],
            history: None,
            tech: None,
            cart: None,
            stack: None,
            vanishes: false,
            started: false,
            pasture: false,
        }))
    );
    assert_eq!((life.state, c.progress, c.snow), (2, 25.0, true));
    let t = world.terrain.as_ref().unwrap();
    assert_eq!((t.kinds[5], t.layers[5], t.kinds[0]), (6, 32, 32));
}

/// `FUN_1411dbb70`'s tail (`0x1411dd094`..): the castle (82) and the wonder (276) record
/// their building in their owner's history (events 6 and 4, [`Built::history`]), count in
/// attributes 134 and 173, 42 and 174, and add their paid cost to 184 rather than to 98 and
/// 165, an annex's too; the monastery (104) counts in 52 and adds its paid cost to 98 and
/// 165 as other buildings do, but as an annex.
#[test]
fn the_castle_the_wonder_and_the_monastery_count_their_completion() {
    use crate::history::{CASTLE_BUILT, WONDER_BUILT};
    use crate::spatial::Tiles;
    use crate::terrain::{Terrain, TerrainKind};
    fn complete(id: i16, cost: Vec<(i16, i16, bool)>, main: bool) -> Built {
        let mut world = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        world.insert(
            10026,
            crate::spatial::Placed {
                position: [2.0, 2.0, 0.0],
                master: id,
                ..house()
            },
        );
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
        let master = Master {
            id,
            kind: 80,
            class: 3,
            cost,
            building: Some(crate::masters::BuildingMaster {
                adjacent_mode: 0,
                disappears_when_built: false,
                stack_unit: -1,
                foundation_terrain: -1,
                tech: -1,
                garrison_type: 0,
                garrison_heal_rate: 0.0,
                garrison_repair_rate: 0.0,
                annexes: [-1; 4],
                annex_offsets: [[0.0; 2]; 4],
                head_unit: -1,
                transform_unit: -1,
                destruction_sprite: -1,
                construction_sprite: -1,
            }),
            ..Master::default()
        };
        let mut life = Lifecycle {
            state: 0,
            hp: 100.0,
            master_id: id,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress: 100.0,
            desolid: false,
            builders: 1,
            hit_points: 100,
            build_time: 100,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut site = Site {
            id: 10026,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: Some(&master),
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main,
                now: 0,
                rice: false,
            },
            chain: WorkChain::default(),
            main_builders: None,
        };
        let built = Construction::complete(&mut site, &mut world, &mut Pathing::default());
        assert_eq!(life.state, 2);
        built.unwrap()
    }
    let castle = || vec![(2, 650, true)];
    let wonder = || vec![(0, 1000, true), (1, 1000, true), (2, 1000, false)];
    let built = |attributes, history| Built {
        attributes,
        history,
        ..Built::default()
    };
    for main in [false, true] {
        assert_eq!(
            complete(82, castle(), main),
            built(
                vec![(134, 1.0), (173, 1.0), (184, 650.0)],
                Some(CASTLE_BUILT)
            )
        );
        assert_eq!(
            complete(276, wonder(), main),
            built(
                vec![(42, 1.0), (174, 1.0), (184, 2000.0)],
                Some(WONDER_BUILT)
            )
        );
    }
    assert_eq!(
        complete(104, vec![(1, 175, true)], false),
        built(vec![(52, 1.0), (98, 175.0), (165, 175.0)], None)
    );
    assert_eq!(
        complete(104, vec![(1, 175, true)], true),
        built(vec![(52, 1.0)], None)
    );
}

/// MarvinBlue/426048731's house 10028 in update 731: 9177's first work starts it (its record
/// marks its tiles) and adds 0.021 s of work: 550 hit points over 25 s, so 1.462 hit points.
#[test]
fn the_first_work_starts_the_foundation_and_adds_its_share_of_hit_points() {
    use crate::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    world.insert(10028, house());
    let mut life = Lifecycle {
        state: 0,
        hp: 1.0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut c = Construction {
        progress: 0.0,
        desolid: true,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let mut pathing = Pathing::default();
    let mut site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut c,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let rate = 1.0f32 * 1.0 * 0.021;
    assert_eq!(
        Construction::work(&mut site, rate, &mut world, &mut pathing, 0),
        Ok(Worked::Going)
    );
    assert_eq!(life.hp, 1.462);
    assert!(!c.desolid);
    assert_eq!(c.progress, f32::from_bits(STARTED) + rate);
    assert!(world.get(10028).unwrap().obstruction.is_some());
    assert!(world.tiles().word(29, 87).unwrap() & crate::spatial::BLOCKED != 0);
}

/// A gate's foundation (master 665) with its two posts (1192, annexes of a building type):
/// `FUN_1411cc430` starts each desolid post first (its record, its missiles per volley, the
/// clearing after the update), a post already started stays as it is, then the gate's own
/// first work starts it.
#[test]
fn the_work_starts_the_desolid_annexes_first() {
    use crate::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    let at = |x: f32, id: i16| crate::spatial::Placed {
        position: [x, 88.0, 0.0],
        master: id,
        ..house()
    };
    world.insert(10028, at(30.0, 665));
    world.insert(10029, at(28.5, 1192));
    world.insert(10030, at(31.5, 1192));
    let foundation = |desolid| Construction {
        progress: 0.0,
        desolid,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let (mut post, mut started_post) = (foundation(true), foundation(false));
    let (mut post_fire, mut started_fire) = (0.0, 0.0);
    let mut life = Lifecycle {
        state: 0,
        hp: 1.0,
        master_id: 665,
        ..Lifecycle::default()
    };
    let mut c = foundation(true);
    let mut pathing = Pathing::default();
    let mut site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut c,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: true,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain {
            annexes: vec![
                AnnexStart {
                    id: 10029,
                    construction: &mut post,
                    mover: None,
                    firepower: &mut post_fire,
                    volley: 2.0,
                },
                AnnexStart {
                    id: 10030,
                    construction: &mut started_post,
                    mover: None,
                    firepower: &mut started_fire,
                    volley: 2.0,
                },
            ],
            main: None,
        },
        main_builders: None,
    };
    assert_eq!(
        Construction::work(&mut site, 0.021, &mut world, &mut pathing, 0),
        Ok(Worked::Going)
    );
    assert_eq!(world.take_started(), vec![10029, 10028]);
    assert!(!post.desolid && post.started);
    assert_eq!(post.progress, f32::from_bits(STARTED));
    assert_eq!(post_fire, 2.0);
    assert!(world.get(10029).unwrap().obstruction.is_some());
    assert_eq!((started_post, started_fire), (foundation(false), 0.0));
    assert!(!c.desolid);
}

/// The construction's start (`FUN_1411e5670`) sets the foundation's last move time to the
/// world's time plus 1, which a walker later in the same update reads (slot 0x5e8) when it
/// meets the new record: it outranks a path made before (429513284's builder 30830 meeting
/// the house 33439 that 30828 started in update 199).
#[test]
fn a_construction_s_start_moves_its_foundation_for_the_walks_of_the_update() {
    use crate::spatial::Tiles;
    let mut world = Spatial::with_tiles([120, 120], Tiles::new(120, 120, vec![4; 120 * 120]));
    world.insert(
        33439,
        crate::spatial::Placed {
            position: [194.0, 64.0, 0.0],
            ..house()
        },
    );
    world.set_gait(
        33439,
        Some(crate::spatial::Gait {
            last_move: 7000,
            ..crate::spatial::Gait::default()
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
        started: false,
    };
    let mut mover = crate::hunt::tests::walker();
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
/// work (`FUN_1411cc430`'s loop): the post itself is not worked, and the result is the main
/// building's (its start's build points, then the work): 0 while it goes on, 1 once it
/// reaches its build time.
#[test]
fn an_annex_s_work_goes_to_its_main_building() {
    let mut world = Spatial::default();
    let foundation = Construction {
        progress: 0.0,
        desolid: true,
        builders: 1,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let mut run = |main: Construction, amount: f32| {
        let mut life = Lifecycle {
            state: 0,
            hp: 1.0,
            master_id: 1192,
            ..Lifecycle::default()
        };
        let mut c = foundation;
        let mut site = Site {
            id: 10029,
            life: &mut life,
            construction: &mut c,
            mover: None,
            firepower: None,
            roo_called: None,
            store: None,
            completion: Completion {
                master: None,
                annexed: false,
                attribute_204: false,
                mode_680: 0,
                farm_bonus: [0.0, -1.0, -1.0],
                masters: None,
                computer: false,
                ai: false,
                main: true,
                now: 0,
                rice: false,
            },
            chain: WorkChain {
                annexes: Vec::new(),
                main: Some(MainWork {
                    construction: main,
                    hp: 1.0,
                }),
            },
            main_builders: None,
        };
        let worked = Construction::work(&mut site, amount, &mut world, &mut Pathing::default(), 0);
        assert_eq!(
            (c, life.hp),
            (foundation, 1.0),
            "the post itself is not worked"
        );
        worked
    };
    assert_eq!(
        run(foundation, 0.021),
        Ok(Worked::Main {
            amount: 0.021,
            going: true
        })
    );
    let nearly = Construction {
        progress: 24.99,
        desolid: false,
        ..foundation
    };
    assert_eq!(
        run(nearly, 0.021),
        Ok(Worked::Main {
            amount: 0.021,
            going: false
        })
    );
}

use crate::movement::Me;
use crate::spatial::Spatial;

/// MarvinBlue/426048731 after tick 679: builder 9175 stands on the house foundation 10028's
/// footprint, works on it (state 6) and walks off it in update 681 (state 223) to the free tile
/// the game picks, (29.5, 89.5).
#[test]
fn a_builder_on_its_footprint_faces_it_then_walks_to_a_free_tile_beside_it() {
    use crate::movement::{MoveStats, Mover};
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
    let unit = |x, y, radius, master, kind| Placed {
        position: [x, y, 0.0],
        radius: [radius, radius],
        class: 4,
        player: 2,
        master,
        obstruction_type: kind,
        buildable_on: 0,
        tall: true,
        speed: 0.0,
        unknown: None,
        obstruction: None,
    };
    world.insert(9175, unit(30.0, 89.0, 0.2, 118, 5));
    world.insert(10028, unit(30.0, 88.0, 1.0, 70, 2));
    let mut mover = Mover {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 0.0, -1.0],
        velocity: None,
        paths: vec![],
        factor: 1.0,
        stats: MoveStats {
            fly_mode: 0,
            speed: 0.8,
            turn_rate: f32::MAX,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 0.5],
            restriction: 7,
            restriction_mask: 6,
            turn_time: 0.0,
            move_algorithm: 0,
            min_collision: 1.0,
            fast_turn: false,
            walk_speed: 1.0,
            stand_speed: 1.0,
            run_speed: 1.0,
            facets: Some(crate::facing::SpriteFacets {
                count: 16,
                flag_1: true,
            }),
            weapon_range: 0.0,
        },
        waiting: 0,
        wait_delays: 0,
        attack_moving: false,
        last_move: 0,
        moved_at: [0.0; 3],
        substitute_count: 0,
        ignore_units: Vec::new(),
        user_waypoints: Vec::new(),
        last_turn: 0.0,
        angle: 0.0,
        move_to_point: None,
        speed_stopped: false,
        pauses: Vec::new(),
        in_the_way: Vec::new(),
        sprite_facets: None,
    };
    let mut build = BuildAction {
        target: 10028,
        state: 6,
        move_to: None,
        goal: None,
        walk_target: -1,
        point: [-1.0; 3],
        task: TASK,
        added: false,
        timer: 0.0,
        move_wait: MoveWait::default(),
        site: -1,
        drop_range: 0.0,
    };
    let mut life = Lifecycle {
        state: 0,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 0.0,
        desolid: true,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let _ = build
        .update(
            Walk {
                mover: Some(&mut mover),
                me: Me {
                    id: 9175,
                    position: [30.0, 89.0, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(0),
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert_eq!(
        (build.state, build.move_to, build.goal),
        (223, Some(MoveTo::Start), Some([29.5, 89.5, 0.0]))
    );
}

/// One update of `build` by builder 5 of `class` at `at` (a walker of the hunt's tests, its
/// AI's stance `stance`) on player 1's desolid house 10028 at (10.5, 10.5), in a world of
/// 20 x 20 tiles where `prepare` adds what the test needs; without a planner.
fn walk_update(
    build: &mut BuildAction,
    (class, at, computer): (i16, [f32; 3], bool),
    prepare: impl FnOnce(&mut Spatial),
) -> (BuildStep, Spatial) {
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    world.insert(
        10028,
        Placed {
            position: [10.5, 10.5, 0.0],
            player: 1,
            ..house()
        },
    );
    prepare(&mut world);
    let mut life = Lifecycle {
        state: 0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 0.0,
        desolid: true,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let mut mover = crate::hunt::tests::walker();
    mover.moved_at = at;
    let step = build
        .update(
            Walk {
                mover: Some(&mut mover),
                me: Me {
                    id: 5,
                    position: at,
                    group: -1,
                    action_state: 0,
                },
                dt: 0.25,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(0),
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    (step, world)
}

/// The waiting `MoveTo` (state 2): the world's dt goes onto its timer; at its delay it asks
/// again (`set_state(3)`), and that start marks the wait asked (`+0x6c` 2).
#[test]
fn a_build_s_waiting_move_to_asks_again_at_its_delay() {
    let mut b = BuildAction {
        move_to: Some(MoveTo::Waiting),
        move_wait: MoveWait {
            state: 1,
            timer: 0.5,
            delay: 1.0,
        },
        ..BuildAction::start(10028, [10.5, 10.5, 0.0], 5.0, TASK)
    };
    let _ = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!(
        (b.move_to, b.move_wait.timer),
        (Some(MoveTo::Waiting), 0.75)
    );
    let _ = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!((b.move_to, b.move_wait.timer), (Some(MoveTo::Start), 0.0));
    // in reach now: its start arrives at once, no path asked
    let _ = walk_update(&mut b, (VILLAGER, [11.6, 10.5, 0.0], false), |_| {});
    assert_eq!((b.move_wait.state, b.move_to), (2, Some(MoveTo::Arrived)));
}

/// The update's case 3 at the `MoveTo`'s end: a villager works wherever it stands
/// (`set_state(6)`); another builder out of the task's range and [`REACH`] of the footprint
/// walks to it again (`set_state(4)`), one within it works.
#[test]
fn a_builder_at_its_move_to_s_end_works_or_walks_again() {
    let arrived = || BuildAction {
        move_to: Some(MoveTo::Arrived),
        ..BuildAction::start(10028, [10.5, 10.5, 0.0], 5.0, TASK)
    };
    let mut b = arrived();
    let _ = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!(b.state, WORK);
    let mut b = arrived();
    let _ = walk_update(&mut b, (21, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!((b.state, b.move_to), (MOVE, Some(MoveTo::Start)));
    let mut b = arrived();
    let _ = walk_update(&mut b, (21, [11.6, 10.5, 0.0], false), |_| {});
    assert_eq!(b.state, WORK);
    // without a sub-action the list's update returns 1 as well
    let mut b = BuildAction {
        move_to: None,
        ..arrived()
    };
    let _ = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!(b.state, WORK);
}

/// The update's case 3 on a failed `MoveTo` (3): a desolid foundation with another player's
/// marks its owner sees goes (a human owner's, the caller's) and the Build halts; without
/// the marks (and without a planner here) it waits (`set_state(3)`).
#[test]
fn a_builder_whose_move_to_failed_waits_or_halts_on_marks() {
    let failed = || BuildAction {
        move_to: Some(MoveTo::Failed),
        ..BuildAction::start(10028, [10.5, 10.5, 0.0], 5.0, TASK)
    };
    let mut b = failed();
    let (step, _) = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |_| {});
    assert_eq!((b.state, step.removes), (IDLE, None));
    let mut b = failed();
    let (step, _) = walk_update(&mut b, (VILLAGER, [2.5, 2.5, 0.0], false), |w| {
        w.tiles_mut_for_test().mark_for_test(10, 10, 0x80_0000 << 3);
    });
    // no players' sight in this world: its owner sees nothing, so it waits
    assert_eq!((b.state, step.removes), (IDLE, None));
}

/// census v5's 240 stops "a foundation clears objects off its footprint": the Build's
/// `set_state(7)` on a desolid foundation clears its footprint with `param_6`
/// (`FUN_1415c3620`): a stump (a tree of at most 1 hit point) on it loses its record at once
/// (`FUN_141189670`), its delete is the caller's; the occupants' test passes over it (state
/// 7) and the tiles hold no other player's mark: the builder joins the work.
#[test]
fn a_builder_s_set_state_7_clears_a_stump_and_joins() {
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    world.insert(
        10028,
        Placed {
            position: [10.5, 10.5, 0.0],
            player: 1,
            ..house()
        },
    );
    world.insert(
        77,
        Placed {
            position: [10.2, 10.2, 0.0],
            radius: [0.5, 0.5],
            class: TREE,
            player: 0,
            master: 349,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 0 }),
        },
    );
    assert_ne!(world.tiles().word(10, 10).unwrap() >> 23, 0);
    let looks = |id: i32| {
        (id == 77).then_some(ClearLook {
            hp: 0.5,
            height: 1.0,
            roo_called: false,
            kind: 10,
            state: 2,
            desolid: false,
        })
    };
    let mut life = Lifecycle {
        state: 0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 0.0,
        desolid: true,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let mut build = BuildAction {
        state: WAIT,
        move_to: None,
        timer: 0.01,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = build
        .update(
            Walk {
                mover: None,
                me: Me {
                    id: 5,
                    position: [12.5, 10.5, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&looks),
                walk_target_state: Some(0),
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert_eq!(
        step.cleared,
        [Cleared {
            id: 77,
            wall: false,
            record_goes: true,
        }]
    );
    assert_eq!(step.occupants, Vec::<i32>::new());
    assert_eq!(world.get(77).unwrap().obstruction, None);
    assert_eq!(world.tiles().word(10, 10).unwrap() >> 23, 0);
    assert_eq!(
        (build.state, build.added, construction.builders),
        (PLACE, true, 1)
    );
}

/// `set_state(7)` with another player's marks on the footprint (`FUN_1411d4b80`): the
/// foundation goes when its owner is human (`FUN_1411ccbb0`, the caller's,
/// [`BuildStep::removes`]) and `set_state(2)` halts the Build: the standing sprite, the
/// foundation let go, no point.
#[test]
fn another_player_s_marks_on_the_footprint_halt_the_build() {
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    world.insert(
        10028,
        Placed {
            position: [10.5, 10.5, 0.0],
            player: 1,
            ..house()
        },
    );
    world.insert(
        77,
        Placed {
            position: [10.2, 10.2, 0.0],
            radius: [0.5, 0.5],
            class: TREE,
            player: 0,
            master: 349,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 0 }),
        },
    );
    assert_ne!(world.tiles().word(10, 10).unwrap() >> 23, 0);
    let looks = |id: i32| {
        (id == 77).then_some(ClearLook {
            hp: 0.5,
            height: 1.0,
            roo_called: false,
            kind: 10,
            state: 2,
            desolid: false,
        })
    };
    let mut life = Lifecycle {
        state: 0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 0.0,
        desolid: true,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    // player 3's mark on a tile of the footprint (10, 10)
    world
        .tiles_mut_for_test()
        .mark_for_test(10, 10, 0x80_0000 << 3);
    let mut build = BuildAction {
        state: WAIT,
        move_to: None,
        timer: 0.01,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = build
        .update(
            Walk {
                mover: None,
                me: Me {
                    id: 5,
                    position: [12.5, 10.5, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&looks),
                walk_target_state: Some(0),
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert_eq!(
        (step.removes, step.stands, step.released),
        (Some(10028), true, true)
    );
    assert_eq!(
        (build.state, build.target, build.point),
        (HALTED, -1, [-1.0; 3])
    );
}

/// `set_state(7)`'s `LAB_140aaf5d4` on a footprint someone stands on, the builder outside it:
/// after 21 waits it gives up (the interface's event 0x88, not held): `set_state(2)` halts
/// the Build and its waits (`+0x889`) start again.
#[test]
fn a_builder_waiting_21_times_on_an_occupied_footprint_gives_up() {
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    world.insert(
        10028,
        Placed {
            position: [10.5, 10.5, 0.0],
            player: 1,
            ..house()
        },
    );
    world.insert(
        77,
        Placed {
            position: [10.2, 10.2, 0.0],
            radius: [0.5, 0.5],
            class: TREE,
            player: 0,
            master: 349,
            obstruction_type: 2,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 0 }),
        },
    );
    assert_ne!(world.tiles().word(10, 10).unwrap() >> 23, 0);
    let looks = |id: i32| {
        (id == 77).then_some(ClearLook {
            hp: 0.5,
            height: 1.0,
            roo_called: false,
            kind: 10,
            state: 2,
            desolid: false,
        })
    };
    let mut life = Lifecycle {
        state: 0,
        master_id: 70,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 0.0,
        desolid: true,
        builders: 0,
        hit_points: 550,
        build_time: 25,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: None,
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    // a villager stands on the footprint
    world.insert(
        6,
        Placed {
            position: [10.6, 10.6, 0.0],
            radius: [0.2, 0.2],
            class: VILLAGER,
            player: 1,
            master: 83,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.8,
            unknown: None,
            obstruction: None,
        },
    );
    let mut waits = MAX_WAITS;
    let mut build = BuildAction {
        state: WAIT,
        move_to: None,
        timer: 0.01,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = build
        .update(
            Walk {
                mover: None,
                me: Me {
                    id: 5,
                    position: [12.5, 10.5, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 83,
                held: 0.0,
                held_kind: -1,
                drop_site: false,
                ai: None,
                waits: &mut waits,
                build_targets: &[],
                look: Looks(&looks),
                walk_target_state: Some(0),
                deposit: None,
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert_eq!(
        (step.removes, step.stands, step.released, step.occupants),
        (None, true, true, vec![6])
    );
    assert_eq!(
        (build.state, build.target, build.point, waits),
        (HALTED, -1, [-1.0; 3], 0)
    );
}

/// `0x140ab0370`'s pasture branch: a human player's builder whose foundation of master 1889
/// is built (its state above 0) ends its unit AI's order and action, tells it of the pasture
/// the stack unit made (notify 0x21d, [`BuildStep::pasture_made`]), lets the foundation go,
/// gives its load of berries to its player as food and ends (`set_state(1)`, returns 1).
#[test]
fn a_human_builder_of_a_built_pasture_hands_it_over() {
    let pasture = Master {
        id: MASTER_1889,
        ..Master::default()
    };
    let mut life = Lifecycle {
        state: 2,
        master_id: MASTER_1889,
        ..Lifecycle::default()
    };
    let mut construction = Construction {
        progress: 20.0,
        desolid: false,
        builders: 1,
        hit_points: 550,
        build_time: 20,
        record: Obstruction { kind: 2, class: 2 },
        snow: false,
        sprite_override: false,
        started: false,
    };
    let site = Site {
        id: 10028,
        life: &mut life,
        construction: &mut construction,
        mover: None,
        firepower: None,
        roo_called: None,
        store: None,
        completion: Completion {
            master: Some(&pasture),
            annexed: false,
            attribute_204: false,
            mode_680: 0,
            farm_bonus: [0.0, -1.0, -1.0],
            masters: None,
            computer: false,
            ai: false,
            main: false,
            now: 0,
            rice: false,
        },
        chain: WorkChain::default(),
        main_builders: None,
    };
    let mut world = Spatial::default();
    world.insert(10028, house());
    let mut ai = UnitAi {
        current_order: BUILD_ORDER,
        current_action: 602,
        ..UnitAi::default()
    };
    let mut b = BuildAction {
        state: WORK,
        move_to: None,
        added: true,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = b
        .update(
            Walk {
                mover: None,
                me: Me {
                    id: 9,
                    position: [0.0; 3],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.02,
                world: &mut world,
                plan: None,
            },
            Some(site),
            Builder {
                class: VILLAGER,
                master: 83,
                held: 7.0,
                held_kind: 17,
                drop_site: false,
                ai: Some(&mut ai),
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(2),
                deposit: None,
                attributes: 300,
                gone_orders: &[],
                stack_made: Some(10031),
            },
        )
        .unwrap();
    assert_eq!(step.pasture_made, Some(10031));
    assert_eq!(step.dropped, Some((0, 7.0)));
    assert!(step.done && step.released && step.stands);
    assert_eq!((b.state, b.target, b.added), (DONE, -1, false));
    assert_eq!((ai.current_order, ai.current_action), (-1, -1));
    assert_eq!(construction.builders, 0);
}

/// The update's case 1 and case 0xc (`0x140ab0370`): a Build its unit's stop halted (state
/// 2) fails (`set_state(13)`, as 1: the unit takes its standing sprite,
/// [`BuildStep::stands`]) and the update returns 0; failed, it posts notify 0x1f9 with its
/// action value to its unit AI ([`BuildStep::failed`]), halts (`set_state(2)`: the unit
/// stands, a target goes, [`BuildStep::released`]) and returns 3, which removes it
/// ([`BuildStep::done`]). In state 3 with a target the update does nothing (the default
/// case).
#[test]
fn a_halted_build_fails_and_its_failure_removes_it() {
    fn update(b: &mut BuildAction, world: &mut Spatial) -> BuildStep {
        let mut ai = UnitAi::default();
        let walk = Walk {
            mover: None,
            me: Me {
                id: 9,
                position: [0.0; 3],
                group: -1,
                action_state: 0,
            },
            dt: 0.02,
            world,
            plan: None,
        };
        let builder = Builder {
            class: VILLAGER,
            master: 83,
            held: 0.0,
            held_kind: -1,
            drop_site: false,
            ai: Some(&mut ai),
            waits: &mut 0,
            build_targets: &[],
            look: Looks(&|_| None),
            walk_target_state: Some(0),
            deposit: None,
            attributes: 4,
            gone_orders: &[],
            stack_made: None,
        };
        b.update(walk, None, builder).unwrap()
    }
    let mut world = Spatial::default();
    world.insert(10028, house());
    let mut b = BuildAction {
        state: HALTED,
        move_to: None,
        target: -1,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = update(&mut b, &mut world);
    assert_eq!(b.state, FAILED);
    assert_eq!(
        step,
        BuildStep {
            stands: true,
            ..BuildStep::default()
        }
    );
    let step = update(&mut b, &mut world);
    assert_eq!((b.state, b.target), (HALTED, -1));
    assert_eq!(
        step,
        BuildStep {
            stands: true,
            failed: true,
            done: true,
            ..BuildStep::default()
        }
    );
    // failed with its foundation: the halt lets it go
    let mut holding = BuildAction {
        state: FAILED,
        move_to: None,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let step = update(&mut holding, &mut world);
    assert_eq!((holding.state, holding.target), (HALTED, -1));
    assert!(step.released && step.failed && step.done);
    // waiting with a target: nothing
    let mut waiting = BuildAction {
        state: IDLE,
        move_to: None,
        ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
    };
    let before = waiting;
    let step = update(&mut waiting, &mut world);
    assert_eq!((waiting, step), (before, BuildStep::default()));
}

/// census v5's 217 stops "a dying foundation" went on to "a Build whose foundation died":
/// a builder at work (state 7, among the builders) on a foundation in state 3
/// (`0x140ab0370`'s prelude): `set_state(4)` toward it (out of its builders, a `MoveTo`
/// arrived beside it), slot 0x68 with none (the caller releases it); the `MoveTo` is there,
/// so `set_state(6)` without a target turns to `set_state(3)`: without a unit AI it tells
/// itself 0x202. Master 621 and the gates (class 39) end their work instead once built
/// (`set_state(224)`), a foundation in state 1 or 2 too.
#[test]
fn a_builder_lets_a_dead_foundation_go() {
    use crate::movement::{MoveStats, Mover};
    let mover = || Mover {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 0.0, -1.0],
        velocity: None,
        paths: vec![],
        factor: 1.0,
        stats: MoveStats {
            fly_mode: 0,
            speed: 0.8,
            turn_rate: f32::MAX,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 0.5],
            restriction: 7,
            restriction_mask: 6,
            turn_time: 0.0,
            move_algorithm: 0,
            min_collision: 1.0,
            fast_turn: false,
            walk_speed: 1.0,
            stand_speed: 1.0,
            run_speed: 1.0,
            facets: None,
            weapon_range: 0.0,
        },
        waiting: 0,
        wait_delays: 0,
        attack_moving: false,
        last_move: 0,
        moved_at: [0.0; 3],
        substitute_count: 0,
        ignore_units: Vec::new(),
        user_waypoints: Vec::new(),
        last_turn: 0.0,
        angle: 0.0,
        move_to_point: None,
        speed_stopped: false,
        pauses: Vec::new(),
        in_the_way: Vec::new(),
        sprite_facets: None,
    };
    let run = |master: Master, state: u8, progress: f32| {
        let mut world = Spatial::default();
        world.insert(10028, house());
        let mut life = Lifecycle {
            state,
            master_id: master.id,
            ..Lifecycle::default()
        };
        let mut c = Construction {
            progress,
            desolid: false,
            builders: 2,
            hit_points: 550,
            build_time: 25,
            record: Obstruction { kind: 2, class: 2 },
            snow: false,
            sprite_override: false,
            started: false,
        };
        let mut b = BuildAction {
            state: PLACE,
            move_to: None,
            added: true,
            ..BuildAction::start(10028, [0.0; 3], 0.0, TASK)
        };
        let mut m = mover();
        let mut ai = UnitAi::default();
        let step = b
            .update(
                Walk {
                    mover: Some(&mut m),
                    me: Me {
                        id: 9,
                        position: [30.0, 89.1, 0.0],
                        group: -1,
                        action_state: 0,
                    },
                    dt: 0.02,
                    world: &mut world,
                    plan: None,
                },
                Some(Site {
                    id: 10028,
                    life: &mut life,
                    construction: &mut c,
                    mover: None,
                    firepower: None,
                    roo_called: None,
                    store: None,
                    completion: Completion {
                        master: Some(&master),
                        annexed: false,
                        attribute_204: false,
                        mode_680: 0,
                        farm_bonus: [0.0, -1.0, -1.0],
                        masters: None,
                        computer: false,
                        ai: false,
                        main: false,
                        now: 0,
                        rice: false,
                    },
                    chain: WorkChain::default(),
                    main_builders: None,
                }),
                Builder {
                    class: VILLAGER,
                    master: 83,
                    held: 0.0,
                    held_kind: -1,
                    drop_site: false,
                    ai: Some(&mut ai),
                    waits: &mut 0,
                    build_targets: &[],
                    look: Looks(&|_| None),
                    walk_target_state: Some(0),
                    deposit: None,
                    attributes: 4,
                    gone_orders: &[],
                    stack_made: None,
                },
            )
            .unwrap();
        (
            b.state,
            b.target,
            c.builders,
            step.released,
            step.notified || step.search,
        )
    };
    let house_master = Master {
        id: 70,
        class: 3,
        ..Master::default()
    };
    assert_eq!(
        run(house_master.clone(), 3, 10.0),
        (IDLE, -1, 1, true, true)
    );
    // a built one in state 2 ends the work
    assert_eq!(
        run(house_master, 2, 25.0),
        (FINISHED, 10028, 1, false, true)
    );
    // a dead gate (class 39) not built yet lets go, one built ends the work
    let gate = Master {
        id: 1584,
        class: CLASS_39,
        ..Master::default()
    };
    assert_eq!(run(gate.clone(), 3, 10.0), (IDLE, -1, 1, true, true));
    assert_eq!(run(gate, 3, 25.0), (FINISHED, 10028, 1, false, true));
}

#[test]
fn a_builder_at_the_foundation_works_in_its_first_update_the_others_walk_to_it() {
    fn walk(id: i32, world: &mut Spatial) -> Walk<'_> {
        Walk {
            mover: None,
            me: Me {
                id,
                position: [0.0; 3],
                group: -1,
                action_state: 0,
            },
            dt: 0.02,
            world,
            plan: None,
        }
    }
    let mut near = BuildAction::start(10028, [0.0; 3], 0.0, TASK);
    let mut far = BuildAction::start(10028, [0.0; 3], 0.8, TASK);
    assert_eq!((near.state, near.move_to), (4, Some(MoveTo::Arrived)));
    assert_eq!((far.state, far.move_to), (4, Some(MoveTo::Start)));
    let mut world = Spatial::default();
    let mut waits = [0u8; 3];
    let mut waits = waits.iter_mut();
    let mut builder = || Builder {
        class: VILLAGER,
        master: 83,
        held: 0.0,
        held_kind: -1,
        drop_site: false,
        ai: None,
        waits: waits.next().expect("a wait count per update"),
        build_targets: &[],
        look: Looks(&|_| None),
        walk_target_state: Some(0),
        deposit: None,
        attributes: 4,
        gone_orders: &[],
        stack_made: None,
    };
    world.insert(10028, house());
    let _ = near.update(walk(1, &mut world), None, builder()).unwrap();
    assert_eq!((near.state, near.move_to), (6, None));
    // census v5's 208 stops "a Build whose foundation is gone": the far one finds its
    // foundation's id gone from the world (0x140ab0370's prelude): slot 0x68 with none, then
    // set_state(3): its MoveTo goes, and without orders it tells its unit AI (0x202)
    let mut empty = Spatial::default();
    let step = far.update(walk(2, &mut empty), None, builder()).unwrap();
    assert_eq!(
        (far.state, far.target, far.move_to, step.notified, step.done),
        (IDLE, -1, None, true, false)
    );
}
/// The house 10028 at (10.5, 10.5) and the mill 300 at (15.5, 10.5) of player 1, on 20 x 20
/// tiles.
fn drop_world() -> Spatial {
    use crate::spatial::{Placed, Tiles};
    let mut world = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
    world.insert(
        10028,
        Placed {
            position: [10.5, 10.5, 0.0],
            player: 1,
            ..house()
        },
    );
    world.insert(
        300,
        Placed {
            position: [15.5, 10.5, 0.0],
            player: 1,
            master: 68,
            ..house()
        },
    );
    world
}

/// `set_state(8)` (`0x140aae200` case 8): the prelude takes a counted builder out of the
/// builders; the drop site found becomes the second target (`FUN_140a7a190`) and a quiet
/// `MoveTo` to it within the search's range starts. None found lets the old one go, tells the
/// unit AI (notify 0x203) and walks back to the foundation (`set_state(4)`), as a builder
/// carrying nothing does without a search. Without a foundation that `MoveTo` has no target
/// and walks to its zeroed point.
#[test]
fn a_builder_s_drop_off_walks_to_the_drop_site_its_search_found() {
    let mut world = drop_world();
    let mut mover = crate::hunt::tests::walker();
    let me = Me {
        id: 5,
        position: [2.5, 2.5, 0.0],
        group: -1,
        action_state: 0,
    };
    let mut b = BuildAction {
        state: WORK,
        added: true,
        ..BuildAction::start(10028, [10.5, 10.5, 0.0], 5.0, TASK)
    };
    let mut builders = 3;
    let mut walk = Walk {
        mover: Some(&mut mover),
        me,
        dt: 0.0,
        world: &mut world,
        plan: None,
    };
    let out = b
        .set_drop(
            &mut walk,
            Some(&mut builders),
            DropSearch::Site {
                id: 300,
                range: 0.5,
            },
        )
        .unwrap();
    assert_eq!(
        out,
        DropStart {
            let_go: -1,
            took: 300,
            notified: false
        }
    );
    assert_eq!(
        (b.state, b.site, b.walk_target, b.drop_range, b.move_to),
        (DROP, 300, 300, 0.5, Some(MoveTo::Start))
    );
    assert_eq!((b.added, builders, b.target), (false, 2, 10028));
    let out = b.set_drop(&mut walk, None, DropSearch::Nothing).unwrap();
    assert_eq!(
        out,
        DropStart {
            let_go: 300,
            took: -1,
            notified: true
        }
    );
    assert_eq!(
        (b.state, b.site, b.walk_target, b.move_to, b.goal),
        (MOVE, -1, 10028, Some(MoveTo::Start), None)
    );
    let out = b.set_drop(&mut walk, None, DropSearch::Empty).unwrap();
    assert_eq!((out.let_go, out.took, out.notified), (-1, -1, false));
    assert_eq!((b.state, b.move_to), (MOVE, Some(MoveTo::Start)));
    // beside the site already: arrived at once
    walk.me.position = [14.5, 10.5, 0.0];
    let _ = b
        .set_drop(
            &mut walk,
            None,
            DropSearch::Site {
                id: 300,
                range: 0.5,
            },
        )
        .unwrap();
    assert_eq!((b.state, b.move_to), (DROP, Some(MoveTo::Arrived)));
    let mut lost = BuildAction { target: -1, ..b };
    let _ = lost.set_drop(&mut walk, None, DropSearch::Empty).unwrap();
    assert_eq!(
        (
            lost.state,
            lost.walk_target,
            lost.goal,
            lost.move_to,
            lost.site
        ),
        (MOVE, -1, Some([0.0; 3]), Some(MoveTo::Start), 300)
    );
}

/// One update of `build` in state 8 by villager 5 at (14, 10.5) carrying `held` wood, its
/// drop site read as `deposit`, its unit AI `ai` and the targets gone `gone`, in
/// [`drop_world`].
fn drop_update(
    build: &mut BuildAction,
    (held, deposit): (f32, Option<(u8, crate::hunt::DropOff)>),
    ai: Option<&mut UnitAi>,
    gone: &[i32],
) -> BuildStep {
    let mut world = drop_world();
    let mut mover = crate::hunt::tests::walker();
    build
        .update(
            Walk {
                mover: Some(&mut mover),
                me: Me {
                    id: 5,
                    position: [14.0, 10.5, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.25,
                world: &mut world,
                plan: None,
            },
            None,
            Builder {
                class: VILLAGER,
                master: 83,
                held,
                held_kind: 1,
                drop_site: false,
                ai,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(2),
                deposit,
                attributes: 4,
                gone_orders: gone,
                stack_made: None,
            },
        )
        .unwrap()
}

/// A Build of foundation 10028 in state 8 at its drop site 300.
fn at_the_mill() -> BuildAction {
    BuildAction {
        state: DROP,
        move_to: Some(MoveTo::Arrived),
        site: 300,
        walk_target: 300,
        drop_range: 0.5,
        ..BuildAction::start(10028, [10.5, 10.5, 0.0], 5.0, TASK)
    }
}

/// The drop site as the drop-off reads it: in state `state`, taking the load or not, `gap`
/// squared away.
const fn mill(state: u8, takes: bool, gap: f32) -> (u8, crate::hunt::DropOff) {
    (
        state,
        crate::hunt::DropOff {
            takes,
            gap_squared: gap,
        },
    )
}

/// State 8's update (`0x140ab0370` case 7) at the `MoveTo`'s end: at a drop site in state 2
/// that takes it within 1 the load goes to the player (wood, [`BuildStep::dropped`]), the
/// drop site goes and the builder walks back to its foundation (`set_state(4)`). Farther it
/// waits 3 seconds with its animation stopped, then searches again; a drop site that does not
/// take the load halts the Build; one not in state 2, or none, a failed walk and a walk whose
/// target went past state 6 search again.
#[test]
fn a_builder_at_its_drop_site_gives_its_load_and_walks_back() {
    let mut b = at_the_mill();
    let step = drop_update(&mut b, (10.0, Some(mill(2, true, 0.5))), None, &[]);
    assert_eq!(step.dropped, Some((1, 10.0)));
    assert_eq!(step.site_let_go, Some(300));
    assert_eq!((b.state, b.site, b.target), (MOVE, -1, 10028));
    assert_eq!(b.move_to, Some(MoveTo::Start));
    let mut far = at_the_mill();
    let step = drop_update(&mut far, (10.0, Some(mill(2, true, 1.5))), None, &[]);
    assert!(step.anim_stopped && step.dropped.is_none());
    assert_eq!((far.state, far.timer, far.move_to), (DROP, -3.0, None));
    let step = drop_update(&mut far, (10.0, Some(mill(2, true, 1.5))), None, &[]);
    assert!(!step.drop_off);
    assert_eq!(far.timer, -2.75);
    far.timer = -0.1;
    let step = drop_update(&mut far, (10.0, Some(mill(2, true, 1.5))), None, &[]);
    assert!(step.drop_off);
    assert_eq!(far.timer, 0.0);
    let mut refused = at_the_mill();
    let step = drop_update(&mut refused, (10.0, Some(mill(2, false, 0.5))), None, &[]);
    assert_eq!(
        (step.stands, step.released, step.site_let_go),
        (true, true, Some(300))
    );
    assert_eq!(
        (refused.state, refused.target, refused.site),
        (HALTED, -1, -1)
    );
    for (deposit, move_to) in [
        (Some(mill(3, true, 0.5)), MoveTo::Arrived),
        (None, MoveTo::Arrived),
        (Some(mill(2, true, 0.5)), MoveTo::Failed),
    ] {
        let mut b = BuildAction {
            move_to: Some(move_to),
            ..at_the_mill()
        };
        let step = drop_update(&mut b, (10.0, deposit), None, &[]);
        assert!(step.drop_off && step.dropped.is_none(), "{deposit:?}");
        assert_eq!(b.state, DROP);
    }
}

/// State 8 without a foundation: after the load its unit AI drops its orders 0x2c5, 0x2c9
/// and 0x2db on targets gone (`FUN_140c5de60`); with orders left it has no order and the
/// Build is done (`set_state(1)`, the update returns 1), with none it waits (`set_state(3)`:
/// notify 0x202, the standing sprite). A builder without a unit AI and a point of −1 halts
/// (a human's, `set_state(2)`: the drop site goes); with a point it walks there
/// (`set_state(11)`).
#[test]
fn a_builder_without_a_foundation_after_its_drop_off() {
    let lone = || BuildAction {
        target: -1,
        ..at_the_mill()
    };
    let order = |order, target| crate::unit_ai::QueuedOrder {
        order,
        target,
        ..crate::unit_ai::QueuedOrder::default()
    };
    let mut ai = UnitAi {
        current_order: 0x2c9,
        orders: vec![order(0x2c9, 77), order(0x2be, 78)],
        ..UnitAi::default()
    };
    let mut b = lone();
    let step = drop_update(
        &mut b,
        (10.0, Some(mill(2, true, 0.5))),
        Some(&mut ai),
        &[77],
    );
    assert_eq!(ai.orders, vec![order(0x2be, 78)]);
    assert_eq!(ai.current_order, -1);
    assert!(step.done && step.stands);
    assert_eq!((b.state, step.dropped), (DONE, Some((1, 10.0))));
    let mut empty = UnitAi {
        orders: vec![order(0x2db, 77)],
        ..UnitAi::default()
    };
    let mut b = lone();
    let step = drop_update(
        &mut b,
        (10.0, Some(mill(2, true, 0.5))),
        Some(&mut empty),
        &[77],
    );
    assert_eq!(empty.orders, Vec::new());
    assert!(step.notified && step.stands && !step.done);
    assert_eq!((b.state, b.site), (IDLE, 300));
    let mut b = BuildAction {
        point: [-1.0; 3],
        ..lone()
    };
    let step = drop_update(&mut b, (10.0, Some(mill(2, true, 0.5))), None, &[]);
    assert_eq!(
        (b.state, step.site_let_go, step.stands),
        (HALTED, Some(300), true)
    );
    let mut b = BuildAction {
        point: [5.0, 6.0, 0.0],
        ..lone()
    };
    let step = drop_update(&mut b, (10.0, Some(mill(2, true, 0.5))), None, &[]);
    assert_eq!(
        (b.state, b.goal, b.move_to, step.dropped),
        (
            WALK_POINT,
            Some([5.0, 6.0, 0.0]),
            Some(MoveTo::Start),
            Some((1, 10.0))
        )
    );
}

/// State 8's quiet `MoveTo` lets a target past state 6 go and returns 5: the drop-off
/// searches again; while its timer is below 0 the `MoveTo` waits.
#[test]
fn a_drop_off_whose_site_went_past_state_6_searches_again() {
    let mut b = BuildAction {
        move_to: Some(MoveTo::Moving),
        ..at_the_mill()
    };
    let mut world = drop_world();
    let mut mover = crate::hunt::tests::walker();
    let step = b
        .update(
            Walk {
                mover: Some(&mut mover),
                me: Me {
                    id: 5,
                    position: [2.5, 2.5, 0.0],
                    group: -1,
                    action_state: 0,
                },
                dt: 0.25,
                world: &mut world,
                plan: None,
            },
            None,
            Builder {
                class: VILLAGER,
                master: 83,
                held: 10.0,
                held_kind: 1,
                drop_site: false,
                ai: None,
                waits: &mut 0,
                build_targets: &[],
                look: Looks(&|_| None),
                walk_target_state: Some(7),
                deposit: Some(mill(7, true, 30.0)),
                attributes: 4,
                gone_orders: &[],
                stack_made: None,
            },
        )
        .unwrap();
    assert!(step.drop_off);
    assert_eq!((b.walk_target, b.state), (-1, DROP));
}
