//! The inline frame's pure view-model (#33).
//!
//! `build_frame` turns `(connections, query, selection, mode)` into a `Frame`:
//! a concrete list of styled lines carrying the Clack grammar — a `◆` step
//! icon, a `│` left rail, a `❯` cursor, rows as `[folder] alias
//! (user@host:port)`, and the empty / no-match / hint-rail states.
//!
//! It is a value, not a terminal session. Nothing here opens a terminal, reads
//! the environment, or touches ratatui's render loop, which is what makes the
//! whole inline surface testable by asserting on spans, and demoable by
//! serializing a frame to ANSI and `cat`-ing it. The inline renderer (#34) and
//! the command cut-over (#35) are consumers of this value; neither is built
//! here.
//!
//! Two rules bind every span produced here:
//!
//! 1. **Colour comes from a `theme.rs` role, never a literal.** The frame draws
//!    with [`Theme::clack`], the named-ANSI palette.
//! 2. **The frame is transparent.** It borrows the terminal background and never
//!    paints one — no span here sets a background. Selection is carried by the
//!    `❯` glyph and a bold alias, not by a filled row.

use crate::config::Connection;
use crate::picker::compute_matches;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// What the frame is being used for.
///
/// The three commands of the redesign share one frame and differ by what Enter
/// does, so the difference has to be a domain concept on the seam rather than a
/// boolean a caller has to remember the polarity of. `Pick` is the
/// choose-a-Connection frame (bare `sshm` and `sshm pick`); `Manage` is
/// `sshm manage`, where Enter edits and the Ctrl chords add and delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMode {
    /// Choose a Connection.
    Pick,
    /// Add, edit and delete Connections.
    Manage,
}

/// Which layout the frame took, given its inputs.
///
/// The three states are the whole of the frame's branching, and each is carried
/// by a glyph rather than by colour, so the frame communicates the same thing
/// with the colour turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    /// There are no Connections at all.
    Empty,
    /// There are Connections, but none match the query.
    NoMatch,
    /// At least one row to show.
    Rows,
}

/// A rendered frame: the lines to print, top to bottom, rail included.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    lines: Vec<Line<'static>>,
    state: FrameState,
    matched: Vec<usize>,
    selection: usize,
}

impl Frame {
    /// Every line the frame draws, in order.
    pub fn lines(&self) -> &[Line<'static>] {
        &self.lines
    }

    /// Which layout the frame took.
    ///
    /// A consumer that has to branch on the frame (a settle trace, a test) reads
    /// this instead of re-deriving it from the line count.
    pub fn state(&self) -> FrameState {
        self.state
    }

    /// Which entries of the caller's `connections` slice the frame is showing,
    /// in display order.
    ///
    /// The frame does the filtering, so it is the only thing that knows what a
    /// visible row actually is. Without this, a consumer acting on the cursor
    /// would have to re-run the filter and hope it agreed.
    pub fn matched(&self) -> &[usize] {
        &self.matched
    }

    /// The row the cursor is on, clamped to the rows that exist.
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// The index into the caller's `connections` slice that the cursor points
    /// at, or `None` when the frame has no rows to point at.
    pub fn selected_connection_index(&self) -> Option<usize> {
        self.matched.get(self.selection).copied()
    }

    /// Serialize the frame to an ANSI string, one line per frame line.
    ///
    /// This is what makes a frame reviewable without a live terminal session:
    /// write it to a file, `cat` it, and the colours, bold and dim are what the
    /// user would see. Because no span carries a background, `cat`-ing the
    /// result leaves the terminal's own background untouched — the frame stays
    /// transparent all the way to the bytes.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();

        for line in &self.lines {
            let mut current: Option<Style> = None;
            for span in &line.spans {
                if current != Some(span.style) {
                    // Reset first so an attribute dropped between spans
                    // actually disappears rather than bleeding into the next run.
                    out.push_str("\x1b[0m");
                    out.push_str(&crate::theme::ansi::style_to_ansi(span.style));
                    current = Some(span.style);
                }
                out.push_str(&span.content);
            }
            out.push_str("\x1b[0m\n");
        }

        out
    }
}

/// Build the frame for a set of Connections, a live query, a selected row and a
/// mode.
pub fn build_frame(
    connections: &[Connection],
    query: &str,
    selection: usize,
    mode: FrameMode,
) -> Frame {
    let t = crate::theme::Theme::clack();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(connections, &matcher, query);

    let state = if connections.is_empty() {
        FrameState::Empty
    } else if matches.is_empty() {
        FrameState::NoMatch
    } else {
        FrameState::Rows
    };

    let matched: Vec<usize> = matches.iter().map(|(conn_idx, _, _)| *conn_idx).collect();
    let selection = selection.min(matched.len().saturating_sub(1));

    let mut lines = vec![header_line(mode, &t)];

    match state {
        FrameState::Empty => lines.push(state_line(empty_message(mode), &t)),
        FrameState::NoMatch => lines.push(state_line(&format!("No matches for {query:?}"), &t)),
        FrameState::Rows => {
            for (i, (conn_idx, _score, hits)) in matches.iter().enumerate() {
                lines.push(row_line(&connections[*conn_idx], i == selection, hits, &t));
            }
        }
    }

    lines.push(hint_rail_line(mode, &t));
    lines.push(corner_line(&t));

    Frame {
        lines,
        state,
        matched,
        selection,
    }
}

/// The `└` that closes the rail.
///
/// Chrome, not content: it is drawn with the border role so it reads as
/// structure and carries no state.
fn corner_line(t: &crate::theme::Theme) -> Line<'static> {
    Line::from(vec![Span::styled("└", Style::default().fg(t.border))])
}

/// What the empty frame tells the user to do next.
fn empty_message(mode: FrameMode) -> &'static str {
    match mode {
        FrameMode::Pick => "No Connections yet — run sshm manage to add one",
        FrameMode::Manage => "No Connections yet — Ctrl+A to add one",
    }
}

/// A state line: the `│` rail plus muted copy.
fn state_line(text: &str, t: &crate::theme::Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled("│", Style::default().fg(t.border)),
        Span::styled(format!(" {text}"), Style::default().fg(t.fg_muted)),
    ])
}

/// The dim hint line: the escape hatch first, then Enter, then movement, then
/// management.
///
/// The order is the contract, not a stylistic choice — on a narrow terminal the
/// tail gets dropped, so what is listed first is what survives (user story 23).
/// The chords only appear in `Manage`; a pick frame points at the command
/// instead, because its Enter chooses rather than edits.
fn hint_rail_line(mode: FrameMode, t: &crate::theme::Theme) -> Line<'static> {
    let hints: &[&str] = match mode {
        FrameMode::Pick => &[
            "Esc cancel",
            "Enter select",
            "↑↓ navigate",
            "sshm manage to add or edit",
        ],
        FrameMode::Manage => &[
            "Esc cancel",
            "Enter edit",
            "↑↓ navigate",
            "Ctrl+A add",
            "Ctrl+E edit",
            "Ctrl+X delete",
        ],
    };

    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

    let mut spans = vec![
        Span::styled("│", Style::default().fg(t.border)),
        Span::styled(" ", dim),
    ];

    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", dim));
        }
        spans.push(Span::styled(*hint, dim));
    }

    Line::from(spans)
}

/// The `◆ <question>` line that opens every frame.
fn header_line(mode: FrameMode, t: &crate::theme::Theme) -> Line<'static> {
    let title = match mode {
        FrameMode::Pick => "Select a Connection",
        FrameMode::Manage => "Manage Connections",
    };

    Line::from(vec![
        Span::styled("◆", Style::default().fg(t.accent)),
        Span::raw(" "),
        Span::styled(title, Style::default().add_modifier(Modifier::BOLD)),
    ])
}

/// The cursor column: `❯` on the selected row, a blank of the same width on
/// every other row so the rows stay aligned.
///
/// The glyph is what carries selection. A transparent frame has no background to
/// fill, so a hue alone could not mark the target of the next action — and a
/// hue would vanish under `NO_COLOR` anyway (user story 9).
fn cursor_span(selected: bool, t: &crate::theme::Theme) -> Span<'static> {
    if selected {
        Span::styled(
            "❯",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(" ")
    }
}

/// One Connection row: the `│` rail, the cursor column, then
/// `[folder] alias (user@host:port)`.
///
/// The folder segment is emitted only when the Connection has one, so rows
/// without a folder start at the alias and stay compact.
fn row_line(
    conn: &Connection,
    selected: bool,
    hits: &[usize],
    t: &crate::theme::Theme,
) -> Line<'static> {
    let meta = Style::default().fg(t.fg_muted);
    let alias = Style::default().add_modifier(Modifier::BOLD);
    let hit = Style::default()
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD);

    let mut spans = vec![
        Span::styled("│", Style::default().fg(t.border)),
        Span::raw(" "),
        cursor_span(selected, t),
        Span::raw(" "),
    ];

    // Segment offsets are byte offsets into the row's display text — the same
    // coordinate system `compute_matches` reports hit positions in, so a hit
    // needs no translation to land on the right character here.
    let mut pos = 0usize;

    if let Some(folder) = conn.folder.as_deref().filter(|f| !f.is_empty()) {
        let seg = format!("[{folder}]");
        push_split(&mut spans, &seg, pos, meta, hit, hits);
        pos += seg.len();
        spans.push(Span::raw(" "));
        pos += 1;
    }

    push_split(&mut spans, &conn.alias, pos, alias, hit, hits);
    pos += conn.alias.len();
    spans.push(Span::raw(" "));
    pos += 1;

    let meta_seg = format!("({}@{}:{})", conn.user, conn.host, conn.port);
    push_split(&mut spans, &meta_seg, pos, meta, hit, hits);

    Line::from(spans)
}

/// Append `text` as one span per run of matched / unmatched characters.
///
/// `start` is where `text` begins in the row's display coordinates; `hits` are
/// the matched byte offsets for this row. Runs are grouped rather than emitted
/// per character so a row stays a handful of spans instead of hundreds.
fn push_split(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    start: usize,
    base: Style,
    hit: Style,
    hits: &[usize],
) {
    let mut buf = String::new();
    let mut run: Option<bool> = None;

    for (i, ch) in text.char_indices() {
        let matched = hits.contains(&(start + i));
        if run != Some(matched) {
            if !buf.is_empty() {
                let style = if run == Some(true) { hit } else { base };
                spans.push(Span::styled(std::mem::take(&mut buf), style));
            }
            run = Some(matched);
        }
        buf.push(ch);
    }

    if !buf.is_empty() {
        let style = if run == Some(true) { hit } else { base };
        spans.push(Span::styled(buf, style));
    }
}
