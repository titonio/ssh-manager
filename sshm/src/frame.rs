//! The inline frame's pure view-model (#33).
//!
//! `build_frame` turns `(connections, query, selection, mode, canvas)` into a
//! `Frame`: a concrete list of styled lines carrying the Clack grammar — a
//! `◆` step icon, a `│` left rail, a `❯` cursor, rows as `[folder] alias
//! (user@host:port)`, and the empty / no-match / hint-rail states.
//!
//! It is a value, not a terminal session. Nothing in the frame itself opens a
//! terminal or touches ratatui's render loop, which is what makes the whole
//! inline surface testable by asserting on spans, and demoable by serializing a
//! frame to ANSI and `cat`-ing it. The inline renderer (#34) and the command
//! cut-over (#35) are consumers of this value; neither is built here.
//!
//! The frame cannot see the terminal it is being drawn into, so the terminal is
//! an argument. [`Canvas`] carries the two facts about it that change what the
//! frame emits: how wide the hint rail has to fit inside, and how much colour
//! the palette has to degrade to. Both are inputs rather than constants baked
//! into `build_frame` because a seam that hardcodes them cannot honour
//! `NO_COLOR` or a 60-column terminal later without breaking its own signature
//! — and #34 has to do exactly that.
//!
//! Three rules bind every span produced here:
//!
//! 1. **Colour comes from a `theme.rs` role, never a literal.** The frame
//!    draws with [`Theme::clack`], downgraded through
//!    [`Theme::resolve`](crate::theme::Theme::resolve) by the canvas's
//!    [`ColorSupport`].
//! 2. **The frame is transparent.** It borrows the terminal background and never
//!    paints one — no span here sets a background. Selection is carried by the
//!    `❯` glyph and a bold alias, not by a filled row.
//! 3. **Glyphs and modifiers carry meaning, not hue.** Every state the frame can
//!    be in says the same thing with the colour turned off.

use crate::config::Connection;
use crate::picker::compute_matches;
use crate::theme::{ColorSupport, Theme};
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// The `│` rail glyph that every body line hangs off.
const RAIL: &str = "│";

/// The blank that pads a rail line out to the column row content starts at.
///
/// A row spends `│ ` + cursor + ` ` — four columns — before its first real
/// character. A line with no cursor has to spend the same four, or the state
/// copy sits two columns left of the rows it belongs to.
const GUTTER_PAD: &str = "   ";

/// Columns from the start of a rail line to where its content begins.
const GUTTER: usize = 1 + GUTTER_PAD.len();

/// What separates two hints in the hint rail.
const HINT_SEP: &str = " · ";

/// The lines of a frame that are not list rows: the `◆` header, the hint rail
/// and the `└` corner.
pub const CHROME_LINES: usize = 3;

/// The list rows a frame shows when the terminal has room for all of them.
///
/// Eight is the frame's shape, not a limit on the list: the list is a window
/// over as many Connections as match, and the window slides ([`visible_window`])
/// so the frame never has to grow to fit it.
pub const VISIBLE_ROWS: usize = 8;

/// The lines of a full frame: the 8-row list area plus the chrome around it.
///
/// The spec's "~12 lines" counts the blank line the frame opens on below the
/// prompt (#34); the frame itself is eleven.
pub const FRAME_LINES: usize = VISIBLE_ROWS + CHROME_LINES;

/// The line the frame leaves for the user's own prompt.
///
/// An inline frame that ate the whole terminal would push the prompt off the
/// top, which is the opposite of the reason it is inline.
const PROMPT_LINES: usize = 1;

/// How many list rows a terminal `height` rows tall can show.
///
/// `None` means the terminal is too short to hold a frame at all — the caller
/// has somewhere to go with that answer (#34's resize falls back to a clean
/// cancel-and-restore) rather than drawing a frame with no room for a row.
pub fn fit_visible_rows(terminal_height: usize) -> Option<usize> {
    let room = terminal_height.saturating_sub(CHROME_LINES + PROMPT_LINES);
    if room == 0 {
        None
    } else {
        Some(room.min(VISIBLE_ROWS))
    }
}

/// The terminal the frame is being drawn into.
///
/// The frame is pure, so the terminal arrives as data. `width` decides which
/// hint segments survive; `support` decides whether any of them arrive in
/// colour at all; `height` decides how many list rows the frame can hold.
/// Grouped into one argument rather than three loose parameters because they
/// are one fact — *this is the terminal* — and because every consumer of the
/// frame needs all three together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canvas {
    /// Columns available to the frame.
    pub width: usize,
    /// Rows available to the terminal the frame is drawn into.
    pub height: usize,
    /// How much colour this terminal can actually render.
    pub support: ColorSupport,
}

impl Canvas {
    /// A canvas `width` by `height` with the given colour capability.
    pub const fn new(width: usize, height: usize, support: ColorSupport) -> Self {
        Self {
            width,
            height,
            support,
        }
    }

    /// A canvas `width` by `height` whose colour capability is whatever this
    /// terminal reports — the constructor a live caller uses.
    ///
    /// This is the one place in the inline path that reads the environment, and
    /// it reads it once, at the edge. `build_frame` stays pure.
    pub fn detect(width: usize, height: usize) -> Self {
        Self::new(width, height, ColorSupport::detect())
    }

    /// How many list rows this canvas leaves the frame, or `None` when it is
    /// too short to hold one.
    pub fn visible_rows(&self) -> Option<usize> {
        fit_visible_rows(self.height)
    }
}

/// The rows a fixed-height window shows for a given selection.
///
/// The selection lands in the middle of the window — `visible / 2` rows down
/// from its top — and the window slides to keep it there while the user walks
/// the list. At either end it pins instead of scrolling past, so the range is
/// always clipped to `[0, total)` and never names a row that does not exist.
///
/// This is the whole of the frame's scrolling: the list may be any length, the
/// window is always `visible` rows, which is what lets the frame hold one
/// height while active (user story 12).
pub fn visible_window(total: usize, selection: usize, visible: usize) -> std::ops::Range<usize> {
    if total == 0 || visible == 0 {
        return 0..0;
    }

    let visible = visible.min(total);
    let start = selection.saturating_sub(visible / 2).min(total - visible);

    start..start + visible
}

/// What the frame is being used for.
///
/// The three commands of the redesign share one frame and differ by what Enter
/// does, so the difference has to be a domain concept on the seam rather than
/// a boolean a caller has to remember the polarity of. `Pick` is the
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

    /// Which Connections of the caller's `connections` slice the frame is
    /// showing, in display order.
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
    ///
    /// The escape codes come from the same [`RunWriter`] the fullscreen
    /// serializer uses, so the same `Style` produces the same bytes on both
    /// surfaces.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();
        let mut runs = crate::theme::ansi::RunWriter::new(&mut out);

        for line in &self.lines {
            for span in &line.spans {
                runs.push(span.style, &span.content);
            }
            runs.end_line();
        }

        out
    }
}

/// Build the frame for a set of Connections, a live query, a selected row, a
/// mode, and the terminal it is being drawn into.
///
/// The palette is the Clack palette *resolved against the canvas*: under
/// `NO_COLOR` or `TERM=dumb` every role collapses to `Reset` before a span is
/// built, so the frame emits no colour rather than emitting cyan and hoping.
/// The hint rail is fitted to `canvas.width`, dropping whole segments from the
/// least-needed end rather than letting the terminal clip mid-word.
pub fn build_frame(
    connections: &[Connection],
    query: &str,
    selection: usize,
    mode: FrameMode,
    canvas: Canvas,
) -> Frame {
    let t = Theme::clack().resolve(canvas.support);
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
    let visible = canvas.visible_rows().unwrap_or(0);

    let mut lines = vec![header_line(mode, &t)];

    match state {
        FrameState::Empty => {
            lines.push(state_line(empty_message(mode), &t));
            pad_rows(&mut lines, visible.saturating_sub(1), &t);
        }
        FrameState::NoMatch => {
            lines.push(state_line(&format!("No matches for {query:?}"), &t));
            pad_rows(&mut lines, visible.saturating_sub(1), &t);
        }
        FrameState::Rows => {
            // The list area is the window, not the list: the frame emits the
            // `visible` rows the window covers and pads the rest with bare
            // rail, so narrowing the query changes which rows are there but
            // never how many lines the frame takes.
            let window = visible_window(matches.len(), selection, visible);
            for (i, (conn_idx, _score, hits)) in matches.iter().enumerate() {
                if window.contains(&i) {
                    lines.push(row_line(&connections[*conn_idx], i == selection, hits, &t));
                }
            }
            pad_rows(&mut lines, visible.saturating_sub(window.len()), &t);
        }
    }

    lines.push(hint_rail_line(mode, canvas.width, &t));
    lines.push(corner_line(&t));

    // Nothing the frame emits may exceed the canvas: the inline driver's
    // row-diff collapse (#34) counts one frame line as one physical row,
    // and a wrapped line breaks that count — and the constant height with it.
    let lines: Vec<Line<'static>> = lines
        .into_iter()
        .map(|line| fit_line(line, canvas.width))
        .collect();

    Frame {
        lines,
        state,
        matched,
        selection,
    }
}

/// Trim a line to `width` display columns, keeping the styles of what survives.
///
/// The cut falls where the budget runs out; the tail is dropped whole. This
/// is safe against the frame's grammar because everything meaningful — the
/// rail, the cursor, the folder prefix — sits at the *head* of a line, so a
/// fitted row loses its meta tail, never its structure.
fn fit_line(line: Line<'static>, width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;

    for span in line.spans {
        let w = span.content.chars().count();
        if used + w <= width {
            used += w;
            spans.push(span);
        } else {
            let keep = width - used;
            if keep > 0 {
                let cut: String = span.content.chars().take(keep).collect();
                spans.push(Span::styled(cut, span.style));
            }
            break;
        }
    }

    Line::from(spans)
}

/// The `└` that closes the rail.
///
/// Chrome, not content: it is drawn with the border role so it reads as
/// structure and carries no state.
fn corner_line(t: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled(CORNER, Style::default().fg(t.border))])
}

/// Pad the list area out to a constant height with rows that carry the rail
/// and nothing else.
///
/// The rail runs unbroken from the header to the corner even when the list is
/// shorter than the window, so the frame reads as one fixed shape rather than
/// a box with a ragged bottom.
fn pad_rows(lines: &mut Vec<Line<'static>>, rows: usize, t: &Theme) {
    for _ in 0..rows {
        lines.push(Line::from(vec![Span::styled(
            RAIL,
            Style::default().fg(t.border),
        )]));
    }
}

/// The `└` that closes the rail at the bottom left.
const CORNER: &str = "└";

/// What the empty frame tells the user to do next.
fn empty_message(mode: FrameMode) -> &'static str {
    match mode {
        FrameMode::Pick => "No Connections yet — run sshm manage to add one",
        FrameMode::Manage => "No Connections yet — Ctrl+A to add one",
    }
}

/// A state line: the `│` rail plus muted copy, lined up with row content.
fn state_line(text: &str, t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(
            format!("{GUTTER_PAD}{text}"),
            Style::default().fg(t.fg_muted),
        ),
    ])
}

/// The dim hint line: the escape hatch first, then Enter, then movement, then
/// management.
///
/// The order is the contract, not a stylistic choice — on a narrow terminal the
/// tail gets dropped, so what is listed first is what survives (user story 23).
/// The chords only appear in `Manage`; a pick frame points at the command
/// instead, because its Enter chooses rather than edits.
///
/// The rail is fitted with the same drop-from-the-end rule the fullscreen footer
/// uses ([`crate::app::fit_hints_with`]), against the width left after the
/// gutter. That is what turns "82 columns of hints in an 80-column terminal"
/// from a mid-word clip into a clean loss of the least-needed segment.
fn hint_rail_line(mode: FrameMode, width: usize, t: &Theme) -> Line<'static> {
    let hints: &[&str] = match mode {
        FrameMode::Pick => &[
            "Esc cancel",
            "Enter select",
            "↑↓ navigate",
            // Already last, which under drop-from-the-end ordering means it is
            // the first thing a narrow terminal loses — the ordering #39 asks
            // for, free here because the line was written in that order.
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
    let fitted = crate::app::fit_hints_with(hints, width.saturating_sub(GUTTER), HINT_SEP);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(format!("{GUTTER_PAD}{fitted}"), dim),
    ])
}

/// The `◆ <question>` line that opens every frame.
fn header_line(mode: FrameMode, t: &Theme) -> Line<'static> {
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
fn cursor_span(selected: bool, t: &Theme) -> Span<'static> {
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
fn row_line(conn: &Connection, selected: bool, hits: &[usize], t: &Theme) -> Line<'static> {
    // The ticket asks for *dim* meta, and the hint rail already spells "dim" as
    // `DIM` on `fg_muted`. Muted-without-`DIM` is a different claim — it is
    // just a darker colour, and on a terminal with a bright bright-black it
    // does not recede at all. One word, one spelling.
    let meta = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let alias = Style::default().add_modifier(Modifier::BOLD);
    let hit = Style::default()
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD);

    let mut spans = vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
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
