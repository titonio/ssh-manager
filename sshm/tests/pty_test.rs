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
//! It used to be driven by a Python script because the pty and `termios` calls
//! were a dozen lines there and a dependency here. They are a dependency now,
//! in `sshm-e2e`, which is the harness the end-to-end suite is built on — so
//! the four checks below are made with the same primitives as every E2E test,
//! and there is one harness language in the repo instead of two.
//!
//! This test stays inside the `sshm` crate rather than moving to `sshm-e2e`
//! along with the E2E suite, and the reason is the mutation sweep: these four
//! checks are the only ones that reach the raw-mode and restore paths, and a
//! mutant that deletes them has to be caught by the sweep that runs the crate's
//! own tests.

use std::time::Duration;

use sshm_e2e::fixture::connection;
use sshm_e2e::{Fixture, Pty};

/// How long to wait, at most, for each stage.
///
/// Generous for a loaded CI runner, and short enough that the whole run
/// finishes inside the ~20 s cargo-mutants gives a test: a mutant that wedges
/// the binary should be reported by the check that caught it, not by a timeout
/// that says nothing.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(6);

/// The frame takes the terminal for as long as it is open and hands it back
/// exactly as it found it.
///
/// Four things at once, all of them facts about a terminal: the frame draws and
/// waits rather than drawing and leaving; the terminal is out of cooked mode
/// while it does, so a keystroke reaches the frame instead of the shell; Escape
/// ends it the way a cancelled command ends; and when the frame is gone the
/// terminal echoes and line-buffers again, because the next command in this
/// shell did not ask to inherit a raw terminal. A terminal left raw is the
/// failure the user cannot undo without leaving the shell.
#[test]
fn the_frame_takes_the_terminal_and_hands_it_back() {
    let fixture = Fixture::new()
        .with_connections(&[connection("web-01", "10.0.0.7", "deploy")])
        .expect("a seeded store");
    let mut pty = Pty::spawn(env!("CARGO_BIN_EXE_sshm"), &["pick"], &fixture)
        .expect("the binary runs under a pty");

    // One and two: the frame drew, and it is still here. A frame that printed
    // and exited would satisfy the first half of this and fail the second.
    pty.wait_until(
        "the picker drawing the seeded Connection",
        OPEN_TIMEOUT,
        |screen| screen.contents().contains("web-01"),
    )
    .expect("the frame draws");
    assert!(
        pty.pid().is_some_and(still_running),
        "the frame drew and left without waiting for an answer\n{}",
        pty.grid()
    );

    // Three: the terminal itself says it is raw. `tcgetattr` on the master
    // answers for the slave on Linux, so this is what the terminal is running
    // under rather than what the program claims to have done.
    assert!(
        !pty.termios_is_cooked().expect("termios of the pty"),
        "the frame is open but the terminal is still cooked: keys are being \
         echoed and buffered by the line discipline\n{}",
        pty.grid()
    );

    // Escape, as the keyboard sends it. The frame is open and the terminal is
    // ours, so the keystroke can reach it.
    pty.key(b"\x1b");
    pty.wait_exit(EXIT_TIMEOUT)
        .expect("the frame comes back from Escape")
        .assert_code(130);

    // Four: handed back as it was found, or left raw for whatever runs next in
    // this shell.
    assert!(
        pty.termios_is_cooked().expect("termios of the pty"),
        "the terminal was left out of cooked mode: the next command in this \
         shell inherits a terminal that does not echo"
    );
}

/// Whether a pid is still ours to lose.
///
/// `kill(pid, 0)` is the question "are you there" asked without meaning to
/// disturb anybody; `ESRCH` is the only answer that means gone.
fn still_running(pid: u32) -> bool {
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
