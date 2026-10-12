//! The Inline Picker in a terminal you can actually check.
//!
//! The picker is the one surface every command opens, so it is where the suite
//! starts: if a seeded Connection does not appear on a real screen and Escape
//! does not leave the way a cancelled command leaves, nothing above this is
//! worth testing.

use sshm_e2e::fixture::{connection, sshm_binary};
use sshm_e2e::{Fixture, Pty, DEFAULT_TIMEOUT};

/// The frame draws in a real terminal, waits, and hands the terminal back.
///
/// Four facts, none of which any in-crate test can see because all four live
/// behind the seam those tests drive through: the frame only draws when it has
/// a terminal to draw on; the Connection the store holds reaches the grid; a
/// keystroke sent as bytes reaches the frame rather than the line discipline;
/// and the terminal is cooked again once the frame is gone — the failure a user
/// cannot undo without leaving their shell.
#[test]
fn a_seeded_connection_renders_in_a_real_terminal_and_escape_hands_the_terminal_back() {
    let fixture = Fixture::new()
        .with_connections(&[connection("web-01", "10.0.0.7", "deploy")])
        .expect("seed");

    let mut pty = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");

    // The frame is up: the Connection it was seeded with is on the grid.
    pty.wait_for_text("web-01")
        .expect("the picker draws the store");
    // …and the terminal is the frame's while it is: echoing and line-buffering
    // would mean keystrokes going to the shell instead of to the frame.
    assert!(
        !pty.termios_is_cooked().expect("termios of the pty"),
        "the frame is open but the terminal is still cooked:\n{}",
        pty.grid()
    );

    // Escape, as the keyboard sends it.
    pty.key(b"\x1b");
    let status = pty.wait_exit(DEFAULT_TIMEOUT).expect("the frame closes");
    status.assert_code(130);

    assert!(
        pty.termios_is_cooked().expect("termios of the pty"),
        "the terminal was left out of cooked mode for whatever runs next"
    );
}

/// A `Host` stanza in `~/.ssh/config` is on the picker with no import step.
///
/// This is the whole point of the live merge, seen from outside: the user edits
/// their own config in their own editor, opens the picker, and the host is
/// there — tagged so it is legible as a config row, and showing the target the
/// stanza names. The store file is checked too, because liveness must not be a
/// copy: looking at the list has to leave `connections.json` exactly as it was.
#[test]
fn a_host_in_the_ssh_config_is_on_the_picker_without_any_import() {
    let fixture = Fixture::new()
        .with_ssh_config("Host web-01\n  HostName 10.0.0.1\n  User deploy\n")
        .expect("seed");

    let mut pty = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");

    pty.wait_for_text("[ssh-config] web-01")
        .expect("the stanza is listed, tagged as a config row");
    let grid = pty.grid();
    assert!(
        grid.contains("(deploy@10.0.0.1:22)"),
        "the row shows the target the stanza describes\n{grid}"
    );
    assert!(
        !grid.contains("Import"),
        "the picker is bound to a shell key, so it asks for nothing\n{grid}"
    );

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT).expect("the frame closes");

    assert!(
        !fixture.store_exists(),
        "listing a config host must not write it into the store"
    );
}
