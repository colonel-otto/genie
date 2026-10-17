//! A player section: the player list's load of one player, generated from the game's load code.

use super::generated::world::{Object, Player, PlayerObjects, WorldPlayer};

/// Where a player section starts and ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerEnds {
    pub start: usize,
    pub end: usize,
}

/// One player's section: the player list's load of one player (`WorldPlayerFactory.cpp`): the
/// class byte, the class's load (`WorldPlayer.cpp` and the class's own fields), then the masters,
/// maps and objects (WorldPlayer vtable slot 29).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerSection<'a> {
    pub ends: PlayerEnds,
    pub player: &'a Player,
}

impl<'a> PlayerSection<'a> {
    /// The player load of the section's class.
    pub fn world_player(&self) -> &'a WorldPlayer {
        self.player
            .world_player()
            .expect("the save reader accepts only a type with a class")
    }

    /// The masters, maps and objects.
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
