//! A recorded command the slice does not run: it keeps its packet and why.

use crate::GenieError;

/// A command the slice runs part of or none, for a reason the packet's reader names: its run
/// fails with the reason, which stops the issuing player's lane. It keeps its packet's bytes,
/// so it writes back as it was recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnmodelledCommand {
    /// The issuing player (the packet's byte 1).
    pub player: usize,
    /// The packet.
    pub packet: Vec<u8>,
    /// Why its command is not modelled.
    pub why: GenieError,
}
