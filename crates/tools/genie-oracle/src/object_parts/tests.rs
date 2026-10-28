use crate::fixtures::{combat_mut, combat_object};
use crate::object_parts::unit_ai;
use genie_core::ErrorKind;
use genie_core::unit_ai::OrderKind;
use genie_save::generated::world::{TribeTowerUnitAiModule, UnitAiModule, UnitAiModuleEntry1};

/// A saved queue record holding order value `order`.
fn record(order: i32) -> [u8; 52] {
    let mut b = [0; 52];
    b[4..8].copy_from_slice(&order.to_le_bytes());
    b
}

/// A combat object whose unit AI is `ai`.
fn with_ai(ai: UnitAiModule) -> genie_save::Object {
    let mut object = combat_object(5, 4);
    combat_mut(&mut object).unit_ai_module = Some(Box::new(ai));
    object
}

/// A combat object without a unit AI imports none.
#[test]
fn an_object_without_a_unit_ai_imports_none() {
    assert_eq!(
        unit_ai(&combat_object(5, 4)).map(|ai| ai.is_none()),
        Ok(true)
    );
}

/// The orders import as their kinds: a field's −1 as none, the history's and the queue's
/// records as theirs.
#[test]
fn the_saved_orders_import_as_their_kinds() {
    let ai = unit_ai(&with_ai(UnitAiModule {
        current_order_value: -1,
        last_order_value: 0x2c5,
        oh: vec![UnitAiModuleEntry1 {
            new_oh_order: 0x2c9,
            ..UnitAiModuleEntry1::default()
        }],
        temp_order: vec![record(0x2c6)],
        ..UnitAiModule::default()
    }))
    .unwrap()
    .unwrap();
    assert_eq!(
        (ai.current_order, ai.last_order),
        (None, Some(OrderKind::Gather))
    );
    assert_eq!(ai.order_history[0].order, Some(OrderKind::Hunt));
    assert_eq!(ai.orders[0].order, OrderKind::Move);
    assert_eq!(ai.building_orders, []);
}

/// A tower's saved orders import as the building's queue.
#[test]
fn a_tower_s_saved_orders_import() {
    let mut object = combat_object(5, 4);
    combat_mut(&mut object).tribe_tower_unit_ai_module = Some(Box::new(TribeTowerUnitAiModule {
        unit_ai_module: UnitAiModule {
            current_order_value: -1,
            last_order_value: -1,
            ..UnitAiModule::default()
        },
        saved_order: vec![record(700)],
        ..TribeTowerUnitAiModule::default()
    }));
    let ai = unit_ai(&object).unwrap().unwrap();
    assert_eq!(ai.building_orders[0].order, OrderKind::Attack);
}

/// An order value no case names fails the import, wherever the save holds it.
#[test]
fn an_order_no_case_names_fails_the_import() {
    let saves = [
        UnitAiModule {
            current_order_value: 0x262,
            ..UnitAiModule::default()
        },
        UnitAiModule {
            current_order_value: -1,
            last_order_value: 0x2cf,
            ..UnitAiModule::default()
        },
        UnitAiModule {
            current_order_value: -1,
            last_order_value: -1,
            oh: vec![UnitAiModuleEntry1 {
                new_oh_order: 0x269,
                ..UnitAiModuleEntry1::default()
            }],
            ..UnitAiModule::default()
        },
        UnitAiModule {
            current_order_value: -1,
            last_order_value: -1,
            temp_order: vec![record(0x323)],
            ..UnitAiModule::default()
        },
    ];
    for save in saves {
        assert_eq!(
            unit_ai(&with_ai(save)).map_err(|e| e.kind()).err(),
            Some(ErrorKind::InvalidInput)
        );
    }
}
