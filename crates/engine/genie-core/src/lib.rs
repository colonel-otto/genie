//! Deterministic simulation core of the genie engine reimplementation.

// The port's exceptions to the workspace's lints: the engine reproduces build 158041 bit for bit,
// so it keeps the game's arithmetic and conversions as they are.
#![allow(
    // the game's f32 sums and products, each rounded on its own (no fused multiply-add)
    clippy::suboptimal_flops,
    clippy::imprecise_flops,
    // the game compares floats exactly, in conditions and loops
    clippy::float_cmp,
    clippy::while_float,
    // the game's conversions (cvttss2si, movsx, movzx, truncating stores), written as `as`
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    // the decompiled routines' math names (dx, dy, x0, y1 …), formulas, flag fields and
    // parameters, assignment order, and member functions whose `this` goes unused
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::suspicious_operation_groupings,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools,
    clippy::useless_let_if_seq,
    clippy::unused_self,
    // the contexts are bundles of references, passed by value
    clippy::needless_pass_by_value,
    // a routine is ported whole, its documentation opens with the routine and its addresses
    clippy::too_many_lines,
    clippy::too_long_first_doc_paragraph,
    // every error is a `GenieError`, which names the part the slice does not model
    clippy::missing_errors_doc
)]

pub use genie_error::{ErrorKind, GenieError, GenieResult};

pub mod ai_kinds;
pub mod best_target;
pub mod checkpoint;
pub mod combat;
pub mod construction;
pub mod delete;
pub mod facing;
pub mod garrison;
pub mod gathering;
pub mod group;
pub mod lifecycle;
pub mod lines;
pub mod masters;
pub mod math;
pub mod movement;
pub mod order;
pub mod ownership;
pub mod pathing;
pub mod players;
pub mod production;
pub mod settings;
pub mod spatial;
pub mod sprite;
pub mod start_work;
pub mod tasks;
pub mod technology;
pub mod terrain;
pub mod town_bell;
pub mod trace;
pub mod turn;
pub mod unit_ai;
pub mod vision;
pub mod wall;
