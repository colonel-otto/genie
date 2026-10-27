use crate::GenieError;
use crate::movement::move_to::{MoveTo, MoveToAction, MoveWait};
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
        mover: Some(crate::gathering::hunt::tests::walker()),
        ..Unit::default()
    };
    u.take_sprite_facets();
    assert_eq!(u.mover.as_ref().unwrap().sprite_facets, None);
    u.sprite = Some(ActiveSprite::start(
        WALKING,
        &mut crate::math::rng::Well512::default(),
    ));
    u.take_sprite_facets();
    assert_eq!(
        u.mover.as_ref().unwrap().sprite_facets,
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
    let animated = |speed| {
        let mut t = forage_world();
        let u = &mut t.players[1].objects[0];
        u.sprite = Some(ActiveSprite::start(
            WALKING,
            &mut crate::math::rng::Well512::default(),
        ));
        u.anim_speed = speed;
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
    u.move_to = Some(MoveToAction {
        run: false,
        state: MoveTo::Moving,
        goal: Some([40.0, 80.0, 0.0]),
        target: -1,
        range: 0.0,
        wait: MoveWait::default(),
    });
    in_update(&mut t, (1, 0), 50, |u, c, _| u.actions_past_life(c)).unwrap();
    let u = &t.players[1].objects[0];
    assert_eq!(u.move_to, None);
    assert!(u.mover.as_ref().unwrap().speed_stopped);
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
    t.players[1].objects[0].mover.as_mut().unwrap().pauses = vec![(600, 0.5)];
    let pauses = in_update(&mut t, (1, 0), 50, |u, c, _| {
        u.after_actions(c)?;
        Ok(c.pauses.clone())
    })
    .unwrap();
    assert_eq!(pauses, [(600, 0.5)]);
    let u = &t.players[1].objects[0];
    assert_eq!(u.position, [29.75, 89.5, 0.0]);
    assert_eq!(u.mover.as_ref().unwrap().pauses, []);
    t.players[1].objects[0].mover.as_mut().unwrap().in_the_way = vec![600];
    let r = in_update(&mut t, (1, 0), 50, |u, c, _| u.after_actions(c));
    assert_eq!(
        r.unwrap_err().reason(),
        "a walk's unit in the way (notify 0x215) the action does not pass on"
    );
}
