//! What the manage flow *looks like* (#36).
//!
//! The frame seam is a pure value, so the confirm step, the settled `■` line
//! and the dim `◇` note are all asserted as text and as styles — no terminal.
//! The states are driven through `manage::step` rather than hand-built, so
//! these are the real transitions rendered, not a sketch of them.
//!
//! The glyph roles are the point of the file: `◆` asks, `■` has been answered,
//! `◇` is the dim trace of what happened. A glyph that goes missing is a lost
//! state, not a cosmetic loss, so each one is checked under monochrome too.

use crossterm::event::KeyCode;
use ratatui::style::Modifier;
use ratatui::text::Line;
use sshm::config::{Connection, Source};
use sshm::frame::{build_frame_with_flow, Canvas, FormFlow, FrameFlow, FrameMode, FRAME_LINES};
use sshm::manage::{self, step, DeleteOutcome, FormCursor, ManageState};
use sshm::theme::{ColorSupport, Theme};

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn web01() -> Connection {
    Connection {
        id: "id-web-01".into(),
        alias: "web-01".into(),
        host: "10.0.0.4".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("prod".into()),
        source: Source::Stored,
    }
}

fn web02() -> Connection {
    Connection {
        id: "id-web-02".into(),
        alias: "web-02".into(),
        host: "10.0.0.5".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("prod".into()),
        source: Source::Stored,
    }
}

fn conns() -> Vec<Connection> {
    vec![web01(), web02()]
}

/// A Config Host: a `Host` stanza merged in from `~/.ssh/config`, so sshm may
/// show it and connect to it and may not change it.
fn gpu_server() -> Connection {
    Connection {
        id: "cfg-gpu-server".into(),
        alias: "gpu-server".into(),
        host: "gpu.example.com".into(),
        user: "root".into(),
        port: 22,
        key_path: None,
        folder: None,
        source: Source::SshConfig,
    }
}

fn wide() -> Canvas {
    Canvas::new(120, 24, ColorSupport::Truecolor)
}

fn line_text(line: &Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn frame_text(frame: &sshm::frame::Frame) -> Vec<String> {
    frame.lines().iter().map(line_text).collect()
}

fn ctrl(ch: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(ch), crossterm::event::KeyModifiers::CONTROL)
}

fn plain(ch: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(ch), crossterm::event::KeyModifiers::NONE)
}

fn key(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

/// Render the manage frame for whatever state the interaction is in.
fn render(state: &ManageState) -> sshm::frame::Frame {
    render_list(&conns(), state)
}

/// Render the manage frame over an arbitrary list, for the states whose
/// list is not the fixture's — an empty set, and the CTA that stays under
/// it, is part of what is pinned.
fn render_list(list: &[Connection], state: &ManageState) -> sshm::frame::Frame {
    build_frame_with_flow(
        list,
        &state.query,
        state.selection,
        FrameMode::Manage,
        wide(),
        &FrameFlow::from(state),
    )
}

/// The state a real delete leaves behind: the confirm armed, answered `y`,
/// and the store's answer folded back in.
///
/// The frame's claim about the deletion comes from the fold, not from the
/// keystroke, so a frame built here is built the way the driver builds one.
fn deleted_state() -> ManageState {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));
    manage::settle_delete(&answered.state, &web01(), DeleteOutcome::Removed(web01()))
        .expect("a Connection that was really removed settles back onto the list")
}

/// The frame's `◇` note line, with the rail taken off the front: what the
/// note says, and nothing else.
fn note(frame: &sshm::frame::Frame) -> String {
    let lines = frame_text(frame);
    lines
        .iter()
        .find(|line| line.contains('◇'))
        .unwrap_or_else(|| panic!("no ◇ note in {lines:?}"))
        .trim_start_matches('│')
        .trim()
        .to_string()
}

/// The line of the frame that carries `glyph`, if any.
fn line_with(frame: &sshm::frame::Frame, glyph: char) -> &Line<'static> {
    let text = frame_text(frame);
    let idx = text
        .iter()
        .position(|l| l.contains(glyph))
        .unwrap_or_else(|| panic!("no line containing {glyph:?} in {text:?}"));
    &frame.lines()[idx]
}

// ─────────────────────────────────────────────────────────────────────────────
// The confirm step
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_confirm_step_asks_with_the_ticket_s_own_words() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let frame = render(&armed.state);

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Delete [prod] web-01? (y/N)",
        "the confirm header is the ticket's string, verbatim"
    );
}

#[test]
fn the_confirm_step_hints_only_the_answers_it_actually_reads() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let frame = render(&armed.state);
    let rail = line_text(frame.lines().iter().rev().nth(1).expect("hint rail"));

    for answer in ["y confirm", "N abort", "Esc back"] {
        assert!(
            rail.contains(answer),
            "the confirm rail must name {answer:?}, got {rail:?}"
        );
    }
}

/// The list stays behind the confirm: the frame is asking about one row, and
/// the user can still see it. Nothing is hidden and nothing moves.
#[test]
fn the_confirm_step_still_shows_the_list() {
    let plain_frame = render(&ManageState::new());
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let confirm = render(&armed.state);

    assert_eq!(
        confirm.lines().len(),
        plain_frame.lines().len(),
        "opening the confirm must not change the frame's height"
    );
    assert!(
        frame_text(&confirm).iter().any(|l| l.contains("web-02")),
        "the rest of the list must still be on screen behind the confirm"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The settled confirm and the dim note
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn an_answered_confirm_settles_with_the_black_diamond() {
    let deleted = deleted_state();

    let frame = render(&deleted);

    assert!(
        frame_text(&frame)
            .iter()
            .any(|l| l.starts_with("│   ■ Delete [prod] web-01? Yes")),
        "the settled confirm wears ■, got {:?}",
        frame_text(&frame)
    );
}

#[test]
fn a_declined_confirm_settles_with_the_black_diamond_too() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let declined = step(&armed.state, plain('N'), Some(&web01()));

    let frame = render(&declined.state);

    assert!(
        frame_text(&frame)
            .iter()
            .any(|l| l.starts_with("│   ■ Delete [prod] web-01? No")),
        "a declined delete leaves its own trace, got {:?}",
        frame_text(&frame)
    );
    assert!(
        !frame_text(&frame).iter().any(|l| l.contains("deleted")),
        "nothing was deleted, so nothing may say it was: {:?}",
        frame_text(&frame)
    );
}

/// The frame-level half of the honest-note rule: a delete the store did not
/// perform must not render as `◇ deleted`, in any wording, at any width.
/// The Connection is still on disk, and the frame is the user's evidence of
/// what just happened to it.
#[test]
fn a_delete_that_removed_nothing_never_renders_as_deleted() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let answered = step(&armed.state, plain('y'), Some(&web01()));

    let state = manage::settle_delete(&answered.state, &web01(), DeleteOutcome::Absent)
        .expect("the frame stays up: nothing was destroyed");

    let joined = frame_text(&render(&state)).join("\n");

    assert!(
        !joined.contains("deleted"),
        "the frame claims a deletion the store never performed: {joined}"
    );
    assert!(
        joined.contains("■ Delete [prod] web-01? Yes"),
        "the question was answered yes, and that stays on the record: {joined}"
    );
    assert!(
        joined.contains('◇') && joined.contains("no such Connection"),
        "the note must say the delete did not happen and why: {joined}"
    );
}

#[test]
fn the_delete_leaves_a_dim_note_naming_the_connection() {
    let deleted = deleted_state();

    let frame = render(&deleted);

    let text = frame_text(&frame);
    let note = text
        .iter()
        .find(|l| l.contains('◇'))
        .unwrap_or_else(|| panic!("no ◇ note in {text:?}"));

    assert_eq!(
        note.trim_start_matches('│').trim(),
        "◇ deleted [prod] web-01",
        "the note is the ticket's string"
    );

    let glyph = line_with(&frame, '◇')
        .spans
        .iter()
        .find(|s| s.content.contains('◇'))
        .expect("the ◇ glyph span");
    assert_eq!(
        glyph.style.fg,
        Some(sshm::theme::Theme::clack().fg_muted),
        "the note is a *muted* note: {:?}",
        glyph.style
    );
    assert!(
        !glyph.style.add_modifier.contains(Modifier::DIM),
        "the note must not carry DIM — Windows Terminal ignores SGR 2 \
         (microsoft/terminal#6703, issue #42): {:?}",
        glyph.style
    );
}

/// The settled `■` and the dim `◇` are different glyphs for different facts,
/// and neither may be carried by colour alone.
#[test]
fn the_flow_glyphs_survive_monochrome() {
    let deleted = deleted_state();

    let mono = build_frame_with_flow(
        &conns(),
        &deleted.query,
        deleted.selection,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&deleted),
    );
    let text = frame_text(&mono);
    let joined = text.join("\n");

    assert!(joined.contains('■'), "the settled glyph vanished: {joined}");
    assert!(joined.contains('◇'), "the note glyph vanished: {joined}");
    assert!(
        joined.contains("deleted"),
        "the note word vanished: {joined}"
    );
}

/// The write family's notes are the frame's only claim that a write happened,
/// and each one is earned through `settle_add` / `settle_edit` rather than set
/// by a keystroke — so all three are rendered here the way the driver builds
/// them: `◇ added`, `◇ edited`, and the wording that replaces the claim when
/// the store had no such Connection to change.
#[test]
fn every_write_the_store_answered_for_gets_its_own_note() {
    let added = manage::settle_add(&ManageState::new(), &conns(), Ok(web02()))
        .expect("a Connection the store really wrote");
    assert_eq!(
        note(&render(&added)),
        "◇ added [prod] web-02",
        "the add note names the Connection the store made"
    );

    let renamed = Connection {
        alias: "web-99".into(),
        ..web01()
    };
    let edited = manage::settle_edit(
        &ManageState::new(),
        &conns(),
        &web01(),
        manage::EditOutcome::Updated(renamed),
    )
    .expect("a Connection the store really changed");
    assert_eq!(
        note(&render(&edited)),
        "◇ edited [prod] web-99",
        "the note reports the alias the store ended up with"
    );

    let absent = manage::settle_edit(
        &ManageState::new(),
        &conns(),
        &web01(),
        manage::EditOutcome::Absent,
    )
    .expect("an edit that landed on nothing still settles");
    assert_eq!(
        note(&render(&absent)),
        "◇ edit failed — no such Connection: [prod] web-01",
        "no edit happened, so no edit is claimed"
    );
}

/// Backing out of a half-filled add answers the one question the user has —
/// *did any of it get saved?* — and the answer is the frame's to give: the
/// dim note sits where every other note sits, and it names no Connection
/// because none was ever written.
#[test]
fn the_abandoned_add_notes_that_nothing_was_saved() {
    let opened = step(&ManageState::new(), ctrl('a'), None);
    let abandoned = step(&opened.state, key(KeyCode::Esc), None);

    let text = frame_text(&render(&abandoned.state));
    let note = text
        .iter()
        .find(|l| l.contains('◇'))
        .unwrap_or_else(|| panic!("no ◇ note in {text:?}"));

    assert_eq!(
        note.trim_start_matches('│').trim(),
        "◇ add abandoned — nothing saved",
        "the note is the ticket's string, and it is about the add, not about a row"
    );
}

/// The edit half of the same answer. The map was opened from a real row, so
/// the temptation is to word this like `◇ edited [prod] web-01` — but
/// nothing was edited, and a note that read as a write over a list still
/// showing the old value would be the frame lying.
#[test]
fn the_abandoned_edit_notes_that_nothing_was_saved() {
    let opened = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let abandoned = step(&opened.state, key(KeyCode::Esc), Some(&web01()));

    let text = frame_text(&render(&abandoned.state));
    let note = text
        .iter()
        .find(|l| l.contains('◇'))
        .unwrap_or_else(|| panic!("no ◇ note in {text:?}"));

    assert_eq!(
        note.trim_start_matches('│').trim(),
        "◇ edit abandoned — nothing saved",
        "the edit's abandonment says the same thing as the add's, about the edit"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The read-only refusal over a Config Host
//
// `Ctrl+X` and `Ctrl+E` over a row merged from `~/.ssh/config` stop at a `◇`
// note: the frame does not open a confirm or a map for a row sshm may not
// write. The note is in the `◇ added` / `◇ edited` grammar — same glyph,
// same recession — and the list underneath it is the list the user was
// already reading, which is what makes the refusal read as *heard and
// refused* rather than as a screen that did nothing.
// ─────────────────────────────────────────────────────────────────────────────

/// The refusal says why the chord stopped, and the frame stays open on the
/// list: the config row and its stored neighbour are both still drawn.
#[test]
fn a_read_only_refusal_notes_the_note_over_the_list_it_refused_to_change() {
    let list = vec![gpu_server(), web01()];
    let refused = step(&ManageState::new(), ctrl('x'), Some(&gpu_server()));

    let frame = render_list(&list, &refused.state);
    let text = frame_text(&frame);

    assert_eq!(
        note(&frame),
        "◇ read-only — from ssh-config",
        "the note names the provenance, because that is the reason the chord \
         stopped; the row it names is already on the glass underneath"
    );
    assert!(
        text.iter().any(|l| l.contains("[ssh-config] gpu-server")),
        "the refused row is still drawn: {text:?}"
    );
    assert!(
        text.iter()
            .any(|l| l.contains("web-02") || l.contains("[prod] web-01")),
        "the rest of the list is still drawn: {text:?}"
    );
    assert_eq!(
        frame.lines().len(),
        FRAME_LINES,
        "a refusal is a note inside the frame, not a new frame"
    );
    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Manage Connections",
        "the header is still the list's: no confirm was asked and no map opened"
    );
    assert!(
        !text.iter().any(|l| l.contains('▶')),
        "no form map is on the glass, got {text:?}"
    );
}

/// The note is a `◇` note like every other one: the `fg_muted` role carries
/// it — no literal hue, and no `DIM`, which Windows Terminal ignores
/// (microsoft/terminal#6703, issue #42) — and the words survive monochrome,
/// where the colour is gone and only the glyph and the sentence are left.
#[test]
fn the_read_only_note_is_a_muted_note_that_survives_monochrome() {
    let refused = step(&ManageState::new(), ctrl('e'), Some(&gpu_server()));
    let frame = render_list(&[gpu_server()], &refused.state);

    let glyph = line_with(&frame, '◇')
        .spans
        .iter()
        .find(|s| s.content.contains('◇'))
        .expect("the ◇ glyph span");
    assert_eq!(
        glyph.style.fg,
        Some(sshm::theme::Theme::clack().fg_muted),
        "the note is a *muted* note: {:?}",
        glyph.style
    );
    assert!(
        !glyph.style.add_modifier.contains(Modifier::DIM),
        "the note must not carry DIM: {:?}",
        glyph.style
    );

    let mono = build_frame_with_flow(
        &[gpu_server()],
        &refused.state.query,
        refused.state.selection,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&refused.state),
    );
    let joined = frame_text(&mono).join("\n");
    assert!(
        joined.contains("◇ read-only — from ssh-config"),
        "the whole note is words, so colour off must not cost a word: {joined}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The constant-height contract (#34)
// ─────────────────────────────────────────────────────────────────────────────

/// The note and the settled line are drawn *inside* the frame's row budget,
/// so the frame never grows. This is the #34 invariant the ticket warns
/// about: a taller frame would break the settle-collapse's row count.
#[test]
fn the_flow_lines_come_out_of_the_row_budget_not_out_of_the_frame() {
    let empty_flow = render(&ManageState::new());
    assert_eq!(empty_flow.lines().len(), FRAME_LINES);

    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let declined = step(&armed.state, plain('N'), Some(&web01()));
    let deleted = deleted_state();
    let added = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    for (label, state) in [
        ("declined (1 flow line)", &declined.state),
        ("deleted (2 flow lines)", &deleted),
        ("add requested (1 flow line)", &added.state),
    ] {
        let frame = render(state);
        assert_eq!(
            frame.lines().len(),
            FRAME_LINES,
            "{label}: the frame grew past its constant height"
        );
    }
}

/// A short terminal that cannot hold both flow lines and a row keeps its
/// height by dropping the settled line before the note — the note is the
/// thing the ticket asks to be able to read.
#[test]
fn a_short_terminal_drops_the_settled_line_before_the_note() {
    let deleted = deleted_state();

    let short = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 5, ColorSupport::Truecolor),
        &FrameFlow::from(&deleted),
    );

    assert_eq!(
        short.lines().len(),
        1 + 1 + 2,
        "header + one flow line + hint + corner, got {:?}",
        frame_text(&short)
    );
    assert!(
        frame_text(&short).iter().any(|l| l.contains('◇')),
        "the note survives; the settled line is what gives, got {:?}",
        frame_text(&short)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The `Ctrl+A` add step-sequence (#37)
//
// The same grammar the delete confirm established, applied five times:
// `◆` asks on the header, `◇` records what was answered, and the note
// about the write only appears once the write has been reported.
// ─────────────────────────────────────────────────────────────────────────────

/// Type `text` into the live step and leave it there.
fn typed(state: &ManageState, text: &str) -> ManageState {
    let mut s = state.clone();
    for ch in text.chars() {
        s = step(&s, plain(ch), Some(&web01())).state;
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// The form map, as rendered
//
// These drive the real `manage::step` transitions and assert what lands on
// the glass. The state machine's own contract lives in `manage_test.rs`;
// what belongs here is the *picture*: which glyph is on which row, whether
// the cursor row is findable with the colour off, and whether the frame
// keeps its height while all of that changes.
// ─────────────────────────────────────────────────────────────────────────────

/// The add map as `Ctrl+A` leaves it: cursor on Alias, nothing typed.
fn add_open() -> ManageState {
    step(&ManageState::new(), ctrl('a'), Some(&web01())).state
}

/// The two required rows filled the way a user fills them — type, Enter,
/// type, Enter. The cursor ends up on User.
fn add_required_filled() -> ManageState {
    let s = typed(&add_open(), "web-01");
    let s = step(&s, key(KeyCode::Enter), Some(&web01())).state;
    let s = typed(&s, "10.0.0.4");
    step(&s, key(KeyCode::Enter), Some(&web01())).state
}

/// The form projection of a state.
fn form(state: &ManageState) -> FormFlow {
    FrameFlow::from(state)
        .form
        .clone()
        .expect("expected the form map to be open")
}

/// Walk down to the `▶` row, stopping there rather than wrapping.
fn at_submit(state: &ManageState) -> ManageState {
    let mut s = state.clone();
    for _ in 0..FormCursor::ROWS {
        if form(&s).submit_focused {
            return s;
        }
        s = step(&s, key(KeyCode::Down), Some(&web01())).state;
    }
    panic!("Down never reached the submit row");
}

/// The map row drawn for one field label.
fn map_row(frame: &sshm::frame::Frame, label: &str) -> String {
    let padded = format!("{label:<6}");
    frame_text(frame)
        .into_iter()
        .find(|l| l.contains(&padded))
        .unwrap_or_else(|| panic!("no map row for {label:?} in {:?}", frame_text(frame)))
}

/// The frame's hint rail.
fn rail_of(frame: &sshm::frame::Frame) -> String {
    line_text(frame.lines().iter().rev().nth(1).expect("hint rail"))
}

/// The map shows the whole form at once. This is the property the whole
/// redesign exists to buy: the user can see every row they owe before they
/// start, instead of holding the remaining work in their head while one
/// field at a time scrolls past.
#[test]
fn the_map_shows_every_field_at_once() {
    let frame = render(&add_open());
    let text = frame_text(&frame).join("\n");

    for label in ["Alias", "Host", "User", "Port", "Key", "Folder"] {
        assert!(
            text.contains(label),
            "{label} is not on the glass before it has been touched: {text}"
        );
    }
}

/// The header names the *ask*, not the row under the cursor. On the map
/// every row is visible, so the field word at the top would answer a
/// question the map has already answered, and leave the real one — am I
/// adding, or editing what — unanswered.
#[test]
fn the_add_header_names_the_ask_not_the_field() {
    let frame = render(&typed(&add_open(), "web-01"));

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ New connection",
        "the header says what the form is for; the row under the ◆ says \
         which field is live"
    );
}

/// Exactly one row wears the cursor diamond, and it is the row the cursor
/// is on. Two diamonds would mean two live fields; none would mean the user
/// cannot see where they are.
#[test]
fn exactly_one_row_wears_the_cursor_diamond() {
    let frame = render(&add_open());
    let rows: Vec<String> = frame_text(&frame)
        .into_iter()
        .filter(|l| l.contains("◆ ") && !l.starts_with("◆"))
        .collect();

    assert_eq!(
        rows.len(),
        1,
        "expected exactly one cursor-marked row, got {rows:?}"
    );
    assert!(
        rows[0].contains("Alias"),
        "the map opens on Alias, got {:?}",
        rows[0]
    );
}

/// A filled row ticks, a required-empty row shows an open circle, and the
/// two are never confusable — with the colour off.
#[test]
fn filled_and_required_empty_rows_are_told_apart_with_no_colour() {
    let mono = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&add_open()),
    );
    let text = frame_text(&mono).join("\n");

    assert!(
        text.contains("◆ Alias"),
        "the cursor row lost its diamond with colour off: {text}"
    );
    assert!(
        text.contains("○ Host"),
        "a required empty row lost its open circle with colour off: {text}"
    );
    assert!(
        text.contains("· User"),
        "an optional empty row lost its middot with colour off: {text}"
    );
}

/// A required row that is still empty must never be labelled `<optional>`.
/// The first real-terminal dump of the map showed exactly that on Alias
/// and Host — an invitation to skip the two fields that block the save.
#[test]
fn a_required_empty_row_is_labelled_required_never_optional() {
    let frame = render(&add_open());

    assert!(
        map_row(&frame, "Alias").contains("<required>"),
        "Alias is required and empty, got {:?}",
        map_row(&frame, "Alias")
    );
    assert!(
        !map_row(&frame, "Alias").contains("<optional>"),
        "a required row advertised itself as optional: {:?}",
        map_row(&frame, "Alias")
    );
    assert!(
        map_row(&frame, "User").contains("<optional>"),
        "an optional row on the add map should read optional, got {:?}",
        map_row(&frame, "User")
    );
}

/// A value that will not validate marks its row as soon as the cursor
/// passes it — the glyph is live, not deferred to a refused commit.
///
/// The cursor row itself wears `◆` and hides its own state glyph. That is
/// forced, not an oversight: under `NO_COLOR` the diamond is the *only*
/// thing that says "this is the row you are on", so it cannot be shared
/// with `!`. The invalid mark is read on the way past, and the dim `▶`
/// plus the eventual error line cover the moment of typing.
#[test]
fn an_invalid_value_marks_its_row_once_the_cursor_leaves_it() {
    let bad = typed(&goto_row(&add_required_filled(), "Port"), "not-a-port");

    let focused = map_row(&render(&bad), "Port");
    assert!(
        focused.contains('\u{25c6}'),
        "the row under the cursor must wear the diamond, got {focused:?}"
    );

    let port = map_row(&render(&goto_row(&bad, "Alias")), "Port");
    assert!(
        port.contains('!'),
        "a bad port must mark its own row once the cursor is off it, got {port:?}"
    );
}

/// Enter on a field that will not validate still advances. The `!` glyph
/// and the dim `▶` row already carry the whole error signal, so refusing
/// the move would trap the user on a row for no added safety — which is
/// the exact feeling this map was drawn to remove.
#[test]
fn enter_on_an_invalid_field_still_advances() {
    let bad = typed(&goto_row(&add_required_filled(), "Port"), "not-a-port");
    let before = form(&bad);
    let after = step(&bad, key(KeyCode::Enter), Some(&web01()));
    let after = form(&after.state);

    assert!(!before.submit_ready, "setup: the form must not be ready");
    assert!(
        !after.submit_ready,
        "advancing must not make a bad form ready"
    );
    assert!(
        after.error.is_none(),
        "moving off a bad row is not a refusal, so it must not raise one: {:?}",
        after.error
    );
}

/// The error line names every offending field with its reason, on one
/// line, in the row the separator occupied.
#[test]
fn the_error_line_names_every_offending_field() {
    let refused = step(&at_submit(&add_open()), key(KeyCode::Enter), Some(&web01()));
    let frame = render(&refused.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("alias is required"),
        "the refusal must name Alias: {joined}"
    );
    assert!(
        joined.contains("host is required"),
        "the refusal must name Host: {joined}"
    );
    assert!(
        joined.contains("! "),
        "the refusal wears the bang: {joined}"
    );
}

/// The error line takes the separator's row rather than adding one, so
/// the map is eight body rows whether or not anything is wrong. This is
/// what keeps the frame's height constant across the whole form instead
/// of jumping a line every time validation flips.
#[test]
fn the_error_line_replaces_the_rule_row_so_the_height_never_changes() {
    let clean = render(&add_required_filled());
    let refused = render(&step(&at_submit(&add_open()), key(KeyCode::Enter), Some(&web01())).state);

    assert_eq!(
        clean.lines().len(),
        refused.lines().len(),
        "an error changed the frame's height"
    );
    assert_eq!(clean.lines().len(), FRAME_LINES);

    let has_rule = |f: &sshm::frame::Frame| frame_text(f).iter().any(|l| l.contains('─'));
    let has_error = |f: &sshm::frame::Frame| frame_text(f).iter().any(|l| l.contains("! alias"));

    assert!(has_rule(&clean) && !has_error(&clean), "setup");
    assert!(
        !has_rule(&refused) && has_error(&refused),
        "the error did not take the rule's row"
    );
}

/// The `▶` row is present whatever the form's state. A button that
/// appeared only when the form was ready would hide the shape of the form
/// until it was ready, and read as a rendering glitch when it arrived.
#[test]
fn the_submit_row_is_present_whatever_the_form_state() {
    for (label, state) in [
        ("blank", add_open()),
        ("half filled", add_required_filled()),
        (
            "refused",
            step(&at_submit(&add_open()), key(KeyCode::Enter), Some(&web01())).state,
        ),
    ] {
        for support in [ColorSupport::Truecolor, ColorSupport::Monochrome] {
            let frame = build_frame_with_flow(
                &conns(),
                "",
                0,
                FrameMode::Manage,
                Canvas::new(120, 24, support),
                &FrameFlow::from(&state),
            );
            let joined = frame_text(&frame).join("\n");
            assert!(
                joined.contains('▶'),
                "the ▶ vanished for the {label} form under {support:?}: {joined}"
            );
        }
    }
}

/// The rail names what Enter means on the row the cursor is on, and
/// nothing the form does not read.
#[test]
fn the_map_rail_names_only_what_the_form_reads() {
    let frame = render(&add_open());
    let rail = rail_of(&frame);

    for live in ["Esc back", "Enter next", "Ctrl+C quit"] {
        assert!(rail.contains(live), "missing {live:?} from {rail:?}");
    }
    for not_live in ["Ctrl+X delete", "y confirm"] {
        assert!(
            !rail.contains(not_live),
            "{not_live:?} is not a key this form reads: {rail:?}"
        );
    }
}

/// On the `▶` row the same key means *save*, and the rail says so.
#[test]
fn the_submit_row_rail_says_what_enter_does_there() {
    let frame = render(&at_submit(&add_required_filled()));
    let rail = rail_of(&frame);

    assert!(rail.contains("Enter save"), "the ▶ row commits: {rail:?}");
    assert!(
        !rail.contains("Enter next"),
        "there is no next row: {rail:?}"
    );
}

/// The map never grows the frame, whatever it is showing.
#[test]
fn the_map_never_grows_the_frame() {
    let base = render(&ManageState::new());

    for (label, state) in [
        ("blank map", add_open()),
        ("filled map", add_required_filled()),
        ("cursor on submit", at_submit(&add_required_filled())),
        (
            "refused map",
            step(&at_submit(&add_open()), key(KeyCode::Enter), Some(&web01())).state,
        ),
    ] {
        let frame = render(&state);
        assert_eq!(
            frame.lines().len(),
            base.lines().len(),
            "the {label} changed the frame height"
        );
        assert_eq!(frame.lines().len(), FRAME_LINES, "{label}");
    }
}

/// Every glyph the map uses to carry a state survives with colour off.
#[test]
fn the_map_survives_monochrome() {
    let mono_of = |state: &ManageState| {
        frame_text(&build_frame_with_flow(
            &conns(),
            "",
            0,
            FrameMode::Manage,
            Canvas::new(120, 24, ColorSupport::Monochrome),
            &FrameFlow::from(state),
        ))
        .join("\n")
    };

    // The blank map carries the cursor, the required-empty and the
    // optional-empty glyphs; the tick only lives on a map with something
    // filled in. Asserting all five against one state would be a test that
    // can never pass for the wrong reason.
    let blank = mono_of(&add_open());
    let filled = mono_of(&add_required_filled());

    for glyph in ['◆', '○', '·', '▶'] {
        assert!(
            blank.contains(glyph),
            "the {glyph} glyph is missing from the blank map with colour off: {blank}"
        );
    }
    assert!(
        filled.contains('✓'),
        "the tick is missing from the filled map with colour off: {filled}"
    );
}

/// `Ctrl+A` answers its chord by putting the whole map on the glass.
#[test]
fn ctrl_a_answers_visibly_by_opening_the_map() {
    let opened = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let joined = frame_text(&render(&opened.state)).join("\n");

    assert!(
        joined.contains("◆ New connection"),
        "Ctrl+A must open the map on the glass, got {joined}"
    );
    assert!(
        !joined.contains("deleted"),
        "the add map must not read as a delete: {joined}"
    );
}

/// The rail lists only keys that actually work in this build.
///
/// Both `Ctrl+A add` and `Ctrl+E edit` are on it now (#37). Each was
/// taken off in an earlier round for the same reason — advertising an
/// action the chord did not perform — and each is back because the chord
/// now does the thing its label names: `Ctrl+A` walks the five-step
/// sequence and writes the Connection, `Ctrl+E` opens the in-place
/// single-field editor and writes through the store.
#[test]
fn the_manage_rail_advertises_only_the_keys_that_work() {
    let frame = render(&ManageState::new());
    let rail = line_text(
        frame
            .lines()
            .iter()
            .rev()
            .nth(1)
            .expect("the frame has a hint rail"),
    );

    for working in [
        "Esc cancel",
        "↑↓ navigate",
        "Ctrl+X delete",
        "Ctrl+A add",
        "Ctrl+E edit",
    ] {
        assert!(
            rail.contains(working),
            "{working:?} works in this build and must be on the rail: {rail:?}"
        );
    }
}

/// The confirm rail is unchanged by this: it names the answers that step
/// actually reads, and all of them work.
#[test]
fn the_confirm_rail_still_names_every_answer_it_reads() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let frame = render(&armed.state);
    let rail = line_text(frame.lines().iter().rev().nth(1).expect("hint rail"));

    for answer in ["Esc back", "y confirm", "N abort", "Ctrl+C quit"] {
        assert!(rail.contains(answer), "missing {answer:?} from {rail:?}");
    }
}

/// produced, and must be byte-identical to the flow-aware builder handed an
/// empty flow. The two entry points cannot drift.
#[test]
fn an_empty_flow_renders_the_plain_manage_frame() {
    let plain = sshm::frame::build_frame(&conns(), "", 0, FrameMode::Manage, wide());
    let with_flow = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        wide(),
        &FrameFlow::default(),
    );

    assert_eq!(plain, with_flow);
}

/// The flow is a manage-only idea: a pick frame given a manage flow still
/// renders the pick grammar.
#[test]
fn the_pick_frame_ignores_the_manage_flow() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));

    let frame = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Pick,
        wide(),
        &FrameFlow::from(&armed.state),
    );

    assert_eq!(line_text(&frame.lines()[0]), "◆ Select a Connection");
    assert!(
        !frame_text(&frame).join("\n").contains('■'),
        "a pick frame has no settled confirm to show"
    );
}

/// Colour discipline: the new lines draw with theme roles, never a hue of
/// their own. `■` and `◇` are muted; the confirm icon stays accent.
#[test]
fn the_flow_lines_use_theme_roles_only() {
    let t = sshm::theme::Theme::clack();
    let deleted = deleted_state();
    let frame = render(&deleted);

    let settled = line_with(&frame, '■');
    let settled_glyph = settled
        .spans
        .iter()
        .find(|s| s.content.contains('■'))
        .expect("settled glyph");
    assert_eq!(
        settled_glyph.style.fg,
        Some(t.fg_muted),
        "the settled confirm recedes with the muted role"
    );

    let header = &frame.lines()[0];
    let icon = header
        .spans
        .iter()
        .find(|s| s.content.contains('◆'))
        .expect("confirm icon");
    assert_eq!(icon.style.fg, Some(t.accent), "the ask icon stays accent");

    for line in frame.lines() {
        for span in &line.spans {
            assert!(
                span.style.bg.is_none(),
                "the frame stays transparent: {:?} carries a background",
                span.content
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The Ctrl+E form map, as rendered
//
// `Ctrl+E` opens the same map `Ctrl+A` does, seeded from a Connection that
// already exists. These assert the picture: the target named at the top,
// changed rows marked so the save is reviewable, and the frame holding its
// height while all of that moves.
// ─────────────────────────────────────────────────────────────────────────────

/// The edit map as `Ctrl+E` leaves it over `web-01`: cursor on Alias,
/// every row seeded from the stored Connection.
fn edit_open() -> ManageState {
    step(&ManageState::new(), ctrl('e'), Some(&web01())).state
}

/// The row index of a field label in map order.
fn row_index(label: &str) -> usize {
    ["Alias", "Host", "User", "Port", "Key", "Folder"]
        .iter()
        .position(|l| *l == label)
        .unwrap_or_else(|| panic!("no such map row: {label}"))
}

/// Walk the cursor to a named row with `Down`/`Up`.
///
/// Bounded by the map's own length and asserted on arrival. The previous
/// helper in this slot looped on `KeyCode::Right` until the rendered text
/// matched — a key the form map does not bind at all, so the state never
/// changed and the loop never ended. A test that cannot terminate is worse
/// than a test that fails.
fn goto_row(state: &ManageState, label: &str) -> ManageState {
    let target = row_index(label);
    let mut s = state.clone();
    for _ in 0..FormCursor::ROWS {
        let current = form(&s).rows.iter().position(|r| r.focused).unwrap_or(0);
        if current == target {
            assert!(
                form(&s).rows[target].label == label,
                "cursor landed on {} while looking for {label}",
                form(&s).rows[target].label
            );
            return s;
        }
        let step_key = if target > current {
            KeyCode::Down
        } else {
            KeyCode::Up
        };
        s = step(&s, key(step_key), Some(&web01())).state;
    }
    panic!("could not walk the cursor to {label}");
}

/// Backspace a whole seeded value off the row under the cursor.
fn clear_row(state: &ManageState, len: usize) -> ManageState {
    let mut s = state.clone();
    for _ in 0..len {
        s = step(&s, key(KeyCode::Backspace), Some(&web01())).state;
    }
    s
}

/// The header names the **Connection**, not the row under the cursor.
/// The user is one Enter away from a write to `connections.json`, and
/// "which Connection am I changing?" must be answerable from the top of
/// the frame, not by scanning a list the map has hidden.
#[test]
fn the_edit_header_names_the_connection_being_edited() {
    let frame = render(&edit_open());

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Edit [prod] web-01",
        "same grammar as the delete confirm: the chord that changes a \
         Connection reads as the same kind of thing"
    );
}

/// The header keeps naming the target after the rows have been edited.
/// The identity of the thing being written is not something the user
/// should have to re-derive after their first keystroke.
#[test]
fn the_edit_header_survives_the_field_being_changed() {
    let changed = type_into(&goto_row(&edit_open(), "Host"), ".9");
    let frame = render(&changed);

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Edit [prod] web-01",
        "the header tracks the target, not the edit"
    );
}

/// A map opened over a stored Connection shows nothing as changed. If
/// every row arrived wearing `●` the marker would say nothing at all.
#[test]
fn the_edit_map_opens_with_nothing_marked_changed() {
    let frame = render(&edit_open());
    let joined = frame_text(&frame).join("\n");

    assert!(
        !joined.contains('\u{25cf}'),
        "a freshly opened edit map shows changed rows: {joined}"
    );
    assert!(
        joined.contains("\u{2713} Host"),
        "the seeded rows should read as valid, got {joined}"
    );
}

/// A row the user has changed wears `●`, so the save is reviewable
/// before it happens — the property that replaces the old "one field per
/// commit" guarantee with "see everything you are about to write".
#[test]
fn a_changed_row_wears_the_filled_circle() {
    let changed = type_into(&clear_row(&goto_row(&edit_open(), "Port"), 2), "2222");
    // The cursor row wears ◆ and hides its own state glyph, so the changed
    // marker is read with the cursor parked on another row.
    let frame = render(&goto_row(&changed, "Alias"));

    assert!(
        map_row(&frame, "Port").contains('\u{25cf}'),
        "a changed Port must be marked, got {:?}",
        map_row(&frame, "Port")
    );
    // Alias is where the cursor is parked, so it wears `◆` and its own
    // glyph is hidden. What must hold is that it is *not* marked changed.
    let alias = map_row(&frame, "Alias");
    assert!(
        alias.contains('\u{25c6}'),
        "the parked row wears the diamond, got {alias:?}"
    );
    assert!(
        !alias.contains('\u{25cf}'),
        "an untouched row must not read as changed: {alias:?}"
    );
}

/// Two rows can be changed before one commit, and both are marked.
#[test]
fn two_changed_rows_are_both_marked() {
    let mut s = type_into(&goto_row(&edit_open(), "Host"), "-9");
    s = type_into(&goto_row(&s, "User"), "-admin");
    let frame = render(&goto_row(&s, "Alias"));
    let joined = frame_text(&frame).join("\n");

    assert!(map_row(&frame, "Host").contains('\u{25cf}'), "{joined}");
    assert!(map_row(&frame, "User").contains('\u{25cf}'), "{joined}");
    assert_eq!(
        joined.matches('\u{25cf}').count(),
        2,
        "exactly the two touched rows should be marked: {joined}"
    );
}

/// The row under the cursor is the row typing writes to.
#[test]
fn the_cursor_row_is_the_row_typing_writes_to() {
    let s = type_into(&goto_row(&edit_open(), "User"), "-x");
    let frame = render(&s);

    assert!(
        map_row(&frame, "User").contains("deploy-x"),
        "typing must land on the row under the cursor, got {:?}",
        map_row(&frame, "User")
    );
    assert!(
        map_row(&frame, "Host").contains("10.0.0.4"),
        "and nowhere else, got {:?}",
        map_row(&frame, "Host")
    );
}

/// An optional field the stored Connection does not have reads `<not set>`,
/// not `<optional>`: in an edit that is a fact about the Connection, not
/// an invitation.
#[test]
fn an_absent_optional_field_reads_not_set() {
    let frame = render(&edit_open());

    assert!(
        map_row(&frame, "Key").contains("<not set>"),
        "web-01 has no key_path, got {:?}",
        map_row(&frame, "Key")
    );
    assert!(
        !map_row(&frame, "Key").contains("<optional>"),
        "an edit must not phrase a stored absence as an option: {:?}",
        map_row(&frame, "Key")
    );
}

/// A save refused for a bad row says why, in the row the separator
/// occupied, and names the offending field.
#[test]
fn a_refused_save_shows_the_reason_in_the_rule_row() {
    let cleared = clear_row(&goto_row(&edit_open(), "Host"), "10.0.0.4".len());
    let refused = step(&at_submit(&cleared), key(KeyCode::Enter), Some(&web01()));
    let frame = render(&refused.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("host is required"),
        "the refusal must name the field, got {joined}"
    );
    assert!(
        !joined.contains('\u{2500}'),
        "the error should have taken the separator's row, not joined it: {joined}"
    );
}

/// A save with nothing changed is refused with its own reason. Writing the
/// file back unchanged would let the user believe something was saved.
#[test]
fn a_save_with_nothing_changed_says_so() {
    let refused = step(
        &at_submit(&edit_open()),
        key(KeyCode::Enter),
        Some(&web01()),
    );
    let joined = frame_text(&render(&refused.state)).join("\n");

    assert!(
        joined.contains("nothing to save"),
        "an unchanged edit must say it changed nothing, got {joined}"
    );
}

/// The rail names what Enter means on the row the cursor is on.
#[test]
fn the_edit_rail_names_what_enter_means_on_the_row_under_the_cursor() {
    let on_field = rail_of(&render(&edit_open()));
    assert!(
        on_field.contains("Enter next"),
        "Enter on a field advances: {on_field:?}"
    );

    let on_submit = rail_of(&render(&at_submit(&edit_open())));
    assert!(
        on_submit.contains("Enter save"),
        "Enter on the ▶ row saves: {on_submit:?}"
    );
    assert!(
        !on_submit.contains("Enter next"),
        "there is no next row past the commit row: {on_submit:?}"
    );
}

/// The edit map never grows the frame.
#[test]
fn the_edit_never_grows_the_frame() {
    let base = render(&ManageState::new());

    let states = vec![
        edit_open(),
        type_into(&goto_row(&edit_open(), "Port"), "2222"),
        at_submit(&edit_open()),
        step(
            &at_submit(&edit_open()),
            key(KeyCode::Enter),
            Some(&web01()),
        )
        .state,
        step(
            &at_submit(&clear_row(&goto_row(&edit_open(), "Host"), 9)),
            key(KeyCode::Enter),
            Some(&web01()),
        )
        .state,
    ];

    for state in states {
        let frame = render(&state);
        assert_eq!(
            frame.lines().len(),
            base.lines().len(),
            "an edit state changed the frame height"
        );
        assert_eq!(frame.lines().len(), FRAME_LINES);
    }
}

/// Every glyph the edit map uses to carry a state survives with colour off.
#[test]
fn the_edit_survives_monochrome() {
    // Port seeds "22"; clearing first keeps the typed value valid so the
    // row really is `●` rather than `!`.
    let changed = type_into(&clear_row(&goto_row(&edit_open(), "Port"), 2), "2222");
    // Park the cursor off Port so `●` is drawn instead of masked by `◆`.
    let changed = goto_row(&changed, "Alias");
    let mono = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&changed),
    );
    let joined = frame_text(&mono).join("\n");

    for glyph in ['\u{25c6}', '\u{25cf}', '\u{2713}', '\u{25b6}'] {
        assert!(
            joined.contains(glyph),
            "the {glyph} glyph is missing with colour off: {joined}"
        );
    }
}

/// No literal colour anywhere in the edit frame.
#[test]
fn the_edit_lines_use_theme_roles_only() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let frame = render(&armed.state);
    for line in frame.lines() {
        for span in &line.spans {
            assert!(
                !format!("{:?}", span.style).contains("Rgb"),
                "a literal colour in the edit frame: {:?}",
                span.style
            );
        }
    }
}

fn type_into(state: &ManageState, text: &str) -> ManageState {
    let mut s = state.clone();
    for ch in text.chars() {
        s = step(&s, plain(ch), Some(&web01())).state;
    }
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// The glyph colours of a map row
//
// The glyph says whether a field is filled, changed, or blocking the save, and
// it says it in colour first — the character is the fallback for a terminal
// that will not render it. A glyph whose colour is wrong is a state that reads
// wrong, so these assert the role each one wears, and that focus and validity
// are two facts the frame can state separately.
// ─────────────────────────────────────────────────────────────────────────────

/// The map row as drawn, spans and all — for the assertions about colour, which
/// `map_row` flattens away.
fn map_row_drawn<'a>(frame: &'a sshm::frame::Frame, label: &str) -> &'a Line<'static> {
    let padded = format!("{label:<6}");
    frame
        .lines()
        .iter()
        .find(|l| line_text(l).contains(&padded))
        .unwrap_or_else(|| panic!("no map row for {label:?} in {:?}", frame_text(frame)))
}

/// The glyph span of a row: `"<glyph> "`, after the rail and the gutter pad.
fn glyph_span<'a>(line: &'a Line<'static>, glyph: char) -> &'a ratatui::text::Span<'static> {
    let want = format!("{glyph} ");
    line.spans
        .iter()
        .find(|s| s.content == want)
        .unwrap_or_else(|| panic!("no {want:?} glyph span in {:?}", line_text(line)))
}

/// A field whose content will not validate wears the warning role, and keeps
/// wearing it after the cursor moves on. The problem does not go away when the
/// reader looks elsewhere, and a `!` that recedes to the dim of an untouched
/// row is a blocked save that looks ready.
#[test]
fn an_invalid_field_keeps_the_warning_role_after_the_cursor_moves_on() {
    let filled = type_into(&goto_row(&edit_open(), "Port"), "abc");
    let moved_on = step(&filled, key(KeyCode::Down), Some(&web01())).state;

    let frame = render(&moved_on);
    let glyph = glyph_span(map_row_drawn(&frame, "Port"), '!');

    assert_eq!(
        glyph.style.fg,
        Some(Theme::clack().warning),
        "the ! must wear the warning role, not the dim of an untouched row"
    );
    assert!(
        glyph.style.add_modifier.contains(Modifier::BOLD),
        "the ! is bold so it reads without colour: {:?}",
        glyph.style
    );
}

/// A field that differs from the stored Connection is marked in the accent —
/// but without the bold the focused row owns. "Changed" and "the cursor is
/// here" are two facts, and a frame that can only say them together cannot say
/// either.
#[test]
fn a_changed_field_is_marked_in_accent_without_the_focus_bold() {
    let filled = type_into(&edit_open(), "x");
    let moved_on = step(&filled, key(KeyCode::Down), Some(&web01())).state;

    let frame = render(&moved_on);
    let glyph = glyph_span(map_row_drawn(&frame, "Alias"), '●');

    assert_eq!(glyph.style.fg, Some(Theme::clack().accent));
    assert!(
        !glyph.style.add_modifier.contains(Modifier::BOLD),
        "the changed glyph must not borrow the focus row's bold: {:?}",
        glyph.style
    );
}

/// The row the cursor is on is unmistakable whatever its own state says: the
/// `◆` in accent bold, in place of the `✓` the field would draw for itself.
/// Focus has to win, because every key the user presses next acts on this row.
#[test]
fn the_focused_row_wears_the_accent_glyph_whatever_its_own_state() {
    let frame = render(&edit_open());
    let glyph = glyph_span(map_row_drawn(&frame, "Alias"), '◆');

    assert_eq!(glyph.style.fg, Some(Theme::clack().accent));
    assert!(
        glyph.style.add_modifier.contains(Modifier::BOLD),
        "focus is the one state that must be obvious: {:?}",
        glyph.style
    );
}

/// A row that is neither focused nor a problem recedes to `fg_muted`. Six rows
/// all emphasised is six rows saying nothing, and the two rows that do need
/// shouting at — the focused one and the broken one — lose their edge.
#[test]
fn an_untouched_field_recedes_behind_the_focus_and_the_problems() {
    let frame = render(&edit_open());
    let glyph = glyph_span(map_row_drawn(&frame, "Host"), '✓');

    assert_eq!(
        glyph.style.fg,
        Some(Theme::clack().fg_muted),
        "an untouched row must recede so the focused and invalid rows stand out"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Where the caret is, and where it is not
//
// The caret is the only thing on the glass that says which row takes the next
// keystroke. Two ways to get that wrong, and each has to be pinned separately:
// painting a caret on every row that holds text, and painting one nowhere.
// ─────────────────────────────────────────────────────────────────────────────

/// The row the cursor is on shows its text bold and ends in the caret: the text
/// is the fact and the caret is chrome, so the caret is dim and the text is
/// not. Drop the focus from this row's rendering and the reader is typing into
/// a field they have to find by counting.
#[test]
fn the_focused_row_shows_its_text_bold_and_ends_with_the_caret() {
    let frame = render(&edit_open());
    let alias = map_row_drawn(&frame, "Alias");

    let value = alias
        .spans
        .iter()
        .find(|s| s.content == "web-01")
        .unwrap_or_else(|| panic!("no alias value span in {:?}", line_text(alias)));
    assert!(
        value.style.add_modifier.contains(Modifier::BOLD),
        "the text being edited is the fact on the row: {:?}",
        value.style
    );
    assert!(
        alias.spans.iter().any(|s| s.content == "_"),
        "the row that takes the next keystroke has to say so: {:?}",
        line_text(alias)
    );
}

/// A row the cursor has left shows its text plain and carries no caret, however
/// much text it holds. A caret on every filled row is a caret that says nothing,
/// and the reader types into the wrong field believing the glass.
#[test]
fn a_row_the_cursor_has_left_shows_its_text_plain_and_carries_no_caret() {
    let frame = render(&edit_open());
    let host = map_row_drawn(&frame, "Host");

    let value = host
        .spans
        .iter()
        .find(|s| s.content == "10.0.0.4")
        .unwrap_or_else(|| panic!("no host value span in {:?}", line_text(host)));
    assert!(
        !value.style.add_modifier.contains(Modifier::BOLD),
        "a row that is not focused must not dress its text as if it were: {:?}",
        value.style
    );
    assert!(
        !host.spans.iter().any(|s| s.content == "_"),
        "one caret per frame, on the row the cursor is on: {:?}",
        line_text(host)
    );
}
