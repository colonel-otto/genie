//! `GenieSwissKnife` Config
//!
//! See instructions in `commands.rs` to specify the path to your
//! application's configuration file and/or command-line options
//! for specifying it.

// serde's derive declares an enum of the struct's fields, which has no variant while the struct
// has no field and denies the unknown ones. That enum is an item beside the struct, so an allow
// on the struct does not reach it: the module holds the struct alone.
#![allow(clippy::empty_enums)]

use serde::{Deserialize, Serialize};

/// `GenieSwissKnife` Configuration. Empty until a subcommand needs a setting.
// No `Copy`: the settings to come (paths, names) are not `Copy`.
#[allow(missing_copy_implementations)]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenieSwissKnifeConfig {}
