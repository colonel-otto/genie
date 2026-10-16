//! The placeholder must not pretend to work: `gsk` exits with an error and says why.

use std::process::Command;

#[test]
fn the_placeholder_exits_with_an_error_and_says_so() {
    let output = Command::new(env!("CARGO_BIN_EXE_gsk"))
        .output()
        .expect("run gsk");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "gsk exited with success; stderr: {stderr}"
    );
    assert!(
        stderr.contains("nothing is implemented yet"),
        "stderr: {stderr}"
    );
}
