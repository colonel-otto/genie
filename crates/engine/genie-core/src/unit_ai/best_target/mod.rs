//! `UnitAIModule::bestUnitToAttack` (158041 `0x140c44680`) and what it keeps: the attack
//! categories (`FUN_140c4c8e0`) by which a unit AI's order history weighs its choice.

mod choice;
mod statistics;
mod world_view;

pub use self::choice::{Choice, best_unit_to_attack};
use self::statistics::EPSILON;
pub use self::statistics::{Candidate, Entry, STATISTICS, Statistics, order_weights};
use self::world_view::ORDER_WINDOW;
pub use self::world_view::{Ask, Battlefield, Searcher, Thing};

/// `FUN_140c4c8e0`: the attack category of an object of master `master` and class `class`, by
/// which `bestUnitToAttack` weighs and narrows its choice; −1 for a class it does not rank.
#[must_use]
pub const fn category(master: i16, class: i16) -> i32 {
    match class {
        0 | 23 | 26 | 36 | 44 => {
            if matches!(master, 6 | 7) {
                4
            } else {
                3
            }
        }
        2 | 21 => 11,
        3 | 27 | 39 | 49 | 51 | 52 => 0,
        4 | 19 | 58 | 59 => 7,
        6 | 12 | 24 | 35 | 45 | 47 | 50 | 56 | 57 => {
            if matches!(master, 1744 | 1746) {
                5
            } else {
                1
            }
        }
        13 | 54 | 55 => {
            if matches!(master, 35 | 422 | 548 | 1258) {
                5
            } else {
                6
            }
        }
        17 | 18 | 43 => 8,
        20 | 62 => 10,
        22 | 53 => 9,
        28 | 46 => 2,
        _ => -1,
    }
}

#[cfg(test)]
mod tests;
