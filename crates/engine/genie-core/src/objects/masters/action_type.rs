//! The task action types (`Task::action_type`, `i16`) that more than one routine names: the
//! action a task makes a unit do. The action types only one routine names stay beside it (the
//! Gather's 5, the Attack's 9, the Make's 21).

/// `TRIBE_Action_Build`'s type (101, `0x65`): the task a builder works by.
pub const BUILD: i16 = 101;
/// `TRIBE_Action_Trade`'s type (111, `0x6f`): the task of the trade carts and ships, the task whose carrying sprite a trade cart with goods shows
/// (`FUN_1412028a0`, `object_id` 0x54), and whose load keeps a stopped unit's AI going
/// (`FUN_14106a940`).
pub const TRADE: i16 = 0x6f;
/// `TRIBE_Action_Pack`'s type (124, `0x7c`): the Gather's drop-site searches pass over it with [`UNPACK`]
/// (`FUN_1411671e0`).
pub const PACK: i16 = 0x7c;
/// `TRIBE_Action_Unpack`'s type (125, `0x7d`): the task of the packed siege units (master type
/// 80, class 51: the trebuchet and the mangonel), whose target a gather drops
/// (`FUN_140a903a0`). A building whose top action is an Unpack takes a hunt sent to it to
/// `set_state` 15 (a farm's top action is `TRIBE_Action_Farm`, 121).
pub const UNPACK: i16 = 0x7d;
