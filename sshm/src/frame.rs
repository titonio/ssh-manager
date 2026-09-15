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
//! The geometry lives in [`fit`]: the row budget, the sliding window, the
//! column-accurate line fitter, and how many physical rows a drawn line
//! becomes once the terminal narrows under it. That split is deliberate —
//! the grammar changes when the design changes, the geometry changes when
//! the terminal mechanics do, and keeping them in one pile meant one file
//! changing for two unrelated reasons.
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
use crate::theme::{ColorSupport, Theme};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::ops::Range;

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

/// What a frame spends, and where it lands on the glass.
///
/// Everything that answers *how does this fit* lives here; everything that
/// answers *what does it look like* stays out. The split is the one #34
/// forced: #33 owns the Clack grammar above, #34 owns the geometry, and the
/// two change for unrelated reasons — a new hint segment and a new resize
/// rule should not have to live in the same handful of lines.
///
/// The unit throughout is the **terminal column**, never the character. A CJK
/// glyph is one `char` and two columns, and every count downstream of this
/// module — `up(N)`, `\x1b[M` — is counted in the columns the terminal
/// actually spent. Measuring in characters is what let a wide alias wrap a
/// row the collapse could not count.
pub mod fit {
    use ratatui::text::{Line, Span};
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    /// The lines of a frame that are not list rows: the `◆` header, the hint
    /// rail and the `└` corner.
    pub const CHROME_LINES: usize = 3;

    /// The list rows a frame shows when the terminal has room for all of them.
    ///
    /// Eight is the frame's shape, not a limit on the list: the list is a
    /// window over as many Connections as match, and the window slides
    /// ([`visible_window`]) so the frame never has to grow to fit it.
    pub const VISIBLE_ROWS: usize = 8;

    /// The lines of a full frame: the 8-row list area plus the chrome around it.
    ///
    /// The spec's "~12 lines" counts the blank line the frame opens on below
    /// the prompt (#34); the frame itself is eleven.
    pub const FRAME_LINES: usize = VISIBLE_ROWS + CHROME_LINES;

    /// The line the frame leaves for the user's own prompt.
    ///
    /// An inline frame that ate the whole terminal would push the prompt off
    /// the top, which is the opposite of the reason it is inline.
    pub const PROMPT_LINES: usize = 1;

    /// How many list rows a terminal `height` rows tall can show.
    ///
    /// `None` means the terminal is too short to hold a frame at all — the
    /// caller has somewhere to go with that answer (the open path and the
    /// resize path both fall back to a clean cancel-and-restore) rather than
    /// drawing a frame with no room for a row.
    pub fn fit_visible_rows(terminal_height: usize) -> Option<usize> {
        let room = terminal_height.saturating_sub(CHROME_LINES + PROMPT_LINES);
        if room == 0 {
            None
        } else {
            Some(room.min(VISIBLE_ROWS))
        }
    }

    /// The rows a fixed-height window shows for a given selection.
    ///
    /// The selection lands in the middle of the window — `visible / 2` rows
    /// down from its top — and the window slides to keep it there while the
    /// user walks the list. At either end it pins instead of scrolling past,
    /// so the range is always clipped to `[0, total)` and never names a row
    /// that does not exist.
    ///
    /// This is the whole of the frame's scrolling: the list may be any length,
    /// the window is always `visible` rows, which is what lets the frame hold
    /// one height while active (user story 12).
    pub fn visible_window(
        total: usize,
        selection: usize,
        visible: usize,
    ) -> std::ops::Range<usize> {
        if total == 0 || visible == 0 {
            return 0..0;
        }

        let visible = visible.min(total);
        let start = selection.saturating_sub(visible / 2).min(total - visible);

        start..start + visible
    }

    /// Trim a line to `width` **display columns**, keeping the styles of what
    /// survives.
    ///
    /// The cut falls where the budget runs out; the tail is dropped whole.
    /// This is safe against the frame's grammar because everything meaningful
    /// — the rail, the cursor, the folder prefix — sits at the *head* of a
    /// line, so a fitted row loses its meta tail, never its structure.
    ///
    /// Truncating *every* line rather than just the ones known to overflow is
    /// deliberate and load-bearing: the row-count invariant the settle-collapse
    /// depends on is "one frame line = one physical row", and the only way to
    /// guarantee it for arbitrary Connection data is to make the frame itself
    /// the last line of defence rather than trusting that no row is too long.
    pub fn fit_line(line: Line<'static>, width: usize) -> Line<'static> {
        let mut spans = Vec::new();
        let mut used = 0usize;

        for span in line.spans {
            if used + span.content.width() <= width {
                used += span.content.width();
                spans.push(span);
                continue;
            }

            // The budget runs out inside this span. Keep the whole characters
            // that still fit and drop the rest: half a wide glyph would render
            // as a hole and occupy a column the row count does not know about.
            let mut keep = String::new();
            let mut kept = 0usize;
            for ch in span.content.chars() {
                let w = ch.width().unwrap_or(0);
                if used + kept + w > width {
                    break;
                }
                keep.push(ch);
                kept += w;
            }
            if !keep.is_empty() {
                spans.push(Span::styled(keep, span.style));
            }
            break;
        }

        Line::from(spans)
    }

    /// How many physical rows a drawn line of `columns` display columns
    /// occupies in a terminal `width` columns wide.
    ///
    /// One while it fits; more once the terminal has narrowed under it and
    /// re-wrapped it. The driver's `up(N)` / `\x1b[M` count physical rows,
    /// so a collapse after a narrowing has to sum this across the drawn lines
    /// instead of counting the lines it drew.
    pub fn physical_rows(columns: usize, width: usize) -> usize {
        if width == 0 {
            return 1;
        }
        columns.div_ceil(width).max(1)
    }
}

pub use fit::{
    fit_line, fit_visible_rows, physical_rows, visible_window, CHROME_LINES, FRAME_LINES,
    VISIBLE_ROWS,
};

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

    /// The columns the frame may spend: one fewer than the canvas owns.
    ///
    /// A narrowing terminal re-wraps any line that reaches its new right
    /// edge, and a re-wrapped line stops being the single physical row the
    /// driver counted it as. Spending at most `width - 1` means a
    /// one-column narrowing re-wraps nothing, so the drawn row count stays
    /// the physical row count and the collapse keeps its footing. This is
    /// the cheap half of making the resize fallback clean; the honest row
    /// count ([`physical_rows`]) is the other half.
    pub fn fit_width(&self) -> usize {
        self.width.saturating_sub(1)
    }
}

/// What the frame is being used for.
///
/// The three commands of the redesign share one frame and differ by what Enter
/// does, so the difference has to be a domain concept on the seam rather than
/// a boolean a caller has to remember the polarity of. `Pick` is the
/// choose-a-Connection frame (bare `sshm` and `sshm pick`); `Manage` is
/// `sshm manage`, where Enter means the edit path rather than a session. The
/// management chords that go with it are #36's; until they land the manage
/// frame hints nothing it cannot honour.
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
    /// The escape codes come from the same [`crate::theme::ansi::RunWriter`]
    /// the inline driver draws its rows through (`ansi::line_to_ansi`), so
    /// the same `Style` produces the same bytes whichever of the two
    /// serializes it.
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

// ─────────────────────────────────────────────────────────────────────────────
// Filtering — pure, and inside the frame seam (#31: "filtering
// (`compute_matches`) … remain pure and move into the frame seam")
// ─────────────────────────────────────────────────────────────────────────────

/// A fuzzy match result: (original connection index, score, highlight indices
/// within the best-matching field).
pub type MatchResult = (usize, i64, Vec<usize>);

/// Pure fuzzy filter over `&[Connection]` + `SkimMatcherV2` + query.
///
/// Searches alias, host, user, and **folder** fields. For each connection, the
/// field yielding the highest `fuzzy_match` score is used for the highlight
/// indices. Results are sorted by score descending (best match first), with
/// original index as tiebreaker.
///
/// An empty query returns all connections with score `0` and no highlights.
///
/// The highlight indices are positions in the row's **display text**
/// ([`build_row_text`]), which is the coordinate system [`row_line`] splits
/// its spans in — the two must agree or the highlight lands on the wrong
/// character.
pub fn compute_matches(
    connections: &[Connection],
    matcher: &SkimMatcherV2,
    query: &str,
) -> Vec<MatchResult> {
    if query.is_empty() {
        return connections
            .iter()
            .enumerate()
            .map(|(i, _)| (i, 0, vec![]))
            .collect();
    }

    let mut results: Vec<MatchResult> = Vec::new();

    for (i, conn) in connections.iter().enumerate() {
        let fields: Vec<(&str, &str)> = vec![
            ("alias", &conn.alias),
            ("host", &conn.host),
            ("user", &conn.user),
            ("folder", conn.folder.as_deref().unwrap_or("")),
        ];

        let offsets = compute_field_offsets(conn);

        let mut best_score: i64 = i64::MIN;
        let mut best_display_indices: Option<Vec<usize>> = None;

        for (field_name, field) in &fields {
            if let Some((score, indices)) = matcher.fuzzy_indices(field, query) {
                if score > best_score {
                    best_score = score;
                    // Map field-relative indices to absolute display positions.
                    let field_start = offsets
                        .iter()
                        .find(|(n, _, _)| n == field_name)
                        .map(|(_, s, _)| *s)
                        .unwrap_or(0);
                    best_display_indices = Some(indices.iter().map(|&i| field_start + i).collect());
                }
            }
        }

        if let Some(display_indices) = best_display_indices {
            results.push((i, best_score, display_indices));
        }
    }

    results.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    results
}

/// The display text of a frame row (without the rail or cursor) — the single
/// source of truth for what a row says.
///
/// Format:
///   `[folder] alias (user@host:port)` when folder is present,
///   `alias (user@host:port)` when it is not.
///
/// [`compute_field_offsets`] indexes into this string and [`row_line`] draws
/// its content spans as slices of it, so the text the matcher scores and the
/// text the user reads are the same bytes. A row is not formatted anywhere
/// else.
pub fn build_row_text(conn: &Connection) -> String {
    let folder = conn.folder.as_deref().unwrap_or("");
    if folder.is_empty() {
        format!("{} ({}@{}:{})", conn.alias, conn.user, conn.host, conn.port)
    } else {
        format!(
            "[{}] {} ({}@{}:{})",
            folder, conn.alias, conn.user, conn.host, conn.port
        )
    }
}

/// The three segments a row is drawn from, as byte ranges into the row's
/// display text ([`build_row_text`]).
///
/// The folder segment carries its brackets and the meta segment carries its
/// parentheses. The single space between segments belongs to neither: it is
/// drawn as a raw span.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RowSegments {
    /// `[folder]` — absent when the Connection has no folder.
    folder: Option<Range<usize>>,
    /// The alias.
    alias: Range<usize>,
    /// `(user@host:port)`.
    meta: Range<usize>,
}

/// The row's geometry: where each drawn segment sits inside
/// [`build_row_text`]'s output.
///
/// One function owns the `[folder] alias (user@host:port)` layout.
/// [`compute_field_offsets`] derives the matcher's field offsets from these
/// ranges and [`row_line`] cuts its spans out of the row text at them, so the
/// text the highlight indices count into and the text the frame draws are the
/// same bytes. Before this, `row_line` formatted the row a second time by
/// hand and nothing joined the two — see the note on [`row_line`].
fn row_segments(conn: &Connection) -> RowSegments {
    let folder = conn.folder.as_deref().filter(|f| !f.is_empty());
    let port = conn.port.to_string();

    // `[folder] ` is the folder name wrapped in brackets plus a space; a row
    // with no folder starts straight at the alias.
    let alias_start = folder.map_or(0, |f| f.len() + 3);
    let alias_end = alias_start + conn.alias.len();

    // ` (user@host:port)` — the `(` is the segment's first byte, the `)` its
    // last, with one separator column between each of the three fields.
    let meta_start = alias_end + 1;
    let meta_end = meta_start + 1 + conn.user.len() + 1 + conn.host.len() + 1 + port.len() + 1;

    RowSegments {
        folder: folder.map(|f| 0..f.len() + 2),
        alias: alias_start..alias_end,
        meta: meta_start..meta_end,
    }
}

/// Compute the start/end character-offsets of each searchable field within the
/// row display text produced by [`build_row_text`].
///
/// The boundaries come from [`row_segments`], the one place the row's layout
/// is written down, so these offsets cannot land on a different row than the
/// one [`row_line`] draws.
pub fn compute_field_offsets(conn: &Connection) -> Vec<(&str, usize, usize)> {
    let segs = row_segments(conn);
    let folder = conn.folder.as_deref().unwrap_or("");
    let port = conn.port.to_string();
    let mut offsets: Vec<(&str, usize, usize)> = Vec::new();

    if !folder.is_empty() {
        offsets.push(("folder", 1, 1 + folder.len()));
    }

    offsets.push(("alias", segs.alias.start, segs.alias.end));

    // Inside the meta segment `(user@host:port)`: one past the `(` is the
    // user, one past the `@` is the host, one past the `:` is the port.
    let user_start = segs.meta.start + 1;
    offsets.push(("user", user_start, user_start + conn.user.len()));

    let host_start = user_start + conn.user.len() + 1;
    offsets.push(("host", host_start, host_start + conn.host.len()));

    let port_start = host_start + conn.host.len() + 1;
    offsets.push(("port", port_start, port_start + port.len()));

    offsets
}

// ─────────────────────────────────────────────────────────────────────────────
// Hint fitting — the rule the frame's hint rail is drawn under
// ─────────────────────────────────────────────────────────────────────────────

/// Join hint labels with ` | `, dropping the ones that do not fit.
///
/// Hints are dropped from the **end**, so a caller controls exactly what
/// survives in a narrow terminal purely by ordering them by importance.
/// Without this the footer was one long string that the terminal clipped
/// mid-word at whatever width it happened to have.
pub fn fit_hints(hints: &[&str], width: usize) -> String {
    fit_hints_with(hints, width, " | ")
}

/// [`fit_hints`] with the separator chosen by the caller.
///
/// The drop-from-the-end rule is the useful part, not the pipe: the inline
/// frame's hint rail separates its segments with ` · ` and has to fit inside
/// a gutter as well as a width. One rule means the frame never invents a
/// second answer to "what survives a narrow terminal".
pub fn fit_hints_with(hints: &[&str], width: usize, sep: &str) -> String {
    let sep_len = sep.chars().count();
    let mut out = String::new();
    let mut out_len = 0usize;
    for hint in hints {
        let hint_len = hint.chars().count();
        let extra = if out.is_empty() { 0 } else { sep_len };
        if out_len + extra + hint_len > width {
            break;
        }
        if !out.is_empty() {
            out.push_str(sep);
        }
        out.push_str(hint);
        out_len += extra + hint_len;
    }
    out
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

    lines.push(hint_rail_line(mode, canvas.fit_width(), &t));
    lines.push(corner_line(&t));

    // Nothing the frame emits may exceed the frame's own budget. The inline
    // driver's row-diff collapse (#34) counts one frame line as one
    // physical row, and a wrapped line breaks that count — and the constant
    // height with it. The budget sits one column inside the canvas so a
    // narrowing terminal has nothing to re-wrap either; see
    // [`Canvas::fit_width`].
    let budget = canvas.fit_width();
    let lines: Vec<Line<'static>> = lines
        .into_iter()
        .map(|line| fit_line(line, budget))
        .collect();

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
///
/// Both arms name a *command*, because a command is what works today. The
/// manage arm used to say `Ctrl+A to add one`; no handler reads that chord
/// until #36, and an empty frame is exactly where a hint the user tries
/// first has to be true.
fn empty_message(mode: FrameMode) -> &'static str {
    match mode {
        FrameMode::Pick => "No Connections yet — run sshm manage to add one",
        FrameMode::Manage => "No Connections yet — run sshm add to create one",
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

/// The dim hint line: the escape hatch first, then Enter, then movement.
///
/// The order is the contract, not a stylistic choice — on a narrow terminal the
/// tail gets dropped, so what is listed first is what survives (user story 23).
///
/// No mode advertises a Ctrl chord. The manage chords the spec calls for
/// (`Ctrl+A`/`Ctrl+E`/`Ctrl+X`, stories 18-20) are #36's work: `run_inline`
/// reads no Ctrl key but `Ctrl+C`, so a hint naming them would teach a
/// binding that silently does nothing. They come back here when the handlers
/// do.
///
/// The rail is fitted with the shared drop-from-the-end rule
/// ([`fit_hints_with`], the same one [`fit_hints`] is built on), against the
/// width left after the gutter. That is what turns "82 columns of hints in an
/// 80-column terminal" from a mid-word clip into a clean loss of the
/// least-needed segment.
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
        FrameMode::Manage => &["Esc cancel", "Enter edit", "↑↓ navigate"],
    };

    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let fitted = fit_hints_with(hints, width.saturating_sub(GUTTER), HINT_SEP);

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
///
/// This function does not format a row. Every content span is a slice of
/// [`build_row_text`] cut at the boundaries [`row_segments`] reports, which
/// are the same byte coordinates [`compute_matches`] reports hits in — so the
/// characters a hit lights up are characters of the one text the matcher
/// scored. `row_line` used to build the row string a second time by hand,
/// and with nothing joining the two, the drawn characters and the matcher's
/// offsets were free to disagree while every rendered string still looked
/// right.
fn row_line(conn: &Connection, selected: bool, hits: &[usize], t: &Theme) -> Line<'static> {
    // The ticket asks for *dim* meta, and the hint rail already spells "dim" as
    // `DIM` on `fg_muted`. Muted-without-`DIM` is a different claim — it is
    // just a darker colour, and on a terminal with a bright bright-black it
    // does not recede at all. One word, one spelling.
    let meta_style = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let alias_style = Style::default().add_modifier(Modifier::BOLD);
    let hit_style = Style::default()
        .fg(t.highlight)
        .add_modifier(Modifier::BOLD);

    let mut spans = vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::raw(" "),
        cursor_span(selected, t),
        Span::raw(" "),
    ];

    let text = build_row_text(conn);
    let segs = row_segments(conn);

    if let Some(folder) = segs.folder {
        let start = folder.start;
        push_split(
            &mut spans,
            &text[folder],
            start,
            meta_style,
            hit_style,
            hits,
        );
        spans.push(Span::raw(" "));
    }

    let alias_start = segs.alias.start;
    push_split(
        &mut spans,
        &text[segs.alias],
        alias_start,
        alias_style,
        hit_style,
        hits,
    );
    spans.push(Span::raw(" "));

    let meta_start = segs.meta.start;
    push_split(
        &mut spans,
        &text[segs.meta],
        meta_start,
        meta_style,
        hit_style,
        hits,
    );

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
