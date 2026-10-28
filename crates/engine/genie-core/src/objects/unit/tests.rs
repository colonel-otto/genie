use crate::GenieError;
use crate::movement::move_to::{MoveTo, MoveToAction, MoveWait};
use crate::objects::actions::ActionList;
use crate::objects::layers::{Animated, Body};
use crate::objects::lifecycle::{Class, Lifecycle};
use crate::objects::sprite::{ActiveSprite, Graphic};
use crate::objects::unit::Unit;
use crate::world::tests::{forage_world, in_update};

/// A graphic of sixteen facets, its sequence flag bit 1 set (`flag_1`).
const WALKING: Graphic = Graphic {
    id: 300,
    frames: 10,
    facets: 16,
    speed_multiplier: 1.0,
    frame_duration: 0.1,
    replay_delay: 0.0,
    flags: 3,
};

/// `RGE_Static_Object::update`'s sleep: a static object flagged to sleep drops the flag and its
/// update ends; one not flagged goes on; one above the static type is not modelled.
#[test]
fn a_static_object_flagged_to_sleep_ends_its_update() {
    let flagged = |class| Unit {
        life: Lifecycle {
            class,
            goto_sleep: true,
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let mut u = flagged(Class::Static);
    assert_eq!(u.go_to_sleep(), Ok(true));
    assert!(!u.life.goto_sleep);
    assert_eq!(u.go_to_sleep(), Ok(false));
    let mut animated = flagged(Class::Animated);
    assert_eq!(
        animated.go_to_sleep().unwrap_err().reason(),
        "an object above the static type goes to sleep"
    );
}

/// The movement state takes the current sprite's facets and its sequence flag bit 1; without a
/// sprite it keeps what it had.
#[test]
fn the_movement_state_takes_the_sprite_s_facets() {
    let mut u = Unit {
        body: Body::Moving(crate::gathering::hunt::tests::walker()),
        ..Unit::default()
    };
    u.take_sprite_facets();
    assert_eq!(u.body.mover().unwrap().sprite_facets, None);
    u.sprite = Some(ActiveSprite::start(
        WALKING,
        &mut crate::math::rng::Well512::default(),
    ));
    u.take_sprite_facets();
    assert_eq!(
        u.body.mover().unwrap().sprite_facets,
        Some(crate::objects::facing::SpriteFacets {
            count: 16,
            flag_1: true,
        })
    );
}

/// The animated object's update: a sprite with an animation speed runs its clock by the
/// update's dt; one without a speed stands still.
#[test]
fn an_animated_object_s_sprite_runs_by_the_update_s_dt() {
    let animated = |speed: Option<f32>| {
        let mut t = forage_world();
        let u = &mut t.players[1].objects[0];
        u.sprite = Some(ActiveSprite::start(
            WALKING,
            &mut crate::math::rng::Well512::default(),
        ));
        // a static object has no animation speed, so no clock
        u.body = speed.map_or(Body::Static, |speed| Body::Animated(Animated { speed }));
        let before = u.sprite;
        in_update(&mut t, (1, 0), 50, |u, c, _| {
            u.animate(c);
            Ok(())
        })
        .unwrap();
        (before, t.players[1].objects[0].sprite)
    };
    let (before, after) = animated(Some(1.0));
    assert_ne!(before, after);
    let (before, after) = animated(None);
    assert_eq!(before, after);
}

/// `RGE_Action_Object::update` after the moving update: an object past state 2 loses its action
/// list (a `MoveTo`'s teardown stops the unit); an alive object with an unmodelled action fails.
#[test]
fn an_object_past_state_2_loses_its_actions() {
    let mut t = forage_world();
    let u = &mut t.players[1].objects[0];
    u.life.state = 3;
    u.actions.set_move_to(Some(MoveToAction {
        run: false,
        state: MoveTo::Moving,
        goal: Some([40.0, 80.0, 0.0]),
        target: -1,
        range: 0.0,
        wait: MoveWait::default(),
    }));
    in_update(&mut t, (1, 0), 50, |u, c, _| u.actions_past_life(c)).unwrap();
    let u = &t.players[1].objects[0];
    assert_eq!(u.actions.move_to().copied(), None);
    assert_eq!(u.body.mover().unwrap().animated.speed, 0.0);
    let mut t = forage_world();
    t.players[1].objects[0].unmodelled_action = Some(GenieError::unsupported("an action"));
    let r = in_update(&mut t, (1, 0), 50, |u, c, _| u.actions_past_life(c));
    assert_eq!(r.unwrap_err().reason(), "an action");
}

/// After the actions the unit takes its place as the world has it and hands its pauses to the
/// Context; a walk that met others in its way is not modelled.
#[test]
fn after_its_actions_a_unit_takes_the_world_s_place_and_hands_its_pauses_on() {
    let mut t = forage_world();
    t.spatial.set_position(9175, [29.75, 89.5, 0.0]);
    t.players[1].objects[0].body.mover_mut().unwrap().pauses = vec![(600, 0.5)];
    let pauses = in_update(&mut t, (1, 0), 50, |u, c, _| {
        u.after_actions(c)?;
        Ok(c.pauses.clone())
    })
    .unwrap();
    assert_eq!(pauses, [(600, 0.5)]);
    let u = &t.players[1].objects[0];
    assert_eq!(u.position, [29.75, 89.5, 0.0]);
    assert_eq!(u.body.mover().unwrap().pauses, []);
    t.players[1].objects[0].body.mover_mut().unwrap().in_the_way = vec![600];
    let r = in_update(&mut t, (1, 0), 50, |u, c, _| u.after_actions(c));
    assert_eq!(
        r.unwrap_err().reason(),
        "a walk's unit in the way (notify 0x215) the action does not pass on"
    );
}

/// The update up to the actions: a forager goes on to its actions with its life step's answer;
/// a static object flagged to sleep ends there (None).
#[test]
fn the_update_up_to_the_actions_ends_for_a_sleeper() {
    let mut t = forage_world();
    let step = in_update(&mut t, (1, 0), 50, |u, c, o| u.update_to_actions(c, o)).unwrap();
    assert!(step.is_some());
    let bush = &mut t.players[0].objects[0];
    bush.life.class = Class::Static;
    bush.life.goto_sleep = true;
    let step = in_update(&mut t, (0, 0), 50, |u, c, o| u.update_to_actions(c, o)).unwrap();
    assert_eq!(step, None);
    assert!(!t.players[0].objects[0].life.goto_sleep);
}

/// The actions' update: an idle forager runs none and captures nothing; a wait for a
/// foundation the world does not hold ends; a Gather walks on to its bush.
#[test]
fn the_actions_update_runs_each_action_held() {
    let mut t = forage_world();
    let captured = in_update(&mut t, (1, 0), 50, |u, c, o| u.update_actions(c, o)).unwrap();
    assert!(captured.is_none());
    t.players[1].objects[0].actions.set_wait_solidify(Some(
        crate::construction::wait_solidify::WaitSolidify {
            state: 0,
            task: crate::objects::masters::Task::default(),
            target: 99_999,
        },
    ));
    let captured = in_update(&mut t, (1, 0), 50, |u, c, o| u.update_actions(c, o)).unwrap();
    assert!(captured.is_none());
    assert_eq!(
        t.players[1].objects[0].actions.wait_solidify().copied(),
        None
    );
    t.gather_command(1, 0, 600).unwrap();
    let before = t.players[1].objects[0].actions.gather().copied();
    let captured = in_update(&mut t, (1, 0), 50, |u, c, o| u.update_actions(c, o)).unwrap();
    assert!(captured.is_none());
    let after = t.players[1].objects[0].actions.gather().copied();
    assert_eq!(after.map(|g| g.target), Some(600));
    assert_ne!(before, after);
}

/// Slot 0x190 (`FUN_14106d090`, `FUN_140a78450`): the object the top action targets, none for an
/// action without one (a wait for a foundation) or without an action, and an error for an action the slice does not hold.
#[test]
fn the_top_action_s_object_is_its_target_unless_it_has_none() {
    use crate::construction::build::{BuildAction, BuildTask};
    let task = BuildTask {
        range: 0.1,
        work_value: 1.0,
        work_rate: 1.0,
        resource: -1,
        id: -1,
    };
    let building = Unit {
        actions: ActionList::default().with_build(Some(BuildAction::start(
            800,
            [1.0, 2.0, 0.0],
            0.0,
            task,
        ))),
        ..Unit::default()
    };
    assert_eq!(building.top_action_object(), Ok(Some(800)));
    // a wait for a foundation keeps it at +0x68, its +0x30 is −1
    let waiting = Unit {
        actions: ActionList::default().with_wait_solidify(Some(
            crate::construction::wait_solidify::WaitSolidify::new(
                crate::objects::masters::Task::default(),
                900,
            ),
        )),
        ..Unit::default()
    };
    assert_eq!(waiting.top_action_object(), Ok(None));
    assert_eq!(Unit::default().top_action_object(), Ok(None));
    let unmodelled = Unit {
        unmodelled_action: Some(GenieError::unsupported("an action")),
        ..building
    };
    assert!(unmodelled.top_action_object().is_err());
}

/// The top action's sub-action: a Hunt's `MoveTo` with its state number, else its Attack; a
/// Build's `MoveTo`; none for a `MoveTo` of its own or without an action.
#[test]
fn the_top_action_s_sub_action_is_its_move_to_or_its_attack() {
    use crate::movement::move_to::{MOVE_TO, MoveTo};
    use crate::world::checkpoint::TopAction;
    let walking = |state| TopAction {
        kind: MOVE_TO,
        state,
    };
    let mut hunt = crate::gathering::hunt::tests::hunting(7);
    hunt.move_to = Some(MoveTo::Arrived);
    let hunter = Unit {
        actions: ActionList::default().with_hunt(Some(hunt)),
        ..Unit::default()
    };
    assert_eq!(hunter.top_sub_action(), Some(walking(1)));
    let mut idle = hunt;
    idle.move_to = None;
    let resting = Unit {
        actions: ActionList::default().with_hunt(Some(idle)),
        ..Unit::default()
    };
    assert_eq!(resting.top_sub_action(), None);
    let mut build = crate::construction::build::BuildAction::start(
        800,
        [1.0, 2.0, 0.0],
        0.0,
        crate::construction::build::BuildTask {
            range: 0.1,
            work_value: 1.0,
            work_rate: 1.0,
            resource: -1,
            id: -1,
        },
    );
    build.move_to = Some(MoveTo::Moving);
    let builder = Unit {
        actions: ActionList::default().with_build(Some(build)),
        ..Unit::default()
    };
    assert_eq!(builder.top_sub_action(), Some(walking(4)));
    assert_eq!(Unit::default().top_sub_action(), None);
}
