use crate::fixtures::{action_list, combat_mut, combat_object};
use crate::masters::{DefaultAction, MasterTable};
use crate::saved_unit::unit;
use genie_save::generated::world::{
    Action, PlayerObjects, RgeAction, RgeActionByType, RgeMissileObject, RgeObject,
};

/// An object without a static part is no unit the slice can run.
#[test]
fn an_object_without_its_static_part_is_unmodelled() {
    let u = unit(
        &genie_save::Object::default(),
        &PlayerObjects::default(),
        &[],
        0,
        None,
    );
    assert_eq!(u.id, -1);
    assert!(u.unmodelled_action.is_some());
}

/// A save from the start: livestock's default task is the Artifact, which exists from the
/// load on and starts again when its action list empties.
#[test]
fn livestock_starts_with_its_artifact() {
    let table = MasterTable::of_master(0, 83, None, 0).with_default_action(
        0,
        83,
        DefaultAction::Type(genie_core::ownership::artifact::ARTIFACT_TYPE),
    );
    let sheep = combat_object(5, 83);
    let u = unit(&sheep, &PlayerObjects::default(), &[], 0, Some(&table));
    assert_eq!(u.id, 5);
    assert!(u.artifact.is_some() && u.default_artifact);
    assert!(u.default_task.is_none());
    // a combat object's mover needs its master, which this save does not hold
    assert!(u.unmodelled_action.is_some());
}

/// Without a table the default task is unknown: reported, as the object's.
#[test]
fn without_a_table_the_default_task_is_reported() {
    let u = unit(
        &combat_object(5, 83),
        &PlayerObjects::default(),
        &[],
        0,
        None,
    );
    assert!(u.default_task.is_some());
    assert!(u.artifact.is_none());
}

/// A saved action the slice does not model stops the object.
#[test]
fn a_saved_action_the_slice_does_not_model_is_reported() {
    let mut scout = combat_object(5, 83);
    let explore = Action {
        rge_action_by_type: Some(Box::new(RgeActionByType {
            rge_action_explore: Some(Box::new(RgeAction::default())),
            ..RgeActionByType::default()
        })),
        ..Action::default()
    };
    combat_mut(&mut scout)
        .rge_combat_object
        .rge_action_object
        .actions = action_list(vec![(4, explore)]);
    let u = unit(&scout, &PlayerObjects::default(), &[], 0, None);
    assert!(u.unmodelled_action.is_some());
    assert!(u.default_fly.is_none());
}

/// A missile without its action (it met its target) flies on with its own fields.
#[test]
fn a_missile_without_its_action_keeps_flying() {
    let mut missile = RgeMissileObject {
        max_range: 7.0,
        fired_from_id: 900,
        ..RgeMissileObject::default()
    };
    missile.rge_combat_object.attack_count = 2;
    missile
        .rge_combat_object
        .rge_action_object
        .rge_moving_object
        .velocity_x = 0.5;
    missile
        .rge_combat_object
        .rge_action_object
        .rge_moving_object
        .rge_animated_object
        .rge_static_object
        .id = 6;
    let arrow = genie_save::Object {
        rge_object: Some(Box::new(RgeObject {
            rge_missile_object: Some(Box::new(missile)),
            ..RgeObject::default()
        })),
        ..genie_save::Object::default()
    };
    let u = unit(&arrow, &PlayerObjects::default(), &[], 0, None);
    let flying = u.missile.unwrap();
    assert_eq!((flying.fired_from, flying.attack_count), (900, 2));
    assert!(flying.flying);
    assert_eq!(flying.step[0].to_bits(), 0.5_f32.to_bits());
    assert_eq!(u.attack_count, 2);
}
