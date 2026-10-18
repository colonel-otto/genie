//! `GenieSwissKnife` Config
//!
//! See instructions in `commands.rs` to specify the path to your
//! application's configuration file and/or command-line options
//! for specifying it.

use serde::{Deserialize, Serialize};

/// `GenieSwissKnife` Configuration. Empty until a subcommand needs a setting.
// No `Copy`: the settings to come (paths, names) are not `Copy`.
#[allow(missing_copy_implementations)]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GenieSwissKnifeConfig {}
