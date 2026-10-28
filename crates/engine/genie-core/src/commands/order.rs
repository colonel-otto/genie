//! The orders of units (INTERACT, WORK, MOVE, RETREAT, FOLLOW, GUARD): their units, the check,
//! the reset and the order's run.

use crate::GenieError;
use crate::orders::{Interact, MoveOrder};
use crate::world::World;

/// WORK's and RETREAT's runs (`FUN_140e6a9f0`, `FUN_140e69f00`).
const ORDER_UNMODELLED: GenieError = GenieError::unsupported("an order moves its units");

/// The units an order command names (its unit-list reader, `FUN_140e6b290`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderUnits {
    /// The packet's ids, as many as its count says.
    Listed(Vec<i32>),
    /// A count of `0xffff`: the comm player's last commanded units
    /// ([`World::last_commanded`]).
    LastCommanded,
}

/// What an order command gives its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// INTERACT: work on, attack or follow a target.
    Interact(Interact),
    /// MOVE: walk to a point.
    Move(MoveOrder),
    /// FOLLOW's target (`+0x8`).
    Follow(i32),
    /// GUARD's target (`+0x8`).
    Guard(i32),
    /// WORK: its units' work on a target (`FUN_140e6a9f0`), which is not modelled.
    Work,
}

/// INTERACT (0x00), WORK (0x02), MOVE (0x03), RETREAT (0x23), FOLLOW or GUARD: before the order
/// runs, each listed unit's AI resets its idle timeout ([`UnitAi::reset_for_order`], WORK's
/// not), in list order, wherever the unit is. INTERACT's order then runs ([`Interact`]),
/// MOVE's ([`MoveOrder`]), FOLLOW's and GUARD's; WORK and RETREAT are not modelled.
///
/// [`UnitAi::reset_for_order`]: crate::unit_ai::UnitAi::reset_for_order
#[derive(Clone, Debug, PartialEq)]
pub struct OrderCommand {
    /// The player who orders.
    pub player: usize,
    /// The units of the packet.
    pub units: OrderUnits,
    /// The order they take, None for RETREAT, which only resets their idle timeouts.
    pub order: Option<Order>,
}

impl OrderCommand {
    /// Its units after the command's check ([`OrderCommand::units`]). FOLLOW (`FUN_140e66a60`)
    /// and GUARD (`FUN_140e67620`) of a target the world lacks do nothing. The units reset for
    /// the order (WORK's not), a human's take the world time as their last human order time
    /// (`+0x15`, `FUN_140e5e780`; FOLLOW's when a unit the world holds belongs to a player who
    /// is not a computer, `FUN_141659cf0`), then the order runs.
    pub(crate) fn run(&self, w: &mut World) -> Result<(), GenieError> {
        let (player, order) = (self.player, self.order);
        let Some(units) = self.units(w) else {
            return Ok(());
        };
        if order == Some(Order::Work) {
            return Err(ORDER_UNMODELLED);
        }
        if let Some(Order::Follow(target) | Order::Guard(target)) = order
            && w.object(target).is_none()
        {
            return Ok(());
        }
        let human = match order {
            Some(Order::Interact(o)) => o.human,
            Some(Order::Move(o)) => o.human,
            Some(Order::Follow(_) | Order::Guard(_)) => units
                .iter()
                .any(|&id| w.object(id).is_some_and(|(p, _)| !w.players[p].computer)),
            Some(Order::Work) | None => false,
        };
        w.reset_order_units(&units);
        if human {
            w.human_order(&units);
        }
        match order {
            Some(Order::Interact(o)) => w.interact(player, &units, &o),
            Some(Order::Move(o)) => w.move_order(player, &units, &o),
            Some(Order::Follow(target)) => w.follow(player, &units, target),
            Some(Order::Guard(target)) => w.guard(player, &units, target),
            Some(Order::Work) | None => Err(ORDER_UNMODELLED),
        }
    }

    /// Its units after the command's check. The checks of INTERACT (`FUN_140e55250`), WORK
    /// (`FUN_140e55eb0`), MOVE (`FUN_140e553f0`) and RETREAT (`FUN_140e55710`) read the units
    /// ([`World::command_units`]) and make each one the world does not hold, or of another
    /// player without the setting `+0x1074`, −1 ([`World::checked_units`]); read from the last
    /// commanded units, they leave the −1s there. A human player's command left without a unit,
    /// or a command of a player the world lacks, does nothing (None). Else the run reads its
    /// units again (`FUN_140e6b290` with 0): a list of the command's own becomes the last
    /// commanded units, −1s and all. FOLLOW and GUARD take the first 60 units of their packet
    /// (500 for a computer); GUARD's check is BUILD's builders' (`FUN_140e550d0`,
    /// [`World::command_check`]), FOLLOW's runs in [`World::follow`].
    fn units(&self, w: &mut World) -> Option<Vec<i32>> {
        let (player, source, order) = (self.player, &self.units, self.order);
        if let Some(Order::Follow(_) | Order::Guard(_)) = order {
            // the decoder lists FOLLOW's and GUARD's own units: their count is never 0xffff
            let ids = match source {
                OrderUnits::Listed(ids) => ids.as_slice(),
                OrderUnits::LastCommanded => &[],
            };
            let ids = &ids[..ids.len().min(w.most_units(player))];
            return match order {
                Some(Order::Guard(_)) => w.command_check(player, ids),
                _ => Some(ids.to_vec()),
            };
        }
        let computer = w.players.get(player)?.computer;
        let checked = w.checked_units(player, &w.command_units(player, source));
        if *source == OrderUnits::LastCommanded {
            w.set_last_commanded(player, checked.clone());
        }
        if !computer && checked.iter().all(|&id| id == -1) {
            return None;
        }
        if let OrderUnits::Listed(_) = source {
            w.set_last_commanded(player, checked.clone());
        }
        Some(checked)
    }
}
