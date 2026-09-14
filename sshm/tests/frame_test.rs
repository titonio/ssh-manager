//! The inline frame seam: `build_frame` is a pure view-model, so every test here
//! asserts on the `Frame` value it returns — no terminal, no PTY, no ratatui
//! `Frame`. The frame is a concrete span value; that is the whole point of the
//! seam (#33).
//!
//! Fixture rows are the worked examples from the parent spec (#31): the settle
//! trace `◆ picked web-01 (deploy@10.0.0.4:22)` and the delete confirm
//! `◆ Delete [prod] web-01? (y/N)` both name these Connections, so the
//! expectations below come from the spec rather than from the code under test.

use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use sshm::config::Connection;
use sshm::frame::{build_frame, Canvas, FrameMode, FrameState};
use sshm::theme::{ColorSupport, Theme};

/// The palette the frame draws with. Tests name the *role*, never the hue; the
/// hue itself is pinned by the palette tests in `design_system_test.rs`.
fn t() -> Theme {
    Theme::clack()
}

/// A truecolour canvas `width` columns wide.
fn canvas(width: usize) -> Canvas {
    Canvas::new(width, ColorSupport::Truecolor)
}

/// Wide enough that nothing is fitted away, for tests about content rather than
/// about width.
fn wide() -> Canvas {
    canvas(120)
}

/// The Connections every test filters over.
fn conns() -> Vec<Connection> {
    vec![
        Connection {
            id: "1".into(),
            alias: "web-01".into(),
            host: "10.0.0.4".into(),
            user: "deploy".into(),
            port: 22,
            key_path: None,
            folder: Some("prod".into()),
        },
        Connection {
            id: "2".into(),
            alias: "db-01".into(),
            host: "10.0.0.9".into(),
            user: "postgres".into(),
            port: 5432,
            key_path: None,
            folder: None,
        },
    ]
}

/// Flatten a rendered line back to plain text.
fn line_text(line: &Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Flatten the whole frame to plain text, one string per line.
fn frame_text(frame: &sshm::frame::Frame) -> Vec<String> {
    frame.lines().iter().map(line_text).collect()
}

/// Display width of a rendered line, in terminal columns.
fn line_width(line: &Line<'static>) -> usize {
    line.spans.iter().map(|s| s.content.chars().count()).sum()
}

// ─────────────────────────────────────────────────────────────────────────────
// The header
// ─────────────────────────────────────────────────────────────────────────────

/// The frame opens with the Clack step icon, and the icon carries the accent
/// role — the grammar is `◆ <question>`, not a bordered title bar.
#[test]
fn pick_frame_opens_with_the_clack_step_icon() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let header = &frame.lines()[0];

    assert_eq!(line_text(header), "◆ Select a Connection");
    assert_eq!(
        header.spans[0].style.fg,
        Some(t().accent),
        "the ◆ step icon must be drawn with the accent role"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The rows
// ─────────────────────────────────────────────────────────────────────────────

/// A row reads `[folder] alias (user@host:port)` behind the `│` rail, and the
/// folder prefix is there only when the Connection has one (user stories 7 and 8).
#[test]
fn rows_render_the_folder_alias_and_host_shape_behind_the_rail() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));

    assert_eq!(
        line_text(&frame.lines()[1]),
        "│ ❯ [prod] web-01 (deploy@10.0.0.4:22)"
    );
    assert_eq!(
        line_text(&frame.lines()[2]),
        "│   db-01 (postgres@10.0.0.9:5432)"
    );
}

/// The row's emphasis is carried by style, not by the reader guessing which
/// part is which: the folder and the `user@host:port` meta recede, the alias
/// is bold.
#[test]
fn folder_and_host_meta_recede_while_the_alias_is_bold() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let spans = &frame.lines()[1].spans;

    assert_eq!(
        span_with(spans, "[prod]").style.fg,
        Some(t().fg_muted),
        "the folder prefix should recede"
    );
    assert_eq!(
        span_with(spans, "(deploy@10.0.0.4:22)").style.fg,
        Some(t().fg_muted),
        "the host meta should recede"
    );
    assert!(
        span_with(spans, "web-01")
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "the alias must be bold so the row's subject is unmistakable"
    );
}

/// "Dim meta" has to mean the `DIM` attribute, not merely a dark colour.
///
/// The ticket asks for dim meta and the hint rail already spells dim as
/// `Modifier::DIM`. A muted-but-not-dim span emits `90;49` with no `2`, and on
/// a terminal whose bright-black is bright it does not recede at all — so the
/// same word was being rendered two ways in one frame.
#[test]
fn the_row_meta_is_actually_dim_not_just_muted() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let spans = &frame.lines()[1].spans;

    for text in ["[prod]", "(deploy@10.0.0.4:22)"] {
        let span = span_with(spans, text);
        assert!(
            span.style.add_modifier.contains(Modifier::DIM),
            "meta {text:?} is muted but not DIM — it will not recede on a terminal \
             with a bright bright-black: {:?}",
            span.style
        );
    }
}

/// The rail, the state copy and the hint rail all put their content in the same
/// column.
///
/// Rows spend four columns before their text (`│ ` + cursor + ` `). When the
/// state and hint lines spent only two, the copy sat two columns left of the
/// rows it belonged to and the frame read as two misaligned blocks.
#[test]
fn every_rail_line_puts_its_content_in_the_same_column() {
    let cases = [
        (conns(), "", FrameMode::Pick),
        (conns(), "web", FrameMode::Manage),
        (vec![], "", FrameMode::Pick),
        (conns(), "zzz", FrameMode::Manage),
    ];

    for (connections, query, mode) in cases {
        let frame = build_frame(&connections, query, 0, mode, canvas(80));
        for line in frame.lines() {
            let text = line_text(line);
            if !text.starts_with('│') {
                continue;
            }
            // Display columns, not bytes: `│` is three bytes wide.
            let column = 1 + text
                .chars()
                .skip(1)
                .take_while(|c| *c == ' ' || *c == '❯')
                .count();
            assert_eq!(
                column, 4,
                "{mode:?} query={query:?}: rail content starts at column {column}, \
                 not 4 — the gutters are misaligned: {text:?}"
            );
        }
    }
}

/// Look up the span carrying exactly `text`.
fn span_with<'a>(
    spans: &'a [ratatui::text::Span<'static>],
    text: &str,
) -> &'a ratatui::text::Span<'static> {
    spans.iter().find(|s| s.content == text).unwrap_or_else(|| {
        panic!(
            "no span {text:?} in {:?}",
            spans.iter().map(|s| s.content.as_ref()).collect::<Vec<_>>()
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Filtering
// ─────────────────────────────────────────────────────────────────────────────

/// Typing narrows the frame to the Connections that match (user story 5).
#[test]
fn the_frame_shows_only_the_connections_matching_the_query() {
    let frame = build_frame(&conns(), "db", 0, FrameMode::Pick, canvas(80));
    let rows = data_rows(&frame);

    assert_eq!(rows.len(), 1, "only db-01 matches \"db\": {rows:?}");
    assert!(rows[0].contains("db-01"), "wrong row shown: {rows:?}");
}

/// The frame's row lines: rail lines that are neither the hint rail nor the
/// closing corner.
fn data_rows(frame: &sshm::frame::Frame) -> Vec<String> {
    frame_text(frame)
        .into_iter()
        .skip(1)
        .filter(|line| line.starts_with("│ ") && !line.contains("Esc cancel"))
        .collect()
}

/// The cursor marks the selected row with a glyph, and it moves when the
/// selection does — so selection is legible with the colour turned off.
#[test]
fn the_cursor_moves_with_the_selection() {
    let first = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let second = build_frame(&conns(), "", 1, FrameMode::Pick, canvas(80));

    assert_eq!(cursor_row(&first), Some(0));
    assert_eq!(cursor_row(&second), Some(1));
}

/// Which row line (0-based, relative to the first row) carries the `❯`.
fn cursor_row(frame: &sshm::frame::Frame) -> Option<usize> {
    frame
        .lines()
        .iter()
        .skip(1)
        .position(|line| line_text(line).contains('❯'))
}

// ─────────────────────────────────────────────────────────────────────────────
// Fuzzy-hit highlighting
// ─────────────────────────────────────────────────────────────────────────────

/// The characters that matched the query carry the highlight role, so the row
/// explains *why* it is in the list (user story 6). Asserted on the span
/// contents, not on colour: the test says which characters got the attribute.
#[test]
fn a_match_in_the_alias_highlights_exactly_the_matched_characters() {
    let frame = build_frame(&conns(), "web", 0, FrameMode::Pick, canvas(80));

    assert_eq!(
        highlighted_text(&frame),
        vec!["web"],
        "the matched run should be split out of the alias as its own span"
    );
    assert!(
        highlighted_spans(&frame)
            .iter()
            .all(|s| s.style.add_modifier.contains(Modifier::BOLD)),
        "a hit must stay bold so it survives a colourless terminal"
    );
}

/// A match in the folder meta splits the `[folder]` segment rather than
/// replacing it — the brackets stay, the matched name lights up.
#[test]
fn a_match_in_the_folder_highlights_the_name_inside_the_brackets() {
    let frame = build_frame(&conns(), "prod", 0, FrameMode::Pick, canvas(80));

    assert_eq!(
        highlighted_text(&frame),
        vec!["prod"],
        "only the folder name lights up, never the brackets around it"
    );
    assert_eq!(
        frame_text(&frame)[1],
        "│ ❯ [prod] web-01 (deploy@10.0.0.4:22)",
        "the row text must be unchanged by highlighting"
    );
}

/// The content of every span drawn with the highlight role.
fn highlighted_spans(frame: &sshm::frame::Frame) -> Vec<&ratatui::text::Span<'static>> {
    frame
        .lines()
        .iter()
        .flat_map(|l| l.spans.iter())
        .filter(|s| s.style.fg == Some(t().highlight))
        .collect()
}

fn highlighted_text(frame: &sshm::frame::Frame) -> Vec<String> {
    highlighted_spans(frame)
        .iter()
        .map(|s| s.content.to_string())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// The hint rail
// ─────────────────────────────────────────────────────────────────────────────

/// The manage frame names itself and offers the management chords, ordered
/// escape hatch → Enter → movement → management (user stories 23 and 47-50).
#[test]
fn manage_frame_hints_lead_with_the_escape_hatch_and_end_with_the_chords() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Manage, wide());

    assert_eq!(line_text(&frame.lines()[0]), "◆ Manage Connections");
    assert_eq!(
        hint_rail(&frame),
        "│   Esc cancel · Enter edit · ↑↓ navigate · Ctrl+A add · Ctrl+E edit · Ctrl+X delete"
    );
}

/// The pick frame keeps the same order but has no chords; the management entry
/// point is a pointer to the command instead (spec: "`sshm manage to add or
/// edit` is a dim line in the picker's hint rail").
#[test]
fn pick_frame_hints_lead_with_the_escape_hatch_and_point_at_manage() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, wide());

    assert_eq!(
        hint_rail(&frame),
        "│   Esc cancel · Enter select · ↑↓ navigate · sshm manage to add or edit"
    );
}

/// The hint rail is a dim line: it must never out-shout the rows above it.
#[test]
fn the_hint_rail_is_dim() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Manage, wide());
    let rail = hint_spans(&frame);

    assert!(
        rail.iter()
            .all(|s| s.style.fg == Some(t().fg_muted)
                && s.style.add_modifier.contains(Modifier::DIM)),
        "every hint-rail span should be muted and dim, got {:?}",
        rail.iter()
            .map(|s| (s.content.as_ref(), s.style))
            .collect::<Vec<_>>()
    );
}

/// The full Manage rail is 84 columns and cannot fit an 80-column terminal.
///
/// Before the rail was fitted it was emitted whole and the terminal clipped the
/// tail mid-word, so `Ctrl+X delete` arrived as `Ctrl+X dele`. Fitted, the
/// least-needed segment goes and every surviving segment arrives intact.
#[test]
fn the_hint_rail_never_overflows_the_canvas() {
    for width in [40usize, 60, 80, 100, 120] {
        for mode in [FrameMode::Pick, FrameMode::Manage] {
            let frame = build_frame(&conns(), "", 0, mode, canvas(width));
            let rail = frame
                .lines()
                .iter()
                .find(|l| line_text(l).contains("Esc cancel"))
                .expect("frame has a hint rail");
            assert!(
                line_width(rail) <= width,
                "{mode:?} at {width} columns: hint rail is {} wide and would be clipped: {:?}",
                line_width(rail),
                line_text(rail)
            );
        }
    }
}

/// A narrow terminal keeps the escape hatch and loses the tail — on a segment
/// boundary, never mid-word.
#[test]
fn a_narrow_canvas_keeps_the_escape_hatch_and_drops_the_tail_whole() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Manage, canvas(60));
    let rail = line_text(
        frame
            .lines()
            .iter()
            .find(|l| line_text(l).contains("Esc cancel"))
            .expect("frame has a hint rail"),
    );

    assert!(
        rail.starts_with("│   Esc cancel"),
        "the escape hatch must survive a narrow terminal, got {rail:?}"
    );
    assert!(
        rail.contains("Enter edit"),
        "Enter is the primary action and must survive: {rail:?}"
    );
    assert!(
        !rail.contains("Ctrl+X delete"),
        "the least-needed segment should have been dropped at 60 columns: {rail:?}"
    );
    assert!(
        !rail.ends_with('·') && !rail.ends_with(' ') && !rail.ends_with("dele"),
        "the rail must end on a segment boundary, not mid-word or with a dangling \
         separator: {rail:?}"
    );
}

/// The `sshm manage to add or edit` pointer is the first thing a narrow pick
/// frame gives up — the user's own movement and cancel matter more than the
/// discovery line (#39 owns the ordering rule; this pins that the frame
/// already honours it).
#[test]
fn a_narrow_pick_canvas_drops_the_manage_discovery_line_first() {
    let roomy = build_frame(&conns(), "", 0, FrameMode::Pick, wide());
    let roomy = hint_rail(&roomy);
    assert!(
        roomy.contains("sshm manage to add or edit"),
        "the discovery line is shown when there is room: {roomy:?}"
    );

    let tight = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(60));
    let tight = hint_rail(&tight);
    assert!(
        !tight.contains("sshm manage to add or edit"),
        "the discovery line must be the first thing dropped: {tight:?}"
    );
    assert!(
        tight.contains("Esc cancel") && tight.contains("↑↓ navigate"),
        "cancel and movement out-rank discovery and must survive: {tight:?}"
    );
}

/// The hint rail's text, or a panic if the frame has none.
fn hint_rail(frame: &sshm::frame::Frame) -> String {
    line_text(find_rail(frame))
}

fn find_rail(frame: &sshm::frame::Frame) -> &ratatui::text::Line<'static> {
    frame
        .lines()
        .iter()
        .find(|l| line_text(l).contains("Esc cancel"))
        .expect("frame has a hint rail")
}

fn hint_spans(frame: &sshm::frame::Frame) -> Vec<&ratatui::text::Span<'static>> {
    find_rail(frame).spans.iter().skip(1).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Colour degradation through the seam
// ─────────────────────────────────────────────────────────────────────────────

/// The seam resolves the palette against the canvas, so `NO_COLOR` reaches the
/// bytes.
///
/// This is the shape #34 depends on: the colour decision is an *input*, so
/// honouring `NO_COLOR` is a matter of passing a different `Canvas` rather
/// than changing `build_frame`'s signature.
#[test]
fn a_monochrome_canvas_emits_no_colour_at_all() {
    let mono = Canvas::new(80, ColorSupport::Monochrome);

    for (connections, query, mode) in [
        (conns(), "", FrameMode::Pick),
        (conns(), "web", FrameMode::Manage),
        (vec![], "", FrameMode::Pick),
        (conns(), "zzz", FrameMode::Manage),
    ] {
        let ansi = build_frame(&connections, query, 0, mode, mono).to_ansi();
        let colours: Vec<String> = sgr_params(&ansi)
            .into_iter()
            .filter(|p| !matches!(p.as_str(), "0" | "1" | "2" | "39" | "49"))
            .collect();
        assert!(
            colours.is_empty(),
            "{mode:?} query={query:?} under Monochrome emitted colour parameters \
             {colours:?} — the whole palette must downgrade (user story 9)"
        );
    }
}

/// Degradation is not deletion: the monochrome frame still says everything the
/// colour frame says.
#[test]
fn the_monochrome_frame_keeps_every_glyph_and_modifier_that_carries_state() {
    let mono = build_frame(
        &conns(),
        "web",
        0,
        FrameMode::Pick,
        Canvas::new(80, ColorSupport::Monochrome),
    )
    .to_ansi();

    assert!(mono.contains('◆'), "the step icon vanished under NO_COLOR");
    assert!(mono.contains('│'), "the rail vanished under NO_COLOR");
    assert!(mono.contains('❯'), "the cursor vanished under NO_COLOR");
    assert!(mono.contains('└'), "the corner vanished under NO_COLOR");
    assert!(mono.contains("Esc cancel"), "the escape hatch vanished");
    assert!(
        mono.contains("\x1b[1;"),
        "bold (the alias, the cursor) vanished under NO_COLOR: {mono:?}"
    );
    assert!(
        mono.contains("\x1b[2;") || mono.contains(";2;"),
        "DIM (the meta, the hint rail) vanished under NO_COLOR: {mono:?}"
    );
}

/// `NO_COLOR` and `TERM=dumb` both mean "suppress colour", and both are the
/// canvas the frame is handed.
///
/// Asserted against the pure env mapping rather than the live environment:
/// mutating a process-global that every other test in this binary reads would
/// make the suite flaky, and the rule needs to be provable, not hopeful.
#[test]
fn no_color_and_dumb_terminals_both_resolve_to_monochrome() {
    use std::ffi::OsStr;

    assert_eq!(
        ColorSupport::from_env_vars(Some(OsStr::new("1")), "xterm-256color", "truecolor"),
        ColorSupport::Monochrome,
        "NO_COLOR set must win over every other capability signal"
    );
    assert_eq!(
        ColorSupport::from_env_vars(None, "dumb", "truecolor"),
        ColorSupport::Monochrome,
        "TERM=dumb must suppress colour even if COLORTERM claims truecolour"
    );
    assert_eq!(
        ColorSupport::from_env_vars(Some(OsStr::new("")), "xterm-256color", "truecolor"),
        ColorSupport::Truecolor,
        "an empty NO_COLOR is not a request to suppress colour (no-color.org)"
    );
    assert_eq!(
        ColorSupport::from_env_vars(None, "xterm-256color", ""),
        ColorSupport::Ansi256
    );
    assert_eq!(
        ColorSupport::from_env_vars(None, "xterm", ""),
        ColorSupport::Ansi16
    );
}

/// The Clack palette survives the 16-colour downgrade without collapsing the
/// roles that carry state.
///
/// `fg_muted` and `border` are the same colour in Clack by design — the rail
/// and the meta are meant to recede together — but the accent that marks the
/// cursor and the green that marks a hit must stay distinct from each other and
/// from the rail, or selection and matching become invisible.
#[test]
fn clack_degrades_to_16_colours_without_losing_the_state_carrying_roles() {
    let resolved = Theme::clack().resolve(ColorSupport::Ansi16);

    assert_eq!(resolved.accent, Color::Cyan, "accent drifted off cyan");
    assert_eq!(
        resolved.highlight,
        Color::Green,
        "highlight drifted off green"
    );
    assert_ne!(
        resolved.accent, resolved.highlight,
        "the cursor marker and the match marker collapsed onto one colour"
    );
    assert_ne!(
        resolved.accent, resolved.border,
        "the cursor collapsed onto the rail — selection is invisible"
    );
    assert_ne!(
        resolved.highlight, resolved.border,
        "the match highlight collapsed onto the rail"
    );
}

/// Every colour mode the canvas can name produces a frame that still renders.
#[test]
fn every_canvas_colour_mode_still_renders_the_grammar() {
    for support in [
        ColorSupport::Truecolor,
        ColorSupport::Ansi256,
        ColorSupport::Ansi16,
        ColorSupport::Monochrome,
    ] {
        let ansi = build_frame(
            &conns(),
            "web",
            0,
            FrameMode::Manage,
            Canvas::new(80, support),
        )
        .to_ansi();
        for glyph in ['◆', '│', '❯', '└'] {
            assert!(
                ansi.contains(glyph),
                "{support:?}: glyph {glyph:?} missing from the rendered frame"
            );
        }
    }
}
// ─────────────────────────────────────────────────────────────────────────────
// The empty state
// ─────────────────────────────────────────────────────────────────────────────

/// With nothing to show, the frame keeps its grammar — header, rail, hint rail —
/// and gives the user a call to action instead of a blank box (user story 29).
#[test]
fn an_empty_connection_set_renders_a_call_to_action_behind_the_rail() {
    let frame = build_frame(&[], "", 0, FrameMode::Pick, canvas(80));

    assert_eq!(frame.state(), FrameState::Empty);
    assert_eq!(
        line_text(&frame.lines()[1]),
        "│   No Connections yet — run sshm manage to add one"
    );
    assert_eq!(
        frame.lines().len(),
        4,
        "empty frame is header + message + hint rail + corner, got {:?}",
        frame_text(&frame)
    );
}

/// The call to action is mode-specific: a manage user has the chord in the
/// frame, a pick user does not.
#[test]
fn the_manage_empty_state_points_at_the_add_chord() {
    let frame = build_frame(&[], "", 0, FrameMode::Manage, canvas(80));

    assert_eq!(frame.state(), FrameState::Empty);
    assert_eq!(
        line_text(&frame.lines()[1]),
        "│   No Connections yet — Ctrl+A to add one"
    );
}

/// The empty message is a state line, so it is muted — and it is the glyph and
/// the words that carry the state, not a colour.
#[test]
fn the_empty_message_is_muted() {
    let frame = build_frame(&[], "", 0, FrameMode::Pick, canvas(80));
    let message = &frame.lines()[1].spans[1];

    assert_eq!(message.style.fg, Some(t().fg_muted));
}

// ─────────────────────────────────────────────────────────────────────────────
// The no-match state
// ─────────────────────────────────────────────────────────────────────────────

/// Connections exist but none match: the frame echoes the query so the user can
/// see what they typed, and keeps the rail and the escape hatch.
#[test]
fn a_query_matching_nothing_names_the_query_it_could_not_match() {
    let frame = build_frame(&conns(), "zzz", 0, FrameMode::Pick, canvas(80));

    assert_eq!(frame.state(), FrameState::NoMatch);
    assert_eq!(line_text(&frame.lines()[1]), "│   No matches for \"zzz\"");
    assert_eq!(
        frame.lines().len(),
        4,
        "no-match frame is header + message + hint rail + corner, got {:?}",
        frame_text(&frame)
    );
}

/// There is nothing to point at, so there is no cursor — a `❯` over a message
/// would promise a selectable row that does not exist.
#[test]
fn the_no_match_state_shows_no_cursor() {
    let frame = build_frame(&conns(), "zzz", 0, FrameMode::Pick, canvas(80));

    assert!(
        !frame_text(&frame).iter().any(|l| l.contains('❯')),
        "no-match frame must not mark a cursor: {:?}",
        frame_text(&frame)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The frame's constant structure
// ─────────────────────────────────────────────────────────────────────────────

/// Every frame — rows, empty or no-match, pick or manage — has the same
/// skeleton: `◆` header, body behind the `│` rail, hint rail, `└` corner
/// (user story 11).
#[test]
fn every_frame_has_the_same_skeleton() {
    let cases = [
        (conns(), "", FrameMode::Pick),
        (conns(), "web", FrameMode::Manage),
        (vec![], "", FrameMode::Pick),
        (conns(), "zzz", FrameMode::Manage),
    ];

    for (connections, query, mode) in cases {
        let frame = build_frame(&connections, query, 0, mode, canvas(80));
        let lines = frame_text(&frame);
        let label = format!("{mode:?} query={query:?}");

        assert!(lines[0].starts_with('◆'), "{label}: no ◆ header: {lines:?}");
        assert_eq!(
            lines.last().map(String::as_str),
            Some("└"),
            "{label}: frame does not close with the corner: {lines:?}"
        );
        assert!(
            lines[lines.len() - 2].contains("Esc cancel"),
            "{label}: the hint rail is not the line above the corner: {lines:?}"
        );
    }
}

/// The corner is chrome: it is drawn with the border role, so it never reads as
/// content or as state.
#[test]
fn the_corner_is_chrome() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let corner = frame.lines().last().expect("frame has a corner");

    assert_eq!(corner.spans.len(), 1);
    assert_eq!(corner.spans[0].content, "└");
    assert_eq!(corner.spans[0].style.fg, Some(t().border));
}

// ─────────────────────────────────────────────────────────────────────────────
// Transparency
// ─────────────────────────────────────────────────────────────────────────────

/// The frame borrows the terminal background and never paints one.
///
/// This is the whole difference between the inline frame and a panel: a painted
/// background would cover the user's own terminal and would have to be
/// contrast-checked against a surface it cannot see. No span in any frame sets
/// a background, in any state, in any mode.
#[test]
fn no_span_in_any_frame_sets_a_background() {
    let cases = [
        (conns(), "", FrameMode::Pick),
        (conns(), "web", FrameMode::Manage),
        (vec![], "", FrameMode::Pick),
        (conns(), "zzz", FrameMode::Manage),
    ];

    for (connections, query, mode) in cases {
        let frame = build_frame(&connections, query, 0, mode, canvas(80));
        for line in frame.lines() {
            for span in &line.spans {
                assert_eq!(
                    span.style.bg, None,
                    "{mode:?} query={query:?}: span {:?} sets a background — the \
                     frame must stay transparent",
                    span.content
                );
            }
        }
    }
}

/// The same rule read off the serialized bytes: no SGR parameter asks the
/// terminal for a background colour.
#[test]
fn the_serialized_frame_asks_for_no_background_colour() {
    let ansi = build_frame(&conns(), "web", 0, FrameMode::Pick, canvas(80)).to_ansi();

    for param in sgr_params(&ansi) {
        let paints_background = ("40"..="47").contains(&param.as_str()) || param.starts_with("48;");
        assert!(
            !paints_background,
            "frame emitted a background parameter {param:?} — it must stay transparent"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ANSI serialization
// ─────────────────────────────────────────────────────────────────────────────

/// A frame can be serialized to ANSI and looked at in a real terminal. The
/// codes asserted here are the SGR standard's, matched to the palette the frame
/// is supposed to draw with.
#[test]
fn a_frame_serializes_to_ansi_carrying_its_palette() {
    let ansi = build_frame(&conns(), "web", 0, FrameMode::Pick, canvas(80)).to_ansi();
    let params: Vec<String> = sgr_params(&ansi);

    assert!(
        params.contains(&"36".into()),
        "no cyan (36) for the accent: {params:?}"
    );
    assert!(
        params.contains(&"32".into()),
        "no green (32) for a fuzzy hit: {params:?}"
    );
    assert!(
        params.contains(&"90".into()),
        "no bright black (90) for the muted meta: {params:?}"
    );
    assert!(
        params.contains(&"1".into()),
        "no bold (1) for the alias: {params:?}"
    );
    assert!(
        params.contains(&"2".into()),
        "no dim (2) for the hint rail: {params:?}"
    );
    assert!(
        params.contains(&"49".into()),
        "no default background (49) — the frame must emit the default, not a colour: {params:?}"
    );
    assert!(
        ansi.contains('◆') && ansi.contains('│') && ansi.contains('❯') && ansi.contains('└'),
        "the grammar glyphs did not survive serialization"
    );
}

/// Every SGR parameter list emitted by an ANSI string.
fn sgr_params(ansi: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = ansi.chars().peekable();

    while let Some(c) = chars.next() {
        if c != '\x1b' {
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            let mut param = String::new();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
                param.push(c);
            }
            out.extend(param.split(';').map(str::to_string));
        }
    }

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// What the frame is showing
// ─────────────────────────────────────────────────────────────────────────────

/// The frame filters internally, so it has to report what it kept: a consumer
/// acting on the selection (Enter, #34) maps a row back to a Connection
/// through the frame rather than by re-running the filter and hoping to agree.
#[test]
fn the_frame_reports_which_connections_it_is_showing() {
    let frame = build_frame(&conns(), "db", 0, FrameMode::Pick, canvas(80));

    assert_eq!(frame.matched(), &[1], "db-01 is connections[1]");
    assert_eq!(frame.selection(), 0);
    assert_eq!(frame.selected_connection_index(), Some(1));
}

/// A selection past the end lands on the last row rather than pointing at
/// nothing — the frame always has a target while it has rows.
#[test]
fn the_selection_is_clamped_to_the_rows_that_exist() {
    let frame = build_frame(&conns(), "db", 7, FrameMode::Pick, canvas(80));

    assert_eq!(
        frame.selection(),
        0,
        "a selection past the last row lands on the last row"
    );
    assert_eq!(frame.selected_connection_index(), Some(1));
}

/// With no rows there is nothing to select, and the frame says so rather than
/// returning a dangling index.
#[test]
fn a_frame_with_no_rows_selects_nothing() {
    assert_eq!(
        build_frame(&[], "", 0, FrameMode::Pick, canvas(80)).selected_connection_index(),
        None
    );
    assert_eq!(
        build_frame(&conns(), "zzz", 0, FrameMode::Manage, canvas(80)).selected_connection_index(),
        None
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The emphasis vocabulary
// ─────────────────────────────────────────────────────────────────────────────

/// The frame emphasises with `BOLD`, `DIM` and `UNDERLINED` and nothing else.
///
/// Those three survive a 16-colour terminal, `NO_COLOR`, and every colour-vision
/// deficiency — which is why the palette leans on modifiers instead of hues.
/// An italic or a reversed run would be a different claim about the surface, so
/// it has to be a deliberate change to this rule rather than an accident.
#[test]
fn the_frame_uses_only_the_allowed_modifiers() {
    let allowed = Modifier::BOLD | Modifier::DIM | Modifier::UNDERLINED;
    let cases = [
        (conns(), "", FrameMode::Pick),
        (conns(), "web", FrameMode::Manage),
        (vec![], "", FrameMode::Pick),
        (conns(), "zzz", FrameMode::Manage),
    ];

    for (connections, query, mode) in cases {
        let frame = build_frame(&connections, query, 0, mode, canvas(80));
        for line in frame.lines() {
            for span in &line.spans {
                let extra = span.style.add_modifier.difference(allowed);
                assert!(
                    extra.is_empty(),
                    "{mode:?} query={query:?}: span {:?} uses {extra:?}, outside the \
                     allowed BOLD/DIM/UNDERLINED",
                    span.content
                );
            }
        }
    }
}
