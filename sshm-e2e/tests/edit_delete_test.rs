//! Editing and deleting a Connection, and every way out of both.
//!
//! Two surfaces live here. The Form Map returns already filled in, which makes
//! it a different animal from the blank one: the values on screen came from the
//! store, so a save that writes the wrong row, or an abandon that wrote
//! something anyway, corrupts a Connection the user never meant to touch. The
//! delete confirm is the only place in `sshm` that destroys anything, and it has
//! three ways out — confirm, abort, back — two of which must leave the store
//! exactly as it was.
//!
//! "Left exactly as it was" is checked against the file, before and after, not
//! against the note the frame draws. The note is a claim about what happened;
//! the store is what happened.

use std::time::Duration;

use sshm_e2e::fixture::{seeded_fixture, sshm_binary};
use sshm_e2e::{Fixture, Pty};

/// `sshm manage` over two seeded Connections, cursor on the first.
fn manage() -> (Fixture, Pty) {
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("the list is drawn");
    (fixture, pty)
}

/// Wait until the frame is showing the list again, not a form or a question.
fn back_at_the_list(pty: &mut Pty) {
    pty.wait_for_text("Manage Connections")
        .expect("the frame is the list again");
}

/// The rows the frame is using, one per line.
fn rows(pty: &mut Pty) -> Vec<String> {
    pty.screen_document()
        .lines()
        .map(|line| line.to_string())
        .collect()
}

/// Which row of the frame holds `needle`.
///
/// Positions are found by content rather than counted by hand: the grid begins
/// with a blank row, and a test that hard-codes that off-by-one breaks when the
/// frame grows a row for a reason that has nothing to do with it.
fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row of {rows:?} holds {needle:?}"))
}

/// Ctrl+E opens the same map the add flow uses, already filled in.
///
/// The claim is that this is *one* surface wearing different clothes, not two
/// screens that happen to look alike: the six rows are the same rows, in the same
/// places, with the same rule and the same decision row under them. What differs
/// is that the values came from the store — `✓` where the add map has `○` and
/// `·`, and `<not set>` where it has `<optional>`, because a stored Connection
/// genuinely has no key rather than an optional field left blank.
#[test]
fn ctrl_e_opens_the_same_map_already_filled_in() {
    let (_fixture, mut pty) = manage();

    pty.key(b"\x01"); // Ctrl+A
    pty.wait_for_text("New connection")
        .expect("the blank map opens");
    pty.wait_for_text("\u{25b6} Add connection")
        .expect("the blank map is drawn");
    let blank = rows(&mut pty);

    pty.key(b"\x1b");
    back_at_the_list(&mut pty);
    pty.key(b"\x05"); // Ctrl+E
    pty.wait_for_text("Edit web-01")
        .expect("the seeded map opens");
    pty.wait_for_text("\u{25b6} Save changes to web-01")
        .expect("the seeded map is drawn");
    let seeded = rows(&mut pty);

    assert_eq!(
        blank.len(),
        seeded.len(),
        "the two maps are the same height\nblank:\n{blank:?}\nseeded:\n{seeded:?}"
    );
    for label in ["Alias", "Host", "User", "Port", "Key", "Folder"] {
        assert_eq!(
            row_of(&blank, label),
            row_of(&seeded, label),
            "{label} is on the same row of both maps"
        );
    }
    assert_eq!(
        row_of(&blank, "\u{25b6}"),
        row_of(&seeded, "\u{25b6}"),
        "the decision row sits under the fields in both maps"
    );
    let rule = row_of(&blank, "\u{2500}");
    assert_eq!(
        blank[rule], seeded[rule],
        "the rule row is the same line in both maps"
    );
    assert_eq!(
        blank[blank.len() - 1],
        seeded[seeded.len() - 1],
        "both maps say the same thing about how to leave"
    );

    // What the seed actually put on the map.
    let grid = pty.grid();
    assert!(
        grid.contains("\u{25c6} Alias  web-01"),
        "the cursor opens on Alias, holding the stored alias\n{grid}"
    );
    assert!(
        grid.contains("\u{2713} Host   10.0.0.1"),
        "a stored value is a valid one\n{grid}"
    );
    assert!(
        grid.contains("\u{b7} Key    <not set>"),
        "a Connection with no key says not set, not optional\n{grid}"
    );
}

/// Changing a field and saving rewrites that Connection and no other.
///
/// The id has to survive: it is what the row was found by, and a save that mints
/// a new one leaves the old Connection behind as a second entry rather than
/// replacing it — a duplicate that looks like a successful edit.
#[test]
fn changing_a_field_and_saving_rewrites_the_connection() {
    let (fixture, mut pty) = manage();
    let before = fixture.read_store().expect("the store before");

    pty.key(b"\x05"); // Ctrl+E
    pty.wait_for_text("Edit web-01")
        .expect("the seeded map opens");
    pty.key(b"-old");
    pty.wait_until(
        "the change showing on its row",
        Duration::from_secs(4),
        |screen| screen.contents().contains("web-01-old"),
    )
    .expect("the edit shows as it is typed");
    pty.key(b"\r");

    // A settled row the user changed wears the changed glyph, which is what
    // tells them - and the frame - that there is something worth saving. While
    // the row is still focused it wears the focus glyph instead: focus and
    // change are two different facts and the map never draws them alike.
    pty.wait_until(
        "the changed row marking itself",
        Duration::from_secs(4),
        |screen| screen.contents().contains("\u{25cf} Alias"),
    )
    .expect("a changed field is marked as changed");

    for _ in 0..6 {
        pty.key(b"\x1b[B");
    }
    pty.key(b"\r");
    pty.wait_for_text("\u{25c7} edited web-01-old")
        .expect("the frame reports the edit");

    let after = fixture.read_store().expect("the store after");
    let saved = after["connections"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|row| row["alias"] == serde_json::json!("web-01-old"))
        .expect("the edited alias is in the store");
    assert_eq!(
        saved["id"], before["connections"][0]["id"],
        "the edit rewrote the Connection it opened on"
    );
    assert_eq!(
        after["connections"].as_array().unwrap().len(),
        2,
        "an edit replaces, it does not append"
    );
}

/// Escape out of an edit leaves the store exactly as it was.
///
/// The map is full of the user's own data the whole time they are in it, and
/// leaving is the common case — a wrong key, a change of mind. A frame that
/// writes on the way out would turn every abandoned edit into a silent edit.
#[test]
fn backing_out_of_an_edit_leaves_the_store_alone() {
    let (fixture, mut pty) = manage();
    let before = fixture.read_store().expect("the store before");

    pty.key(b"\x05"); // Ctrl+E
    pty.wait_for_text("Edit web-01")
        .expect("the seeded map opens");
    pty.key(b"-gone");
    pty.wait_until(
        "the draft showing the change",
        Duration::from_secs(4),
        |screen| screen.contents().contains("web-01-gone"),
    )
    .expect("the keystrokes landed");

    pty.key(b"\x1b");
    pty.wait_for_text("edit abandoned \u{2014} nothing saved")
        .expect("the frame says what it did not do");

    assert_eq!(
        fixture.read_store().expect("the store after"),
        before,
        "an abandoned edit must not touch the store"
    );
}

/// The map's own movement keys, on a map that starts full.
///
/// Arrows move a focus that began as bytes from the terminal, Enter settles the
/// row it is on and steps to the next, and Escape backs up one row - out of the
/// map only once it is on the top one. That last clause is the interesting one:
/// "Esc back" beside "Enter next" is a pair of row movements, not a cancel, and
/// a test that assumed otherwise would have been asserting a keybinding the app
/// never claimed.
/// rows already hold values, so a focus that lands on the wrong row and gets
/// Enter'd edits a real Connection.
#[test]
fn the_map_moves_by_row_and_walks_back_out() {
    let (fixture, mut pty) = manage();
    let before = fixture.read_store().expect("the store before");

    pty.key(b"\x05"); // Ctrl+E
    pty.wait_for_text("Edit web-01")
        .expect("the seeded map opens");

    pty.key(b"\x1b[B"); // Down: Alias -> Host
    pty.wait_until("the focus on Host", Duration::from_secs(4), |screen| {
        screen.contents().contains("\u{25c6} Host")
    })
    .expect("Down moves to the next row");

    pty.key(b"\x1b[B"); // Down: Host -> User
    pty.wait_until("the focus on User", Duration::from_secs(4), |screen| {
        screen.contents().contains("\u{25c6} User")
    })
    .expect("Down moves again");

    pty.key(b"\x1b[A"); // Up: User -> Host
    pty.wait_until("the focus back on Host", Duration::from_secs(4), |screen| {
        screen.contents().contains("\u{25c6} Host")
    })
    .expect("Up moves back the other way");

    // Enter on a row the user never touched settles it unchanged and steps on.
    pty.key(b"\r");
    pty.wait_until("the focus on User", Duration::from_secs(4), |screen| {
        screen.contents().contains("\u{25c6} User")
    })
    .expect("Enter settles the row and moves to the next");

    // Escape is the escape hatch, not a second arrow: from any row it backs up
    // one, and only backs *out* from the top row. Two presses to climb back to
    // Alias, one more to leave — which is what "Esc back" beside "Enter next"
    // has to mean, and what the note on leaving then confirms.
    pty.key(b"\x1b");
    pty.wait_until("the focus back on Host", Duration::from_secs(4), |screen| {
        screen.contents().contains("\u{25c6} Host")
    })
    .expect("Escape backs up one row");
    pty.key(b"\x1b");
    pty.wait_until(
        "the focus on the top row",
        Duration::from_secs(4),
        |screen| screen.contents().contains("\u{25c6} Alias"),
    )
    .expect("Escape backs up again");

    pty.key(b"\x1b");
    back_at_the_list(&mut pty);
    assert_eq!(
        fixture.read_store().expect("the store after"),
        before,
        "walking the map and leaving it changes nothing"
    );
}

/// `N` at the delete question leaves the Connection where it is.
///
/// The capital is the point as much as the answer is: the question is `(y/N)`,
/// the default is no, and the default has to be typeable. A frame that read
/// `Shift+n` as a chord would swallow the refusal of a delete, which is the one
/// keystroke a user must never lose.
#[test]
fn declining_a_delete_leaves_the_connection_in_place() {
    let (fixture, mut pty) = manage();
    let before = fixture.read_store().expect("the store before");

    pty.key(b"\x18"); // Ctrl+X
    pty.wait_for_text("Delete web-01? (y/N)")
        .expect("the question is asked");

    pty.key(b"N");
    pty.wait_for_text("Delete web-01? No")
        .expect("the refusal is recorded on screen");

    assert!(
        pty.grid().contains("web-01 (deploy@10.0.0.1:22)"),
        "the row is still in the frame\n{}",
        pty.grid()
    );
    assert_eq!(
        fixture.read_store().expect("the store after"),
        before,
        "a declined delete must not touch the store"
    );
}

/// Escape at the delete question is the same refusal, by another key.
///
/// Two keys that must do exactly the same thing, because a user reaching for
/// Escape is refusing and does not expect to have to think about it. The note
/// is shared too: an Escape that quietly returned to the list would leave the
/// user unsure whether they had answered at all.
#[test]
fn escaping_a_delete_leaves_the_connection_in_place() {
    let (fixture, mut pty) = manage();
    let before = fixture.read_store().expect("the store before");

    pty.key(b"\x18"); // Ctrl+X
    pty.wait_for_text("Delete web-01? (y/N)")
        .expect("the question is asked");

    pty.key(b"\x1b");
    pty.wait_for_text("Delete web-01? No")
        .expect("Escape refuses in the same words N does");

    assert_eq!(
        fixture.read_store().expect("the store after"),
        before,
        "an escaped delete must not touch the store"
    );
}

/// `y` deletes it, out of the store and off the screen.
///
/// Both halves are asserted because they are two different failures: the store
/// can lose the row while the frame keeps drawing it, and then the next Enter
/// runs ssh against a Connection that no longer exists. The frame is read after
/// the write, so the order the user sees is the order that happened.
#[test]
fn confirming_a_delete_removes_it_from_the_store_and_the_frame() {
    let (fixture, mut pty) = manage();

    pty.key(b"\x18"); // Ctrl+X
    pty.wait_for_text("Delete web-01? (y/N)")
        .expect("the question is asked");

    pty.key(b"y");
    pty.wait_for_text("\u{25c7} deleted web-01")
        .expect("the frame reports the deletion");

    // The row rendering is what must go, not the name: the notes the frame
    // just drew say `deleted web-01`, and they should. A row that keeps
    // rendering after the store dropped it is the failure - the next Enter
    // would run ssh against a Connection that is no longer there.
    pty.wait_until(
        "the row leaving the frame",
        Duration::from_secs(4),
        |screen| !screen.contents().contains("web-01 (deploy@10.0.0.1:22)"),
    )
    .expect("the deleted row is gone from the screen");
    assert!(
        pty.grid().contains("db-01"),
        "the Connection nobody deleted is still there\n{}",
        pty.grid()
    );

    let aliases = fixture.stored_aliases().expect("the store after");
    assert_eq!(
        aliases,
        vec!["db-01".to_string()],
        "the store holds what is left, and nothing else"
    );
}
