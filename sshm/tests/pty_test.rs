//! The frame in a terminal you can actually check.
//!
//! Everything else in this suite drives the frame through a seam — a fake
//! `Events`, a `Vec<u8>` standing in for a terminal — which is what keeps the
//! keystroke rules and the painting testable at all. The seam is also a blind
//! spot: the real raw-mode call, the real `/dev/tty`, and the plain fact of a
//! frame sitting on screen waiting for a keystroke are all behind it, so a
//! mutant that quietly stops taking the terminal survives every test in the
//! crate. This file has no seam. It runs the built binary under a
//! pseudo-terminal and asks the terminal, not the program, what state it is in.
//!
//! The driving is in `pty_frame_check.py` because the pty and `termios` calls
//! are a dozen lines there and a dependency here.

use std::path::Path;
use std::process::Command;

/// The frame takes the terminal for as long as it is open and hands it back
/// exactly as it found it.
///
/// Three things at once, all of them facts about a terminal: the frame draws
/// and waits rather than drawing and leaving; the terminal is out of cooked
/// mode while it does, so a keystroke reaches the frame instead of the shell;
/// and when the frame is gone the terminal echoes and line-buffers again,
/// because the next command in this shell did not ask to inherit a raw
/// terminal. A terminal left raw is the failure the user cannot undo without
/// leaving the shell.
#[test]
fn the_frame_takes_the_terminal_and_hands_it_back() {
    let harness = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pty_frame_check.py");
    assert!(
        harness.exists(),
        "the pty harness belongs beside this test: {}",
        harness.display()
    );

    let output = Command::new("python3")
        .arg(&harness)
        .arg(env!("CARGO_BIN_EXE_sshm"))
        .output()
        .expect("python3 drives the pty harness");

    assert!(
        output.status.success(),
        "the terminal disagrees with the frame: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
