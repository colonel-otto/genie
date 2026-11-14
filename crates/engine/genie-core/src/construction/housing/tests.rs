//! The Housing's tests on a house of player 1, and the fixtures `UNIT_ORDER`'s tests share.

use super::*;
use crate::objects::HitPoints;
use crate::objects::ListIndex;
use crate::objects::MasterHitPoints;
use crate::objects::ObjectId;
use crate::objects::ObjectState;
use crate::objects::WorkRate;
use crate::objects::actions::Action as ListAction;
use crate::objects::actions::ActionList;
use crate::objects::layers::Action;
use crate::objects::layers::Combat;
use crate::objects::lifecycle::Lifecycle;
use crate::objects::masters::ActionKind;
use crate::objects::masters::ActionPart;
use crate::objects::masters::MasterKind;
use crate::objects::masters::{Master, Masters};
use crate::objects::teardown::{HeadCode, Refund};
use crate::objects::unit::Unit;
use crate::players::PlayerId;
use crate::test_ids::object_id;
use crate::testing::class;
use crate::testing::ids;
use crate::testing::roles;
use crate::testing::slots;
use crate::world::FrameSeed;
use crate::world::Millis;
use crate::world::PlayState;
use crate::world::World;
use crate::world::WorldTime;
use crate::world::tests::{delete_head_at, in_update, wait_over};
use std::sync::Arc;

/// The house's id in the tests' worlds.
pub(crate) const HOUSE_ID: ObjectId = object_id(700);

/// The house's place: player 1's first object.
pub(crate) const AT: (PlayerId, ListIndex) = (PlayerId::new(1), ListIndex::new(0));

/// The order's queue id.
pub(crate) const ORDER: QueueId = QueueId::of_data(7);

/// The house master 70 (a building of class 3, 550 hit points), its work rate 0.5 (the `.dat`'s
/// is 0, which no update would run down).
pub(crate) fn house_master() -> Master {
    Master {
        id: ids::HOUSE,
        kind: MasterKind::BUILDING,
        class: class::BUILDING,
        hit_points: MasterHitPoints::of_data(550),
        action: Some(ActionPart {
            work_rate: WorkRate::new(0.5),
            ..ActionPart::default()
        }),
        ..Master::default()
    }
}

/// Player 1's attributes: `wood` wood, 5 headroom and 20 of attribute 62.
fn attributes(wood: f32) -> Vec<Amount> {
    let slots = slots().attribute;
    let mut amounts = vec![Amount::ZERO; 100];
    amounts[slots.wood.index()] = Amount::new(wood);
    amounts[slots.headroom.index()] = Amount::new(5.0);
    amounts[slots.building_house_rate.index()] = Amount::new(20.0);
    amounts
}

/// Player 1's object 700 of master `master` (its masters the house and `master`), alive with 550
/// hit points, holding `held` of attribute `kind`, its action list `list`; its owner holds `wood`
/// wood.
pub(crate) fn house_world(
    master: Master,
    (kind, held): (Option<AttributeId>, f32),
    list: ActionList,
    wood: f32,
) -> World {
    let house = Unit {
        id: HOUSE_ID,
        life: Lifecycle {
            state: ObjectState::Alive,
            hp: HitPoints::new(550.0),
            master_id: master.id,
            attribute_kind: kind,
            amount: Amount::new(held),
            ..Lifecycle::default()
        },
        ..Unit::default()
    };
    let owner = Player {
        objects: vec![house].into(),
        attributes: attributes(wood),
        ..Player::default()
    };
    let mut world = World::new(
        WorldTime::ZERO,
        FrameSeed::of_data(1),
        PlayerId::new(1),
        vec![Millis::ZERO, Millis::ZERO],
        vec![Player::default(), owner],
    )
    .with_actions([(
        HOUSE_ID,
        Action {
            list,
            ..Action::default()
        },
    )]);
    world.model.masters = vec![
        Arc::new(Masters::default()),
        Arc::new(Masters::new([house_master(), master])),
    ];
    world
}

/// The house holding 5 headroom with the action list `list`, its owner `wood` wood.
pub(crate) fn house_with(list: ActionList, wood: f32) -> World {
    house_world(
        house_master(),
        (Some(slots().attribute.headroom), 5.0),
        list,
        wood,
    )
}

/// A Housing of the order in `state` with `timer` work left.
pub(crate) fn housing(state: HousingState, timer: f32) -> HousingAction {
    HousingAction {
        state,
        timer: Seconds::new(timer),
        ..HousingAction::new(ORDER)
    }
}

/// The house holding `held` alone.
pub(crate) fn holding(held: HousingAction) -> ActionList {
    ActionList::default().with_housing(Some(held))
}

/// Player 1's wood.
pub(crate) fn wood(world: &World) -> f32 {
    world.players[1].attributes[slots().attribute.wood.index()].get()
}

/// Player 1's headroom.
fn headroom(world: &World) -> f32 {
    world.players[1].attributes[slots().attribute.headroom.index()].get()
}

/// The house's Housing, if it holds one.
pub(crate) fn held(world: &World) -> Option<HousingAction> {
    world.parts.actions_of(HOUSE_ID).list.housing().copied()
}

/// What the house holds: its attribute and how much.
fn house_holds(world: &World) -> (Option<AttributeId>, f32) {
    let life = &world.players[1].objects[AT.1].life;
    (life.attribute_kind, life.amount.get())
}

/// The house's Housing update (slot 0x30) in an update of `dt` ms.
fn updated(world: &mut World, dt: u32) -> HeadCode {
    in_update(world, AT, dt, |unit, actions, step| {
        unit.housing_part(actions, step)
    })
    .unwrap()
}

/// The house's work of its Housing in an update of `dt` ms.
fn work_of(world: &mut World, dt: u32) -> f32 {
    in_update(world, AT, dt, |unit, _, step| unit.housing_work(step))
        .unwrap()
        .get()
}

/// Each state reads back from its byte; a byte without a state is an error.
#[test]
fn its_states_read_back_from_their_bytes() {
    for byte in 0..=u8::MAX {
        match HousingState::of_byte(byte) {
            Ok(state) => assert_eq!(state.get(), byte),
            Err(error) => {
                assert!(!matches!(byte, 0 | 1 | 6 | 13), "{byte}");
                assert_eq!(error.reason(), "no state of a Housing");
            }
        }
    }
    assert_eq!(HousingState::of_byte(13).unwrap(), HousingState::Failed);
}

/// The constructor (`FUN_140ac1b80`): state 0, the timer 0, the order's queue id, type 0x7b.
#[test]
fn new_housing_holds_its_order_queue_id() {
    let made = HousingAction::new(ORDER);
    assert_eq!(
        (made.state, made.timer, made.unique_id),
        (HousingState::New, Seconds::ZERO, ORDER)
    );
    assert_eq!(ListAction::from(made).kind(), ActionKind::HOUSING);
    assert_eq!(ListAction::from(made).state().get(), 0);
}

/// The start (slot 0x28 with 1): the cost comes off the wood straight and the timer starts at
/// attribute 62; with exactly the cost it pays; with less it fails and pays nothing.
#[test]
fn start_pays_the_cost_or_fails_without_it() {
    for (held, state, left, timer) in [
        (120.0, HousingState::Working, 70.0, 20.0),
        (50.0, HousingState::Working, 0.0, 20.0),
        (49.5, HousingState::Failed, 49.5, 0.0),
    ] {
        let mut owner = Player {
            attributes: attributes(held),
            ..Player::default()
        };
        let mut started = HousingAction::new(ORDER);
        started
            .start(&mut owner, &slots().attribute, Amount::new(50.0))
            .unwrap();
        assert_eq!(
            (
                started.state,
                owner.attributes[slots().attribute.wood.index()].get(),
                started.timer.get()
            ),
            (state, left, timer),
            "{held}"
        );
    }
}

/// `COMISS cost, wood` with `JBE`: wood that is not a number pays, and stays not a number.
#[test]
fn start_with_nan_wood_pays() {
    let mut owner = Player {
        attributes: attributes(f32::NAN),
        ..Player::default()
    };
    let mut started = HousingAction::new(ORDER);
    started
        .start(&mut owner, &slots().attribute, Amount::new(50.0))
        .unwrap();
    assert_eq!(started.state, HousingState::Working);
    assert!(
        owner.attributes[slots().attribute.wood.index()]
            .get()
            .is_nan()
    );
}

/// An owner whose attributes end before attribute 62 pays, then stops at the timer's read; one
/// without wood stops at once.
#[test]
fn start_without_attributes_stops() {
    let mut short = Player {
        attributes: vec![Amount::new(80.0); 10],
        ..Player::default()
    };
    let mut started = HousingAction::new(ORDER);
    assert!(
        started
            .start(&mut short, &slots().attribute, Amount::new(50.0))
            .is_err()
    );
    assert_eq!(short.attributes[slots().attribute.wood.index()].get(), 30.0);
    assert!(
        started
            .start(
                &mut Player::default(),
                &slots().attribute,
                Amount::new(50.0)
            )
            .is_err()
    );
}

/// The work at 6 comes off the timer; at most 0 it is done, a timer that is not a number works
/// on.
#[test]
fn work_ends_once_the_timer_runs_out() {
    let mut working = housing(HousingState::Working, 1.0);
    assert_eq!(working.work(Seconds::new(0.25)), WorkLeft::Some);
    assert_eq!(working.timer.get(), 0.75);
    assert_eq!(working.work(Seconds::new(0.75)), WorkLeft::None);
    let mut overrun = housing(HousingState::Working, 1.0);
    assert_eq!(overrun.work(Seconds::new(3.0)), WorkLeft::None);
    let mut nan = housing(HousingState::Working, f32::NAN);
    assert_eq!(nan.work(Seconds::new(3.0)), WorkLeft::Some);
}

/// The destructor gives the cost back at work alone.
#[test]
fn destructor_refunds_at_work_alone() {
    for (state, refund) in [
        (HousingState::New, Refund::Nothing),
        (HousingState::Done, Refund::Nothing),
        (HousingState::Working, Refund::HousingCost),
        (HousingState::Failed, Refund::Nothing),
    ] {
        assert_eq!(housing(state, 3.0).refund(), refund, "{state:?}");
    }
}

/// The update answers 1 in state 1; in 13 it is done and answers 3; in 0 it goes on: no state of
/// them works.
#[test]
fn update_answers_by_its_state() {
    for (state, code, after) in [
        (HousingState::Done, HeadCode::Deletes, HousingState::Done),
        (HousingState::Failed, HeadCode::Deletes, HousingState::Done),
        (HousingState::New, HeadCode::Keeps, HousingState::New),
    ] {
        let mut world = house_with(holding(housing(state, 3.0)), 120.0);
        assert_eq!(updated(&mut world, 2000), code, "{state:?}");
        assert_eq!(held(&world), Some(housing(after, 3.0)), "{state:?}");
        assert_eq!(headroom(&world), 5.0);
    }
}

/// An update of a building without a Housing keeps its list.
#[test]
fn update_without_housing_keeps_the_list() {
    let mut world = house_with(ActionList::default(), 120.0);
    assert_eq!(updated(&mut world, 2000), HeadCode::Keeps);
    assert_eq!(held(&world), None);
}

/// At work the building's work comes off the timer: its work rate's powerup plus its master's,
/// over the update's seconds ((0.25 + 0.5) × 2); the work goes on with no headroom.
#[test]
fn update_at_work_takes_work_rate_off_the_timer() {
    let mut world =
        house_with(holding(housing(HousingState::Working, 20.0)), 120.0).with_combats([(
            HOUSE_ID,
            Combat {
                work_rate_powerup: WorkRate::new(0.25),
                ..Combat::default()
            },
        )]);
    assert_eq!(updated(&mut world, 2000), HeadCode::Keeps);
    assert_eq!(held(&world), Some(housing(HousingState::Working, 18.5)));
    assert_eq!((headroom(&world), house_holds(&world).1), (5.0, 5.0));
}

/// Once the timer ran out the house holds 5 more headroom and its owner gains 5; the Housing is
/// done (1).
#[test]
fn update_houses_once_the_timer_runs_out() {
    let mut world = house_with(holding(housing(HousingState::Working, 1.0)), 120.0);
    assert_eq!(updated(&mut world, 2000), HeadCode::Deletes);
    assert_eq!(held(&world), Some(housing(HousingState::Done, 0.0)));
    assert_eq!(
        house_holds(&world),
        (Some(slots().attribute.headroom), 10.0)
    );
    assert_eq!((headroom(&world), wood(&world)), (10.0, 120.0));
}

/// A building that holds another attribute, or none, holds the gain alone once its Housing is
/// done.
#[test]
fn update_of_building_holding_no_headroom_starts_at_the_gain() {
    for kind in [Some(slots().attribute.wood), None] {
        let mut world = house_world(
            house_master(),
            (kind, 100.0),
            holding(housing(HousingState::Working, 1.0)),
            120.0,
        );
        assert_eq!(updated(&mut world, 2000), HeadCode::Deletes);
        assert_eq!(
            house_holds(&world),
            (Some(slots().attribute.headroom), 5.0),
            "{kind:?}"
        );
        assert_eq!(headroom(&world), 10.0);
    }
}

/// The list's update runs the Housing and deletes it once it is done: its destructor in state 1
/// gives nothing back.
#[test]
fn actions_update_deletes_housing_once_done() {
    let mut world = house_with(holding(housing(HousingState::Working, 1.0)), 120.0);
    let captured = in_update(&mut world, AT, 2000, |unit, actions, step| {
        unit.update_actions(actions, step, &roles(), slots())
    })
    .unwrap();
    assert!(captured.is_none());
    assert_eq!(held(&world), None);
    assert_eq!((headroom(&world), wood(&world)), (10.0, 120.0));
}

/// The work of the quick-build cheat (`+0xc14` not 0) is 1000, whatever the rate.
#[test]
fn work_with_instant_build_is_its_tuning() {
    let mut world = house_with(holding(housing(HousingState::Working, 20.0)), 120.0);
    world.instant_work = 1;
    assert_eq!(work_of(&mut world, 2000), 1000.0);
    assert_eq!(updated(&mut world, 2000), HeadCode::Deletes);
}

/// A damaged building works at its hit points' share (440 of 550: 0.8), at least half; at full
/// hit points, above them or at hit points that are not a number, it does the whole work.
#[test]
fn work_takes_hit_points_share_at_least_half() {
    for (hp, work) in [
        (440.0, 0.8),
        (110.0, 0.5),
        (550.0, 1.0),
        (600.0, 1.0),
        (f32::NAN, 1.0),
    ] {
        let mut world = house_with(holding(housing(HousingState::Working, 20.0)), 120.0);
        world.players[1].objects[AT.1].life.hp = HitPoints::new(hp);
        assert_eq!(work_of(&mut world, 2000), work, "{hp}");
    }
}

/// A building whose master the world lacks has no work.
#[test]
fn work_without_master_fails() {
    let mut world = house_with(holding(housing(HousingState::Working, 20.0)), 120.0);
    world.players[1].objects[AT.1].life.master_id = ids::TOWN_CENTRE;
    assert!(
        in_update(&mut world, AT, 2000, |unit, _, step| unit
            .housing_work(step))
        .is_err()
    );
}

/// The destructor's refund goes to the owner's wood while the game is played; when the game is
/// over, or for nothing to give back, the wood stays.
#[test]
fn torn_refund_goes_back_while_played() {
    for (refund, state, left) in [
        (Refund::HousingCost, PlayState::Playing, 170.0),
        (Refund::HousingCost, PlayState::Over, 120.0),
        (Refund::Nothing, PlayState::Playing, 120.0),
    ] {
        let mut world = house_with(ActionList::default(), 120.0);
        world.game_state = state;
        world.give_back_torn(refund, AT.0).unwrap();
        assert_eq!(wood(&world), left, "{refund:?} {state:?}");
    }
}

/// The list's deletion (`FUN_140a98620`) of a Housing at work gives its cost back; one that is
/// done gives nothing.
#[test]
fn cleared_housing_at_work_gives_its_cost_back() {
    for (state, left) in [(HousingState::Working, 170.0), (HousingState::Done, 120.0)] {
        let mut world = house_with(holding(housing(state, 3.0)), 120.0);
        world.clear_actions(AT.0, AT.1).unwrap();
        assert_eq!((held(&world), wood(&world)), (None, left), "{state:?}");
    }
}

/// The head's deletion from the step (`FUN_140a98e20`) gives a Housing's cost back; a Housing
/// under a deleted head restarts (slot 0x28 with 0) as it was.
#[test]
fn deleted_head_refunds_housing_and_restarts_none() {
    let mut world = house_with(holding(housing(HousingState::Working, 3.0)), 120.0);
    delete_head_at(&mut world, AT.0, AT.1).unwrap();
    assert_eq!((held(&world), wood(&world)), (None, 170.0));
    let mut under = house_with(
        wait_over(holding(housing(HousingState::Working, 3.0))),
        120.0,
    );
    delete_head_at(&mut under, AT.0, AT.1).unwrap();
    assert_eq!(
        (held(&under), wood(&under)),
        (Some(housing(HousingState::Working, 3.0)), 120.0)
    );
    assert_eq!(
        under.parts.actions_of(HOUSE_ID).top_action_kind(),
        Some(ActionKind::HOUSING)
    );
}

/// The stop (slot 0x38) ends the Housing and gives the cost back in any state, also where it
/// never paid; the list keeps it.
#[test]
fn stop_ends_housing_and_gives_cost_back_in_any_state() {
    for state in [
        HousingState::New,
        HousingState::Done,
        HousingState::Working,
        HousingState::Failed,
    ] {
        let mut world = house_with(holding(housing(state, 3.0)), 120.0);
        world.object_stop(AT.0, AT.1).unwrap();
        assert_eq!(
            (held(&world), wood(&world)),
            (Some(housing(HousingState::Done, 3.0)), 170.0),
            "{state:?}"
        );
    }
}

/// A stop whose owner has no wood writes the Housing back done, then fails.
#[test]
fn stop_without_wood_keeps_housing_done() {
    let mut world = house_with(holding(housing(HousingState::Working, 3.0)), 120.0);
    world.players[1].attributes.clear();
    assert!(world.object_stop(AT.0, AT.1).is_err());
    assert_eq!(held(&world), Some(housing(HousingState::Done, 3.0)));
}
