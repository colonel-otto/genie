use crate::groups::SlotState;
use crate::test_ids::{object_id, player_id};
use crate::unit_ai::UnitStance;
use crate::world::Seconds;
use crate::world::load::groups::{formation, group};
use genie_save::generated::world as saved;

/// A formation slot's saved state reads as its state (3, in place); 4 names none and the
/// formation does not import.
#[test]
fn slot_takes_its_saved_state() {
    let saved = |state: i32| saved::Formation {
        slots: vec![saved::FormationEntry1 {
            slots_state: state,
            ..saved::FormationEntry1::default()
        }],
        ..saved::Formation::default()
    };
    let loaded = formation(&saved(3)).unwrap();
    assert_eq!(loaded.slots[0].state, SlotState::InPlace);
    assert!(formation(&saved(4)).is_err());
}

/// A saved group reads each member's id and specific move, the 32 bits of its owner as a player
/// (all ones for none) and its state as a stance (−1 for none); an owner or a stance past a byte
/// does not import.
#[test]
fn group_reads_its_members_owner_and_stance() {
    let saved_group = |owner: u32, state: i32| saved::UnitGroup {
        owner: f32::from_bits(owner),
        state,
        units: vec![saved::UnitGroupEntry1 {
            units: 7,
            specific_moves_timer: 2.5,
            ..saved::UnitGroupEntry1::default()
        }],
        ..saved::UnitGroup::default()
    };
    let loaded = group(&saved_group(1, -1), &[]).unwrap();
    assert_eq!((loaded.owner, loaded.stance), (Some(player_id(1)), None));
    assert_eq!(loaded.members.len(), 1);
    assert_eq!(loaded.members[0].id, object_id(7));
    assert_eq!(loaded.members[0].specific_move.timer, Seconds::new(2.5));
    let unowned = group(&saved_group(u32::MAX, 2), &[]).unwrap();
    assert_eq!(
        (unowned.owner, unowned.stance),
        (None, Some(UnitStance::StandGround))
    );
    assert!(group(&saved_group(256, -1), &[]).is_err());
    assert!(group(&saved_group(1, 256), &[]).is_err());
}
