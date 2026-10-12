//! The first-run offer to import `~/.ssh/config`, asked and answered.
//!
//! This is the only moment in `sshm` where the program proposes to write
//! something the user did not type, on the strength of a file it found. Two
//! things have to go right at once: the offer has to actually appear for someone
//! whose whole reason for using the tool is the config they already have, and it
//! has to be a real question — a refusal that still wrote the store would be the
//! same thing as doing it without asking.
//!
//! The refusal is checked against the filesystem rather than against the note on
//! screen, because "declined" drawn over a store that exists is exactly the lie
//! this file exists to catch.

use sshm_e2e::fixture::{sshm_binary, Fixture};
use sshm_e2e::{Pty, DEFAULT_TIMEOUT};

/// A config with two importable stanzas in it.
///
/// Two, not one: the offer states a count, and a count is only tested by a
/// number that could have come out wrong.
const CONFIG: &str = "Host web-01\n  HostName 10.0.0.1\n  User deploy\n\nHost db-01\n  HostName 10.0.0.2\n  User deploy\n";

/// A brand-new machine: no store, one `~/.ssh/config`.
fn fresh_machine() -> Fixture {
    Fixture::new()
        .with_ssh_config(CONFIG)
        .expect("the config is written")
}

/// A first run with something to import opens on the offer, not on an empty list.
///
/// The offer is the first screen a new user sees, so it has to say what was
/// found, where it was found, and that the default is no — the count and the
/// path are the difference between "I recognised your file" and a prompt that
/// could be about anything.
#[test]
fn a_first_run_with_something_to_import_opens_on_the_offer() {
    let fixture = fresh_machine();
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("the offer is drawn");

    let grid = pty.grid();
    assert!(
        grid.contains("Import 2 connections from ~/.ssh/config? (y/N)"),
        "the offer names the count, the file, and the default\n{grid}"
    );
    assert!(
        grid.contains("y confirm") && grid.contains("N abort"),
        "the hint rail says how to answer\n{grid}"
    );
}

/// Answering yes imports them, and they are there afterwards.
///
/// Both halves: the frame has to show what it made, and the store has to hold
/// it. A note that says `imported 2 connections` over a store with one Connection
/// in it is the failure this test is for, which is why the count is checked
/// against the file and not against the note.
#[test]
fn accepting_the_offer_imports_and_renders_the_connections() {
    let fixture = fresh_machine();
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("Import 2 connections")
        .expect("the offer is asked");

    pty.key(b"y");
    pty.wait_for_text("\u{25c7} imported 2 connections")
        .expect("the frame reports the import");

    let aliases = fixture.stored_aliases().expect("the store is readable");
    assert_eq!(
        aliases,
        vec!["web-01".to_string(), "db-01".to_string()],
        "the store holds what the config held"
    );
    pty.wait_until("both rows in the list", DEFAULT_TIMEOUT, |screen| {
        let text = screen.contents();
        text.contains("web-01 (deploy@10.0.0.1:22)") && text.contains("db-01 (deploy@10.0.0.2:22)")
    })
    .expect("the imported Connections are on the screen the user is looking at");
}

/// Refusing with Escape writes nothing at all.
///
/// Not "writes nothing to the store" — nothing. The store file is the whole
/// evidence of what the frame did, and its absence is the only proof that a
/// refusal was treated as a refusal. The note that appears instead is what tells
/// the user the empty list underneath is their choice and not a bug.
#[test]
fn declining_the_offer_writes_nothing() {
    let fixture = fresh_machine();
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("Import 2 connections")
        .expect("the offer is asked");

    pty.key(b"\x1b");
    pty.wait_for_text("import declined")
        .expect("the refusal is recorded on screen");

    assert!(
        !fixture.store_exists(),
        "a declined import must not leave a store behind"
    );
}

/// The picker never asks.
///
/// `sshm pick` is the keystroke bound to a shell key, often in a binding that
/// replaces the command line: an offer there would be a prompt in the middle of
/// a command the user was typing, and the answer would be whatever the next
/// keystroke happened to be. The empty list is the right screen for a pick with
/// nothing in it, and nothing gets written on the way out.
#[test]
fn the_picker_never_asks_to_import() {
    let fixture = fresh_machine();
    let mut pty = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");
    pty.wait_for_frame().expect("the picker is drawn");

    let grid = pty.grid();
    assert!(
        !grid.contains("Import"),
        "the picker must not ask anything\n{grid}"
    );
    assert!(
        grid.contains("No Connections yet"),
        "and it says why the list is empty\n{grid}"
    );

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the picker closes")
        .assert_code(130);
    assert!(
        !fixture.store_exists(),
        "and looking at an empty picker writes nothing"
    );
}
