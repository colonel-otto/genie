use crate::checked_rows::{AiRow, MoveRow, Same};
use genie_core::movement::PathData;
use genie_core::unit_ai::{OrderKind, QueuedOrder, UnitAi};

/// A path level with a value in every field.
fn path() -> PathData {
    PathData {
        level: 1,
        linked_type: 2,
        points: vec![[1.0, 2.0, 0.0]],
        has_path: true,
        path_flags: 3,
        waypoint: 4,
        state: 5,
        range: 0.5,
        target: 600,
        pause_time: 1.5,
        flags: 7,
        length: 8.0,
        created: 9,
        continue_counter: 10,
        repath_time: 11.0,
        ignore: vec![12],
        ignored: vec![13],
    }
}

/// A moving object's row with a value in every field.
fn movement() -> MoveRow {
    MoveRow {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 1.0, 0.0],
        velocity: Some(([0.5, 0.0, 0.0], [0.0; 3])),
        paths: vec![path()],
        waiting: 1,
        wait_delays: 2,
        attack_moving: true,
        last_move: 300,
        substitute_count: 4,
        ignore_units: vec![5],
        angle: 0.25,
        last_turn: 0.75,
    }
}

/// A field's name and an edit of it.
type Edit<T> = (&'static str, fn(&mut T));

/// `base` against itself is the same; with any one of `edits` it is not, named by the field.
fn each_field_counts<T: Same + Clone>(base: &T, edits: &[Edit<T>]) {
    assert!(base.same(&base.clone()));
    for (field, edit) in edits {
        let mut other = base.clone();
        edit(&mut other);
        assert!(!base.same(&other), "{field} does not count");
    }
}

#[test]
fn every_field_of_a_path_level_counts() {
    each_field_counts(
        &path(),
        &[
            ("level", |p| p.level = 0),
            ("linked_type", |p| p.linked_type = 0),
            ("points", |p| p.points[0][0] = 9.0),
            ("has_path", |p| p.has_path = false),
            ("path_flags", |p| p.path_flags = 0),
            ("waypoint", |p| p.waypoint = 0),
            ("state", |p| p.state = 0),
            ("range", |p| p.range = 0.0),
            ("target", |p| p.target = 0),
            ("pause_time", |p| p.pause_time = 0.0),
            ("flags", |p| p.flags = 0),
            ("length", |p| p.length = 0.0),
            ("created", |p| p.created = 0),
            ("continue_counter", |p| p.continue_counter = 0),
            ("repath_time", |p| p.repath_time = 0.0),
            ("ignore", |p| p.ignore.clear()),
            ("ignored", |p| p.ignored.clear()),
        ],
    );
}

#[test]
fn every_field_of_a_movement_row_counts() {
    each_field_counts(
        &movement(),
        &[
            ("forward", |m| m.forward[1] = 1.0),
            ("right", |m| m.right[0] = 1.0),
            ("velocity", |m| m.velocity = None),
            ("paths", |m| m.paths[0].state = 0),
            ("waiting", |m| m.waiting = 0),
            ("wait_delays", |m| m.wait_delays = 0),
            ("attack_moving", |m| m.attack_moving = false),
            ("last_move", |m| m.last_move = 0),
            ("substitute_count", |m| m.substitute_count = 0),
            ("ignore_units", |m| m.ignore_units.clear()),
            ("angle", |m| m.angle = 0.0),
            ("last_turn", |m| m.last_turn = 0.0),
        ],
    );
}

/// The same to the bit: a NaN is the same as itself, 0.0 is not -0.0.
#[test]
fn floats_are_the_same_by_their_bits() {
    let mut nan = movement();
    nan.angle = f32::NAN;
    assert!(nan.same(&nan.clone()));
    let mut negative = movement();
    negative.angle = -0.0;
    let mut positive = movement();
    positive.angle = 0.0;
    assert!(!negative.same(&positive));
}

/// A row's field by its name.
fn field(row: &AiRow, name: &str) -> String {
    row.fields
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// The slice's orders read as the save holds them: none as the field's −1, a queued order as
/// its record.
#[test]
fn an_ai_row_names_the_orders_as_the_save_does() {
    let mut record = [0u8; 52];
    record[4..8].copy_from_slice(&0x2c5i32.to_le_bytes());
    let saved = genie_save::generated::world::UnitAiModule {
        current_order_value: -1,
        last_order_value: 0x2c9,
        temp_order: vec![record],
        ..Default::default()
    };
    let ai = UnitAi {
        current_order: None,
        last_order: Some(OrderKind::Hunt),
        orders: vec![QueuedOrder::from_bytes(&record).unwrap()],
        ..UnitAi::default()
    };
    let (row, save) = (AiRow::of(&ai, false), AiRow::saved(&saved, None, false));
    for name in ["current_order", "last_order", "orders"] {
        assert_eq!(field(&row, name), field(&save, name), "{name}");
    }
    assert_eq!(field(&save, "current_order"), "-1");
}

/// A saved queue with an order no case names shows the error, which no slice row equals.
#[test]
fn a_saved_queue_with_an_unknown_order_shows_the_error() {
    let mut record = [0u8; 52];
    record[4..8].copy_from_slice(&0x262i32.to_le_bytes());
    let saved = genie_save::generated::world::UnitAiModule {
        temp_order: vec![record],
        ..Default::default()
    };
    let orders = field(&AiRow::saved(&saved, None, false), "orders");
    assert!(orders.contains("an order value no case"), "{orders}");
}
