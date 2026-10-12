//! Every keystroke the frames answer, sent as bytes and read off the grid.
//!
//! The in-crate suite already covers this grammar through a seam that hands the
//! frame a synthesised `KeyEvent`. That seam is why the grammar is testable, and
//! why it cannot catch the failure that actually happens to users: a key that
//! never arrives as the event the seam would have handed over. Arrow keys that
//! the terminal sends as `ESC [ A` and crossterm has to decode, a Ctrl+C that
//! only means "cancel" if raw mode turned off `ISIG` first, a capital letter that
//! has to survive as a letter rather than be read as a chord — all of that lives
//! between the keyboard and the seam, and nothing behind the seam can see it.
//!
//! Expectations here are literals and worked examples. Where a key has no
//! binding, the test says so rather than inventing one: the frame's documented
//! behaviour for a key it has no use for is to leave the screen alone.

use sshm::theme::Theme;
use sshm_e2e::fixture::{seeded_fixture, sshm_binary};
use sshm_e2e::{token, Fixture, Pty, DEFAULT_TIMEOUT};

/// Wait until the `❯` cursor sits on `row`.
///
/// Reading the grid straight after a keystroke would read whatever had arrived
/// so far, and the repaint is still in flight: the wait is not politeness, it is
/// the difference between asserting what the frame painted and asserting a race.
/// The cursor is the selection — the frame carries no highlighted background — so
/// "where is the cursor" is the whole question about movement, and asking it of
/// the grid means the answer comes from what was painted.
fn cursor_is_on(pty: &mut Pty, row: u16) {
    pty.wait_until(
        &format!("the cursor on row {row}"),
        DEFAULT_TIMEOUT,
        move |screen| {
            screen
                .contents()
                .lines()
                .position(|line| line.contains('❯'))
                == Some(row as usize)
        },
    )
    .expect("the cursor is where the keystroke should have put it");
}

/// The cursor walks the list and stops at both ends.
///
/// Clamping is the whole point: a selection that runs off the end of the list
/// is a selection that points at nothing, and the next Enter either picks the
/// wrong row or indexes past the end. Both ends, because saturating on one and
/// wrapping on the other is exactly the half-fix these two assertions tell apart.
#[test]
fn the_cursor_walks_the_list_and_stops_at_both_ends() {
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");

    cursor_is_on(&mut pty, 2);

    pty.key(b"\x1b[B"); // Down
    cursor_is_on(&mut pty, 3);

    // One row past the end, twice: the cursor must still be on the last row.
    pty.key(b"\x1b[B");
    pty.key(b"\x1b[B");
    cursor_is_on(&mut pty, 3);

    // And the same at the other end, with one press too many.
    pty.key(b"\x1b[A"); // Up
    pty.key(b"\x1b[A");
    pty.key(b"\x1b[A");
    cursor_is_on(&mut pty, 2);
}

/// PageDown is a key the frame has no use for, and says so by doing nothing.
///
/// The ticket for this matrix names PageDown as a navigation key. The shipped
/// frames do not bind it: the row list is windowed to the terminal, the hint rail
/// names `↑↓` as the movement, and an in-crate test asserts that an unbound key
/// leaves the glass alone. Binding it would be a change to the app, which this
/// suite is not allowed to make, so what is proven here is the real behaviour —
/// the byte sequence arrives, is decoded, and moves nothing — and the gap is
/// recorded on the ticket rather than papered over.
#[test]
fn pagedown_arrives_as_bytes_and_moves_nothing() {
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");
    cursor_is_on(&mut pty, 2);

    pty.key(b"\x1b[6~"); // PageDown, as xterm sends it
                         // A key the frame ignores does not repaint, so there is nothing to wait for.
                         // The proof the bytes arrived and were decoded rather than dropped is that
                         // the frame is still itself afterwards: the next real key moves the cursor by
                         // exactly one, with no extra jump from the PageDown before it.
    pty.key(b"\x1b[B");
    cursor_is_on(&mut pty, 3);
}

/// Typing filters the list, and a capital letter is a letter rather than a chord.
///
/// Capitals are the interesting half. A terminal does not send "capital W", it
/// sends the byte `W` with SHIFT in the modifier report, and a frame whose
/// printable rule is "no modifiers at all" throws the character away: the alias
/// `DB-01` becomes untypeable, and the user watches the list empty out under
/// keys that did something. The same byte has to survive as filter text here and
/// as an alias character in the Form Map, and only the real byte path can show
/// that it does.
#[test]
fn typing_filters_the_list_and_a_capital_letter_is_still_a_letter() {
    let fixture = seeded_fixture(&["web-01", "DB-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("both rows are drawn");
    assert!(pty.grid().contains("DB-01"), "both rows before typing");

    pty.key(b"DB");
    pty.wait_until("the list narrowing to DB-01", DEFAULT_TIMEOUT, |screen| {
        let text = screen.contents();
        text.contains("DB-01") && !text.contains("web-01")
    })
    .expect("a capital letter filters the list like any other printable");
}

/// Backspace takes the query back one character at a time.
///
/// The filter is live, so Backspace is the only way out of a mistyped query short
/// of cancelling the whole frame. Three presses undo three characters, and the
/// list that comes back is the list that was there before any of them.
#[test]
fn backspace_takes_the_query_back_one_character_at_a_time() {
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");

    pty.key(b"web");
    pty.wait_until("the list narrowing to web-01", DEFAULT_TIMEOUT, |screen| {
        let text = screen.contents();
        text.contains("web-01") && !text.contains("db-01")
    })
    .expect("the query hides the row it does not match");

    pty.key(b"\x7f");
    pty.key(b"\x7f");
    pty.key(b"\x7f");
    pty.wait_until("the full list returning", DEFAULT_TIMEOUT, |screen| {
        let text = screen.contents();
        text.contains("web-01") && text.contains("db-01")
    })
    .expect("three backspaces undo three characters");
}

/// Escape leaves the way a cancelled command leaves.
///
/// 130 is the shell's own answer to a cancelled command — `128 + SIGINT` — and a
/// shell binding that returns anything else tells the calling shell that
/// something ran.
#[test]
fn escape_leaves_with_the_cancel_status() {
    let fixture = seeded_fixture(&["web-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}

/// Ctrl+C leaves the same way, and only because raw mode is really on.
///
/// In a cooked terminal `0x03` is a signal, not a byte: the line discipline eats
/// it and sends SIGINT to the foreground group. The frame turning `ISIG` off is
/// what makes Ctrl+C a keystroke the frame can decide about, so this test is as
/// much about the terminal setup as about the key.
#[test]
fn ctrl_c_leaves_with_the_cancel_status() {
    let fixture = seeded_fixture(&["web-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");
    assert!(
        !pty.termios_is_cooked().expect("termios"),
        "Ctrl+C is only a keystroke if the line discipline is out of the way"
    );

    pty.key(b"\x03");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);

    // Every way out of a frame owes the terminal back cooked, not just the one
    // the design names: Ctrl+C leaves through the signal path rather than the
    // Escape path, and a guard that only runs on one of them is a guard that
    // leaves the user's shell echoing weird characters afterwards.
    assert!(
        pty.termios_is_cooked().expect("termios"),
        "Ctrl+C closed the frame and left the terminal raw for whatever runs next"
    );
}

/// The empty state tells a new user what to do next, and leaves on Escape.
///
/// This is the first screen anybody sees on a fresh machine, and the first thing
/// they need from it is the next action — not an error, and not a blank box.
#[test]
fn the_empty_state_names_the_next_action_and_leaves_on_escape() {
    let fixture = Fixture::new();
    let mut pty =
        Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage on nothing");
    pty.wait_for_text("Ctrl+A")
        .expect("the empty state names the way to add one");
    pty.wait_for_frame().expect("the whole frame is drawn");

    insta::assert_snapshot!("manage-empty", pty.screen_document());

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}

/// The no-match state repeats the query back, and leaves on Escape.
///
/// Without the query echoed, an empty list is ambiguous: no matches, or did the
/// filter silently drop everything? The quoted text is what makes the two
/// distinguishable, and it is also the thing to notice is wrong.
#[test]
fn the_no_match_state_shows_what_was_typed_and_leaves_on_escape() {
    let fixture = seeded_fixture(&["web-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_text("web-01").expect("the list draws");

    pty.key(b"zzz");
    pty.wait_for_text("No matches for \"zzz\"")
        .expect("the no-match state names the query");
    pty.wait_for_frame().expect("the whole frame is drawn");

    insta::assert_snapshot!("manage-no-match", pty.screen_document());

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("the frame closes")
        .assert_code(130);
}

/// The picker with data in it, as a committed screen.
///
/// Text only, on purpose: a golden that carried SGR runs would turn every palette
/// change into a hundred-line diff, and the styling has its own test below that
/// names the roles. What this catches is layout — row order, the hint rail's
/// wording, how tall the frame is, where the rail sits.
#[test]
fn the_picker_with_connections_is_a_committed_screen() {
    let fixture = seeded_fixture(&["web-01", "db-01", "cache-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");
    pty.wait_for_text("cache-01").expect("all three draw");
    pty.wait_for_frame().expect("the whole frame is drawn");

    insta::assert_snapshot!("picker-with-data", pty.screen_document());
}

/// Which row is selected is carried by a theme role, not by a colour literal.
///
/// The frame marks the selection with a `❯` in the accent role plus a bold alias,
/// and no filled row — so the assertion names the roles and asks the terminal
/// whether the cells agree. A literal hue here would pass on a palette change that
/// made the selection unreadable, which is precisely the bug the glyph-only
/// snapshots this replaced sat on.
#[test]
fn the_selection_and_the_frame_are_painted_in_theme_roles() {
    let theme = Theme::clack();
    let fixture = seeded_fixture(&["web-01", "db-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");
    pty.wait_for_frame().expect("the whole frame is drawn");

    let (row, col, _) = pty.locate("❯").expect("the cursor is drawn");
    let cursor = pty.cell_style(row, col).expect("the cursor cell exists");
    assert_eq!(
        cursor.fg,
        token(theme.accent),
        "the selection marker is the accent role"
    );
    assert!(cursor.bold, "the selection marker is bold");

    // The rail the whole frame hangs off, and the hint rail under it: two roles
    // that decide whether the frame reads as one object or as noise.
    let (border_row, border_col, _) = pty.locate("│").expect("the rail is drawn");
    assert_eq!(
        pty.cell_style(border_row, border_col).unwrap().fg,
        token(theme.border),
        "the left rail is the border role"
    );
    let (hint_row, hint_col, _) = pty.locate("Esc cancel").expect("the hint rail is drawn");
    assert_eq!(
        pty.cell_style(hint_row, hint_col).unwrap().fg,
        token(theme.fg_muted),
        "the hint rail is the muted role"
    );

    // And the cursor is on exactly one row: a second marker would mean two
    // selections, and the frame has no way to say which one Enter takes.
    let grid = pty.grid();
    assert_eq!(
        grid.matches('❯').count(),
        1,
        "exactly one row may carry the cursor\n{grid}"
    );
}
