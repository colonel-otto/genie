//! `UNIT_ORDER`'s Housing on the house of `construction::housing`'s tests.

use super::*;
use crate::commands::{Shift, UnitList, UnitOrderAction, UnitOrderCommand};
use crate::construction::housing::HousingState;
use crate::construction::housing::tests::{
    AT, HOUSE_ID, ORDER, held, holding, house_master, house_with, house_world, housing, wood,
};
use crate::garrison::GarrisonScope;
use crate::map::Point;
use crate::objects::actions::ActionList;
use crate::objects::masters::Master;
use crate::players::PlayerKind;
use crate::production::UnitIndex;
use crate::production::{Entry, MakeObj, MakeTech, MakeTechState, Making, Production, QueueLength};
use crate::test_ids::object_ids;
use crate::test_ids::tech_id;
use crate::testing::class;
use crate::testing::ids;
use crate::testing::slots;
use crate::world::Seconds;
use std::iter::once;

/// The order to the house alone.
fn order(world: &mut World) -> GenieResult<()> {
    world.housing_order(&object_ids([700]), ORDER)
}

/// The Housing on top of the house in its state and timer, if any.
fn working(world: &World) -> Option<(HousingState, f32)> {
    held(world).map(|housing_action| (housing_action.state, housing_action.timer.get()))
}

/// The house with the production part `production`, its owner of kind `kind`, holding no
/// headroom but 100 of nothing.
fn producing(kind: PlayerKind, production: Production) -> World {
    let mut world = house_world(house_master(), (None, 100.0), ActionList::default(), 120.0);
    world.players[1].kind = kind;
    world.parts.production = once((HOUSE_ID, production)).collect();
    world
}

/// A production whose queue holds `count` entries and that makes `making`.
fn production(count: i16, making: Option<Making>) -> Production {
    let mut production = Production {
        action: making,
        ..Production::default()
    };
    production.queue.count = QueueLength::of_data(count);
    production
}

/// A research in progress.
fn research() -> Making {
    Making::Research(MakeTech {
        state: MakeTechState::Researching,
        paid: false,
        tech: tech_id(22),
        master: -1,
        progress: Seconds::ZERO,
    })
}

/// The villager master 83 (type 70, class 4): below the buildings' type.
fn villager() -> Master {
    Master {
        id: ids::VILLAGER_MALE,
        kind: MasterKind::COMBAT,
        class: class::VILLAGER,
        ..Master::default()
    }
}

/// The new Housing pays 50 of the owner's 120 wood and goes to work on attribute 62, with the
/// order's queue id.
#[test]
fn housing_order_installs_paid_housing() {
    let mut world = house_with(ActionList::default(), 120.0);
    order(&mut world).unwrap();
    assert_eq!(
        (held(&world), wood(&world)),
        (Some(housing(HousingState::Working, 20.0)), 70.0)
    );
}

/// Without the wood the new Housing fails at its start: it stays on top in state 13, and the
/// wood stays.
#[test]
fn housing_order_without_wood_leaves_failed_housing() {
    let mut world = house_with(ActionList::default(), 10.0);
    order(&mut world).unwrap();
    assert_eq!(
        (working(&world), wood(&world)),
        (Some((HousingState::Failed, 0.0)), 10.0)
    );
}

/// The guard: a building takes the order below 45 (`COMISS`, `JBE`: a NaN does not), at work
/// or not; at 45 or more it takes it only holding no headroom and not at work.
#[test]
fn housing_order_takes_buildings_below_full_or_without_headroom() {
    let headroom = Some(slots().attribute.headroom);
    let wood_kind = Some(slots().attribute.wood);
    let at_work = || holding(housing(HousingState::Working, 4.0));
    for (holds, list, takes) in [
        ((headroom, 44.9), ActionList::default(), true),
        ((headroom, 45.0), ActionList::default(), false),
        ((headroom, f32::NAN), ActionList::default(), false),
        ((headroom, 50.0), at_work(), false),
        ((headroom, 5.0), at_work(), true),
        ((wood_kind, 100.0), ActionList::default(), true),
        ((None, 100.0), ActionList::default(), true),
        ((None, 100.0), at_work(), false),
        ((None, 0.0), at_work(), true),
    ] {
        let mut world = house_world(house_master(), holds, list, 120.0);
        order(&mut world).unwrap();
        assert_eq!(
            working(&world) == Some((HousingState::Working, 20.0)),
            takes,
            "{holds:?}"
        );
    }
}

/// A Housing at work that the order replaces gives its cost back before the new one pays: the
/// wood comes out even.
#[test]
fn housing_order_again_refunds_the_old_housing() {
    let mut world = house_with(holding(housing(HousingState::Working, 4.0)), 120.0);
    order(&mut world).unwrap();
    assert_eq!(
        (held(&world), wood(&world)),
        (Some(housing(HousingState::Working, 20.0)), 120.0)
    );
}

/// The types that keep a building from the order, the order's own compares (`0x14161ea3e`): the
/// Make (0x66), the research (0x67), the Housing (0x7b), the Pack (0x7c) and the Unpack
/// (0x7d), which genie's list does not hold yet (the task pack-unit-order).
#[test]
fn order_busy_kinds_are_the_order_compares() {
    assert_eq!(
        ORDER_BUSY.map(ActionKind::get),
        [0x66, 0x67, 0x7b, 0x7c, 0x7d]
    );
}

/// The work a building's owner reads: a computer's production first (its make 0x66, its research
/// 0x67, a queue that holds an entry 0x66), else the top action's type; a human's production
/// counts for nothing.
#[test]
fn building_work_kind_reads_computer_production_first() {
    let make = Making::Unit(MakeObj::for_entry(&Entry::default()));
    for (kind, production, work) in [
        (
            PlayerKind::Computer,
            production(0, Some(make)),
            Some(ActionKind::MAKE_OBJECT),
        ),
        (
            PlayerKind::Computer,
            production(0, Some(research())),
            Some(ActionKind::MAKE_TECH),
        ),
        (
            PlayerKind::Computer,
            production(1, None),
            Some(ActionKind::MAKE_OBJECT),
        ),
        (PlayerKind::Computer, production(0, None), None),
        (PlayerKind::Human, production(1, Some(research())), None),
    ] {
        let world = producing(kind, production);
        assert_eq!(
            world.building_work_kind(AT.0, AT.1).unwrap(),
            work,
            "{kind:?} {work:?}"
        );
    }
    let at_work = house_with(holding(housing(HousingState::Working, 4.0)), 120.0);
    assert_eq!(
        at_work.building_work_kind(AT.0, AT.1).unwrap(),
        Some(ActionKind::HOUSING)
    );
}

/// A computer's building whose queue holds an entry is busy and holds 45 or more of nothing: it
/// takes no Housing; a human's takes it.
#[test]
fn computer_building_with_queue_entry_takes_no_housing() {
    for (kind, takes) in [(PlayerKind::Computer, false), (PlayerKind::Human, true)] {
        let mut world = producing(kind, production(1, None));
        order(&mut world).unwrap();
        assert_eq!(held(&world).is_some(), takes, "{kind:?}");
    }
}

/// A building the load could not read whole stops at the work's read.
#[test]
fn housing_order_of_unread_building_fails() {
    let mut world = house_world(house_master(), (None, 100.0), ActionList::default(), 120.0);
    world.parts.unread =
        once((HOUSE_ID, Unread::new(GenieError::invalid_input("unread")))).collect();
    assert_eq!(order(&mut world).unwrap_err().reason(), "unread");
}

/// An object below the buildings' type holding less than 45 faults in the game; one holding 45
/// or more takes nothing.
#[test]
fn housing_order_of_no_building_faults_below_full() {
    let mut world = house_world(villager(), (None, 0.0), ActionList::default(), 120.0);
    assert_eq!(
        order(&mut world).unwrap_err().reason(),
        HOUSING_WITHOUT_BUILDING.reason()
    );
    let mut full = house_world(villager(), (None, 45.0), ActionList::default(), 120.0);
    order(&mut full).unwrap();
    assert_eq!(held(&full), None);
}

/// An object not in state 2 (a foundation, a dying building) and an id the world lacks take
/// nothing; a sleeping object is a gap.
#[test]
fn housing_order_skips_objects_not_alive() {
    for state in [ObjectState::Foundation, ObjectState::Dying] {
        let mut world = house_with(ActionList::default(), 120.0);
        world.players[1].objects[AT.1].life.state = state;
        order(&mut world).unwrap();
        assert_eq!((held(&world), wood(&world)), (None, 120.0), "{state:?}");
    }
    let mut lacking = house_with(ActionList::default(), 120.0);
    lacking.housing_order(&object_ids([701]), ORDER).unwrap();
    assert_eq!(held(&lacking), None);
    let mut asleep = house_with(ActionList::default(), 120.0);
    let house = asleep.players[1].objects.swap_remove(AT.1).unwrap();
    asleep.players[1].sleeping.push(house);
    asleep.reindex();
    assert_eq!(order(&mut asleep), Err(ORDER_ASLEEP));
}

/// The command's run routes case 3 to the Housing; a byte the switch has no case for does
/// nothing.
#[test]
fn unit_order_runs_housing_and_ignores_other_bytes() {
    let command = |action| UnitOrderCommand {
        player: PlayerId::new(1),
        units: UnitList {
            count: 1,
            ids: object_ids([700]),
        },
        target: None,
        point: Point::default(),
        queue_id: ORDER,
        shift: Shift::default(),
        scope: GarrisonScope::default(),
        action,
        param: UnitIndex::default(),
        human: false,
    };
    let mut world = house_with(ActionList::default(), 120.0);
    command(UnitOrderAction::Housing).run(&mut world).unwrap();
    assert_eq!(held(&world), Some(housing(HousingState::Working, 20.0)));
    let mut other = house_with(ActionList::default(), 120.0);
    command(UnitOrderAction::Other(9)).run(&mut other).unwrap();
    assert_eq!((held(&other), wood(&other)), (None, 120.0));
}
