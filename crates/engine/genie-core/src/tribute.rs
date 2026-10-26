//! A player's tribute: `TRIBE_COMMAND_GIVE_ATTRIBUTES` (0xc4, `TRIBE_Command::do_command`'s case,
//! `FUN_140e7d040`). A player with a market gives up to four of its resources to another, and
//! pays the market's fee on top; what it lacks lowers both alike.

use crate::GenieError;
use crate::math::fmath::maxss;
use crate::turn::Turns;

/// The masters one of which a player needs to pay tribute (`FUN_1416259e0`: its tracker's count
/// of the market, 84, or of 1647 above 0).
const MARKETS: [i16; 2] = [84, 1647];
/// The giver's tribute paid (53), with the fee (175), and to each player (450 + the receiver's
/// id); the receiver's tribute received (171) and from each player (475 + the giver's id).
const TRIBUTE_GIVEN: i16 = 53;
const TRIBUTE_PAID: i16 = 175;
const TRIBUTE_TO: i16 = 450;
const TRIBUTE_RECEIVED: i16 = 171;
const TRIBUTE_FROM: i16 = 475;

/// One of a tribute's four entries: the attribute (`+0x24`..), its amount (`+0x04`..) and the
/// fee (`+0x14`..), a share of the amount.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gift {
    /// The attribute given: food, wood, stone or gold (0 to 3); any other entry gives nothing.
    pub attribute: u16,
    /// The amount the receiver gets.
    pub amount: f32,
    /// The fee, the share of the amount the giver pays on top.
    pub fee: f32,
}

/// What one entry moves for a giver that has `have` of the resource: the amount the receiver
/// gets and what the giver pays. The amount and the fee are at least 0 (`MAXSS` with 0: a NaN is
/// 0); the fee on a positive amount is the amount times the fee; a giver that has less than the
/// two (`COMISS`, `JBE`: an unordered sum passes) pays all it has, the fee its share,
/// `have - have / (fee + 1)`, and the receiver gets the rest.
#[must_use]
pub fn settle(have: f32, gift: Gift) -> (f32, f32) {
    let amount = maxss(gift.amount, 0.0);
    let fee = maxss(gift.fee, 0.0);
    let mut extra = if 0.0 < amount { amount * fee } else { 0.0 };
    let mut given = amount;
    if have < given + extra {
        extra = have - have / (fee + 1.0);
        given = have - extra;
    }
    (given, given + extra)
}

impl Turns {
    /// `FUN_1416259e0`: player `p` may pay tribute, its tracker counting a market or a 1647
    /// (`FUN_140b26630`: a master the player has, `FUN_141659d20`, else none; the count of its
    /// done list, `FUN_14104eea0`).
    #[must_use]
    pub fn pays_tribute(&self, p: usize) -> bool {
        let has = |m: i16| self.masters.get(p).is_some_and(|ms| ms.get(m).is_some());
        MARKETS
            .iter()
            .any(|&m| has(m) && !self.players[p].tracker.list(i32::from(m)).is_empty())
    }

    /// `FUN_140e7d040`: player `from` gives player `to` its `gifts` ([`settle`]), unless the two
    /// are one or `from` has no market ([`Turns::pays_tribute`]). For each resource entry the
    /// receiver gains the amount and the giver loses it with the fee (the player's slot 0xf8,
    /// `FUN_141616820`, with 0 and 1: no gathered totals); the giver counts it in attributes 53
    /// and 450 + `to` and with the fee in 175, the receiver in 171 and 475 + `from`; then the
    /// receiver hears notify 0x20a (its slot 0x2e8, `FUN_14162dab0`: no case for it but the old
    /// short pather's hulled area and the shapes of the obstruction manager, neither held; a
    /// computer player's AI hears it, slot 0x3c8, which is not modelled). Not held: the local
    /// player's telemetry (`FUN_14097daf0`), the statistics (`FUN_1416625b0`,
    /// `FUN_1417786d0`) and the game's event 0x60 (`FUN_140c898a0`).
    ///
    /// # Errors
    ///
    /// An attribute a player does not have, and a computer player's receipt.
    pub(crate) fn tribute(
        &mut self,
        from: usize,
        to: usize,
        gifts: &[Gift; 4],
    ) -> Result<(), GenieError> {
        if from == to || !self.pays_tribute(from) {
            return Ok(());
        }
        for &gift in gifts {
            if 3 < gift.attribute {
                continue;
            }
            let kind = gift.attribute as i16;
            let have = self.players[from].resources[usize::from(gift.attribute)];
            let (given, paid) = settle(have, gift);
            self.players[to].add_attribute(kind, given)?;
            self.players[from].add_attribute(kind, -paid)?;
            let giver = &mut self.players[from];
            giver.add_attribute(TRIBUTE_GIVEN, given)?;
            giver.add_attribute(TRIBUTE_PAID, paid)?;
            giver.add_attribute(TRIBUTE_TO + i16::from(to as u8), given)?;
            let receiver = &mut self.players[to];
            receiver.add_attribute(TRIBUTE_RECEIVED, given)?;
            receiver.add_attribute(TRIBUTE_FROM + i16::from(from as u8), given)?;
            if receiver.computer {
                return Err(GenieError::unsupported(
                    "a computer player's AI told of a tribute (0x20a, slot 0x3c8)",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters};
    use crate::turn::Player;

    fn gift(attribute: u16, amount: f32, fee: f32) -> Gift {
        Gift {
            attribute,
            amount,
            fee,
        }
    }

    /// The fee goes on top of the amount; a giver short of the two pays all it has, the receiver
    /// getting `have / (fee + 1)` as `have - (have - have / (fee + 1))`; a negative or NaN
    /// amount or fee is 0.
    #[test]
    fn a_tribute_pays_its_fee_on_top_and_what_the_giver_lacks_lowers_both() {
        assert_eq!(
            settle(1000.0, gift(1, 100.0, 0.3)),
            (100.0, 100.0 + 100.0 * 0.3)
        );
        let extra = 100.0 - 100.0 / 1.3f32;
        assert_eq!(
            settle(100.0, gift(1, 100.0, 0.3)),
            (100.0 - extra, 100.0 - extra + extra)
        );
        assert_eq!(settle(100.0, gift(0, -5.0, 0.3)), (0.0, 0.0));
        assert_eq!(settle(100.0, gift(0, f32::NAN, 0.3)), (0.0, 0.0));
        assert_eq!(settle(100.0, gift(0, 50.0, f32::NAN)), (50.0, 50.0));
        // no fee: exactly what it has
        assert_eq!(settle(40.0, gift(3, 50.0, 0.0)), (40.0, 40.0));
    }

    /// Players 1 and 2, each with 600 attributes and 500 of each resource, player 1 a market
    /// (master 84) when `market`.
    fn world(market: bool) -> Turns {
        let player = || Player {
            resources: [500.0; 4],
            attributes: vec![0.0; 600],
            ..Player::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0; 3],
            vec![Player::default(), player(), player()],
        );
        let masters = Masters::new([Master {
            id: 84,
            kind: 80,
            class: 3,
            ..Master::default()
        }]);
        t.masters = vec![masters.clone(), masters.clone(), masters];
        if market {
            t.players[1].tracker.add(
                7,
                crate::tracker::Key {
                    master: 84,
                    class: 3,
                    pending: false,
                    military: false,
                },
            );
        }
        t
    }

    /// `FUN_140e7d040`: player 1's tribute of 100 wood and 200 gold at a fee of 0.3 to player 2
    /// (an entry of attribute 4 gives nothing): the resources move with the fee, and both count
    /// it; without a market, or to itself, nothing moves.
    #[test]
    fn a_tribute_moves_the_resources_and_counts_them_for_both_players() {
        let gifts = [
            gift(1, 100.0, 0.3),
            gift(3, 200.0, 0.3),
            gift(4, 50.0, 0.3),
            gift(0, 0.0, 0.3),
        ];
        let mut t = world(true);
        t.tribute(1, 2, &gifts).unwrap();
        let (giver, receiver) = (&t.players[1], &t.players[2]);
        assert_eq!(
            giver.resources,
            [500.0, 500.0 - 130.0, 500.0, 500.0 - 260.0]
        );
        assert_eq!(receiver.resources, [500.0, 600.0, 500.0, 700.0]);
        assert_eq!(
            (
                giver.attributes[53],
                giver.attributes[175],
                giver.attributes[452]
            ),
            (300.0, 390.0, 300.0)
        );
        assert_eq!(
            (receiver.attributes[171], receiver.attributes[476]),
            (300.0, 300.0)
        );
        assert_eq!(receiver.attributes[4], 0.0);
        for (from, to, market) in [(1, 2, false), (1, 1, true)] {
            let mut t = world(market);
            let before = t.players.clone();
            t.tribute(from, to, &gifts).unwrap();
            assert_eq!(t.players, before);
        }
    }

    /// A computer player's AI would hear the tribute (its slot 0x3c8).
    #[test]
    fn a_computer_player_s_receipt_is_not_modelled() {
        let mut t = world(true);
        t.players[2].computer = true;
        assert!(t.tribute(1, 2, &[gift(1, 10.0, 0.3); 4]).is_err());
    }
}
