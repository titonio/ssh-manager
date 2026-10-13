//! Adopting a Config Host: the one thing Enter does to a row sshm cannot change.
//!
//! A row merged from `~/.ssh/config` is the user's own stanza. `Ctrl+X` and
//! `Ctrl+E` refuse over it, and Enter — the key that means *this one* — asks
//! whether to copy it into sshm's store. That ask is the whole of this file, and
//! it is worth an end-to-end test rather than a unit one because the interesting
//! part is not the state transition: it is that a keystroke on a row sshm only
//! ever *reads* ends up writing `connections.json`, and that the row it was made
//! from then leaves the list. Both halves are facts about a file and a screen,
//! and only the real binary produces both at once.
//!
//! The store is read back from disk every time. The `◇ adopted` note is the
//! frame's claim about the write; the file is the write.

use sshm_e2e::fixture::{sshm_binary, Fixture};
use sshm_e2e::{Pty, DEFAULT_TIMEOUT};

/// One usable stanza and nothing else: a machine where `ssh build-box` already
/// works and sshm has never been told about it.
const CONFIG: &str = "Host build-box\n  HostName build.example.com\n  User ci\n  Port 2222\n";

/// `sshm manage` over that config, with the cursor already on the config row —
/// it is the only row there is.
fn manage_over_a_config_host() -> (Fixture, Pty) {
    let fixture = Fixture::new()
        .with_ssh_config(CONFIG)
        .expect("the config is written");
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("the list is drawn");
    (fixture, pty)
}

/// Enter over a config row asks, and the ask alone writes nothing.
///
/// The ask is the point of the keystroke, so it is asserted as the frame's own
/// words. And a question is not an answer: backing out of it must leave the
/// store exactly as absent as it was, which is the same promise the delete
/// confirm's `N` makes about a file it is not touching.
#[test]
fn enter_on_a_config_row_asks_and_writes_nothing_on_its_own() {
    let (fixture, mut pty) = manage_over_a_config_host();
    pty.wait_for_text("[ssh-config] build-box")
        .expect("the stanza is on the list");

    pty.key(b"\r"); // Enter over the only row
    pty.wait_for_text("Adopt build-box into sshm's store? (y/N)")
        .expect("Enter asks about the row instead of picking it");

    assert!(
        !fixture.store_exists(),
        "the ask is a question: nothing has been written yet"
    );

    pty.key(b"\x1b"); // Esc backs out of the ask
    pty.wait_for_text("Manage Connections")
        .expect("the frame is the list again");
    assert!(
        !fixture.store_exists(),
        "a declined ask writes nothing, so the store is still not there"
    );
    assert!(
        pty.grid().contains("[ssh-config] build-box"),
        "and the row the ask was about is untouched"
    );

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}

/// Answering `y` writes the copy, says so, and retires the row it was copied
/// from — all in the same keystroke.
///
/// Three claims, each checked against the thing that can prove it. The note is
/// checked on the screen because it is what the user reads. The store is read
/// back from disk because `◇ adopted` is a claim about that file and a note that
/// lies is exactly the bug this suite exists to catch. And the `[ssh-config]` tag
/// is gone because the copy matches its stanza on host and user, so the list
/// would otherwise be showing the same box twice — a duplicate the user just
/// resolved, sitting under a note saying they resolved it.
#[test]
fn answering_y_writes_the_copy_and_retires_the_row_it_came_from() {
    let (fixture, mut pty) = manage_over_a_config_host();
    pty.wait_for_text("[ssh-config] build-box")
        .expect("the stanza is on the list");

    pty.key(b"\r");
    pty.wait_for_text("Adopt build-box into sshm's store? (y/N)")
        .expect("the ask");

    pty.key(b"y");
    // Wait for the frame to have settled into its after shape, rather than for
    // the first byte of it. The repaint is a row diff: the note row and the
    // list row are written separately, so a grid read the moment the note
    // lands can still be showing the row the copy retired.
    pty.wait_until(
        "the adopted row and the note about it",
        DEFAULT_TIMEOUT,
        |screen| {
            let text = screen.contents();
            text.contains("\u{25c7} adopted build-box") && !text.contains("[ssh-config]")
        },
    )
    .expect("the frame reports the write and drops the row the copy replaced");

    let aliases = fixture.stored_aliases().expect("the store is readable");
    assert_eq!(
        aliases,
        vec!["build-box".to_string()],
        "the copy is in the store, and it is the only thing there"
    );

    let grid = pty.grid();
    assert!(
        !grid.contains("[ssh-config]"),
        "the config row the copy hides is gone from the list:\n{grid}"
    );
    assert!(
        grid.contains("build-box (ci@build.example.com:2222)"),
        "the stored copy is on the list, naming the same target:\n{grid}"
    );
    assert!(
        grid.contains("\u{25c7} adopted build-box"),
        "the note is still on the glass above the row it names:\n{grid}"
    );

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}

/// A stored row's Enter still means what it always meant.
///
/// The adopt ask is what Enter means over a *config* row, and nothing else:
/// over a Connection sshm already owns, Enter closes the frame with that
/// Connection. This is the half of the change that could have quietly eaten the
/// pick path, and it is the reason the two rows are tested in one file — the
/// difference between them is the whole design.
#[test]
fn enter_over_a_stored_row_still_closes_the_frame_with_it() {
    let fixture = Fixture::new()
        .with_ssh_config(CONFIG)
        .expect("the config is written");
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("the list is drawn");
    pty.wait_for_text("[ssh-config] build-box")
        .expect("the stanza is on the list");

    // Adopt it first, so the row under the cursor is a stored one.
    pty.key(b"\r");
    pty.wait_for_text("Adopt build-box into sshm's store? (y/N)")
        .expect("the ask");
    pty.key(b"y");
    pty.wait_for_text("\u{25c7} adopted build-box")
        .expect("the write landed");

    pty.key(b"\r"); // Enter over the stored row: the pick, as ever
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("Enter closes the frame on the row under the cursor")
        .assert_code(0);

    assert!(
        !pty.grid().contains("Adopt build-box"),
        "no second ask was opened over a row that is already sshm's own:\n{}",
        pty.grid()
    );
}
