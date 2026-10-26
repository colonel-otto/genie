//! The notifies' processing: their queue, their acceptance and their buffer.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_INVALIDATED, BREAK, DEAD, Effect, GO_ON, MOVE_ACTION, NO_LONGER_ON_ME,
    Notify, PRODUCED, Surroundings, UnitAi, UnitSide,
};

impl UnitAi {
    /// The notify processing of the full update: while notifies are queued and the last handler
    /// said go on, each runs through slot 0x238 ([`UnitAi::handle`]); a pass that went through
    /// them all ends with `FUN_140c5dc10`; 3 and 0xd break off. Returns the last handler's
    /// result (2 without notifies). Notifies posted during the pass (`FUN_140c55270`'s buffer)
    /// do not occur in the slice.
    pub(super) fn process_notifies(
        &mut self,
        dt: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let mut result = GO_ON;
        while !self.notifies.is_empty() && result == GO_ON {
            self.processing = true;
            let mut i = 0;
            while i < self.notifies.len() {
                let n = self.notifies[i];
                result = self.handle(n, dt, at, computer, ai_rng, side, fx)?;
                if result == DEAD {
                    // "I'm dead, purging notifyQueue and returning FALSE.": the full update ends
                    self.processing = false;
                    self.flush_buffered();
                    self.end_notifies(dt, at, fx)?;
                    return Ok(DEAD);
                }
                if result == BREAK || result == PRODUCED {
                    break;
                }
                i += 1;
            }
            if result == GO_ON {
                self.end_notifies(dt, at, fx)?;
            }
            self.processing = false;
            self.flush_buffered();
        }
        Ok(result)
    }

    /// `FUN_140c5dc10`: a completion of the current action (with no order) stops it
    /// (`FUN_140c5e1e0`, slot 0xb0 with 1); then the notifies go and `+0x15c` takes `time`.
    pub fn end_notifies(
        &mut self,
        time: u32,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        let completed = self.notifies.iter().any(|n| {
            n.kind == ACTION_COMPLETED && self.current_order == -1 && self.current_action == n.p[0]
        });
        if completed {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        self.last_update_time = time;
        self.notifies.clear();
        Ok(())
    }

    /// `FUN_140c5dc10` from outside the full update for a unit at `place` (its place and its
    /// master's class, which a stop ending an order reads): true when a completion stopped the
    /// action (slot 0xb0 with 1), whose stopAction (slot 0x3c0) the caller runs on the unit then.
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    pub fn clear_notifies_at(
        &mut self,
        time: u32,
        place: ([f32; 2], i16),
    ) -> Result<bool, GenieError> {
        let at = Surroundings {
            position: [place.0[0], place.0[1], 0.0],
            class: place.1,
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        self.end_notifies(time, &at, &mut fx)?;
        Ok(fx.contains(&Effect::Stop))
    }

    /// `FUN_140c5dc10` from outside the full update (a MOVE, an ownership change: `time` is the
    /// world time), where a completion it would stop on is not modelled.
    pub fn clear_notifies(&mut self, time: u32) -> Result<(), GenieError> {
        let mut fx = Vec::new();
        self.end_notifies(time, &Surroundings::default(), &mut fx)?;
        if !fx.is_empty() {
            return Err(GenieError::unsupported(
                "a completion stopping the action outside the full update",
            ));
        }
        Ok(())
    }

    /// The unit's notify (its slot 0x328, `FUN_141086a10`) to its unit AI: a unit in state 2 (or
    /// state 0 with a damage notify, which slot 0x38 refuses there) passes it to the AI's slot
    /// 0x38 ([`UnitAi::accept`]). A notify the AI does not take (any but 699) goes to the
    /// owner's slot 0x2e8 (`FUN_14162dab0`, the human player's alerts), which the slice does not
    /// hold.
    pub fn post(&mut self, n: Notify, unit_state: u8) -> Result<(), GenieError> {
        let _ = self.accept(n, unit_state);
        Ok(())
    }

    /// The unit AI's slot 0x38 (`FUN_140c556d0`, every type's): outside state 2 it takes nothing
    /// (0). A notify 699 is taken only from one of its attackers (a binary search of `+0xc0`),
    /// an action's invalidation only of the current action, and a movement completion
    /// (`0x1fa` with `0x262`) while the AI has an action other than a move (`0x262`, `0x268`,
    /// `0x2bf`, `0x2c0`) is ignored, answering 1 without joining the queue; any other joins the
    /// queue (`FUN_140c42470`, whose 0 on a failed allocation cannot happen) and answers 1.
    pub fn accept(&mut self, n: Notify, unit_state: u8) -> bool {
        if unit_state != 2 {
            return false;
        }
        let action = self.current_action;
        let taken = match n.kind {
            // "That unit is not attacking me, so I'm ignoring it."
            NO_LONGER_ON_ME => self.attackers.binary_search(&n.caller).is_ok(),
            ACTION_INVALIDATED => n.p[0] == action,
            ACTION_COMPLETED => {
                // ignored (and handled) for a non-movement action
                if n.p[0] == MOVE_ACTION && !matches!(action, 0x262 | 0x268 | 0x2bf | 0x2c0) {
                    return true;
                }
                true
            }
            _ => true,
        };
        if taken {
            self.append(n);
        }
        taken
    }

    /// `FUN_140c42470`: a notify joins the queue; a full queue doubles its capacity, but while
    /// it is processed the notify goes to the buffer instead.
    pub(super) fn append(&mut self, n: Notify) {
        if self.notify_capacity <= self.notifies.len() {
            if self.processing {
                self.buffered.push(n);
                return;
            }
            self.notify_capacity *= 2;
        }
        self.notifies.push(n);
    }

    /// `FUN_140c55270`: the buffered notifies join the queue, in order.
    pub(super) fn flush_buffered(&mut self) {
        for n in std::mem::take(&mut self.buffered) {
            self.append(n);
        }
    }
}
