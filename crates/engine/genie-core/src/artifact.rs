//! The Artifact action (type 107): the default task of livestock, which keeps checking whether a
//! unit of another player is close enough to take the animal over.
//!
//! 158041: `TRIBE_Action_Artifact`, vtable `0x142ed6520`; update `0x140aad250` (slot 6), set_state
//! `0x140aace30` (slot 16). The .dat gives Goat, Goose, Cow and Sheep the default task 0, a task with
//! action type 107. An idle animal holds this action as its top action, so checkpoint word 9 counts
//! `107 + state` for it.

/// One animal's Artifact action, as the checkpoint sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Artifact {
    /// 6 checks for a new owner; 10 waits for the timer.
    pub state: u8,
    /// `+0x44`, in seconds.
    pub timer: f32,
    /// Gaia waits half as long between checks.
    pub gaia: bool,
}

/// The action type the checkpoint adds for an Artifact action.
pub const ARTIFACT_TYPE: i16 = 107;

const CHECK: u8 = 6;
const WAIT: u8 = 10;

impl Artifact {
    /// A new action, about to check for a new owner.
    pub fn new(gaia: bool) -> Self {
        Artifact {
            state: CHECK,
            timer: 0.0,
            gaia,
        }
    }

    /// One update with the tick's `dt_ms`. The check always finds no new owner: the slice assumes
    /// no unit of another player comes near the animal.
    pub fn update(&mut self, dt_ms: u32) {
        match self.state {
            CHECK => {
                self.state = WAIT;
                self.timer = if self.gaia { 0.5 } else { 1.0 };
            }
            WAIT => {
                // world+0x164 is `dt / 1000.0f`; the subtraction is f32
                self.timer -= dt_ms as f32 / 1000.0;
                if self.timer <= 0.0 {
                    self.state = CHECK;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_finds_no_owner_and_waits_one_second() {
        let mut a = Artifact::new(false);
        a.update(50);
        assert_eq!((a.state, a.timer), (10, 1.0));
    }

    #[test]
    fn gaia_waits_half_a_second() {
        let mut a = Artifact::new(true);
        a.update(50);
        assert_eq!(a.timer, 0.5);
    }

    #[test]
    fn the_wait_ends_when_the_timer_reaches_zero_then_one_check_follows() {
        let mut a = Artifact::new(false);
        a.update(50);
        // 1.0 - 20 × 0.05 in f32 is -1.56e-7, so the 20th update ends the wait
        let waits = (0..30).take_while(|_| {
            a.update(50);
            a.state == 10
        });
        assert_eq!(waits.count(), 19);
        assert_eq!(a.state, 6);
        a.update(50);
        assert_eq!((a.state, a.timer), (10, 1.0));
    }
}
