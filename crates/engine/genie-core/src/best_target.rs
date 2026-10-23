//! `UnitAIModule::bestUnitToAttack` (158041 `0x140c44680`) and what it keeps: the attack
//! categories (`FUN_140c4c8e0`) by which a unit AI's order history weighs its choice.

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
mod tests {
    use super::*;

    /// `FUN_140c4c8e0`'s switch: the archers' class 0 is 3, but masters 6 and 7 (the crossbowman
    /// lines' first) are 4; the siege's 13 is 6, but the rams' masters are 5; a class outside the
    /// table is −1.
    #[test]
    fn the_categories_follow_the_classes_and_their_exceptions() {
        assert_eq!(category(4, 0), 3);
        assert_eq!(category(6, 0), 4);
        assert_eq!(category(7, 23), 4);
        assert_eq!(category(83, 4), 7);
        assert_eq!(category(74, 6), 1);
        assert_eq!(category(1744, 6), 5);
        assert_eq!(category(280, 13), 6);
        assert_eq!(category(35, 13), 5);
        assert_eq!(category(1258, 55), 5);
        assert_eq!(category(109, 3), 0);
        assert_eq!(category(448, 47), 1);
        assert_eq!(category(1, 1), -1);
        assert_eq!(category(1, 63), -1);
        assert_eq!(category(1, -1), -1);
    }
}
