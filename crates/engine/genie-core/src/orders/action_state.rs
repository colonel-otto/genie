//! The action states every action type shares (the state byte `+0x18`, set by `set_state`): 1
//! done and 13 failed. The other states mean something else in each action type (state 2 is idle
//! in a Transport, halted in a Repair, the target's death in an Attack) and stay with their action.

/// State 1: the action is done. Its update returns 1 and the action list removes it (a missile
/// ends, a Make reports its unit, an Enter has entered).
pub const DONE: u8 = 1;
/// State 13 (`0xd`, "Failed"): the action failed (the target went out of sight, the transport
/// cannot reach the unit). Its next update tells its unit AI and ends or halts it.
pub const FAILED: u8 = 13;
