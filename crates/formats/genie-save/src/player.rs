//! A player section: the player list's load of one player, generated from the game's load code.

use super::generated::world::{Object, Player, PlayerObjects, WorldPlayer};

/// Where a player section starts and ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerEnds {
    /// The section's first byte.
    pub start: usize,
    /// The byte after its last.
    pub end: usize,
}

/// One player's section of the save.
///
/// The player list's load of one player (`WorldPlayerFactory.cpp`): the class byte, the class's
/// load (`WorldPlayer.cpp` and the class's own fields), then the masters, maps and objects
/// (`WorldPlayer` vtable slot 29).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSection<'a> {
    /// Where it starts.
    pub start: usize,
    /// The player as read.
    pub player: &'a Player,
}

impl<'a> PlayerSection<'a> {
    /// Where it starts and ends: its end is as many bytes on as it writes back, which takes a
    /// write of the whole section.
    #[must_use]
    pub fn ends(&self) -> PlayerEnds {
        PlayerEnds {
            start: self.start,
            end: self.start + super::written_len(self.player),
        }
    }

    /// The player load of the section's class.
    ///
    /// # Panics
    ///
    /// Never for a parsed save: the reader accepts only a player type with a class.
    #[must_use]
    pub fn world_player(&self) -> &'a WorldPlayer {
        self.player
            .world_player()
            .expect("the save reader accepts only a type with a class")
    }

    /// The masters, maps and objects.
    ///
    /// # Panics
    ///
    /// Never for a parsed save: the reader accepts only a player type with a class.
    #[must_use]
    pub fn objects(&self) -> &'a PlayerObjects {
        self.player
            .objects()
            .expect("the save reader accepts only a type with a class")
    }

    /// The live objects (the list the checkpoint sums over).
    pub fn live_objects(&self) -> impl Iterator<Item = &'a Object> {
        self.objects().objects.objects()
    }

    /// The sleeping objects.
    pub fn sleeping_objects(&self) -> impl Iterator<Item = &'a Object> {
        self.objects().sleeping_objects.objects()
    }
}
