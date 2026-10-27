//! Each saved top action the slice imports, and the saved actions it refuses.

use crate::OTHER_SUB_ACTION;
use crate::fixtures::{
    action_list, combat_master, combat_mut, combat_object, master_moving_mut, with_masters,
};
use crate::top_actions::saved_actions;
use genie_core::GenieError;
use genie_core::construction::build::{BUILD, BuildTask};
use genie_core::movement::move_to::{Goal, MOVE_TO, MoveTo};
use genie_core::objects::masters::Task;
use genie_core::objects::unit::Unit;
use genie_core::ownership::artifact::ARTIFACT_TYPE;
use genie_save::generated::world::{
    Action, PlayerObjects, RgeAction, RgeActionAttack, RgeActionByType, RgeActionEnter,
    RgeActionGather, RgeActionMake, RgeActionMissile, RgeActionMoveTo, RgeMissileObject, RgeObject,
    TribeActionArtifact, TribeActionBuild, TribeActionHunt, TribeActionRepair,
    TribeActionWaitSolidify,
};

/// The task of slot `id`, its work range `range`.
fn task(id: i16, range: f32) -> Task {
    Task {
        id,
        action_type: 0,
        object_group: 3,
        object_id: -1,
        combat_level: 0,
        combat_level_flag: 0,
        terrain: -1,
        owner_type: 0,
        holding_attr: 0,
        attribute_type: 1,
        attribute_type2: 2,
        attribute_type3: 3,
        attribute_type4: 4,
        wait_time: 0.0,
        work_val1: 0.5,
        work_val2: 0.0,
        work_range: range,
        work_flag: 0,
        enablement: 0,
        sprites: [-1; 4],
        sound_events: [-1; 2],
    }
}

/// An action's base in state `state`: no targets, its point (1, 2, 0), its timer 1.5.
fn base(state: u8) -> RgeAction {
    RgeAction {
        state,
        target_id: -1,
        target2_id: -1,
        target_x: 1.0,
        target_y: 2.0,
        timer: 1.5,
        ..RgeAction::default()
    }
}

/// An action of the base factory's classes, its part set by `set`.
fn by_type(set: impl FnOnce(&mut RgeActionByType)) -> Action {
    let mut parts = RgeActionByType::default();
    set(&mut parts);
    Action {
        rge_action_by_type: Some(Box::new(parts)),
        ..Action::default()
    }
}

/// An Explore (its base alone) of `b`.
fn explore(b: RgeAction) -> Action {
    by_type(|t| t.rge_action_explore = Some(Box::new(b)))
}

/// A `MoveTo` in state `state` to its base's point, its range `range`, waiting (state 2, timer
/// 0.25, delay 0.5).
fn move_to(state: u8, range: f32) -> Action {
    by_type(|t| {
        t.rge_action_move_to = Some(Box::new(RgeActionMoveTo {
            rge_action: base(state),
            range,
            movement_wait_state: 2,
            timer: 0.25,
            k_default_wait_time: 0.5_f32.to_bits(),
        }));
    })
}

/// `b` with the sub-action `sub` of type `kind`.
fn with_sub(mut b: RgeAction, kind: i16, sub: Action) -> RgeAction {
    b.sub_actions = action_list(vec![(kind, sub)]);
    b
}

/// Unit 5 (master 83) acting `action` of type `kind`.
fn acting(kind: i16, action: Action) -> genie_save::Object {
    let mut object = combat_object(5, 83);
    combat_mut(&mut object)
        .rge_combat_object
        .rge_action_object
        .actions = action_list(vec![(kind, action)]);
    object
}

/// `object`'s top action imported into `unit`: each task slot's task of range 2, the fly task
/// 8, the build task 7 (range 1, work rate 2), no sprites' table.
fn import(
    object: &genie_save::Object,
    objects: &PlayerObjects,
    mut unit: Unit,
) -> Result<Unit, GenieError> {
    let s = object.static_part().expect("a test object's static part");
    saved_actions(
        object,
        s,
        objects,
        |_| {
            Ok(BuildTask {
                range: 1.0,
                work_value: 0.5,
                work_rate: 2.0,
                resource: -1,
                id: 7,
            })
        },
        |_| Ok(task(8, 3.0)),
        |_, slot| Ok(task(slot, 2.0)),
        None,
        &mut unit,
    )?;
    Ok(unit)
}

/// Unit 5's top action `action` of type `kind` imported, its player without masters.
fn imported(kind: i16, action: Action) -> Result<Unit, GenieError> {
    import(
        &acting(kind, action),
        &PlayerObjects::default(),
        Unit::default(),
    )
}

fn bits3(v: [f32; 3]) -> [u32; 3] {
    v.map(f32::to_bits)
}

#[test]
fn an_action_without_its_base_is_unsupported() {
    assert_eq!(
        imported(4, Action::default()).unwrap_err(),
        GenieError::unsupported("a saved action without its base")
    );
}

/// The second target only where the slice models one; an action the slice does not import is
/// refused.
#[test]
fn a_second_target_or_an_unknown_action_is_unsupported() {
    let mut b = base(3);
    b.target2_id = 9;
    assert_eq!(
        imported(genie_core::movement::explore::EXPLORE, explore(b)).unwrap_err(),
        GenieError::unsupported("a saved action with a second target")
    );
    assert_eq!(
        imported(99, explore(base(0))).unwrap_err(),
        GenieError::unsupported("a saved action the slice does not import")
    );
}

#[test]
fn a_move_to_takes_its_state_point_range_and_wait() {
    let m = imported(MOVE_TO, move_to(4, 0.5)).unwrap().move_to.unwrap();
    assert_eq!(m.state, MoveTo::Moving);
    assert_eq!(m.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    assert_eq!(
        (m.target, m.range.to_bits(), m.run),
        (-1, 0.5_f32.to_bits(), false)
    );
    assert_eq!(
        (m.wait.state, m.wait.timer.to_bits(), m.wait.delay.to_bits()),
        (2, 0.25_f32.to_bits(), 0.5_f32.to_bits())
    );
    for (state, saved) in [
        (1, MoveTo::Arrived),
        (2, MoveTo::Waiting),
        (3, MoveTo::Start),
        (13, MoveTo::Failed),
    ] {
        let m = imported(MOVE_TO, move_to(state, 0.5))
            .unwrap()
            .move_to
            .unwrap();
        assert_eq!(m.state, saved);
    }
    assert_eq!(
        imported(MOVE_TO, move_to(7, 0.5)).unwrap_err(),
        GenieError::unsupported("a saved MoveTo in a state the slice does not model")
    );
    assert_eq!(
        imported(MOVE_TO, explore(base(4))).unwrap_err(),
        GenieError::unsupported("a saved MoveTo without its range")
    );
}

/// A `MoveTo` runs where its sprite is the master's running one, other than its walking one.
#[test]
fn a_move_to_runs_with_the_masters_running_sprite() {
    let mut master = combat_master(83);
    let moving = master_moving_mut(&mut master);
    moving.run_sprite_id = 5;
    moving.move_sprite_id = 4;
    let objects = with_masters(vec![master]);
    let mut action = move_to(4, 0.5);
    let mut running = |sprite: i16| {
        let parts = action.rge_action_by_type.as_deref_mut().unwrap();
        parts
            .rge_action_move_to
            .as_deref_mut()
            .unwrap()
            .rge_action
            .short_val_2 = sprite;
        let unit = import(&acting(MOVE_TO, action.clone()), &objects, Unit::default()).unwrap();
        unit.move_to.unwrap().run
    };
    assert!(running(5));
    assert!(!running(4));
}

fn artifact(b: RgeAction) -> Action {
    Action {
        tribe_action_artifact: Some(Box::new(TribeActionArtifact {
            rge_action: b,
            automove_livestock: 0,
        })),
        ..Action::default()
    }
}

/// An Artifact waits, or walks to its target: its `MoveTo`'s goal, a point or an object, with
/// its range; another sub-action is refused.
#[test]
fn an_artifact_waits_or_walks_to_its_target() {
    let a = imported(ARTIFACT_TYPE, artifact(base(1)))
        .unwrap()
        .artifact
        .unwrap();
    assert_eq!((a.state, a.target, a.move_to, a.goal), (1, -1, None, None));
    assert!(a.gaia && a.walk_to.is_none());
    let mut b = base(5);
    b.target_id = 9;
    b.target_x = 4.0;
    let a = imported(
        ARTIFACT_TYPE,
        artifact(with_sub(b.clone(), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .artifact
    .unwrap();
    assert_eq!(a.move_to, Some(MoveTo::Moving));
    assert_eq!(a.goal.map(bits3), Some(bits3([4.0, 2.0, 0.0])));
    let (goal, range) = a.walk_to.unwrap();
    assert!(matches!(goal, Goal::Point(p) if bits3(p) == bits3([1.0, 2.0, 0.0])));
    assert_eq!(range.to_bits(), 0.5_f32.to_bits());
    let mut to_object = move_to(4, 0.5);
    to_object
        .rge_action_by_type
        .as_deref_mut()
        .unwrap()
        .rge_action_move_to
        .as_deref_mut()
        .unwrap()
        .rge_action
        .target_id = 9;
    let a = imported(
        ARTIFACT_TYPE,
        artifact(with_sub(b.clone(), MOVE_TO, to_object)),
    )
    .unwrap()
    .artifact
    .unwrap();
    assert!(matches!(a.walk_to, Some((Goal::Target(9), _))));
    // without a target the goal is the MoveTo's point, the action's own
    let a = imported(
        ARTIFACT_TYPE,
        artifact(with_sub(base(5), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .artifact
    .unwrap();
    assert_eq!(a.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    assert!(a.walk_to.is_none());
    assert_eq!(
        imported(
            ARTIFACT_TYPE,
            artifact(with_sub(b.clone(), MOVE_TO, explore(base(4))))
        )
        .unwrap_err(),
        GenieError::unsupported("a saved MoveTo without its range")
    );
    assert_eq!(
        imported(ARTIFACT_TYPE, artifact(with_sub(b, 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Artifact action with another sub-action")
    );
}

/// A missile of unit 5's place, flying 5 from its shooter 900 (its second target).
fn missile() -> genie_save::Object {
    let mut b = base(2);
    b.target2_id = 900;
    let action = by_type(|t| {
        t.rge_action_missile = Some(Box::new(RgeActionMissile {
            rge_action: b,
            distance: 5.0,
            ballistic_velocity: 3.0,
            ..RgeActionMissile::default()
        }));
    });
    let mut flying = RgeMissileObject {
        max_range: 7.0,
        fired_from_id: 900,
        ..RgeMissileObject::default()
    };
    flying.rge_combat_object.rge_action_object.actions =
        action_list(vec![(genie_core::combat::missile::MISSILE, action)]);
    flying
        .rge_combat_object
        .rge_action_object
        .rge_moving_object
        .rge_animated_object
        .rge_static_object
        .id = 6;
    genie_save::Object {
        rge_object: Some(Box::new(RgeObject {
            rge_missile_object: Some(Box::new(flying)),
            ..RgeObject::default()
        })),
        ..genie_save::Object::default()
    }
}

#[test]
fn a_missile_flies_with_its_actions_course() {
    let m = import(&missile(), &PlayerObjects::default(), Unit::default())
        .unwrap()
        .missile
        .unwrap();
    assert_eq!(
        (m.fired_from, m.max_range.to_bits()),
        (900, 7.0_f32.to_bits())
    );
    assert!(m.flying);
    let action = m.action.unwrap();
    assert_eq!(
        (action.state, action.fired_from, action.hit_mode),
        (2, 900, 0)
    );
    assert_eq!(action.ballistic_velocity.to_bits(), 3.0_f32.to_bits());
    assert_eq!(
        imported(genie_core::combat::missile::MISSILE, explore(base(2))).unwrap_err(),
        GenieError::unsupported("a saved missile action without its part")
    );
    let not_a_missile = by_type(|t| t.rge_action_missile = Some(Box::default()));
    assert_eq!(
        imported(genie_core::combat::missile::MISSILE, not_a_missile).unwrap_err(),
        GenieError::unsupported("a missile action of an object that is not a missile")
    );
}

#[test]
fn a_farms_make_takes_its_timers_and_task() {
    use genie_core::gathering::make::MAKE;
    let make = |b: RgeAction| {
        by_type(|t| {
            t.rge_action_make = Some(Box::new(RgeActionMake {
                rge_action: b,
                work_timer: 9.0,
            }));
        })
    };
    let mut b = base(1);
    b.short_val = 3;
    let m = imported(MAKE, make(b)).unwrap().make.unwrap();
    assert_eq!((m.state, m.task.id), (1, 3));
    assert_eq!(
        (m.remaining.to_bits(), m.total.to_bits()),
        (1.5_f32.to_bits(), 9.0_f32.to_bits())
    );
    assert_eq!(
        imported(MAKE, make(with_sub(base(1), MOVE_TO, move_to(4, 0.5)))).unwrap_err(),
        GenieError::unsupported("a saved Make with a sub-action")
    );
    assert_eq!(
        imported(MAKE, explore(base(1))).unwrap_err(),
        GenieError::unsupported("a saved Make without its part")
    );
}

#[test]
fn a_birds_fly_takes_its_home_and_move_to() {
    use genie_core::movement::fly::FLY;
    let bird = |b: RgeAction| by_type(|t| t.rge_action_bird = Some(Box::new(b)));
    let f = imported(FLY, bird(base(3))).unwrap().fly.unwrap();
    assert_eq!((f.state, f.move_to, f.goal, f.task.id), (3, None, None, 8));
    assert_eq!(f.home.map(f32::to_bits), [1.0_f32, 2.0].map(f32::to_bits));
    let f = imported(FLY, bird(with_sub(base(3), MOVE_TO, move_to(4, 0.5))))
        .unwrap()
        .fly
        .unwrap();
    assert_eq!(f.move_to, Some(MoveTo::Moving));
    assert_eq!(f.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    assert_eq!(
        imported(FLY, bird(with_sub(base(3), 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Fly action with another sub-action")
    );
}

fn build(b: RgeAction) -> Action {
    Action {
        tribe_action_build: Some(Box::new(TribeActionBuild {
            rge_action: b,
            added_to_num_builders: 1,
        })),
        ..Action::default()
    }
}

/// A Build takes its task with the unit's work-rate powerup, its `MoveTo` and its site; the
/// drop-off's `MoveTo` keeps its range.
#[test]
fn a_build_takes_its_task_move_to_and_drop_range() {
    let mut b = base(4);
    b.target_id = 30;
    b.target2_id = 31;
    let unit = Unit {
        work_rate_powerup: 0.5,
        ..Unit::default()
    };
    let objects = PlayerObjects::default();
    let walking = with_sub(b.clone(), MOVE_TO, move_to(4, 0.75));
    let a = import(&acting(BUILD, build(walking)), &objects, unit.clone())
        .unwrap()
        .build
        .unwrap();
    assert_eq!(
        (a.target, a.site, a.walk_target, a.move_to),
        (30, 31, -1, Some(MoveTo::Moving))
    );
    assert!(a.added);
    assert_eq!(a.task.work_rate.to_bits(), 2.5_f32.to_bits());
    assert_eq!(a.drop_range.to_bits(), 1.0_f32.to_bits());
    assert_eq!(a.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    assert_eq!(a.move_wait.state, 2);
    let mut dropping = b.clone();
    dropping.state = genie_core::construction::build::DROP;
    let dropping = with_sub(dropping, MOVE_TO, move_to(4, 0.75));
    let a = import(&acting(BUILD, build(dropping)), &objects, unit)
        .unwrap()
        .build
        .unwrap();
    assert_eq!(a.drop_range.to_bits(), 0.75_f32.to_bits());
    let a = imported(BUILD, build(b.clone())).unwrap().build.unwrap();
    assert_eq!((a.move_to, a.walk_target, a.goal), (None, -1, None));
    assert_eq!(
        imported(BUILD, build(with_sub(b, 4, explore(base(3))))).unwrap_err(),
        OTHER_SUB_ACTION
    );
}

fn repair(b: RgeAction) -> Action {
    Action {
        tribe_action_repair: Some(Box::new(TribeActionRepair {
            rge_action: b,
            save_target_command_flag: 0,
            added_to_num_builders: 1,
        })),
        ..Action::default()
    }
}

#[test]
fn a_repair_takes_its_task_and_counts_its_builder() {
    use genie_core::construction::repair::REPAIR;
    let mut b = base(4);
    b.short_val = 2;
    b.sub_action_value = 1;
    let r = imported(
        REPAIR,
        repair(with_sub(b.clone(), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .repair
    .unwrap();
    assert_eq!(
        (r.task.id, r.move_to, r.range.to_bits()),
        (2, Some(MoveTo::Moving), 0)
    );
    assert!(r.counted && r.quiet);
    assert_eq!(r.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    let r = imported(REPAIR, repair(b.clone())).unwrap().repair.unwrap();
    assert_eq!((r.move_to, r.goal), (None, None));
    assert_eq!(
        imported(REPAIR, repair(with_sub(b, 4, explore(base(3))))).unwrap_err(),
        OTHER_SUB_ACTION
    );
    assert_eq!(
        imported(REPAIR, explore(base(4))).unwrap_err(),
        GenieError::missing_data("a saved Repair's own part")
    );
}

fn hunt(b: RgeAction) -> Action {
    Action {
        tribe_action_hunt: Some(Box::new(TribeActionHunt {
            rge_action: b,
            target_type: 4,
            moved_sheep: 1,
            sheep_still_time: 3,
            sheep_start_x: 6.0_f32.to_bits(),
            sheep_start_y: 7.0_f32.to_bits(),
            sheep_drop_site_id: 40,
        })),
        ..Action::default()
    }
}

/// A Hunt takes its task, drop site and lead, its `MoveTo`'s range and wait; its Attack needs
/// the sprites' table; a state or sub-action the slice does not model is refused.
#[test]
fn a_hunt_takes_its_task_lead_and_move_to() {
    use genie_core::gathering::hunt::{HUNT, IDLE, WALK};
    let h = imported(HUNT, hunt(base(IDLE))).unwrap().hunt.unwrap();
    assert_eq!((h.target_class, h.drop_site, h.task.id), (4, 40, 0));
    assert_eq!(h.walk_range.to_bits(), 2.0_f32.to_bits());
    assert!(h.lead.moved && h.attack.is_none() && h.move_to.is_none());
    assert_eq!(
        h.lead.start.map(f32::to_bits),
        [6.0_f32, 7.0].map(f32::to_bits)
    );
    let h = imported(HUNT, hunt(with_sub(base(WALK), MOVE_TO, move_to(4, 0.75))))
        .unwrap()
        .hunt
        .unwrap();
    assert_eq!(
        (h.move_to, h.walk_range.to_bits()),
        (Some(MoveTo::Moving), 0.75_f32.to_bits())
    );
    assert_eq!(h.move_wait.state, 2);
    let attack = by_type(|t| {
        t.rge_action_attack = Some(Box::new(RgeActionAttack {
            rge_action: base(4),
            ..RgeActionAttack::default()
        }));
    });
    assert_eq!(
        imported(
            HUNT,
            hunt(with_sub(
                base(WALK),
                genie_core::combat::attack::ATTACK,
                attack
            ))
        )
        .unwrap_err(),
        GenieError::unsupported("a saved Attack without the sprites' table")
    );
    assert_eq!(
        imported(HUNT, hunt(with_sub(base(WALK), 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Hunt with another sub-action")
    );
    assert_eq!(
        imported(HUNT, hunt(base(99))).unwrap_err(),
        GenieError::unsupported("a saved Hunt in a state the slice does not model")
    );
    assert_eq!(
        imported(HUNT, explore(base(IDLE))).unwrap_err(),
        GenieError::unsupported("a saved Hunt without its own part")
    );
}

fn gather(b: RgeAction) -> Action {
    by_type(|t| {
        t.rge_action_gather = Some(Box::new(RgeActionGather {
            rge_action: b,
            target_type: 5,
            number_stops: 2,
            farm_x: 3.0,
            ..RgeActionGather::default()
        }));
    })
}

/// A Gather takes its stops and task; a `MoveTo` without a target walks to its point.
#[test]
fn a_gather_takes_its_stops_and_move_to() {
    use genie_core::gathering::gather::{GATHER, WALK, WORK};
    let g = imported(GATHER, gather(base(WORK)))
        .unwrap()
        .gather
        .unwrap();
    assert_eq!((g.target_type, g.stops, g.task.id), (5, 2, 0));
    assert!(g.worked && g.move_to.is_none() && g.walk_to.is_none());
    assert_eq!(g.farm[0].to_bits(), 3.0_f32.to_bits());
    let g = imported(
        GATHER,
        gather(with_sub(base(WALK), MOVE_TO, move_to(4, 0.75))),
    )
    .unwrap()
    .gather
    .unwrap();
    assert!(!g.worked);
    assert_eq!(g.move_to, Some(MoveTo::Moving));
    assert_eq!(g.walk_to.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    assert_eq!(
        (g.walk_range.to_bits(), g.move_wait.state),
        (0.75_f32.to_bits(), 2)
    );
    assert_eq!(
        imported(GATHER, gather(with_sub(base(WALK), 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Gather with another sub-action")
    );
    assert_eq!(
        imported(GATHER, gather(base(99))).unwrap_err(),
        GenieError::unsupported("a saved Gather in a state the slice does not model")
    );
    assert_eq!(
        imported(GATHER, explore(base(WORK))).unwrap_err(),
        GenieError::unsupported("a saved Gather without its own part")
    );
}

fn enter(b: RgeAction) -> Action {
    Action {
        rge_action_enter: Some(Box::new(RgeActionEnter {
            rge_action: b,
            first_time_enter: 1,
        })),
        ..Action::default()
    }
}

#[test]
fn an_enter_takes_its_task_and_retry() {
    use genie_core::garrison::{ENTER, ENTERED};
    let mut b = base(ENTERED);
    b.short_val = 1;
    let e = imported(ENTER, enter(with_sub(b.clone(), MOVE_TO, move_to(4, 0.5))))
        .unwrap()
        .enter
        .unwrap();
    assert_eq!(
        (e.state, e.move_to, e.task_class),
        (ENTERED, Some(MoveTo::Moving), 3)
    );
    assert!(e.retry);
    assert_eq!(
        (e.range.to_bits(), e.move_wait.state),
        (2.0_f32.to_bits(), 2)
    );
    assert_eq!(
        imported(ENTER, enter(with_sub(b, 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Enter with another sub-action")
    );
    assert_eq!(
        imported(ENTER, enter(base(7))).unwrap_err(),
        GenieError::unsupported("a saved Enter in a state the slice does not model")
    );
    assert_eq!(
        imported(ENTER, explore(base(ENTERED))).unwrap_err(),
        GenieError::unsupported("a saved Enter without its own part")
    );
}

#[test]
fn a_transport_takes_its_base_alone() {
    use genie_core::garrison::transport::TRANSPORT;
    let transport = |b: RgeAction| Action {
        rge_action_transport: Some(Box::new(b)),
        ..Action::default()
    };
    let t = imported(
        TRANSPORT,
        transport(with_sub(base(2), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .transport
    .unwrap();
    assert_eq!((t.state, t.mode, t.move_to), (2, -1, Some(MoveTo::Moving)));
    assert!(!t.retarget);
    assert_eq!(
        imported(TRANSPORT, transport(with_sub(base(2), 4, explore(base(3))))).unwrap_err(),
        GenieError::unreachable("a saved Transport with another sub-action")
    );
}

#[test]
fn an_explore_takes_its_move_to_and_wait() {
    use genie_core::movement::explore::{EXPLORE, WAIT, WALK};
    let e = imported(EXPLORE, explore(base(WAIT)))
        .unwrap()
        .explore
        .unwrap();
    assert_eq!((e.state, e.move_to, e.move_wait.state), (WAIT, None, 0));
    let e = imported(
        EXPLORE,
        explore(with_sub(base(WALK), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .explore
    .unwrap();
    assert_eq!((e.move_to, e.move_wait.state), (Some(MoveTo::Moving), 2));
    assert_eq!(
        imported(EXPLORE, explore(with_sub(base(WALK), 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Explore with another sub-action")
    );
    assert_eq!(
        imported(EXPLORE, explore(base(1))).unwrap_err(),
        GenieError::unsupported("a saved Explore in a state the slice does not model")
    );
}

#[test]
fn an_attack_needs_the_sprites_table() {
    let attack = by_type(|t| {
        t.rge_action_attack = Some(Box::new(RgeActionAttack {
            rge_action: base(4),
            ..RgeActionAttack::default()
        }));
    });
    assert_eq!(
        imported(genie_core::combat::attack::ATTACK, attack).unwrap_err(),
        GenieError::unsupported("a saved Attack without the sprites' table")
    );
}

#[test]
fn a_wait_solidify_waits_for_its_foundation() {
    use genie_core::construction::wait_solidify::WAIT_SOLIDIFY;
    let wait = |b: RgeAction| Action {
        tribe_action_wait_solidify: Some(Box::new(TribeActionWaitSolidify {
            rge_action: b,
            target_building_id: 40,
        })),
        ..Action::default()
    };
    let mut b = base(2);
    b.short_val = 1;
    let w = imported(WAIT_SOLIDIFY, wait(b))
        .unwrap()
        .wait_solidify
        .unwrap();
    assert_eq!((w.state, w.target, w.task.id), (2, 40, 1));
    assert_eq!(
        imported(
            WAIT_SOLIDIFY,
            wait(with_sub(base(2), MOVE_TO, move_to(4, 0.5)))
        )
        .unwrap_err(),
        GenieError::unsupported("a saved Wait_Solidify with a sub-action")
    );
    assert_eq!(
        imported(WAIT_SOLIDIFY, explore(base(2))).unwrap_err(),
        GenieError::unsupported("a saved Wait_Solidify without its own part")
    );
}
