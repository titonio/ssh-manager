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
use sshm::config::Connection;
use sshm::frame::{build_frame_with_flow, Canvas, FrameFlow, FrameMode, FRAME_LINES};
use sshm::manage::{self, step, DeleteOutcome, EditOutcome, ManageState};
use sshm::theme::ColorSupport;

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
    }
}

fn conns() -> Vec<Connection> {
    vec![web01(), web02()]
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
    build_frame_with_flow(
        &conns(),
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
    assert!(
        glyph.style.add_modifier.contains(Modifier::DIM),
        "the note is a *dim* note: {:?}",
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

/// Type `text` into the live step, then press Enter.
fn answer(state: &ManageState, text: &str) -> ManageState {
    let mut typed = state.clone();
    for ch in text.chars() {
        typed = step(&typed, plain(ch), Some(&web01())).state;
    }
    step(&typed, key(KeyCode::Enter), Some(&web01())).state
}

/// Type `text` into the live step and leave it there.
fn typed(state: &ManageState, text: &str) -> ManageState {
    let mut s = state.clone();
    for ch in text.chars() {
        s = step(&s, plain(ch), Some(&web01())).state;
    }
    s
}

/// The state with Alias and Host answered: the live step is Port.
fn at_port() -> ManageState {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    answer(&answer(&adding.state, "web-01"), "10.0.0.4")
}

/// The state with Alias, Host and Port answered: the live step is Key.
fn at_key() -> ManageState {
    answer(&at_port(), "2222")
}

/// The state with everything but Folder answered: the live step is Folder,
/// the last of the five.
fn at_folder() -> ManageState {
    answer(&at_key(), "~/.ssh/id_ed25519")
}

#[test]
fn the_add_step_header_is_the_step_with_what_has_been_typed() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&typed(&adding.state, "web-01"));

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Alias  web-01_",
        "one line, one ask — and the caret the frame draws for itself, \
         because it hides the hardware one"
    );
}

#[test]
fn an_untouched_step_shows_just_the_ask_and_the_caret() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&adding.state);

    assert_eq!(line_text(&frame.lines()[0]), "◆ Alias  _");
}

#[test]
fn a_settled_step_leaves_a_diamond_trace_naming_its_value() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&answer(&adding.state, "web-01"));

    assert!(
        frame_text(&frame)
            .iter()
            .any(|l| l.starts_with("│   ◇ Alias   web-01")),
        "the answered step settles to ◇, got {:?}",
        frame_text(&frame)
    );
}

#[test]
fn the_settled_steps_line_up_down_the_sequence() {
    let frame = render(&at_port());

    let text = frame_text(&frame);
    let alias = text
        .iter()
        .find(|l| l.contains("Alias"))
        .expect("Alias settled line");
    let host = text
        .iter()
        .find(|l| l.contains("Host"))
        .expect("Host settled line");

    // `◇ Alias   web-01` / `◇ Host    10.0.0.4` — the values start in
    // the same column, so the eye reads down a column of answers rather
    // than chasing a ragged right edge of labels.
    assert_eq!(
        alias.find("web-01").unwrap(),
        host.find("10.0.0.4").unwrap()
    );
}

/// An optional field the user left empty is drawn as *absent*, not as
/// nothing. A blank after `◇ Key` reads as a step that lost its answer.
#[test]
fn an_absent_optional_field_is_drawn_as_absent_not_blank() {
    let after_key = answer(&at_key(), "");

    let frame = render(&after_key);

    assert!(
        frame_text(&frame)
            .iter()
            .any(|l| l.starts_with("│   ◇ Key     —")),
        "an empty optional field settles visibly as absent, got {:?}",
        frame_text(&frame)
    );
}

#[test]
fn a_refused_answer_is_shown_under_the_header_in_the_warning_role() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let refused = step(&adding.state, key(KeyCode::Enter), Some(&web01()));
    let frame = render(&refused.state);

    let joined = frame_text(&frame).join("\n");
    assert!(
        joined.contains("! alias is required"),
        "the refusal is on the glass, got {joined}"
    );

    let t = sshm::theme::Theme::clack();
    let glyph = line_with(&frame, '!')
        .spans
        .iter()
        .find(|s| s.content.contains('!'))
        .expect("the ! glyph span");
    assert_eq!(
        glyph.style.fg,
        Some(t.warning),
        "the refusal wears the warning role, not a hue of its own"
    );
}

/// The refusal is the last thing dropped-and-kept: a terminal too short
/// for the whole sequence history keeps the reason the user is stuck and
/// drops the oldest settled step instead.
#[test]
fn a_short_terminal_keeps_the_refusal_over_the_settled_history() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let deep = answer(&answer(&adding.state, "web-01"), "");

    let short = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 6, ColorSupport::Truecolor),
        &FrameFlow::from(&deep),
    );

    let joined = frame_text(&short).join("\n");
    assert!(
        joined.contains("host is required"),
        "the reason survives the squeeze, got {joined}"
    );
}

/// The ticket's string, earned the way the `◇ deleted` note earns its own.
#[test]
fn the_added_note_is_the_ticket_s_own_string() {
    let finalised = answer(&at_folder(), "prod");
    let settled = manage::settle_add(&finalised, &[web01()], Ok(web01()))
        .expect("a Connection that was really written settles onto the list");

    let frame = render(&settled);

    let text = frame_text(&frame);
    let note = text
        .iter()
        .find(|l| l.contains('◇'))
        .unwrap_or_else(|| panic!("no ◇ note in {text:?}"));

    assert_eq!(
        note.trim_start_matches('│').trim(),
        "◇ added [prod] web-01",
        "the note is the ticket's string"
    );
}

/// The honesty rule at the frame level: a sequence that finished without
/// the store's answer shows no `added` note, and an abandoned one shows
/// nothing that could be read as a write.
#[test]
fn nothing_claims_an_add_before_the_store_says_so() {
    let finalised = answer(&at_folder(), "prod");

    let joined = frame_text(&render(&finalised)).join("\n");
    assert!(
        !joined.contains("added"),
        "the frame claimed a write the store has not confirmed: {joined}"
    );

    // A sequence backed off its first step with a partial answer in it.
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let abandoned = step(
        &typed(&adding.state, "web"),
        key(KeyCode::Esc),
        Some(&web01()),
    );

    let joined = frame_text(&render(&abandoned.state)).join("\n");
    assert!(
        !joined.contains("added"),
        "an abandoned sequence must not read as an added Connection: {joined}"
    );
    assert!(
        joined.contains("nothing saved"),
        "it must say the opposite instead: {joined}"
    );
}

/// The constant-height contract (#34) holds across every step of the
/// sequence, including the ones carrying four settled traces and a
/// refusal.
#[test]
fn the_add_sequence_never_grows_the_frame() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let mut states = vec![("Alias, empty", adding.state.clone())];
    let mut cursor = adding.state.clone();
    for label in ["Alias", "Host", "Port", "Key"] {
        cursor = answer(&cursor, "x");
        states.push((label, cursor.clone()));
        let refused = step(&cursor, key(KeyCode::Enter), Some(&web01()));
        states.push((label, refused.state.clone()));
    }

    for (label, state) in states {
        let frame = render(&state);
        assert_eq!(
            frame.lines().len(),
            FRAME_LINES,
            "{label}: the frame grew past its constant height"
        );
    }
}

/// Every glyph the sequence carries survives the colour being turned off.
#[test]
fn the_add_sequence_survives_monochrome() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));
    let deep = answer(&answer(&adding.state, "web-01"), "");

    let mono = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&deep),
    );
    let joined = frame_text(&mono).join("\n");

    assert!(
        joined.contains('◆'),
        "the live step glyph vanished: {joined}"
    );
    assert!(joined.contains('◇'), "the settled trace vanished: {joined}");
    assert!(
        joined.contains('!') && joined.contains("host is required"),
        "the refusal vanished: {joined}"
    );
    assert!(joined.contains('_'), "the caret vanished: {joined}");
}

/// The rail during a step names only what that step reads, escape hatch
/// first. `Ctrl+X delete` is *not* among them: the add step ignores that
/// chord, and a hint for a key the step does not read is the one thing
/// this frame has promised twice already not to do.
#[test]
fn the_add_step_rail_names_only_what_the_step_reads() {
    let adding = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&adding.state);
    let rail = line_text(frame.lines().iter().rev().nth(1).expect("hint rail"));

    for live in ["Esc back", "Enter next", "Ctrl+C quit"] {
        assert!(rail.contains(live), "missing {live:?} from {rail:?}");
    }
    for not_live in ["Ctrl+X delete", "y confirm"] {
        assert!(
            !rail.contains(not_live),
            "{not_live:?} is not a key this step reads: {rail:?}"
        );
    }
}

/// On the last step the same key means something different, and the rail
/// says the different thing.
#[test]
fn the_last_step_s_rail_says_what_enter_does_there() {
    let frame = render(&at_folder());
    let rail = line_text(frame.lines().iter().rev().nth(1).expect("hint rail"));

    assert!(
        rail.contains("Enter add"),
        "the last step commits: {rail:?}"
    );
    assert!(!rail.contains("Enter next"), "there is no next: {rail:?}");
}

// ─────────────────────────────────────────────────────────────────────────────
// The add route's note
// ─────────────────────────────────────────────────────────────────────────────

/// `Ctrl+A` answers its chord by putting the first step on the glass.
/// The visible answer to the chord *is* the live step now (#37), not a
/// note about one.
#[test]
fn ctrl_a_answers_visibly_by_opening_the_sequence() {
    let added = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&added.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("◆ Alias"),
        "Ctrl+A must open the first step on the glass, got {joined}"
    );
    assert!(
        !joined.contains("deleted"),
        "the add step must not read as a delete: {joined}"
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
// The Ctrl+E in-place single-field editor (#37)
// ─────────────────────────────────────────────────────────────────────────────

/// Open the editor and walk the field selector to `label`.
fn focus_edit(label: &str) -> ManageState {
    let mut state = step(&ManageState::new(), ctrl('e'), Some(&web01())).state;
    loop {
        let text = frame_text(&render(&state)).join("\n");
        if text.contains(&format!("◆ {label:<6}")) {
            return state;
        }
        state = step(&state, key(KeyCode::Right), Some(&web01())).state;
    }
}

/// The header names the **Connection**, not the field.
///
/// The add header wears the field word because the field is the whole ask.
/// In an edit the field is drawn on its own line below, and the top of the
/// frame has a more important question to answer: which Connection is
/// about to be written. That must be readable from the first line, not
/// recovered by scanning the list for a cursor.
#[test]
fn the_edit_header_names_the_connection_being_edited() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let frame = render(&armed.state);

    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Edit [prod] web-01",
        "the header must name the target with its folder prefix and bold alias"
    );
}

/// The header keeps naming the same Connection after the field selector
/// moves, and after the field is retyped.
#[test]
fn the_edit_header_survives_the_field_being_changed() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let typed = step(&armed.state, plain('x'), Some(&web01()));

    let frame = render(&typed.state);
    assert_eq!(
        line_text(&frame.lines()[0]),
        "◆ Edit [prod] web-01",
        "the target is what the header reports, whatever is on the field line"
    );
}

/// The field line shows the field word, the value it was seeded from, and
/// the caret.
///
/// The pre-fill is the difference between editing and retyping: the user
/// sees the value they are changing.
#[test]
fn the_edit_field_line_shows_the_field_its_current_value_and_the_caret() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let frame = render(&armed.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("◆ Alias   web-01_"),
        "the field line must show the label, the seeded value and the caret: {joined}"
    );
}

/// Arrowing to another field changes the line to *that* field's value.
///
/// The frame must never show `◆ Port  web-01`. A label and a value that
/// disagree is how a user ends up writing a hostname into a port.
#[test]
fn the_edit_field_line_always_matches_the_field_it_names() {
    for (label, value) in [
        ("Alias", "web-01"),
        ("Host", "10.0.0.4"),
        ("Port", "22"),
        ("Key", "_"),
        ("Folder", "prod"),
    ] {
        let state = focus_edit(label);
        let joined = frame_text(&render(&state)).join("\n");
        assert!(
            joined.contains(&format!("◆ {label:<6}  {value}"))
                || joined.contains(&format!("◆ {label:<6} {value}")),
            "the {label} line must read {value:?}, got: {joined}"
        );
    }
}

/// A field with no value shows the caret alone, not a blank field.
#[test]
fn an_absent_optional_field_still_shows_where_typing_goes() {
    let state = focus_edit("Key");
    let joined = frame_text(&render(&state)).join("\n");

    assert!(
        joined.contains("◆ Key     _"),
        "an empty optional field must still show the caret: {joined}"
    );
}

/// The rejection sits under the field, in the warning role with the `!`
/// glyph — the same grammar the add sequence uses for a refusal.
#[test]
fn a_refused_edit_shows_the_reason_under_the_field() {
    let on_port = focus_edit("Port");
    let cleared = (0..2).fold(on_port, |s, _| {
        step(&s, key(KeyCode::Backspace), Some(&web01())).state
    });
    let typed = type_into(&cleared, "99999");
    let refused = step(&typed, key(KeyCode::Enter), Some(&web01()));

    let frame = render(&refused.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("! port must be a number from 1 to 65535"),
        "the refusal must be visible under the field: {joined}"
    );
    assert!(
        joined.contains("◆ Port    99999_"),
        "what the user typed stays on the line to be fixed: {joined}"
    );
}

/// The note a real edit earns.
#[test]
fn the_edited_note_is_the_tickets_own_string() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let typed = step(&armed.state, plain('x'), Some(&web01()));
    let committed = step(&typed.state, key(KeyCode::Enter), Some(&web01()));
    let edited = Connection {
        alias: "web-01x".into(),
        ..web01()
    };
    let settled = manage::settle_edit(
        &committed.state,
        std::slice::from_ref(&edited),
        &web01(),
        EditOutcome::Updated(edited.clone()),
    )
    .expect("a real write settles");

    let frame = render(&settled);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("◇ edited [prod] web-01x"),
        "the note must name the Connection the store actually wrote: {joined}"
    );
}

/// An abandoned edit says nothing was saved.
#[test]
fn an_abandoned_edit_says_nothing_was_saved() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let typed = type_into(&armed.state, "-typo");
    let abandoned = step(&typed, key(KeyCode::Esc), Some(&web01()));

    let frame = render(&abandoned.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("◇ edit abandoned — nothing saved"),
        "backing out must answer whether the half-typed field was saved: {joined}"
    );
}

/// An Enter on a Connection the store does not have must not read as a
/// successful edit.
#[test]
fn an_edit_that_landed_on_nothing_says_so() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let committed = step(&armed.state, key(KeyCode::Enter), Some(&web01()));
    let settled = manage::settle_edit(&committed.state, &[], &web01(), EditOutcome::Absent)
        .expect("absent settles");

    let frame = render(&settled);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains("edit failed"),
        "a write that did not happen must not be reported as `edited`: {joined}"
    );
    assert!(
        !joined.contains("◇ edited"),
        "the word `edited` never appears where no edit happened: {joined}"
    );
}

/// The editor's rail names the two things true only here.
///
/// Without `←→ field` the arrows would be a mystery: the list uses them
/// for nothing else, and nothing on screen would say they move the field.
#[test]
fn the_edit_rail_names_the_field_selector_and_what_enter_does() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let frame = render(&armed.state);
    let rail = line_text(frame.lines().iter().rev().nth(1).expect("hint rail"));

    for hint in ["Esc back", "Enter save", "←→ field", "Ctrl+C quit"] {
        assert!(
            rail.contains(hint),
            "the edit rail must name {hint:?} — it is what the step reads. Got {rail:?}"
        );
    }
    // Order is the contract, not a style choice: drop-from-the-end means
    // what is listed first is what survives a narrow terminal, and the
    // documented order is escape hatch → Enter → movement → the rest.
    assert_eq!(
        rail, "│   Esc back · Enter save · ←→ field · Ctrl+C quit",
        "the edit rail must keep the documented hint order"
    );
}

/// The editor never grows the frame.
///
/// The settle-collapse counts one frame line as one physical row; a frame
/// that changed height mid-interaction would strand rows on the glass.
#[test]
fn the_edit_never_grows_the_frame() {
    let base = render(&ManageState::new());
    let heights = [
        render(&step(&ManageState::new(), ctrl('e'), Some(&web01())).state),
        render(&focus_edit("Folder")),
        render(&focus_edit("Port")),
    ];

    for frame in heights {
        assert_eq!(
            frame.lines().len(),
            base.lines().len(),
            "the frame is a constant height whatever the editor is showing"
        );
        assert_eq!(frame.lines().len(), FRAME_LINES);
    }
}

/// Every glyph the editor uses to carry a state survives with colour off.
#[test]
fn the_edit_survives_monochrome() {
    let armed = step(&ManageState::new(), ctrl('e'), Some(&web01()));
    let mono = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&armed.state),
    );
    let joined = frame_text(&mono).join("\n");

    assert!(joined.contains("◆ Edit [prod] web-01"));
    assert!(joined.contains("◆ Alias   web-01_"));
    assert!(
        !joined.contains("\u{fffd}"),
        "no tofu in the edit frame: {joined}"
    );
}

/// The editor's lines use theme roles only — no literal colour.
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
