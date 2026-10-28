//! The units a command's packet lists: its count and the ids after the packet's fixed part.

use crate::world::World;

/// The units a command's packet lists: its count (a `u16`, `+0x4` in most packets) and the
/// ids that follow the packet's fixed part. The handlers read the first `count` of them, at
/// most 60 (500 for a computer player, `FUN_140e6b250`); most checks refuse a count past the
/// ids the packet holds ([`UnitList::overruns`]). The orders' reader (`FUN_140e6b290`) takes a
/// count of [`UnitList::LAST_COMMANDED`] as the comm player's last commanded units.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitList {
    /// How many units the command names.
    pub count: u16,
    /// The ids the packet holds.
    pub ids: Vec<i32>,
}

impl UnitList {
    /// The count that names the comm player's last commanded units (`UNIT_NUM_USE_LAST`).
    pub const LAST_COMMANDED: u16 = 0xffff;

    /// A list of `ids`, its count theirs.
    ///
    /// # Panics
    ///
    /// More than `u16::MAX - 1` ids, which no packet holds.
    #[must_use]
    pub fn of(ids: Vec<i32>) -> Self {
        let count = u16::try_from(ids.len())
            .ok()
            .filter(|&c| c != Self::LAST_COMMANDED)
            .expect("a command lists fewer than 0xffff units");
        Self { count, ids }
    }

    /// The comm player's last commanded units: the count `0xffff` and no ids.
    #[must_use]
    pub const fn last_commanded() -> Self {
        Self {
            count: Self::LAST_COMMANDED,
            ids: Vec::new(),
        }
    }

    /// The count names the last commanded units.
    #[must_use]
    pub const fn names_last_commanded(&self) -> bool {
        self.count == Self::LAST_COMMANDED
    }

    /// The count is past the ids the packet holds (the checks' size test, `count * 4` past the
    /// fixed part); the count `0xffff` is past them too.
    #[must_use]
    pub const fn overruns(&self) -> bool {
        self.ids.len() < self.count as usize
    }

    /// The first `count` ids, at most `most` of them.
    #[must_use]
    pub fn first(&self, most: usize) -> &[i32] {
        let n = usize::from(self.count).min(most).min(self.ids.len());
        &self.ids[..n]
    }
}

impl World {
    /// The units a list command's check and handler read (`FUN_140e6b250`): the list's first
    /// `count`, at most [`World::most_units`]. None where the check refuses the command: a
    /// player the world lacks, or a count past the ids the packet holds (the size test every
    /// list command's check makes; the orders' checks let the count `0xffff` through, which
    /// [`OrderCommand`] reads as the last commanded units).
    ///
    /// [`OrderCommand`]: crate::commands::OrderCommand
    pub(crate) fn read_units<'u>(&self, player: usize, units: &'u UnitList) -> Option<&'u [i32]> {
        (player < self.players.len() && !units.overruns())
            .then(|| units.first(self.most_units(player)))
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::{BuildCommand, Command, StopCommand, UnitList};
    use crate::world::{Player, World};

    /// Gaia, a human player and a computer player.
    fn world() -> World {
        let player = |computer| Player {
            computer,
            ..Player::default()
        };
        World::new(
            0,
            1,
            1,
            vec![0; 3],
            vec![player(false), player(false), player(true)],
        )
    }

    #[test]
    fn a_handler_reads_60_units_of_a_human_and_500_of_a_computer() {
        let w = world();
        let many = UnitList::of((0..600).collect());
        assert_eq!(w.read_units(1, &many).map(<[i32]>::len), Some(60));
        assert_eq!(w.read_units(2, &many).map(<[i32]>::len), Some(500));
    }

    #[test]
    fn a_check_refuses_a_count_past_the_ids_and_a_player_the_world_lacks() {
        let w = world();
        let short = UnitList {
            count: 2,
            ids: vec![7],
        };
        assert_eq!(w.read_units(1, &short), None);
        assert_eq!(w.read_units(1, &UnitList::last_commanded()), None);
        assert_eq!(w.read_units(3, &UnitList::of(vec![7])), None);
        assert_eq!(w.read_units(1, &UnitList::of(vec![7])), Some(&[7][..]));
    }

    #[test]
    fn a_refused_list_command_does_nothing_and_stops_no_one() {
        let mut w = world();
        let before = w.clone();
        // BUILD of the last commanded units: its count overruns the packet (do_command's 0x66)
        w.run(&Command::Build(BuildCommand {
            player: 1,
            master: 70,
            position: [1.0, 1.0],
            target: -1,
            sprite_override: -1,
            builders: UnitList::last_commanded(),
            queued: false,
            commits: true,
            frame: 0,
        }));
        w.run(&Command::Stop(StopCommand {
            player: 1,
            units: UnitList {
                count: 3,
                ids: vec![7],
            },
        }));
        assert_eq!(w, before);
    }

    #[test]
    fn a_list_reads_its_first_count_ids_at_most_the_cap() {
        let list = UnitList {
            count: 3,
            ids: vec![1, 2, 3, 4],
        };
        assert_eq!(list.first(60), &[1, 2, 3]);
        assert_eq!(list.first(2), &[1, 2]);
        assert!(!list.overruns());
    }

    #[test]
    fn a_count_past_the_ids_overruns_and_reads_what_the_packet_holds() {
        let list = UnitList {
            count: 5,
            ids: vec![1, 2],
        };
        assert!(list.overruns());
        assert_eq!(list.first(60), &[1, 2]);
        assert!(UnitList::last_commanded().overruns());
        assert!(UnitList::last_commanded().names_last_commanded());
    }

    #[test]
    fn a_list_of_ids_counts_them() {
        assert_eq!(
            UnitList::of(vec![7, 8]),
            UnitList {
                count: 2,
                ids: vec![7, 8]
            }
        );
        assert!(!UnitList::of(vec![7]).names_last_commanded());
    }
}
