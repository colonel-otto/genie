//! Treason (GAME's sub-command 0xa, the old Spies): a player with a castle pays gold to have its
//! allies see the kings of the players it is not allied with, a flare at each king for each of
//! them.
//!
//! 158041: the castle test `FUN_14160ffb0`; the treason `FUN_141620400`; the flare is the
//! player's slot 0x1c0 with master 697 ([`Turns::remains`]).

use crate::GenieError;
use crate::turn::Turns;

/// The castle (`0x52`), and the building of the content mode 1 (`0xd1`) that stands for it
/// (`FUN_140fe0ec0`).
const CASTLE: i16 = 0x52;
const CASTLE_MODE_1: i16 = 0xd1;
/// The king (`0x1b2`), whom the treason reveals.
const KING: i16 = 0x1b2;
/// The flare each ally gets at a king (`0x2b9`).
const FLARE4: i16 = 0x2b9;
/// The gold (attribute 3) the treason costs: 400 (`DAT_142eb5bd4`, read at `0x141620446`), 200
/// (`DAT_142eb0f54`, read at `0x1416204d5`) with attribute 197 above 0.
const GOLD: usize = 3;
const TREASON_COST: f32 = crate::image::treason::TREASON_COST;
const TREASON_DISCOUNT_COST: f32 = crate::image::treason::TREASON_DISCOUNT_COST;
const DISCOUNT: usize = 197;

impl Turns {
    /// `FUN_14160ffb0`: player `p` has a castle (0x52; 0xd1 with the loaded world's content mode
    /// `+0x6b4` at 1, which a load leaves at the game's, [`Turns::mode_680`]): a master it has
    /// (`FUN_140b26630`) with at least one in its tracker's done list (`FUN_14104eea0`).
    #[must_use]
    pub fn has_treason_building(&self, p: usize) -> bool {
        let castle = if self.mode_680 == 1 {
            CASTLE_MODE_1
        } else {
            CASTLE
        };
        self.masters
            .get(p)
            .is_some_and(|ms| ms.get(castle).is_some())
            && !self.players[p].tracker.list(i32::from(castle)).is_empty()
    }

    /// `FUN_141620400`: player `p`'s treason. It costs 400 gold, 200 with attribute 197 above 0
    /// (`COMISS`, `JBE`: a NaN costs 400); with less gold (`COMISS`, `JA`: a NaN gold passes)
    /// or outside the regicide rule ([`crate::settings::Settings::regicide`]) it fails with the
    /// interface's error. Else the gold goes straight off attribute 3 (the local player's
    /// statistics and the event 0x91 are not simulation); then for each player from 1 not the
    /// player's own or allied (`FUN_141663dc0` without allies, [`crate::tasks::allied`]) its
    /// first king in its live list (`FUN_141166ab0`), or what the king is inside, gives each
    /// player from 1 the player counts as its own or allied a flare at its place (that player's
    /// slot 0x1c0 with master 697, [`Turns::remains`]); the sound and the minimap's mark
    /// (`FUN_140c7d6c0`, the game's `+0xa78`) are the interface's, as the local player's message.
    pub(crate) fn treason(&mut self, p: usize) -> Result<(), GenieError> {
        let discount = self.players[p].attribute(DISCOUNT).unwrap_or(0.0);
        let cost = if 0.0 < discount {
            TREASON_DISCOUNT_COST
        } else {
            TREASON_COST
        };
        let gold = self.players[p].resources[GOLD];
        if cost > gold || !self.settings.regicide() {
            return Ok(());
        }
        self.players[p].resources[GOLD] = gold - cost;
        let n = self.players.len();
        for i in 1..n {
            if crate::tasks::allied(&self.players[p].relations, p, i, false) {
                continue;
            }
            let Some(king) = self.players[i]
                .objects
                .iter()
                .find(|u| u.life.master_id == KING)
            else {
                continue;
            };
            let at = match king.life.inside.and_then(|h| self.object(h)) {
                Some((_, holder)) => holder.position,
                None => king.position,
            };
            for j in 1..n {
                if crate::tasks::allied(&self.players[p].relations, p, j, false) {
                    self.remains(j, FLARE4, at, None)?;
                    self.made_new_object(j)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::Lifecycle;
    use crate::turn::{Player, Unit};

    /// Gaia and three players; player 1 holds 500 gold and is allied with 3.
    fn world() -> Turns {
        let player = |p: usize| Player {
            attributes: vec![0.0; 300],
            relations: (0..4)
                .map(|q| u8::from(!(q == p || (p, q) == (1, 3) || (p, q) == (3, 1))) * 3)
                .collect(),
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0; 4], (0..4).map(player).collect());
        t.players[1].resources[GOLD] = 500.0;
        t
    }

    /// Without the regicide rule, or with too little gold, nothing happens.
    #[test]
    fn treason_needs_the_regicide_rule_and_the_gold() {
        let mut t = world();
        t.treason(1).unwrap();
        assert_eq!(t.players[1].resources[GOLD], 500.0);
        t.settings.game_mode = 1;
        t.players[1].resources[GOLD] = 399.0;
        t.treason(1).unwrap();
        assert_eq!(t.players[1].resources[GOLD], 399.0);
        t.players[1].attributes[DISCOUNT] = 1.0;
        t.treason(1).unwrap();
        assert_eq!(t.players[1].resources[GOLD], 199.0);
    }

    /// The treason pays 400 gold; without a king anywhere it flares nothing.
    #[test]
    fn treason_without_kings_only_pays() {
        let mut t = world();
        t.settings.game_mode = 1;
        let before = t.players[3].objects.len();
        t.treason(1).unwrap();
        assert_eq!(t.players[1].resources[GOLD], 100.0);
        assert_eq!(t.players[3].objects.len(), before);
    }

    /// The flare's master (697, animated, no hit points) for every player.
    fn with_flares(t: &mut Turns) {
        let flare = crate::masters::Master {
            id: FLARE4,
            kind: crate::masters::master_type::ANIMATED,
            standing_sprite: -1,
            ..crate::masters::Master::default()
        };
        t.masters = (0..t.players.len())
            .map(|_| crate::masters::Masters::new([flare.clone()]))
            .collect();
    }

    fn king(id: i32, position: [f32; 3], inside: Option<i32>) -> Unit {
        Unit {
            id,
            position,
            life: Lifecycle {
                master_id: KING,
                inside,
                ..Lifecycle::default()
            },
            ..Unit::default()
        }
    }

    /// Each king of a player not allied with player 1 (2's, the first in its list) gives
    /// player 1 and its ally 3 a flare at its place, or at the building it is inside; an
    /// allied player's king (3's) gives none.
    #[test]
    fn treason_flares_the_kings_for_the_player_and_its_allies() {
        let mut t = world();
        t.settings.game_mode = 1;
        with_flares(&mut t);
        t.next_object_id = 100;
        t.players[2].objects = vec![
            king(20, [5.0, 6.0, 0.0], None),
            king(21, [9.0, 9.0, 0.0], None),
        ];
        t.players[3].objects = vec![king(30, [1.0, 1.0, 0.0], None)];
        t.treason(1).unwrap();
        let flares = |t: &Turns, p: usize| -> Vec<(i32, [f32; 3])> {
            t.players[p]
                .objects
                .iter()
                .filter(|u| u.life.master_id == FLARE4)
                .map(|u| (u.id, u.position))
                .collect()
        };
        assert_eq!(flares(&t, 1), [(100, [5.0, 6.0, 0.0])]);
        assert_eq!(flares(&t, 3), [(101, [5.0, 6.0, 0.0])]);
        assert_eq!(flares(&t, 2), []);
        // a king inside a building: the flares go to the building's place
        let mut t = world();
        t.settings.game_mode = 1;
        with_flares(&mut t);
        let mut castle = king(40, [12.0, 13.0, 0.0], None);
        castle.life.master_id = CASTLE;
        t.players[2].objects = vec![king(20, [5.0, 6.0, 0.0], Some(40)), castle];
        t.treason(1).unwrap();
        assert_eq!(flares(&t, 1)[0].1, [12.0, 13.0, 0.0]);
    }

    /// The castle test asks the player's masters and its tracker.
    #[test]
    fn a_player_without_a_castle_has_no_treason() {
        let t = world();
        assert!(!t.has_treason_building(1));
    }
}
