//! The task action types (`Task::action_type`, `i16`) that more than one routine names: the
//! action a task makes a unit do. The action types only one routine names stay beside it (the
//! Gather's 5, the Attack's 9, the Make's 21).

/// Build (`0x65`, 101).
pub const BUILD: i16 = 101;
/// Trade (`0x6f`, 111): the task whose carrying sprite a trade cart with goods shows
/// (`FUN_1412028a0`, `object_id` 0x54), and whose load keeps a stopped unit's AI going
/// (`FUN_14106a940`).
pub const TRADE: i16 = 0x6f;
/// Pack (`0x7c`): the Gather's drop-site searches pass over it with [`UNPACK`]
/// (`FUN_1411671e0`).
pub const PACK: i16 = 0x7c;
/// Unpack (`0x7d`): a building packing or unpacking, whose target a gather drops
/// (`FUN_140a903a0`); a building whose top action is this (a farm at work) takes a hunt sent to
/// it to `set_state` 15.
pub const UNPACK: i16 = 0x7d;
