//! The inline frame seam: `build_frame` is a pure view-model, so every test here
//! asserts on the `Frame` value it returns — no terminal, no PTY, no ratatui
//! `Frame`. The frame is a concrete span value; that is the whole point of the
//! seam (#33).
//!
//! Fixture rows are the worked examples from the parent spec (#31): the settle
//! trace `◆ picked web-01 (deploy@10.0.0.4:22)` and the delete confirm
//! `◆ Delete [prod] web-01? (y/N)` both name these Connections, so the
//! expectations below come from the spec rather than from the code under test.

use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use sshm::config::Connection;
use sshm::frame::{
    build_frame, build_row_text, compute_matches, visible_window, Canvas, FrameMode, FrameState,
    FRAME_LINES, VISIBLE_ROWS,
};
use sshm::theme::{ColorSupport, Theme};

/// The palette the frame draws with. Tests name the *role*, never the hue; the
/// hue itself is pinned by the palette tests in `design_system_test.rs`.
fn t() -> Theme {
    Theme::clack()
}

/// A truecolour canvas `width` columns wide, in a 24-row terminal — tall
/// enough for the full 8-row window.
fn canvas(width: usize) -> Canvas {
    Canvas::new(width, 24, ColorSupport::Truecolor)
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
///
/// Columns, not characters: a CJK glyph is one `char` and two columns. The
/// inline driver's collapse counts *physical* rows, so every width decision
/// has to be made in the unit the terminal actually spends.
fn line_width(line: &Line<'static>) -> usize {
    line.width()
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
            // A padded list-area row carries the rail and nothing else, so
            // there is no content to put in a column (#34's constant height).
            if text.chars().skip(1).all(|c| c == ' ') {
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

/// The byte offsets into a row's display text that the frame painted with the
/// highlight role.
///
/// `gutter` is how many bytes the line spends before its row text starts
/// (`│ ` + cursor + ` `), so the offsets that come back are in
/// [`build_row_text`] coordinates — the same coordinates
/// [`compute_matches`] reports hits in.
fn highlighted_offsets_in_row(line: &Line<'static>, gutter: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    for span in &line.spans {
        let start = pos.saturating_sub(gutter);
        if span.style.fg == Some(t().highlight) {
            out.extend(start..start + span.content.len());
        }
        pos += span.content.len();
    }

    out.sort_unstable();
    out
}

/// The row the frame draws **is** [`build_row_text`] — not a second copy of
/// the same shape.
///
/// This is the half of the pairing that used to be missing. The highlight
/// indices `compute_matches` returns are offsets into `build_row_text`, but
/// `row_line` formatted the Connection a second time by hand, so nothing
/// joined the two: the matcher's coordinate system and the drawn pixels could
/// drift apart while every rendered string still looked right. With one
/// function owning the text, an offset is an offset into what the user sees.
#[test]
fn the_rendered_row_is_the_row_text_the_offsets_index() {
    for conn in conns() {
        let frame = build_frame(std::slice::from_ref(&conn), "", 0, FrameMode::Pick, wide());
        let rendered = line_text(&frame.lines()[1]);
        let row = build_row_text(&conn);

        assert_eq!(
            rendered,
            format!("│ ❯ {row}"),
            "the selected row must be the rail, the cursor and build_row_text — \
             nothing else: {rendered:?} vs {row:?}"
        );
    }
}

/// Every highlighted column in a rendered row is a byte the matcher called,
/// counted in `build_row_text` — across the folder, the alias *and* the meta.
///
/// The two highlight tests above pin one shape each: a hit in the alias, a hit
/// in the folder. Neither covers the `user@host:port` tail, which is exactly
/// where a hand-counted segment offset in the renderer goes wrong — the drawn
/// text stays identical, so no string assertion notices, while the hit lands
/// one column off and lights up `eploy` instead of `deploy`.
#[test]
fn the_highlighted_columns_are_the_row_text_offsets_the_matcher_reports() {
    let matcher = SkimMatcherV2::default();

    for query in ["web", "prod", "deploy", "10.0.0", "db-01", "pos"] {
        let frame = build_frame(&conns(), query, 0, FrameMode::Pick, wide());
        assert!(
            !frame.matched().is_empty(),
            "query {query:?} matches nothing, so this test would assert nothing"
        );
        let conn_idx = frame.matched()[0];
        let conn = &conns()[conn_idx];
        let hits = compute_matches(&conns(), &matcher, query)
            .into_iter()
            .find(|(i, _, _)| *i == conn_idx)
            .map(|(_, _, hits)| hits)
            .unwrap_or_default();

        let rendered = line_text(&frame.lines()[1]);
        let row = build_row_text(conn);
        assert_eq!(
            rendered,
            format!("│ ❯ {row}"),
            "query {query:?}: the rendered row is not build_row_text, so its \
             offsets cannot describe it"
        );

        let gutter = rendered.len() - row.len();
        assert_eq!(
            highlighted_offsets_in_row(&frame.lines()[1], gutter),
            hits,
            "query {query:?}: the columns the frame highlighted are not the \
             offsets compute_matches reports into build_row_text"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The hint rail
// ─────────────────────────────────────────────────────────────────────────────

/// The manage frame names itself, leads with the escape hatch (user story 23),
/// and advertises the management chords — **only because #36 put handlers
/// behind them**.
///
/// This gate used to assert the opposite: that the manage frame carried **no**
/// Ctrl chords, because `run_inline` read no Ctrl key but `Ctrl+C` and a
/// hint for a key that does nothing is worse than no hint at all. The rule
/// was never "no Ctrl chords"; it was "never hint a key the frame does not
/// read". So the ban is replaced with the pairing it was protecting: every
/// `Ctrl+<letter>` the frame prints must be one `manage::step` acts on,
/// checked here rather than asserted by hand, so the hint cannot outlive its
/// handler or the handler lose its hint without this failing.
#[test]
fn manage_frame_hints_lead_with_the_escape_hatch_and_carry_only_live_chords() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Manage, wide());

    assert_eq!(line_text(&frame.lines()[0]), "◆ Manage Connections");
    assert_eq!(
        hint_rail(&frame),
        "│   Esc cancel · Enter edit · ↑↓ navigate · Ctrl+A add · Ctrl+E edit · Ctrl+X delete"
    );
    assert_advertised_chords_are_live(&frame);
}

/// Pull every `Ctrl+<letter>` the frame prints and ask the manage state
/// machine whether it acts on it.
///
/// The chord is exercised with a Connection selected, because that is the
/// situation the hint is aimed at. A chord that returns the state unchanged
/// and asks for nothing is a dead key, and a dead key must not be advertised.
fn assert_advertised_chords_are_live(frame: &sshm::frame::Frame) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use sshm::manage::{step, ManageState};

    let text = frame_text(frame).join("\n");
    let mut advertised: Vec<char> = Vec::new();
    let mut rest = text.as_str();
    while let Some(idx) = rest.find("Ctrl+") {
        let after = &rest[idx + "Ctrl+".len()..];
        if let Some(ch) = after.chars().find(|c| !c.is_whitespace()) {
            if ch.is_ascii_alphabetic() {
                advertised.push(ch.to_ascii_lowercase());
            }
        }
        rest = after;
    }

    assert!(
        !advertised.is_empty(),
        "no Ctrl chord advertised — this gate has gone blind, update it rather \
         than trust a vacuous pass. Frame: {text}"
    );

    for ch in advertised {
        let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
        let step = step(&ManageState::new(), key, Some(&conns()[0]));

        assert!(
            step.state != ManageState::new() || !step.effects.is_empty(),
            "the frame advertises Ctrl+{} but the manage state machine does \
             nothing with it: {:?}",
            ch.to_ascii_uppercase(),
            step.effects
        );
    }
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
    let mono = Canvas::new(80, 24, ColorSupport::Monochrome);

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
        Canvas::new(80, 24, ColorSupport::Monochrome),
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
            Canvas::new(80, 24, support),
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
        FRAME_LINES,
        "the empty frame is padded to the frame's constant height, got {:?}",
        frame_text(&frame)
    );
}

/// The call to action is mode-specific, and in both modes it points at
/// something that actually works. The manage frame used to say `Ctrl+A to add
/// one`; no handler reads that chord yet (#36), so the empty frame now names
/// the command that does add a Connection today.
#[test]
fn the_manage_empty_state_points_at_a_command_that_works() {
    let frame = build_frame(&[], "", 0, FrameMode::Manage, canvas(80));

    assert_eq!(frame.state(), FrameState::Empty);
    assert_eq!(
        line_text(&frame.lines()[1]),
        "│   No Connections yet — run sshm add to create one"
    );
    // The chords this frame advertises are live as of #36, so the empty
    // state is allowed to name them — and is checked for naming only live
    // ones by the same gate the populated manage frame goes through.
    assert_advertised_chords_are_live(&frame);
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
        FRAME_LINES,
        "the no-match frame is padded to the frame's constant height, got {:?}",
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

// ─────────────────────────────────────────────────────────────────────────────
// The sliding window (#34)
//
// The frame is a fixed-height window over the matched rows, so which rows are
// visible is a pure function of (total, selection, visible). The window is
// where the frame's "constant height" comes from: the list can be any length,
// the frame is always the same size.
// ─────────────────────────────────────────────────────────────────────────────

/// The selection sits four rows down from the top of an 8-row window whenever
/// there is room, so the rows above and below it both stay in view while the
/// user scrolls through a long list (user story 12).
#[test]
fn the_window_recentres_the_selection_in_the_middle_of_the_view() {
    assert_eq!(visible_window(20, 0, 8), 0..8);
    assert_eq!(visible_window(20, 4, 8), 0..8);
    assert_eq!(visible_window(20, 5, 8), 1..9);
    assert_eq!(visible_window(20, 10, 8), 6..14);
}

/// Near the ends of the list the window cannot stay centred, so it pins to the
/// end rather than scrolling past it — the frame never shows rows that do not
/// exist.
#[test]
fn the_window_pins_to_the_ends_instead_of_scrolling_past_the_list() {
    assert_eq!(visible_window(20, 19, 8), 12..20);
    assert_eq!(visible_window(20, 12, 8), 8..16);
    assert_eq!(visible_window(8, 7, 8), 0..8);
}

/// A list shorter than the window is shown whole, from the top. The window is
/// never padded with rows that do not exist — padding is the frame's job, not
/// the window's.
#[test]
fn a_short_list_is_shown_whole_from_the_top() {
    assert_eq!(visible_window(3, 0, 8), 0..3);
    assert_eq!(visible_window(3, 2, 8), 0..3);
    assert_eq!(visible_window(0, 0, 8), 0..0);
}

/// A frame built for a terminal that cannot hold 8 rows shows fewer, but still
/// one height for every query — the window shrinks, the frame does not jitter.
#[test]
fn a_short_terminal_shrinks_the_window_not_the_rule() {
    let all = many_conns(20);
    let short = Canvas::new(80, 9, ColorSupport::Truecolor); // 9 rows: 8 visible - 1
    let frame = build_frame(&all, "", 12, FrameMode::Pick, short);

    assert_eq!(
        data_rows(&frame).len(),
        5,
        "a 9-row terminal leaves 5 list rows after the chrome and the prompt line"
    );
    assert_eq!(frame.lines().len(), 1 + 5 + 1 + 1);
}

/// The rows the frame emits are the window's rows, not the whole list: at
/// selection 12 of 20 the frame shows rows 8..16 with the cursor four rows
/// down from the top of the list area (user story 12).
#[test]
fn the_frame_emits_the_windowed_rows_with_the_cursor_centred() {
    let all = many_conns(20);
    let frame = build_frame(&all, "", 12, FrameMode::Pick, canvas(80));
    let rows = data_rows(&frame);

    assert_eq!(
        rows.len(),
        VISIBLE_ROWS,
        "the frame shows the {VISIBLE_ROWS}-row window, got {rows:?}"
    );
    assert!(
        rows[0].contains("c-08"),
        "the window starts at row 8, got {rows:?}"
    );
    assert!(
        rows[4].contains('❯') && rows[4].contains("c-12"),
        "the selection sits four rows down from the top of the window: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r.contains("c-16")),
        "rows past the window must not be emitted: {rows:?}"
    );
}

/// The frame is the same height whatever the query matches: the list area is a
/// fixed 8-row window padded with empty rail rows, so the hint rail and the
/// corner never move as the list narrows or empties (user story 10).
#[test]
fn the_frame_is_the_same_height_whatever_the_query_matches() {
    let all = many_conns(20);

    let roomy = build_frame(&all, "", 0, FrameMode::Pick, canvas(80));
    let narrow = build_frame(&all, "c-03", 0, FrameMode::Pick, canvas(80));
    let none = build_frame(&all, "zzzznothing", 0, FrameMode::Pick, canvas(80));
    let empty = build_frame(&[], "", 0, FrameMode::Pick, canvas(80));

    for (label, frame) in [
        ("20 matches", &roomy),
        ("one match", &narrow),
        ("no match", &none),
        ("no Connections", &empty),
    ] {
        assert_eq!(
            frame.lines().len(),
            FRAME_LINES,
            "{label}: the frame is {} lines, not the constant {FRAME_LINES}: {:?}",
            frame.lines().len(),
            frame_text(frame)
        );
    }
}

/// The padding is rail, not blank space: the `│` runs the full height of the
/// frame so the corner closes a continuous rail whatever the list is doing.
#[test]
fn the_padded_rows_keep_the_rail_continuous() {
    let frame = build_frame(&conns(), "", 0, FrameMode::Pick, canvas(80));
    let body = &frame.lines()[1..frame.lines().len() - 2];

    assert_eq!(
        body.len(),
        VISIBLE_ROWS,
        "the list area is {VISIBLE_ROWS} rows: {:?}",
        frame_text(&frame)
    );
    for line in body {
        assert!(
            line_text(line).starts_with('│'),
            "every list-area row carries the rail, got {:?}",
            line_text(line)
        );
    }
}

/// Fixture of `n` Connections, named `c-00..` so a window slice can be read
/// straight out of the row text.
fn many_conns(n: usize) -> Vec<Connection> {
    (0..n)
        .map(|i| Connection {
            id: format!("{i}"),
            alias: format!("c-{i:02}"),
            host: format!("10.0.0.{i}"),
            user: "deploy".into(),
            port: 22,
            key_path: None,
            folder: None,
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// The width invariant (#34)
//
// The inline driver collapses the frame by row-diffing it, and that arithmetic
// only works if one frame line is one physical terminal row. A line wider than
// the canvas wraps, and a wrapped line breaks both the driver's row count and
// the frame's constant height — so no line the frame emits may exceed the
// canvas width, whatever the Connection data says.
// ─────────────────────────────────────────────────────────────────────────────

/// A Connection whose every field is long enough to blow past a narrow canvas.
fn bloated() -> Vec<Connection> {
    vec![Connection {
        id: "1".into(),
        alias: "the-longest-alias-this-connection-has-ever-carried".into(),
        host: "some.extremely.long.hostname.internal.example.com".into(),
        user: "a-user-with-a-long-name".into(),
        port: 2222,
        key_path: None,
        folder: Some("a-folder-name-for-completeness".into()),
    }]
}

/// Whatever the Connection data, every line of a `width`-column frame fits in
/// `width` columns. The hint rail already fitted itself; the rows must too.
#[test]
fn no_frame_line_is_wider_than_the_canvas() {
    for width in [40, 60, 80] {
        let frame = build_frame(&bloated(), "", 0, FrameMode::Pick, canvas(width));
        for line in frame.lines() {
            let w = line_width(line);
            assert!(
                w <= width,
                "a {width}-column canvas got a {w}-column line: {:?}",
                line_text(line)
            );
        }
    }
}

/// A Connection written in characters that occupy two terminal columns each.
/// `chars().count()` calls the alias 3 wide; the terminal spends 6. Every
/// width decision made in the wrong unit is a row that wraps and breaks the
/// driver's physical-row count.
fn wide_glyphs() -> Vec<Connection> {
    vec![Connection {
        id: "1".into(),
        alias: "服务器".into(),
        host: "10.0.0.4".into(),
        user: "deploy".into(),
        port: 22,
        key_path: None,
        folder: Some("生产".into()),
    }]
}

/// A wide-character alias is measured in columns, not characters: at every
/// width the frame fits inside, in the unit the terminal actually spends.
#[test]
fn a_wide_character_alias_is_measured_in_columns_not_characters() {
    for width in 8..=80usize {
        let frame = build_frame(&wide_glyphs(), "", 0, FrameMode::Pick, canvas(width));
        for line in frame.lines() {
            let w = line_width(line);
            assert!(
                w <= width,
                "a {width}-column canvas got a {w}-column line from a wide-char \
                 Connection: {:?}",
                line_text(line)
            );
        }
    }
}

/// The frame leaves one column of headroom against the canvas.
///
/// A narrowing terminal re-wraps any row that reaches its new right edge, and
/// a re-wrapped row is no longer one physical row — which is precisely the
/// case the resize fallback has to survive. Spending at most `width - 1`
/// columns means a one-column narrowing re-wraps nothing, so the drawn row
/// count stays the physical row count.
#[test]
fn every_frame_line_leaves_a_column_of_headroom_against_reflow() {
    for (label, connections) in [
        ("bloated", bloated()),
        ("wide glyphs", wide_glyphs()),
        ("normal", conns()),
    ] {
        for width in [40, 60, 80] {
            // The most a line may spend on a `width`-column terminal.
            let budget = width - 1;
            let frame = build_frame(&connections, "", 0, FrameMode::Pick, canvas(width));
            for line in frame.lines() {
                let w = line_width(line);
                assert!(
                    w <= budget,
                    "{label} at {width} columns: line is {w} wide, no headroom \
                     left against a narrowing reflow: {:?}",
                    line_text(line)
                );
            }
        }
    }
}

/// Truncation keeps the frame's shape: the rail still leads every body line,
/// the header and corner survive, and the row still starts with its cursor
/// column — it just loses its tail, not its grammar.
#[test]
fn a_fitted_row_keeps_its_rail_and_cursor_and_loses_only_its_tail() {
    let frame = build_frame(&bloated(), "", 0, FrameMode::Pick, canvas(40));
    let rows = data_rows(&frame);

    assert_eq!(rows.len(), 1, "one Connection, one row");
    assert!(
        rows[0].starts_with("│ ❯ [a-folder-name-for-completeness]"),
        "the row keeps its rail, cursor and folder prefix, got {:?}",
        rows[0]
    );
    assert!(
        !rows[0].contains("hostname"),
        "the over-long tail was cut, got {:?}",
        rows[0]
    );
}
