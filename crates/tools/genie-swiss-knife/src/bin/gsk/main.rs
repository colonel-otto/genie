//! Main entry point for GenieSwissKnife

// No `deny(warnings)`: a new compiler warning would break `cargo install`. CI denies them.
#![deny(missing_docs, trivial_casts, unused_qualifications)]
#![forbid(unsafe_code)]

use genie_swiss_knife::application::APP;

/// Boot GenieSwissKnife
fn main() {
    abscissa_core::boot(&APP);
}
