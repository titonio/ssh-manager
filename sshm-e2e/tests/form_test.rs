//! Adding a Connection through the Form Map, keystroke by keystroke.
//!
//! The Form Map is the only surface in `sshm` that *writes* something a user
//! composed, which makes it the surface where a keystroke going missing stops
//! being a cosmetic problem: a character swallowed by a modifier rule is a
//! hostname that points at the wrong machine, and a Submit Row that fires while
//! a required field is empty is a Connection that cannot connect. The in-crate
//! suite drives the same flow through the seam, and so cannot see either.
//!
//! Expectations are literals and worked examples, and the store is read back
//! from disk rather than trusted from the frame's own note: the note is a claim
//! about the write, and the file is the write.

use sshm_e2e::fixture::{seeded_fixture, sshm_binary};
use sshm_e2e::{vt100, Pty, DEFAULT_TIMEOUT};

/// Open `sshm manage` on one seeded Connection and press Ctrl+A.
///
/// One seeded row rather than an empty store: the form's fields, cursor and
/// Submit Row are only worth pinning against a list with something in it, and
/// an empty store now opens on whatever `~/.ssh/config` contributes — a
/// fixture's config is not what this file is about.
fn open_form() -> (sshm_e2e::Fixture, Pty) {
    let fixture = seeded_fixture(&["web-01"]);
    let mut pty = Pty::spawn(sshm_binary(), &["manage"], &fixture).expect("spawn sshm manage");
    pty.wait_for_frame().expect("the list is drawn");
    pty.key(b"\x01"); // Ctrl+A
                      // The map arrives as a burst of small writes, and the corner it ends with
                      // was already on screen from the list before it: waiting for the header, or
                      // for the corner, can return in the middle of the handover - list erased,
                      // fields not yet drawn. Wait for the map's own last row, and for the
                      // terminal to go quiet behind it.
    pty.wait_until_settled("the whole form map", DEFAULT_TIMEOUT, |screen| {
        let grid = screen.contents();
        grid.contains("New connection") && grid.contains("\u{25b6}") && grid.contains("\u{2514}")
    })
    .expect("the form map opens");
    (fixture, pty)
}

/// Which row of the map wears the focus glyph, counted over the rows that are
/// not the header, or `None` when none of them does.
///
/// `None` is a real position and not a miss: the submit row is the one row of
/// the map with no glyph of its own, so the cursor being *nowhere* among the
/// fields is what being on it looks like. The header is skipped because it wears
/// a `◆` of its own, and readiness is a different question again: the hint
/// rail says `Enter save` as soon as the form *could* fire, whether or not the
/// cursor is on that row, which is exactly the pair of states a test must not
/// conflate.
fn focus_row(map: &str) -> Option<usize> {
    map.lines()
        .filter(|line| !line.trim().is_empty())
        .skip(1)
        .position(|line| line.contains('\u{25c6}'))
}

/// Press Down and wait until the focus has actually moved, one row, to where
/// Down puts it.
///
/// The wait is for the move rather than for a row named in advance, because the
/// point of stepping is that the cursor is where the last step left it. It
/// settles: a frame mid-repaint can show the glyph on neither the old row nor
/// the new one, and a test that read that as "moved" would be reading a screen
/// the user never saw.
fn step_down(pty: &mut Pty) {
    let before = focus_row(&pty.grid());
    let Some(from) = before else {
        // Already on the \u{25b6} row, where Down is a clamp and not a move:
        // there is nothing to wait for, and inventing a transition here would
        // be the test watching for something the app correctly never does.
        return;
    };
    pty.key(b"\x1b[B");
    pty.wait_until_settled("the focus one row lower", DEFAULT_TIMEOUT, |screen| match (
        Some(from),
        focus_row(&screen.contents()),
    ) {
        (Some(from), Some(to)) => to == from + 1,
        // Onto the submit row from the last field row, where there is no
        // glyph to look for: ask the row itself.
        (Some(_), None) => submit_row_is_focused(screen),
        _ => false,
    })
    .expect("Down moves the focus exactly one row");
}

/// Whether the cursor is on the `\u{25b6}` row.
///
/// That row has no glyph of its own to move, so the honest signal is the one the
/// frame puts there: focus is carried by **bold** on the `\u{25b6}`, in every
/// readiness state, precisely so the row is readable once colour is stripped.
/// Bold is a positive answer. "No field row has a focus glyph" is only the
/// absence of one, and the absence a frame shows halfway through a repaint is
/// indistinguishable from the absence that means the cursor has arrived.
fn submit_row_is_focused(screen: &vt100::Screen) -> bool {
    screen
        .contents()
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            line.find('\u{25b6}')
                .map(|at| (row, line[..at].chars().count()))
        })
        .is_some_and(|(row, col)| {
            screen
                .cell(row as u16, col as u16)
                .is_some_and(|cell| cell.bold())
        })
}

/// The cursor is on the submit row.
fn cursor_is_on_the_submit_row(pty: &mut Pty) {
    pty.wait_until_settled(
        "the cursor on the submit row",
        DEFAULT_TIMEOUT,
        submit_row_is_focused,
    )
    .expect("the cursor reached the submit row");
}

/// Fill Alias and Host, settling each with Enter the way the hint rail says to.
fn fill_alias_and_host(pty: &mut Pty, alias: &str, host: &str) {
    pty.key(alias.as_bytes());
    pty.key(b"\r");
    pty.wait_until(
        "Alias settled and Host focused",
        DEFAULT_TIMEOUT,
        |screen| screen.contents().contains("\u{25c6} Host"),
    )
    .expect("Enter settles a field and moves to the next");
    pty.key(host.as_bytes());
    pty.key(b"\r");
    pty.wait_until("Host settled", DEFAULT_TIMEOUT, |screen| {
        screen
            .contents()
            .contains(&format!("\u{2713} Host   {host}"))
    })
    .expect("the typed host is what settled");
}

/// Every field keeps what is typed into it, and shows it before it settles.
///
/// The Form Map has no staged value: the row the cursor is on *is* the draft,
/// caret and all, so what a test sees mid-keystroke is what the program means
/// to persist. Filling all six rows is the only way to prove the four optional
/// ones are not decoration — a Key field that swallowed its typing would still
/// leave a submittable form, and a green test about Alias and Host would never
/// notice. The store is read back off disk rather than trusted from the note,
/// because the note is a claim about the write and the file is the write.
#[test]
fn every_field_keeps_what_is_typed_into_it() {
    let (fixture, mut pty) = open_form();

    // Live Draft, proven before any Enter: three letters on a row that is
    // still being typed, wearing the focus glyph and the caret.
    pty.key(b"web");
    pty.wait_until(
        "Alias showing what has been typed into it",
        DEFAULT_TIMEOUT,
        |screen| screen.contents().contains("\u{25c6} Alias  web"),
    )
    .expect("the row being typed is the value, before anything settles");

    let fields = [
        ("Host", "10.0.0.7"),
        ("User", "deploy"),
        ("Port", "2222"),
        ("Key", "/keys/id_web"),
        ("Folder", "~/work"),
    ];
    // Enter settles Alias and hands the cursor to the next row, which is what
    // the rail says it does: `Enter next` while a field is focused.
    pty.key(b"\r");
    for (label, value) in fields {
        pty.wait_until(
            &format!("{label} focused and ready"),
            DEFAULT_TIMEOUT,
            |screen| screen.contents().contains(&format!("\u{25c6} {label:<6}")),
        )
        .unwrap_or_else(|error| panic!("{label} never took the cursor: {error}"));
        pty.key(value.as_bytes());
        pty.key(b"\r");
        pty.wait_until(
            &format!("{label} holding {value:?}"),
            DEFAULT_TIMEOUT,
            |screen| {
                screen
                    .contents()
                    .contains(&format!("\u{2713} {label:<6} {value}"))
            },
        )
        .unwrap_or_else(|error| panic!("{label} did not keep what was typed: {error}"));
    }

    // Settling the last field moves the cursor off the fields altogether, onto
    // the Submit Row — which is where the rail changes its mind and says
    // `Enter save`.
    cursor_is_on_the_submit_row(&mut pty);
    pty.key(b"\r");
    // The note names the folder it filed the Connection under, because that is
    // what the user just typed into the Folder field.
    pty.wait_for_text("\u{25c7} added [~/work] web")
        .expect("the frame reports the write it made");
    // And the list row carries the port the Port field was given, which is the
    // same value the ssh target is built from.
    pty.wait_for_text("\u{276f} [~/work] web (deploy@10.0.0.7:2222)")
        .expect("the new row is selected in the list, with the typed port");

    let store = fixture.read_store().expect("the store is readable");
    let added = store["connections"]
        .as_array()
        .expect("the store holds a list")
        .iter()
        .find(|c| c["alias"] == serde_json::json!("web"))
        .expect("the Connection the map was given is in the store");
    for (field, value) in [
        ("host", "10.0.0.7"),
        ("user", "deploy"),
        ("port", "2222"),
        ("key_path", "/keys/id_web"),
        ("folder", "~/work"),
    ] {
        assert_eq!(
            added[field].as_str().unwrap_or(&added[field].to_string()),
            value,
            "the {field} field persisted what was typed into it, not what the frame said"
        );
    }
}

/// Ctrl+A opens the map with every field it has, and says which matter.
///
/// Six rows in a fixed order, each carrying its state as a glyph rather than as
/// colour: `◆` on the row the cursor is on, `○` on a required field still empty,
/// `·` on an optional one. The two required fields wearing a different glyph
/// from the four optional ones is the whole difference between a form that can
/// be submitted and one that cannot, and it has to survive a monochrome
/// terminal — which is why it is asserted as text here and not as a hue.
#[test]
fn ctrl_a_opens_a_blank_form_map_with_every_field() {
    let (_fixture, mut pty) = open_form();

    insta::assert_snapshot!("form-map-blank", pty.screen_document());

    let grid = pty.grid();
    for label in ["Alias", "Host", "User", "Port", "Key", "Folder"] {
        assert!(
            grid.contains(label),
            "the map is missing the {label} row\n{grid}"
        );
    }
    assert!(
        grid.contains("\u{25c6} Alias  <required>"),
        "the cursor opens on Alias, which is required\n{grid}"
    );
    assert!(
        grid.contains("\u{25cb} Host   <required>"),
        "an empty required field says so, and does not look optional\n{grid}"
    );
    for label in ["User", "Port", "Key", "Folder"] {
        assert!(
            grid.contains(&format!("\u{b7} {label:<6} <optional>")),
            "an empty optional field says so, in the same glyph vocabulary as \
             the required ones\n{grid}"
        );
    }
    assert!(
        grid.contains("\u{25b6} Add connection"),
        "the map ends in the decision, not in another field\n{grid}"
    );
}

/// What is typed appears on the row as it is typed, before anything settles.
///
/// The frame calls this the Live Draft, and the reason it is live is the same
/// reason the caret is drawn rather than left to the terminal: an inline frame
/// has no cursor of its own, so without the drawn caret and the text appearing
/// keystroke by keystroke there is no way to tell a field that is focused from a
/// field that is merely on screen.
#[test]
fn a_typed_field_shows_the_text_as_it_is_typed() {
    let (_fixture, mut pty) = open_form();

    pty.key(b"prod");
    pty.wait_until("the typed alias on its row", DEFAULT_TIMEOUT, |screen| {
        screen.contents().contains("prod")
    })
    .expect("the keystrokes land on the focused row");

    let (row, _, line) = pty.locate("prod").expect("the text is on the grid");
    assert_eq!(row, 2, "the text is on the Alias row, not somewhere else");
    assert!(
        line.contains('\u{25c6}'),
        "the row being typed into is the row wearing the focus glyph\n{line}"
    );
    assert!(
        line.contains('_'),
        "the caret is drawn, because an inline frame has no terminal cursor \
         to park\n{line}"
    );

    // Settling keeps the text and hands the row its done-glyph.
    pty.key(b"\r");
    pty.wait_until("the row settling", DEFAULT_TIMEOUT, |screen| {
        screen.contents().contains("\u{2713} Alias  prod")
    })
    .expect("Enter settles the row and marks it valid");
}

/// The cursor walks the map and Enter on the last row writes the store.
///
/// Two things are being proven at once: that the arrow keys move a focus that
/// started life as a byte the terminal sent, and that the row underneath the
/// cursor when Enter is pressed is the row that acts. The store is read back
/// from disk because `◇ added prod` is the frame's claim about the write, and a
/// note that lies is exactly the bug this suite exists to catch.
#[test]
fn the_cursor_walks_the_map_and_enter_writes_the_new_connection() {
    let (fixture, mut pty) = open_form();
    fill_alias_and_host(&mut pty, "prod", "10.1.1.7");

    // Down past the last field onto the `▶` row, and one further to prove the
    // cursor cannot leave the map.
    for _ in 0..5 {
        step_down(&mut pty);
    }
    cursor_is_on_the_submit_row(&mut pty);

    pty.key(b"\r");
    pty.wait_for_text("\u{25c7} added prod")
        .expect("the frame reports the write it made");

    let aliases = fixture.stored_aliases().expect("the store is readable");
    assert!(
        aliases.contains(&"prod".to_string()),
        "the new Connection is in the store: {aliases:?}"
    );

    // And the frame is a list again, with the new row under the cursor: the
    // user is left where the thing they just made is, not dumped somewhere else.
    pty.wait_for_text("\u{276f} prod (@10.1.1.7:22)")
        .expect("the new row is selected in the list");
}

/// A Connection added through the map is a Connection the picker can pick.
///
/// The store is a file, and a file the picker cannot read is a write that did
/// not happen. This is the round trip the user actually cares about: made it
/// here, see it there.
#[test]
fn a_new_connection_renders_in_the_picker() {
    let (fixture, mut pty) = open_form();
    fill_alias_and_host(&mut pty, "prod", "10.1.1.7");
    for _ in 0..5 {
        step_down(&mut pty);
    }
    cursor_is_on_the_submit_row(&mut pty);
    pty.key(b"\r");
    pty.wait_for_text("\u{25c7} added prod")
        .expect("the write happened");

    pty.key(b"\x1b");
    pty.wait_exit(DEFAULT_TIMEOUT)
        .expect("manage closes")
        .assert_code(130);

    let mut picker = Pty::spawn(sshm_binary(), &["pick"], &fixture).expect("spawn sshm pick");
    picker
        .wait_for_text("prod (@10.1.1.7:22)")
        .expect("the picker shows what was just added");
}

/// A port nobody typed is the SSH default, not a missing field.
///
/// The Port row is optional, and optional means the user is allowed to walk past
/// it — but a Connection with no port is a Connection `ssh` cannot reach, so the
/// blank has to become `22` on the way into the store. The screen is checked
/// first to prove the user really never typed one: without that, the assertion
/// would pass just as happily on a form that filled the field itself.
#[test]
fn a_port_nobody_typed_becomes_the_ssh_default() {
    let (fixture, mut pty) = open_form();
    fill_alias_and_host(&mut pty, "prod", "10.1.1.7");

    for _ in 0..5 {
        step_down(&mut pty);
    }
    cursor_is_on_the_submit_row(&mut pty);

    let grid = pty.grid();
    assert!(
        grid.contains("\u{b7} Port   <optional>"),
        "the Port row is still untouched at the moment of saving\n{grid}"
    );

    pty.key(b"\r");
    pty.wait_for_text("\u{25c7} added prod")
        .expect("the write happened");

    let store = fixture.read_store().expect("the store is readable");
    let saved = store["connections"]
        .as_array()
        .expect("the store holds a list")
        .iter()
        .find(|row| row["alias"] == serde_json::json!("prod"))
        .expect("the new Connection is in the store");
    assert_eq!(
        saved["port"],
        serde_json::json!(22),
        "an empty Port settles as the SSH default, not as nothing"
    );
    assert_eq!(
        saved["user"],
        serde_json::json!(""),
        "an empty User stays empty: ssh should use the local login name, not \
         a name the user never gave"
    );
}
