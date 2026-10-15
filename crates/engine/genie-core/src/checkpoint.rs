//! The per-player 11-word state block that recorded games checkpoint every N syncs.
//!
//! The engine builds it in `WorldPlayerBase::create_checksum` (158041: `0x1416607a0`); the word
//! table and its derivation are in the workspace's `docs/AoE2DE_s_exe_SIMULATION.md`. Every sum is a
//! wrapping 32-bit add, and every float-to-int step is the x86 `CVTTSS2SI` instruction.

/// What the block reads from one object of a player's live list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectState {
    pub id: i32,
    pub main_id: i16,
    pub state: u8,
    pub amount_held: f32,
    pub position: [f32; 3],
    pub top_action: Option<TopAction>,
    pub retarget_timer: Option<i32>,
}

/// The type (virtual slot 166) and state (virtual slot 86) of an object's top action.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TopAction {
    pub kind: i16,
    pub state: u8,
}

/// What the block reads from a player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub id: u32,
    pub resources: [f32; 4],
    pub sleeping_count: u32,
    pub live: Vec<ObjectState>,
}

/// The player's 11-word block, in the order the recording stores it.
pub fn player_block(player: &PlayerState) -> [u32; 11] {
    let mut w = [0u32; 11];
    w[0] = player.sleeping_count;
    w[1] = player
        .resources
        .iter()
        .fold(0u32, |acc, r| acc.wrapping_add(cvttss2si(*r) as u32));
    for o in &player.live {
        w[2] = w[2].wrapping_add(i32::from(o.main_id) as u32);
        w[3] = w[3].wrapping_add(u32::from(o.state));
        w[4] = w[4].wrapping_add(cvttss2si(o.amount_held) as u32);
        for axis in o.position {
            w[7] = w[7].wrapping_add(cvttss2si(axis * 100.0) as u32);
        }
        w[9] = w[9].wrapping_add(action_checksum(o));
        w[10] = w[10].wrapping_add(o.id as u32);
    }
    // w[5] (ObjectWaypoints) stays 0: the engine zeroes it and never writes it.
    w[6] = player.live.len() as u32;
    w[8] = player.id;
    w
}

/// Word 9's term for one object: action type + action state byte + unit-AI retarget timer. An
/// object without an action list reports type -1 and state 0.
fn action_checksum(o: &ObjectState) -> u32 {
    let TopAction { kind, state } = o.top_action.unwrap_or(TopAction { kind: -1, state: 0 });
    (i32::from(kind) as u32)
        .wrapping_add(u32::from(state))
        .wrapping_add(o.retarget_timer.unwrap_or(0) as u32)
}

/// `CVTTSS2SI`: f32 to i32, truncating toward zero. NaN and values outside the i32 range give
/// `i32::MIN` (the "integer indefinite" value), where Rust's `as` would saturate.
pub fn cvttss2si(value: f32) -> i32 {
    if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object() -> ObjectState {
        ObjectState {
            id: 1,
            main_id: 4,
            state: 2,
            ..ObjectState::default()
        }
    }

    fn player(live: Vec<ObjectState>) -> PlayerState {
        PlayerState {
            id: 3,
            live,
            ..PlayerState::default()
        }
    }

    #[test]
    fn cvttss2si_truncates_toward_zero() {
        assert_eq!((cvttss2si(2.9), cvttss2si(-2.9)), (2, -2));
    }

    #[test]
    fn cvttss2si_returns_integer_indefinite_for_nan_and_out_of_range() {
        for v in [f32::NAN, 2_147_483_648.0, -2_147_483_904.0, f32::INFINITY] {
            assert_eq!(cvttss2si(v), i32::MIN, "{v}");
        }
    }

    #[test]
    fn empty_player_carries_sleeping_count_resources_and_id() {
        let p = PlayerState {
            id: 7,
            resources: [200.0, 200.0, 200.0, 100.0],
            sleeping_count: 5,
            live: vec![],
        };
        assert_eq!(player_block(&p), [5, 700, 0, 0, 0, 0, 0, 0, 7, 0, 0]);
    }

    #[test]
    fn resources_are_truncated_one_by_one() {
        let p = PlayerState {
            resources: [1.9; 4],
            ..PlayerState::default()
        };
        assert_eq!(player_block(&p)[1], 4);
    }

    #[test]
    fn main_ids_are_sign_extended() {
        let p = player(vec![
            ObjectState {
                main_id: -1,
                ..object()
            },
            ObjectState {
                main_id: 3,
                ..object()
            },
        ]);
        assert_eq!(player_block(&p)[2], 2);
    }

    #[test]
    fn positions_are_scaled_in_f32_before_truncation() {
        // f32: 0.57 * 100 rounds to 57.0; the same product in f64 is 56.99999928 and truncates to 56
        let p = player(vec![ObjectState {
            position: [0.57, 0.0, 0.0],
            ..object()
        }]);
        assert_eq!(player_block(&p)[7], 57);
    }

    #[test]
    fn object_without_action_or_unit_ai_contributes_minus_one_to_word_9() {
        let p = player(vec![object()]);
        assert_eq!(player_block(&p)[9], u32::MAX);
    }

    #[test]
    fn word_9_adds_action_type_state_byte_and_retarget_timer() {
        let o = ObjectState {
            top_action: Some(TopAction { kind: 5, state: 3 }),
            retarget_timer: Some(100),
            ..object()
        };
        assert_eq!(player_block(&player(vec![o]))[9], 108);
    }

    #[test]
    fn sums_wrap_in_32_bits() {
        let p = player(vec![
            ObjectState {
                id: i32::MAX,
                ..object()
            },
            ObjectState {
                id: i32::MAX,
                ..object()
            },
        ]);
        assert_eq!(player_block(&p)[10], 0xFFFF_FFFE);
    }

    #[test]
    fn words_count_state_amount_live_list_and_zero_waypoints() {
        let p = player(vec![
            ObjectState {
                amount_held: 150.7,
                ..object()
            },
            object(),
        ]);
        let b = player_block(&p);
        assert_eq!((b[3], b[4], b[5], b[6]), (4, 150, 0, 2));
    }
}
