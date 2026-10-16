//! gsk, the genie swiss knife. This release only reserves the crate name: nothing works yet.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!(
        "gsk {}: nothing is implemented yet. Plans and progress: https://github.com/librematch/genie",
        env!("CARGO_PKG_VERSION")
    );
    ExitCode::FAILURE
}
