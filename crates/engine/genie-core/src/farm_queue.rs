//! The mill's and the dock's queues: the farms and pastures (`farm_queue_count`, `+0x141c`) and
//! the fish traps (`fishTrapQueueCount`, `+0x1420`) a player paid for ahead, which GAME's
//! sub-commands 0xd, 0x15 and 0x11 buy and 0xe, 0x16 and 0x12 give back. Each queue holds at
//! most 40 before a purchase.
//!
//! 158041: the farms' purchase `FUN_141617130`, their return `FUN_141630f10`; the fish traps'
//! `FUN_141616540` and `FUN_1416308f0`; the payment `FUN_14162fb60`
//! ([`crate::production::pay`]) and the refund `FUN_141630770`
//! ([`crate::production::refund_master`]).

use crate::production::{cost_test, pay, refund_master};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A queue takes a purchase only below this count (`0x28`).
pub const QUEUE_LIMIT: i32 = 40;
/// The fish trap (master 199, its master table's `+0x638`).
pub const FISH_TRAP: i16 = 199;
/// The farm a mill's queue buys (`0x32`, GAME 0xd and 0xe).
pub const FARM: i16 = 0x32;
/// The pasture a mill's queue buys (`0x761`, GAME 0x15 and 0x16).
pub const PASTURE: i16 = 0x761;

/// Which of a player's queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Queue {
    /// `+0x141c`: farms and pastures.
    Farms,
    /// `+0x1420`: fish traps.
    FishTraps,
}

impl Turns {
    /// `FUN_141617130`: player `p` buys `count` of master `master` (the farm 0x32 or the pasture
    /// 0x761) for its mill's queue ([`Turns::buy_ahead`]).
    pub(crate) fn queue_farms(
        &mut self,
        p: usize,
        count: i32,
        master: i16,
    ) -> Result<bool, Unsupported> {
        self.buy_ahead(p, count, master, Queue::Farms)
    }

    /// `FUN_141616540`: player `p` buys `count` fish traps (199) for its dock's queue
    /// ([`Turns::buy_ahead`]).
    pub(crate) fn queue_fish_traps(&mut self, p: usize, count: i32) -> Result<bool, Unsupported> {
        self.buy_ahead(p, count, FISH_TRAP, Queue::FishTraps)
    }

    /// `FUN_141630f10`: player `p` takes back `count` of master `master` from its mill's queue
    /// ([`Turns::give_back`]).
    pub(crate) fn unqueue_farms(
        &mut self,
        p: usize,
        count: i32,
        master: i16,
    ) -> Result<bool, Unsupported> {
        self.give_back(p, count, master, Queue::Farms)
    }

    /// `FUN_1416308f0`: player `p` takes back `count` fish traps from its dock's queue
    /// ([`Turns::give_back`]).
    pub(crate) fn unqueue_fish_traps(&mut self, p: usize, count: i32) -> Result<bool, Unsupported> {
        self.give_back(p, count, FISH_TRAP, Queue::FishTraps)
    }

    /// The purchase of `FUN_141617130` and `FUN_141616540`: with `count` above 0 and the queue
    /// below [`QUEUE_LIMIT`], from `count` down to 1 the first number the player can pay
    /// without the population's test ([`cost_test`]: a master it has of type 70 or more, an
    /// annex's costs its head's, each entry with an attribute at most what the attribute holds,
    /// `COMISS`, `JA`: a NaN passes) is paid (`FUN_14162fb60` with 1, [`pay`]) and joins the
    /// queue, which may then pass the limit. False when none can, or the payment failed.
    fn buy_ahead(
        &mut self,
        p: usize,
        count: i32,
        master: i16,
        queue: Queue,
    ) -> Result<bool, Unsupported> {
        if count < 1 || QUEUE_LIMIT <= self.queued(p, queue) {
            return Ok(false);
        }
        let Some(masters) = self.masters.get(p) else {
            return Ok(false);
        };
        let player = &mut self.players[p];
        let mut n = count;
        loop {
            if cost_test(masters, master, n as f32, false, &*player)? {
                if !pay(masters, master, n as f32, &mut *player)? {
                    return Ok(false);
                }
                match queue {
                    Queue::Farms => player.farm_queue += n,
                    Queue::FishTraps => player.fish_trap_queue += n,
                }
                return Ok(true);
            }
            if n <= 1 {
                return Ok(false);
            }
            n -= 1;
        }
    }

    /// The return of `FUN_141630f10` and `FUN_1416308f0`: at most the queue's count; with one
    /// or more the queue loses them and the player gets their costs back (`FUN_141630770`,
    /// [`refund_master`]: each entry of a master of type 70 or more with an attribute and the
    /// paid flag, its amount times the count). False with none.
    fn give_back(
        &mut self,
        p: usize,
        count: i32,
        master: i16,
        queue: Queue,
    ) -> Result<bool, Unsupported> {
        let n = count.min(self.queued(p, queue));
        if n < 1 {
            return Ok(false);
        }
        let player = &mut self.players[p];
        match queue {
            Queue::Farms => player.farm_queue -= n,
            Queue::FishTraps => player.fish_trap_queue -= n,
        }
        // the game reads the master without a test: a queue of a master the player lacks
        // cannot have been paid for
        if let Some(m) = self.masters.get(p).and_then(|ms| ms.get(master)) {
            refund_master(m, n, player)?;
        }
        Ok(true)
    }

    /// Player `p`'s queue `queue`.
    fn queued(&self, p: usize, queue: Queue) -> i32 {
        match queue {
            Queue::Farms => self.players[p].farm_queue,
            Queue::FishTraps => self.players[p].fish_trap_queue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters, master_type};
    use crate::turn::Player;

    /// A master of type 70 costing `cost` (attribute, amount, paid).
    fn master(id: i16, cost: Vec<(i16, i16, bool)>) -> Master {
        Master {
            id,
            kind: master_type::COMBAT,
            cost,
            ..Master::default()
        }
    }

    /// Gaia and player 1 holding `wood`, with the farm (60 wood), the pasture (100 wood, its
    /// food entry unpaid) and the fish trap (100 wood).
    fn world(wood: f32) -> Turns {
        let player = Player {
            attributes: vec![0.0; 300],
            resources: [0.0, wood, 0.0, 0.0],
            ..Player::default()
        };
        let mut t = Turns::new(0, 1, 0, vec![0; 2], vec![Player::default(), player]);
        let masters = Masters::new([
            master(FARM, vec![(1, 60, true)]),
            master(PASTURE, vec![(1, 100, true), (0, 50, false)]),
            master(FISH_TRAP, vec![(1, 100, true)]),
        ]);
        t.masters = vec![Masters::new([]), masters];
        t
    }

    /// A purchase pays for the most of `count` the player can, then queues them.
    #[test]
    fn the_mill_s_queue_buys_what_the_player_can_pay() {
        let mut t = world(250.0);
        assert!(t.queue_farms(1, 5, FARM).unwrap());
        assert_eq!(t.players[1].farm_queue, 4);
        assert_eq!(t.players[1].resources[1], 10.0);
        assert!(!t.queue_farms(1, 1, FARM).unwrap());
        assert!(!t.queue_farms(1, 0, FARM).unwrap());
        assert_eq!(t.players[1].farm_queue, 4);
    }

    /// The pasture's test asks for its unpaid entry too, its payment takes only its paid one;
    /// the fish traps have their own queue.
    #[test]
    fn pastures_and_fish_traps_pay_their_paid_entries() {
        let mut t = world(1000.0);
        t.players[1].resources[0] = 99.0;
        assert!(t.queue_farms(1, 2, PASTURE).unwrap());
        assert_eq!(t.players[1].farm_queue, 1);
        assert_eq!(t.players[1].resources, [99.0, 900.0, 0.0, 0.0]);
        t.players[1].resources[0] = 100.0;
        assert!(t.queue_farms(1, 2, PASTURE).unwrap());
        assert_eq!(t.players[1].resources, [100.0, 700.0, 0.0, 0.0]);
        t.players[1].farm_queue = 2;
        assert!(t.queue_fish_traps(1, 3).unwrap());
        assert_eq!(t.players[1].fish_trap_queue, 3);
        assert_eq!(t.players[1].farm_queue, 2);
        assert_eq!(t.players[1].resources[1], 400.0);
    }

    /// A queue at 40 takes no purchase; one below may pass 40.
    #[test]
    fn a_full_queue_takes_no_purchase() {
        let mut t = world(10_000.0);
        t.players[1].farm_queue = 40;
        assert!(!t.queue_farms(1, 1, FARM).unwrap());
        t.players[1].farm_queue = 39;
        assert!(t.queue_farms(1, 3, FARM).unwrap());
        assert_eq!(t.players[1].farm_queue, 42);
    }

    /// A master the player lacks buys nothing.
    #[test]
    fn a_master_the_player_lacks_buys_nothing() {
        let mut t = world(1000.0);
        assert!(!t.queue_farms(1, 2, 1187).unwrap());
        assert_eq!(t.players[1].resources[1], 1000.0);
    }

    /// A return gives back at most the queue's count, each its paid costs.
    #[test]
    fn a_return_refunds_at_most_the_queue() {
        let mut t = world(0.0);
        t.players[1].farm_queue = 3;
        assert!(t.unqueue_farms(1, 5, FARM).unwrap());
        assert_eq!(t.players[1].farm_queue, 0);
        assert_eq!(t.players[1].resources[1], 180.0);
        assert!(!t.unqueue_farms(1, 1, FARM).unwrap());
        t.players[1].farm_queue = 2;
        assert!(t.unqueue_farms(1, 1, PASTURE).unwrap());
        assert_eq!(t.players[1].resources, [0.0, 280.0, 0.0, 0.0]);
        t.players[1].fish_trap_queue = 1;
        assert!(!t.unqueue_fish_traps(1, 0).unwrap());
        assert!(t.unqueue_fish_traps(1, 2).unwrap());
        assert_eq!(t.players[1].fish_trap_queue, 0);
        assert_eq!(t.players[1].resources[1], 380.0);
    }
}
