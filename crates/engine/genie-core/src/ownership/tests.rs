use crate::GenieError;
use crate::movement::move_to::{MoveTo, MoveToAction, MoveWait};
use crate::objects::unit::Unit;
use crate::ownership::artifact::Artifact;
use crate::ownership::capture::{CAPTURE_KEEPS_ACTION, LIVESTOCK};
use crate::world::World;
use crate::world::tests::{forage_world, in_update};

/// [`forage_world`]'s forager walking to (40, 80) on the bush 600, its unit AI on a gather
/// order there.
fn walking_forager() -> World {
    let mut t = forage_world();
    let u = &mut t.players[1].objects[0];
    u.actions.set_move_to(Some(MoveToAction {
        run: false,
        state: MoveTo::Moving,
        goal: Some([40.0, 80.0, 0.0]),
        target: -1,
        range: 0.0,
        wait: MoveWait::default(),
    }));
    let ai = u.ai.as_mut().unwrap();
    (ai.current_order, ai.current_action, ai.current_target) = (610, 510, 600);
    t
}

/// Object 0 of player 1's capture stop as master class `class`, then the unit and the targets
/// the stop let go.
fn stopped(mut t: World, class: i16) -> Result<(Unit, Vec<i32>), GenieError> {
    let released = in_update(&mut t, (1, 0), 50, |u, c, _| {
        u.capture_stop(c, class)?;
        Ok(c.releases.iter().map(|&(id, _)| id).collect::<Vec<_>>())
    })?;
    Ok((t.players[1].objects.swap_remove(0), released))
}

/// `FUN_14108c050` without an Artifact: the action list goes (the `MoveTo`'s teardown stops the
/// unit), then the unit AI of any class but livestock ends its order and forgets its target.
#[test]
fn a_capture_stop_tears_the_actions_down_and_stops_the_unit_ai() {
    let (u, released) = stopped(walking_forager(), 4).unwrap();
    assert_eq!(u.actions.move_to().copied(), None);
    assert_eq!(released, []);
    let m = u.body.mover().unwrap();
    assert_eq!(m.animated.speed, 0.0);
    assert_eq!(m.velocity.unwrap().velocity[2], 0.0);
    let ai = u.ai.as_ref().unwrap();
    assert_eq!(
        (ai.current_order, ai.current_action, ai.current_target),
        (-1, -1, -1)
    );
    assert_eq!((ai.last_order, ai.last_action), (610, 510));
    // livestock's unit AI, and one on action 0x2c2, keep their order and target
    let (u, _) = stopped(walking_forager(), LIVESTOCK).unwrap();
    assert_eq!(u.actions.move_to().copied(), None);
    let kept = |ai: &crate::unit_ai::UnitAi| (ai.current_order, ai.current_target);
    assert_eq!(kept(u.ai.as_ref().unwrap()), (610, 600));
    let mut t = walking_forager();
    t.players[1].objects[0].ai.as_mut().unwrap().current_action = CAPTURE_KEEPS_ACTION;
    let (u, _) = stopped(t, 4).unwrap();
    assert_eq!(kept(u.ai.as_ref().unwrap()), (610, 600));
}

/// `FUN_14108c050` with an Artifact on top: its slot 0x38 keeps the action, drops its walking
/// `MoveTo` (which stops the unit) and lets its target go; an animal walking without movement
/// state is not modelled.
#[test]
fn a_capture_stop_keeps_the_artifact_and_lets_its_target_go() {
    let artifact = || Artifact {
        move_to: Some(MoveTo::Moving),
        goal: Some([32.5, 89.5, 0.0]),
        target: 600,
        ..Artifact::new(false)
    };
    let mut t = forage_world();
    t.players[1].objects[0]
        .actions
        .set_artifact(Some(artifact()));
    let (u, released) = stopped(t, LIVESTOCK).unwrap();
    let a = u.actions.artifact().copied().unwrap();
    assert_eq!(
        (a.move_to, a.goal, a.target, a.state),
        (None, None, -1, crate::ownership::artifact::CHECK)
    );
    assert_eq!(released, [600]);
    assert_eq!(u.body.mover().unwrap().animated.speed, 0.0);
    let mut t = forage_world();
    let u = &mut t.players[1].objects[0];
    u.actions.set_artifact(Some(artifact()));
    u.body.put_mover(None);
    assert_eq!(
        stopped(t, LIVESTOCK).unwrap_err().reason(),
        "a walking animal without movement state"
    );
}
