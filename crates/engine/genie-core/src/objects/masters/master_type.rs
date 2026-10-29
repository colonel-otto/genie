//! The master types (`+0x8`, the object's kind at `+0xf`): the one definition of the type bytes. Every
//! routine that tests a type (a threshold `kind >= COMBAT`, `kind < ACTION`) names it here. The
//! attribute setters (slots 0x18, 0x20, 0x28) differ by type.

/// `RGE_Master_Static_Object`.
pub const STATIC: u8 = 10;
/// `TRIBE_Master_Tree_Object`, with the static object's setters.
pub const TREE: u8 = 15;
/// `RGE_Master_Animated_Object`.
pub const ANIMATED: u8 = 20;
/// `RGE_Master_Doppleganger_Object`, with the animated object's setters; the type of a ghost
/// ([`crate::vision::ghost`]).
pub const DOPPELGANGER: u8 = 25;
/// `RGE_Master_Moving_Object`.
pub const MOVING: u8 = 30;
/// `RGE_Master_Action_Object`.
pub const ACTION: u8 = 40;
/// `RGE_Master_Combat_Object`.
pub const RGE_COMBAT: u8 = 50;
/// `RGE_Master_Missile_Object` (`'<'`).
pub const MISSILE: u8 = 60;
/// `TRIBE_Master_Combat_Object` (`'F'`).
pub const COMBAT: u8 = 70;
/// `TRIBE_Master_Building_Object` (`'P'`).
pub const BUILDING: u8 = 80;
