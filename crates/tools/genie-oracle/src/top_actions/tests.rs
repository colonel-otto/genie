//! Each saved top action the slice imports, and the saved actions it refuses.

use crate::OTHER_SUB_ACTION;
use crate::fixtures::{action_list, combat_master, combat_mut, combat_object, with_masters};
use crate::top_actions::saved_actions;
use genie_core::GenieError;
use genie_core::construction::build::BuildTask;
use genie_core::movement::move_to::{Goal, MOVE_TO, MoveTo};
use genie_core::objects::masters::action_type::BUILD;
use genie_core::objects::masters::{Task, class};
use genie_core::objects::sprite::Graphic;
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
            // no sprite (`+0x58` null)
            rge_action: RgeAction {
                short_val_2: -1,
                ..base(state)
            },
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
/// 8, the build task 7 (range 1, work rate 2, moving sprite 9), no sprites' table.
fn import(
    object: &genie_save::Object,
    objects: &PlayerObjects,
    unit: Unit,
) -> Result<Unit, GenieError> {
    import_drawn(object, objects, unit, None)
}

/// [`import`] with the sprites' table `graphics`.
fn import_drawn(
    object: &genie_save::Object,
    objects: &PlayerObjects,
    mut unit: Unit,
    graphics: Option<&std::collections::BTreeMap<i16, Graphic>>,
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
                moving: Some(9),
            })
        },
        |_| Ok(task(8, 3.0)),
        |_, slot| Ok(task(slot, 2.0)),
        graphics,
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

/// A saved list of two actions: the slice's lists hold one (the restart of the action under a
/// deleted head, slot 0x28, is not ported), so the import refuses the action below the head
/// rather than dropping it, and names it.
#[test]
fn a_second_saved_action_is_refused() {
    let mut object = acting(MOVE_TO, move_to(2, 0.5));
    combat_mut(&mut object)
        .rge_combat_object
        .rge_action_object
        .actions = action_list(vec![
        (MOVE_TO, move_to(2, 0.5)),
        (genie_core::movement::explore::EXPLORE, explore(base(0))),
    ]);
    let err = import(&object, &PlayerObjects::default(), Unit::default()).unwrap_err();
    assert_eq!(
        err,
        GenieError::unsupported("a saved action the slice does not import")
    );
    assert_eq!(
        err.context().collect::<Vec<_>>(),
        [(
            "below the head",
            genie_core::movement::explore::EXPLORE.to_string().as_str()
        )]
    );
}

#[test]
fn a_move_to_takes_its_state_point_range_and_wait() {
    let m = imported(MOVE_TO, move_to(4, 0.5))
        .unwrap()
        .actions
        .move_to()
        .copied()
        .unwrap();
    assert_eq!(m.state, MoveTo::Moving);
    assert_eq!((m.target, bits3(m.point)), (-1, bits3([1.0, 2.0, 0.0])));
    assert_eq!(
        (m.target, m.range.to_bits(), m.sprite),
        (-1, 0.5_f32.to_bits(), None)
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
            .actions
            .move_to()
            .copied()
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

/// A `MoveTo`'s sprite (`+0x58`, the save's second `short_val`) comes from the sprites'
/// table; −1 is none, and an id the table lacks (or no table) is missing data.
#[test]
fn a_move_to_takes_its_sprite_from_the_sprites_table() {
    let walking = Graphic {
        id: 5,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.5,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics = std::collections::BTreeMap::from([(5, walking)]);
    let mut action = move_to(4, 0.5);
    let mut shown = |sprite: i16, graphics| {
        let parts = action.rge_action_by_type.as_deref_mut().unwrap();
        parts
            .rge_action_move_to
            .as_deref_mut()
            .unwrap()
            .rge_action
            .short_val_2 = sprite;
        let object = acting(MOVE_TO, action.clone());
        import_drawn(
            &object,
            &PlayerObjects::default(),
            Unit::default(),
            graphics,
        )
        .map(|u| u.actions.move_to().copied().unwrap().sprite)
    };
    assert_eq!(shown(5, Some(&graphics)), Ok(Some(walking)));
    assert_eq!(shown(-1, None), Ok(None));
    let lacking = shown(4, Some(&graphics)).unwrap_err();
    assert_eq!(
        (lacking.kind(), lacking.reason()),
        (
            genie_core::ErrorKind::MissingData,
            "a saved MoveTo's sprite outside the sprites' table"
        )
    );
    assert!(shown(5, None).is_err());
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
/// its range, quiet toward an object; another sub-action is refused.
#[test]
fn an_artifact_waits_or_walks_to_its_target() {
    let a = imported(ARTIFACT_TYPE, artifact(base(1)))
        .unwrap()
        .actions
        .artifact()
        .copied()
        .unwrap();
    assert_eq!((a.state, a.target, a.move_to, a.goal), (1, -1, None, None));
    assert!(a.gaia);
    let mut b = base(5);
    b.target_id = 9;
    b.target_x = 4.0;
    let a = imported(
        ARTIFACT_TYPE,
        artifact(with_sub(b.clone(), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .actions
    .artifact()
    .copied()
    .unwrap();
    let walk = a.move_to.unwrap();
    assert_eq!((walk.state, walk.quiet), (MoveTo::Moving, true));
    assert_eq!(a.goal.map(bits3), Some(bits3([4.0, 2.0, 0.0])));
    assert!(matches!(walk.walks_to(), Goal::Point(p) if bits3(p) == bits3([1.0, 2.0, 0.0])));
    assert_eq!(walk.range.to_bits(), 0.5_f32.to_bits());
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
    .actions
    .artifact()
    .copied()
    .unwrap();
    assert!(matches!(
        a.move_to.map(|m| m.walks_to()),
        Some(Goal::Target(9))
    ));
    // the target's place is its goal however near the origin; an unset place (-1) is none
    let mut near = base(5);
    near.target_id = 9;
    let a = imported(ARTIFACT_TYPE, artifact(near.clone()))
        .unwrap()
        .actions
        .artifact()
        .copied()
        .unwrap();
    assert_eq!(a.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    near.target_x = -1.0;
    let a = imported(ARTIFACT_TYPE, artifact(near))
        .unwrap()
        .actions
        .artifact()
        .copied()
        .unwrap();
    assert_eq!(a.goal, None);
    // without a target the goal is the MoveTo's point, the action's own
    let a = imported(
        ARTIFACT_TYPE,
        artifact(with_sub(base(5), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .actions
    .artifact()
    .copied()
    .unwrap();
    assert_eq!(a.goal.map(bits3), Some(bits3([1.0, 2.0, 0.0])));
    // a move to the point: its MoveTo is not quiet, its AI hears the end
    assert_eq!(a.move_to.map(|m| m.quiet), Some(false));
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
    missile_of(5.0, 0)
}

/// [`missile`] `distance` from its point, a charge projectile for `charge` 1.
fn missile_of(distance: f32, charge: u8) -> genie_save::Object {
    let mut b = base(2);
    b.target2_id = 900;
    let action = by_type(|t| {
        t.rge_action_missile = Some(Box::new(RgeActionMissile {
            rge_action: b,
            distance,
            ballistic_velocity: 3.0,
            ..RgeActionMissile::default()
        }));
    });
    let mut flying = RgeMissileObject {
        max_range: 7.0,
        fired_from_id: 900,
        is_charge_projectile: charge,
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
    assert!(m.flying && !m.charge);
    // a charge projectile; one at the landing distance has landed
    let landing = genie_core::combat::missile::LANDING;
    let m = import(
        &missile_of(landing, 1),
        &PlayerObjects::default(),
        Unit::default(),
    )
    .unwrap()
    .missile
    .unwrap();
    assert!(m.charge && !m.flying);
    let action = import(&missile(), &PlayerObjects::default(), Unit::default())
        .unwrap()
        .actions
        .missile()
        .cloned()
        .unwrap();
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
    let m = imported(MAKE, make(b))
        .unwrap()
        .actions
        .make()
        .copied()
        .unwrap();
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
    let f = imported(FLY, bird(base(3)))
        .unwrap()
        .actions
        .fly()
        .copied()
        .unwrap();
    assert_eq!((f.state, f.move_to, f.task.id), (3, None, 8));
    assert_eq!(f.home.map(f32::to_bits), [1.0_f32, 2.0].map(f32::to_bits));
    let f = imported(FLY, bird(with_sub(base(3), MOVE_TO, move_to(4, 0.5))))
        .unwrap()
        .actions
        .fly()
        .copied()
        .unwrap();
    let flight = f.move_to.unwrap();
    assert_eq!((flight.state, flight.quiet), (MoveTo::Moving, false));
    assert_eq!(
        (flight.target, bits3(flight.point)),
        (-1, bits3([1.0, 2.0, 0.0]))
    );
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

/// A Build takes its task with the unit's work-rate powerup, its quiet `MoveTo` (its range,
/// point, wait and sprite) and its site.
#[test]
fn a_build_takes_its_task_move_to_and_site() {
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
        .actions
        .build()
        .copied()
        .unwrap();
    let walk = a.move_to.unwrap();
    assert_eq!(
        (a.target, a.site, walk.target, walk.state, walk.quiet),
        (30, 31, -1, MoveTo::Moving, true)
    );
    assert!(a.added);
    assert_eq!(a.task.work_rate.to_bits(), 2.5_f32.to_bits());
    assert_eq!(walk.range.to_bits(), 0.75_f32.to_bits());
    assert_eq!(bits3(walk.point), bits3([1.0, 2.0, 0.0]));
    assert_eq!(walk.wait.state, 2);
    // a MoveTo without a sprite (−1) keeps none
    assert_eq!(walk.sprite, None);
    let mut dropping = b.clone();
    dropping.state = genie_core::construction::build::DROP;
    // the drop-off's MoveTo with the carrying sprite 12 (its `+0x58`)
    let mut carrying = move_to(4, 0.75);
    carrying
        .rge_action_by_type
        .as_mut()
        .and_then(|t| t.rge_action_move_to.as_mut())
        .expect("a MoveTo")
        .rge_action
        .short_val_2 = 12;
    let dropping = with_sub(dropping, MOVE_TO, carrying);
    let carried = Graphic {
        id: 12,
        frames: 10,
        facets: 16,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    let graphics = std::collections::BTreeMap::from([(12, carried)]);
    let a = import_drawn(
        &acting(BUILD, build(dropping)),
        &objects,
        unit,
        Some(&graphics),
    )
    .unwrap()
    .actions
    .build()
    .copied()
    .unwrap();
    let walk = a.move_to.unwrap();
    assert_eq!(walk.range.to_bits(), 0.75_f32.to_bits());
    assert_eq!(walk.sprite, Some(carried));
    let a = imported(BUILD, build(b.clone()))
        .unwrap()
        .actions
        .build()
        .copied()
        .unwrap();
    assert_eq!(a.move_to, None);
    // without a MoveTo, the task's moving sprite for the next
    assert_eq!(a.task.moving, Some(9));
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
    .actions
    .repair()
    .copied()
    .unwrap();
    let walk = r.move_to.unwrap();
    assert_eq!(
        (r.task.id, walk.state, walk.quiet, r.range.to_bits()),
        (2, MoveTo::Moving, true, 0)
    );
    assert!(r.counted && r.quiet);
    // a MoveTo without a target walks to its point, within its own range
    assert!(matches!(walk.walks_to(), Goal::Point(p) if bits3(p) == bits3([1.0, 2.0, 0.0])));
    assert_eq!(walk.range.to_bits(), 0.5_f32.to_bits());
    let r = imported(REPAIR, repair(b.clone()))
        .unwrap()
        .actions
        .repair()
        .copied()
        .unwrap();
    assert_eq!(r.move_to, None);
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

/// A Hunt takes its task, drop site and lead, its quiet `MoveTo` with its range and wait; its Attack needs
/// the sprites' table; a state or sub-action the slice does not model is refused.
#[test]
fn a_hunt_takes_its_task_lead_and_move_to() {
    use genie_core::gathering::hunt::{HUNT, IDLE, WALK};
    let h = imported(HUNT, hunt(base(IDLE)))
        .unwrap()
        .actions
        .hunt()
        .copied()
        .unwrap();
    assert_eq!(
        (h.target_class, h.drop_site, h.task.id),
        (Some(class::VILLAGER), 40, 0)
    );
    assert!(h.lead.moved && h.attack.is_none() && h.move_to.is_none());
    assert_eq!(
        h.lead.start.map(f32::to_bits),
        [6.0_f32, 7.0].map(f32::to_bits)
    );
    let h = imported(HUNT, hunt(with_sub(base(WALK), MOVE_TO, move_to(4, 0.75))))
        .unwrap()
        .actions
        .hunt()
        .copied()
        .unwrap();
    // every MoveTo of the Hunt is quiet
    let walk = h.move_to.unwrap();
    assert_eq!(
        (
            walk.state,
            walk.quiet,
            walk.range.to_bits(),
            walk.wait.state
        ),
        (MoveTo::Moving, true, 0.75_f32.to_bits(), 2)
    );
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
        .actions
        .gather()
        .copied()
        .unwrap();
    assert_eq!((g.target_type, g.stops, g.task.id), (5, 2, 0));
    assert!(g.worked && g.move_to.is_none());
    assert_eq!(g.farm[0].to_bits(), 3.0_f32.to_bits());
    let g = imported(
        GATHER,
        gather(with_sub(base(WALK), MOVE_TO, move_to(4, 0.75))),
    )
    .unwrap()
    .actions
    .gather()
    .copied()
    .unwrap();
    assert!(!g.worked);
    // every MoveTo of the Gather is quiet; one without a target walks to its point
    let walk = g.move_to.unwrap();
    assert_eq!((walk.state, walk.quiet), (MoveTo::Moving, true));
    assert!(matches!(walk.walks_to(), Goal::Point(p) if bits3(p) == bits3([1.0, 2.0, 0.0])));
    assert_eq!(
        (walk.range.to_bits(), walk.wait.state),
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
    use genie_core::garrison::ENTER;
    use genie_core::orders::action_state::DONE;
    let mut b = base(DONE);
    b.short_val = 1;
    let e = imported(ENTER, enter(with_sub(b.clone(), MOVE_TO, move_to(4, 0.5))))
        .unwrap()
        .actions
        .enter()
        .copied()
        .unwrap();
    let m = e.move_to.unwrap();
    assert_eq!(
        (e.state, m.state, m.quiet, e.task_class),
        (DONE, MoveTo::Moving, true, 3)
    );
    assert!(e.retry);
    assert_eq!((e.range.to_bits(), m.wait.state), (2.0_f32.to_bits(), 2));
    assert_eq!(
        imported(ENTER, enter(with_sub(b, 4, explore(base(3))))).unwrap_err(),
        GenieError::unsupported("a saved Enter with another sub-action")
    );
    assert_eq!(
        imported(ENTER, enter(base(7))).unwrap_err(),
        GenieError::unsupported("a saved Enter in a state the slice does not model")
    );
    assert_eq!(
        imported(ENTER, explore(base(DONE))).unwrap_err(),
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
    .actions
    .transport()
    .copied()
    .unwrap();
    let m = t.move_to.unwrap();
    assert_eq!(
        (t.state, t.mode, m.state, m.quiet),
        (2, -1, MoveTo::Moving, true)
    );
    assert!(!t.retarget);
    assert_eq!(
        imported(TRANSPORT, transport(with_sub(base(2), 4, explore(base(3))))).unwrap_err(),
        GenieError::unreachable("a saved Transport with another sub-action")
    );
}

#[test]
fn an_explore_takes_its_quiet_move_to_with_its_wait() {
    use genie_core::movement::explore::{EXPLORE, WAIT, WALK};
    let e = imported(EXPLORE, explore(base(WAIT)))
        .unwrap()
        .actions
        .explore()
        .copied()
        .unwrap();
    assert_eq!((e.state, e.move_to), (WAIT, None));
    let e = imported(
        EXPLORE,
        explore(with_sub(base(WALK), MOVE_TO, move_to(4, 0.5))),
    )
    .unwrap()
    .actions
    .explore()
    .copied()
    .unwrap();
    // a quiet MoveTo (its maker's `+0x60`, which the save does not hold) with the saved wait
    let m = e.move_to.unwrap();
    assert_eq!((m.state, m.quiet, m.wait.state), (MoveTo::Moving, true, 2));
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
        .actions
        .wait_solidify()
        .copied()
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

/// The sprites' table of the Attack tests: an attack sprite 21 of ten frames of 0.1 s.
fn swing_sprites() -> std::collections::BTreeMap<i16, Graphic> {
    let swing = Graphic {
        id: 21,
        frames: 10,
        facets: 8,
        speed_multiplier: 1.0,
        frame_duration: 0.1,
        replay_delay: 0.0,
        flags: 1,
    };
    std::collections::BTreeMap::from([(21, swing)])
}

/// An Attack in state `state` that walks with sprite `walk` (`short_val`) and swings sprite 21
/// (`short_val_2`), with a hit to come (`need_to_attack`) for `pending`.
fn attack(state: u8, walk: u16, pending: bool) -> Action {
    by_type(|t| {
        t.rge_action_attack = Some(Box::new(RgeActionAttack {
            rge_action: base(state),
            short_val: walk,
            short_val_2: 21,
            need_to_attack: u8::from(pending),
            ..RgeActionAttack::default()
        }));
    })
}

/// A Hunt's Attack in its swing past the hit (state 7, no hit to come) rests the unit's
/// reload (its master's 2.5 s) less the attack sprite's length (1 s); one with its hit to
/// come, or in another state, keeps no rest. A swing longer than the reload rests 0.
#[test]
fn a_hunts_swing_past_its_hit_rests_the_reload_less_the_swing() {
    use genie_core::combat::attack::{ATTACK, SWING};
    use genie_core::gathering::hunt::{HUNT, WALK};
    let master = |reload: f32| {
        let mut m = combat_master(83);
        m.tribe_master_combat_object
            .as_deref_mut()
            .unwrap()
            .rge_master_combat_object
            .speed_of_attack = reload;
        with_masters(vec![m])
    };
    let sprites = swing_sprites();
    let swing = |state: u8, pending: bool, objects: &PlayerObjects| {
        let a = import_drawn(
            &acting(
                HUNT,
                hunt(with_sub(base(WALK), ATTACK, attack(state, 4, pending))),
            ),
            objects,
            Unit::default(),
            Some(&sprites),
        )
        .unwrap()
        .actions
        .hunt()
        .copied()
        .unwrap()
        .attack
        .unwrap();
        (a.swing.to_bits(), a.rest.to_bits())
    };
    let rest = 2.5_f32 - 10.0 * 0.1_f32;
    assert_eq!(
        swing(SWING, false, &master(2.5)),
        (rest.to_bits(), rest.to_bits())
    );
    assert_eq!(swing(SWING, true, &master(2.5)), (0, 0));
    assert_eq!(swing(4, false, &master(2.5)), (0, 0));
    let short = 0.5_f32 - 10.0 * 0.1_f32;
    assert_eq!(swing(SWING, false, &master(0.5)), (short.to_bits(), 0));
}
