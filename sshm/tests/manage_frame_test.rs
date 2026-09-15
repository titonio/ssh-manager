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
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use sshm::config::Connection;
use sshm::frame::{build_frame_with_flow, Canvas, FrameFlow, FrameMode, FRAME_LINES};
use sshm::manage::{step, ManageState};
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
    crossterm::event::KeyEvent::new(
        KeyCode::Char(ch),
        crossterm::event::KeyModifiers::CONTROL,
    )
}

fn plain(ch: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(KeyCode::Char(ch), crossterm::event::KeyModifiers::NONE)
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

/// The line of the frame that carries `glyph`, if any.
fn line_with<'a>(frame: &'a sshm::frame::Frame, glyph: char) -> &'a Line<'static> {
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
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let deleted = step(&armed.state, plain('y'), Some(&web01()));

    let frame = render(&deleted.state);

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

#[test]
fn the_delete_leaves_a_dim_note_naming_the_connection() {
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let deleted = step(&armed.state, plain('y'), Some(&web01()));

    let frame = render(&deleted.state);

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
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let deleted = step(&armed.state, plain('y'), Some(&web01()));

    let mono = build_frame_with_flow(
        &conns(),
        &deleted.state.query,
        deleted.state.selection,
        FrameMode::Manage,
        Canvas::new(120, 24, ColorSupport::Monochrome),
        &FrameFlow::from(&deleted.state),
    );
    let text = frame_text(&mono);
    let joined = text.join("\n");

    assert!(joined.contains('■'), "the settled glyph vanished: {joined}");
    assert!(joined.contains('◇'), "the note glyph vanished: {joined}");
    assert!(joined.contains("deleted"), "the note word vanished: {joined}");
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
    let deleted = step(&armed.state, plain('y'), Some(&web01()));
    let added = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    for (label, state) in [
        ("declined (1 flow line)", &declined.state),
        ("deleted (2 flow lines)", &deleted.state),
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
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let deleted = step(&armed.state, plain('y'), Some(&web01()));

    let short = build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(120, 5, ColorSupport::Truecolor),
        &FrameFlow::from(&deleted.state),
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
// The add route's note
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_add_route_answers_visibly() {
    let added = step(&ManageState::new(), ctrl('a'), Some(&web01()));

    let frame = render(&added.state);
    let joined = frame_text(&frame).join("\n");

    assert!(
        joined.contains('◇') && joined.contains("add"),
        "Ctrl+A must leave a visible dim answer, got {joined}"
    );
    assert!(
        !joined.contains("deleted"),
        "the add note must not read as a delete: {joined}"
    );
}

/// A plain list frame — no flow at all — is what `build_frame` already
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
    let armed = step(&ManageState::new(), ctrl('x'), Some(&web01()));
    let deleted = step(&armed.state, plain('y'), Some(&web01()));
    let frame = render(&deleted.state);

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
