//! `SELL_COMMODITY` (0x7a) and `BUY_COMMODITY` (0x7b): the player trades at the market.

use crate::world::World;

/// `SELL_COMMODITY` (0x7a) and `BUY_COMMODITY` (0x7b), `do_command`'s cases: the player trades
/// `times` lots (`+0x6`) of the commodity `attribute` (`+0x4`'s low byte: food, wood or stone;
/// the decoder drops another) at the market ([`World::sell`], [`World::buy`]). The market
/// building (`+0x8`) is not read. Not held: the interface's view (`FUN_140e6de20`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TradeCommand {
    /// The player who trades.
    pub player: usize,
    /// The commodity.
    pub attribute: u8,
    /// How many lots.
    pub times: u16,
    /// A purchase, else a sale.
    pub buy: bool,
}

impl TradeCommand {
    /// The purchase ([`World::buy`]) or the sale ([`World::sell`]); a trade cannot fail.
    pub(crate) fn run(&self, w: &mut World) {
        if self.buy {
            w.buy(self.player, self.attribute, self.times);
        } else {
            w.sell(self.player, self.attribute, self.times);
        }
    }
}
