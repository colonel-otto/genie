//! A player section: the player list's load of one player, generated from the game's load code.

use super::generated::player::{Object, Player, PlayerObjects, WorldPlayer};
use binrw::BinRead;
use binrw::io::{Read, Seek};

/// Where a player section starts and ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerEnds {
    pub start: usize,
    pub end: usize,
}

/// One player's section: the player list's load of one player (`WorldPlayerFactory.cpp`): the
/// class byte, the class's load (`WorldPlayer.cpp` and the class's own fields), then the masters,
/// maps and objects (WorldPlayer vtable slot 29).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSection {
    pub ends: PlayerEnds,
    pub player: Player,
}

impl PlayerSection {
    /// The player load of the section's class.
    pub fn world_player(&self) -> &WorldPlayer {
        self.player
            .world_player()
            .expect("read_player accepts only a type with a class")
    }

    /// The masters, maps and objects.
    pub fn objects(&self) -> &PlayerObjects {
        self.player
            .objects()
            .expect("read_player accepts only a type with a class")
    }

    /// The live objects (the list the checkpoint sums over).
    pub fn live_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects().objects.objects()
    }

    /// The sleeping objects.
    pub fn sleeping_objects(&self) -> impl Iterator<Item = &Object> {
        self.objects().sleeping_objects.objects()
    }
}

/// Why a player section could not be read.
#[derive(Debug)]
pub enum ReadError {
    Bin(binrw::Error),
    /// A part the generated reader does not read yet (`Unsupported` in the generated layouts).
    Unsupported {
        what: &'static str,
        at: usize,
    },
}

impl From<binrw::Error> for ReadError {
    fn from(e: binrw::Error) -> Self {
        ReadError::Bin(e)
    }
}

fn position<R: Seek>(r: &mut R) -> Result<usize, ReadError> {
    Ok(r.stream_position().map_err(binrw::Error::Io)? as usize)
}

/// Read one player section. The next section starts right after it. aoe2rec's pattern instead
/// skips Gaia's tail and searches for the next player.
pub fn read_player<R: Read + Seek>(
    r: &mut R,
    ver: u32,
    num_players: usize,
) -> Result<PlayerSection, ReadError> {
    let start = position(r)?;
    let player = Player::read_args(r, (ver, num_players)).map_err(|e| {
        match super::layout_types::unsupported_in(&e) {
            Some((what, at)) => ReadError::Unsupported { what, at },
            None => ReadError::Bin(e),
        }
    })?;
    if player.world_player().is_none() {
        // the game's WorldPlayerFactory.cpp: "InvalidPlayerType: %d"
        return Err(ReadError::Bin(binrw::Error::AssertFail {
            pos: start as u64,
            message: format!("InvalidPlayerType: {}", player.type_),
        }));
    }
    Ok(PlayerSection {
        ends: PlayerEnds {
            start,
            end: position(r)?,
        },
        player,
    })
}
