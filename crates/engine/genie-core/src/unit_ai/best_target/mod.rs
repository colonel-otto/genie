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
use crate::objects::masters::ClassId;
use crate::objects::masters::MasterId;
use crate::objects::masters::class;
use crate::objects::masters::ids;

/// `FUN_140c4c8e0`: the attack category of an object of master `master` and class `class`, by
/// which `bestUnitToAttack` weighs and narrows its choice; −1 for a class it does not rank.
#[must_use]
pub const fn category(master: MasterId, class: ClassId) -> i32 {
    match class {
        class::CLASS_0 | class::CLASS_23 | class::CLASS_26 | class::CLASS_36 | class::CLASS_44 => {
            if matches!(master, ids::ID_6 | ids::ID_7) {
                4
            } else {
                3
            }
        }
        class::TRADE_BOAT | class::FISHING_SHIP => 11,
        class::BUILDING
        | class::WALL
        | class::GATE
        | class::FARM
        | class::CLASS_51
        | class::TOWER => 0,
        class::VILLAGER | class::TRADE_CART | class::LIVESTOCK | class::KING => 7,
        class::CLASS_6
        | class::CLASS_12
        | class::CLASS_24
        | class::CLASS_35
        | class::CLASS_45
        | class::CLASS_47
        | class::CLASS_50
        | class::CLASS_56
        | class::CLASS_57 => {
            if matches!(master, ids::ID_1744 | ids::ID_1746) {
                5
            } else {
                1
            }
        }
        class::SIEGE | class::CLASS_54 | class::CLASS_55 => {
            if matches!(
                master,
                ids::ID_35 | ids::ID_422 | ids::ID_548 | ids::ID_1258
            ) {
                5
            } else {
                6
            }
        }
        class::CLASS_17 | class::MONK | class::CLASS_43 => 8,
        class::TRANSPORT | class::CLASS_62 => 10,
        class::WARSHIP | class::CLASS_53 => 9,
        class::CLASS_28 | class::CLASS_46 => 2,
        _ => -1,
    }
}

#[cfg(test)]
mod tests;
