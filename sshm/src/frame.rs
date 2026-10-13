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

use crate::config::{Connection, Source};
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

/// The shared muted style for notes, errors, and other receding text.
///
/// `fg_muted` alone. No `DIM`: Windows Terminal ignores SGR 2 entirely
/// (microsoft/terminal#6703), so emitting it is a claim the target terminal
/// cannot honour, and the recession has to come from the token rather than the
/// attribute. Extracted so the three call-sites (`error_line`,
/// `plain_note_line`, `update_note_line`) cannot drift apart.
fn dim_style(t: &Theme) -> Style {
    Style::default().fg(t.fg_muted)
}

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

/// Where the manage interaction has got to, as far as the frame cares (#36).
///
/// The frame does not run the interaction — [`crate::manage`] does. This is
/// the projection of that state into the handful of facts that change what
/// the frame draws: whether it is asking a question right now, and what the
/// last answer left behind.
///
/// Defaulting to nothing is the whole of the pick frame's relationship with
/// it, and the whole of a manage frame that has not acted yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrameFlow {
    /// The Connection the inline delete confirm is asking about.
    pub confirming: Option<Connection>,
    /// What the last management action did, rendered as the lines above the
    /// rows.
    pub trace: Option<crate::manage::Trace>,
    /// The form map, while `Ctrl+A` or `Ctrl+E` has one open.
    ///
    /// One field for both, because they are one surface. Two options here
    /// would mean the renderer had to know which command opened the map,
    /// which is exactly the coupling this shape removes.
    pub form: Option<FormFlow>,
}

/// The form map as far as the frame can see it.
///
/// One projection for both `Ctrl+A` and `Ctrl+E`, because they are the
/// same surface: six field rows, a rule, and a `\u25b6` row. What differs is
/// what the rows start out holding and what the button is called \u2014 and
/// the frame should no more know which command opened it than which
/// terminal it is drawing on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FormFlow {
    /// The six field rows, in map order.
    pub rows: Vec<crate::manage::MapRow>,
    /// Whether the cursor is on the `\u25b6` row rather than on a field.
    pub submit_focused: bool,
    /// Whether the `\u25b6` row would be accepted if Enter landed on it.
    ///
    /// Drives the dim-until-ready treatment. A button that looks live and
    /// then refuses is worse than one that says it is not ready.
    pub submit_ready: bool,
    /// The `\u25b6` row's words, from the open map.
    ///
    /// The map carries its target here rather than on a title row, because
    /// the rule row is the row the error takes over \u2014 and losing "which
    /// Connection am I editing?" at the exact moment something went wrong
    /// is the worst possible time to lose it.
    pub submit_label: String,
    /// What an empty field shows in place of a value, from the open map.
    pub placeholder: &'static str,
    /// The header's words, from the open map.
    pub header: String,
    /// The refusal, drawn in the rule row in place of the separator.
    pub error: Option<String>,
}

impl From<&crate::manage::ManageState> for FrameFlow {
    fn from(state: &crate::manage::ManageState) -> Self {
        let (confirming, form) = match &state.phase {
            crate::manage::Phase::ConfirmDelete { target } => (Some(target.clone()), None),
            crate::manage::Phase::List => (None, None),
            crate::manage::Phase::Form(map) => (
                None,
                Some(FormFlow {
                    rows: map.rows(),
                    submit_focused: map.cursor.is_submit(),
                    submit_ready: map.ready(),
                    submit_label: map.submit_label(),
                    placeholder: map.placeholder(),
                    header: map.header(),
                    error: map.error.clone(),
                }),
            ),
        };

        Self {
            confirming,
            trace: state.trace.clone(),
            form,
        }
    }
}

/// The lines the flow draws between the header and the rows, capped at the
/// room the frame can spare.
///
/// The cap is what keeps the frame's height constant (#34). When it bites,
/// the settled confirm line is dropped before the note: after a delete the
/// user needs to see *what happened*, and the settled question is the
/// redundant half.
fn flow_lines(flow: &FrameFlow, budget: usize, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = trace_lines(&flow.trace, t);

    // The form map: six field rows, the rule row, then the ▶ row.
    if let Some(form) = &flow.form {
        push_form_lines(&mut lines, form, t);
    }

    // Drop the oldest rows that do not fit. Spelled as a saturating subtraction
    // rather than a length test with a subtraction inside it: when the count
    // equals the budget there is nothing to drop either way, and a comparison
    // that cannot change the answer is a comparison no test can ever check.
    let excess = lines.len().saturating_sub(budget);
    lines.drain(..excess);

    lines
}

/// The note lines a [`Trace`] puts above the rows, if it puts any.
///
/// The families are disjoint: a trace belongs to exactly one of them, and each
/// one knows how to word itself. Which one is decided here, by a `match` the
/// compiler checks for exhaustiveness, rather than by asking each family in
/// turn — a chain of fallbacks lets a new `Trace` variant slip through every
/// family and render nothing at all. Split this way, adding a trace means one
/// arm here and the wording in the family that already speaks about that kind
/// of action.
fn trace_lines(trace: &Option<crate::manage::Trace>, t: &Theme) -> Vec<Line<'static>> {
    let Some(trace) = trace else {
        return Vec::new();
    };

    use crate::manage::Trace;
    match trace {
        Trace::Deleted { .. } | Trace::DeleteFailed { .. } | Trace::Declined { .. } => {
            delete_trace_lines(trace, t)
        }
        Trace::Added { .. } | Trace::Edited { .. } | Trace::EditFailed { .. } => {
            write_trace_lines(trace, t)
        }
        Trace::AddAbandoned | Trace::EditAbandoned => abandoned_trace_lines(trace, t),
        // Not a family, because there is nothing for a family to assemble: no
        // question was asked, so no settled line comes first, and no write
        // happened, so no verb is owed. One variant, one wording, one plain
        // note — the same `◇` the abandoned notes use, for the one thing they
        // have in common, which is that nothing was written.
        Trace::ReadOnly { .. } => Some(vec![plain_note_line(READ_ONLY_NOTE, t)]),
    }
    // Not a silent empty note: every variant is routed above, and the frame
    // tests word every one of them, so a family that does not recognise what
    // it was asked for means the routing and the enum have drifted apart.
    .unwrap_or_else(|| unreachable!("every Trace variant routes to the family that words it"))
}

/// The delete family: the settled answer first, then what the store did.
///
/// The settled line and the note are two lines because they are two facts —
/// *you answered yes* and *it was removed* — and the second one is the
/// store's to tell, not the frame's.
fn delete_trace_lines(trace: &crate::manage::Trace, t: &Theme) -> Option<Vec<Line<'static>>> {
    use crate::manage::Trace;
    match trace {
        Trace::Deleted { connection } => Some(vec![
            settled_confirm_line(connection, true, t),
            note_line("deleted", connection, t),
        ]),
        // The user answered yes and the store had nothing to remove. The
        // settled line still records the answer — it was given — but the
        // note says the opposite of `deleted`, because nothing was.
        Trace::DeleteFailed { connection } => Some(vec![
            settled_confirm_line(connection, true, t),
            note_line(DELETE_FAILED_NOTE, connection, t),
        ]),
        Trace::Declined { connection } => Some(vec![settled_confirm_line(connection, false, t)]),
        _ => None,
    }
}

/// The write family: one note line, `◇ added [prod] web-01`.
///
/// One line, not two — there was no confirm question to settle first, only
/// the write that happened. `EditFailed` belongs here rather than with the
/// edits that worked because it is the same shape with the opposite verb: the
/// user pressed Enter on a Connection the store does not have, and a
/// `◇ edited` over a list still showing the old row would contradict the
/// rows underneath it.
fn write_trace_lines(trace: &crate::manage::Trace, t: &Theme) -> Option<Vec<Line<'static>>> {
    use crate::manage::Trace;
    let (verb, connection) = match trace {
        Trace::Added { connection } => ("added", connection),
        Trace::Edited { connection } => ("edited", connection),
        Trace::EditFailed { connection } => (EDIT_FAILED_NOTE, connection),
        _ => return None,
    };
    Some(vec![note_line(verb, connection, t)])
}

/// The abandoned family: a sequence backed out of with nothing written.
///
/// Worded to answer the one question the user has — did the half-typed field
/// get saved? It did not.
fn abandoned_trace_lines(trace: &crate::manage::Trace, t: &Theme) -> Option<Vec<Line<'static>>> {
    use crate::manage::Trace;
    let note = match trace {
        Trace::AddAbandoned => ADD_ABANDONED_NOTE,
        Trace::EditAbandoned => EDIT_ABANDONED_NOTE,
        _ => return None,
    };
    Some(vec![plain_note_line(note, t)])
}

/// The form map's own rows, appended after any trace note.
///
/// The rule row is the one that pays for the error line. It is
/// decorative and the refusal is not, and a `!` at that position still
/// separates the fields from the button — so the map occupies exactly
/// eight rows whether or not anything is wrong, which is what keeps the
/// frame's height constant across the whole form instead of jumping a
/// line every time a keystroke changes the validation state.
fn push_form_lines(lines: &mut Vec<Line<'static>>, form: &FormFlow, t: &Theme) {
    for row in &form.rows {
        lines.push(map_row_line(row, form.placeholder, t));
    }
    match &form.error {
        Some(error) => lines.push(error_line(error, t)),
        None => lines.push(rule_line(t)),
    }
    lines.push(submit_line(form, t));
}

/// One row of the form map: `\u2713 Alias  prod`.
///
/// The glyph carries the state and the label carries the name, so the row
/// reads correctly with no colour at all \u2014 the standing requirement for a
/// surface that owns no background. Colour only *prioritises*: `!` is the
/// one yellow thing on the map so the eye lands on the problem, `\u25cf` is the
/// one cyan thing so the eye lands on what is about to be saved.
///
/// The focused row wears `\u25c6` in the accent with a bold label. The caret is
/// drawn rather than left to the terminal, because an inline frame has no
/// cursor of its own to park on a row it is not on.
fn map_row_line(row: &crate::manage::MapRow, placeholder: &str, t: &Theme) -> Line<'static> {
    use crate::manage::RowGlyph;

    let dim = Style::default().fg(t.fg_muted);
    let bold = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);

    let glyph_style = if row.focused {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        match row.glyph {
            RowGlyph::Invalid => Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
            RowGlyph::Changed => Style::default().fg(t.accent),
            _ => dim,
        }
    };
    let glyph = if row.focused { '◆' } else { row.glyph.char() };

    let mut spans = vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(format!("{glyph} "), glyph_style),
        Span::styled(
            format!("{:<FIELD_LABEL_WIDTH$} ", row.label),
            if row.focused { bold } else { dim },
        ),
    ];

    match &row.value {
        // The typed text is bold and the caret dim: the text is the fact,
        // the caret is chrome.
        Some(value) if row.focused => {
            spans.push(Span::styled(value.clone(), bold));
            spans.push(Span::styled(CARET, dim));
        }
        Some(value) => spans.push(Span::styled(value.clone(), Style::default().fg(t.fg))),
        None => {
            // A required field that is still empty must not read as
            // optional. The mode's placeholder is for the fields that may
            // honestly be left alone; everything else says what it is.
            let text = if row.required {
                "<required>"
            } else {
                placeholder
            };
            spans.push(Span::styled(
                text.to_string(),
                Style::default().fg(t.fg_placeholder),
            ));
        }
    }

    Line::from(spans)
}

/// The separator between the fields and the `\u25b6` row.
///
/// Decorative, which is precisely why the error line takes this row when
/// the two compete: the reason the user is stuck beats a horizontal rule.
fn rule_line(t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, Style::default().fg(t.fg_muted)),
        Span::styled("─".repeat(RULE_WIDTH), Style::default().fg(t.border)),
    ])
}

/// The `\u25b6 Add connection` / `\u25b6 Save changes to web-1` row.
///
/// Deliberately a different look and feel from the field rows above it:
/// those are inputs, this is the decision. Three states, all readable with
/// no colour \u2014 the `\u25b6` is always there, so the difference is carried by
/// weight and tier, never by the glyph appearing or disappearing:
///
/// * **not ready** \u2014 the whole row recedes to `fg_muted`. The user can see
///   the button will not fire, which is why the field glyphs are allowed to
///   be the only loud signal while filling.
/// * **ready, cursor elsewhere** \u2014 accent glyph, plain label.
/// * **ready, cursor here** \u2014 accent + bold glyph and bold label, so the
///   row the user is about to commit from is unmistakable.
fn submit_line(form: &FormFlow, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted);

    // Focus is carried by BOLD on the glyph in *every* readiness state.
    // Without it the cursor on a not-ready `▶` row is byte-identical to
    // the cursor not being on it, which breaks the rule that selection
    // must be readable with no colour at all \u2014 and it breaks it at the
    // worst moment, when the user is arrowing around trying to find the
    // one row that will actually do something.
    let (glyph_style, label_style) = if !form.submit_ready {
        if form.submit_focused {
            // Bold on both halves, as in the ready state. The grey tier is
            // what keeps a form that cannot fire from shouting; the weight is
            // what keeps the cursor visible once colour is gone.
            (
                dim.add_modifier(Modifier::BOLD),
                dim.add_modifier(Modifier::BOLD),
            )
        } else {
            (dim, dim)
        }
    } else if form.submit_focused {
        (
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        )
    } else {
        (Style::default().fg(t.accent), Style::default().fg(t.fg))
    };

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled("▶ ", glyph_style),
        Span::styled(form.submit_label.clone(), label_style),
    ])
}

/// The width the field labels are padded to, so the values line up in a
/// column down the map rather than starting wherever the label ends.
const FIELD_LABEL_WIDTH: usize = 6;

/// How long the form map's separator rule is.
///
/// Fixed rather than width-derived because `flow_lines` is handed a line
/// budget, not a column count, and the rule is decoration: it separates
/// the fields from the `▶` row, and 34 columns does that at every width
/// the frame is designed for. Deriving it would mean threading a width
/// through a function that has no business knowing one.
const RULE_WIDTH: usize = 34;

/// The map's rejection, shown under the header.
///
/// `!` plus the sentence, in the `warning` role. The role existed so a
/// warning never invents a hue; this is the first thing to draw it. The
/// state is carried by the glyph and the words as much as by the colour,
/// so it reads the same with `NO_COLOR` set.
fn error_line(message: &str, t: &Theme) -> Line<'static> {
    let dim = dim_style(t);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(
            "! ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ),
        Span::styled(message.to_string(), Style::default().fg(t.warning)),
    ])
}

/// A `◇` note with no Connection to name — for the routes that ask for
/// something other than a delete and so have no row to point at.
fn plain_note_line(text: &str, t: &Theme) -> Line<'static> {
    let dim = dim_style(t);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(format!("◇ {text}"), dim),
    ])
}

/// The dim update note a previous run's cache leaves above the frame (#39).
///
/// `◆` in the `accent` role so the line reads as ours even with the colour
/// off, the sentence in `fg_muted` + `DIM` so it recedes behind the frame
/// it sits above. The version and the command are the two facts worth
/// scanning for and they ride in the sentence — no second colour, no
/// literal hue. The note carries no rail: it is *above* the framed body,
/// not inside it, so it reads as a whisper over the frame rather than as
/// one of its rows.
///
/// When the full note (with version) does not fit in `width`, the version
/// is dropped and the action survives: `◆ update available — run sshm
/// update`. Narrow terminal keeps what matters (sshm-design rule 5).
fn update_note_line(version: &str, t: &Theme, width: usize) -> Line<'static> {
    let dim = dim_style(t);
    let full = format!("◆ update available: v{version} — run sshm update");
    if unicode_width::UnicodeWidthStr::width(full.as_str()) <= width {
        Line::from(vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::styled(
                format!(" update available: v{version} — run sshm update"),
                dim,
            ),
        ])
    } else {
        // Short form: keep the action, drop the version.
        Line::from(vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::styled(" update available — run sshm update", dim),
        ])
    }
}

/// The note an abandoned add leaves.
///
/// Worded to answer the one question the user has after backing out of a
/// half-filled form: *did the half of it get saved?* It did not, and the
/// note says so — no `Effect::Add` was ever emitted, so nothing was ever
/// written.
const ADD_ABANDONED_NOTE: &str = "add abandoned — nothing saved";

/// The note an abandoned edit leaves.
///
/// The edit half of [`ADD_ABANDONED_NOTE`], worded the same way because
/// it answers the same question: the field was half-typed and none of it
/// was written.
const EDIT_ABANDONED_NOTE: &str = "edit abandoned — nothing saved";

/// The note an Enter on a Connection the store does not have leaves.
///
/// The edit half of [`DELETE_FAILED_NOTE`]. The word "edited" never
/// appears on a frame where an edit did not happen.
const EDIT_FAILED_NOTE: &str = "edit failed — no such Connection:";

/// The note a `y` that removed nothing leaves.
///
/// Worded so it cannot be misread as the `deleted` note above it: the store
/// reported no such Connection, so nothing was removed and nothing was
/// written. The word "deleted" never appears on a frame where a deletion
/// did not happen — that is the whole rule this note exists to keep.
const DELETE_FAILED_NOTE: &str = "delete failed — no such Connection:";

/// The note a management chord leaves over a Config Host.
///
/// The row came from `~/.ssh/config`, which sshm reads and never writes, so
/// `Ctrl+X` and `Ctrl+E` stop rather than opening a confirm or a form: either
/// one would be a UI promising a change sshm must not make. The wording names
/// the provenance and not the Connection, because the provenance is the
/// reason — and the row it is about is still on the glass underneath, wearing
/// its `[ssh-config]` tag.
///
/// A note and not a settle: nothing was asked and nothing was written, so
/// there is no answer to record and no frame to close. The frame stays open
/// on the list, which is what makes the refusal legible as *refused* rather
/// than as a command that ran.
const READ_ONLY_NOTE: &str = "read-only — from ssh-config";

/// The `■` line: the confirm's question, answered.
///
/// `◆` asks and `■` has been answered — the glyph is the whole of the
/// difference between a live confirm and a settled one, which is what keeps
/// the distinction readable with colour off. The line recedes with
/// `fg_muted` + `DIM`; the alias and the answer stay bold so the two facts
/// a user scans for survive the recession.
fn settled_confirm_line(conn: &Connection, answered: bool, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted);
    let bold = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);

    let mut spans = vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled("■ ", dim),
        Span::styled("Delete ", dim),
    ];

    if let Some(folder) = conn.folder.as_deref().filter(|f| !f.is_empty()) {
        spans.push(Span::styled(format!("[{folder}] "), dim));
    }

    spans.push(Span::styled(conn.alias.clone(), bold));
    spans.push(Span::styled("? ", dim));
    spans.push(Span::styled(if answered { "Yes" } else { "No" }, bold));

    Line::from(spans)
}

/// The dim `◇` note: what the last action did.
fn note_line(verb: &str, conn: &Connection, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted);
    let bold = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);

    let mut spans = vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(format!("◇ {verb} "), dim),
    ];

    if let Some(folder) = conn.folder.as_deref().filter(|f| !f.is_empty()) {
        spans.push(Span::styled(format!("[{folder}] "), dim));
    }

    spans.push(Span::styled(conn.alias.clone(), bold));

    Line::from(spans)
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
/// Searches alias, host, user, and the bracketed prefix — the **folder** for a
/// stored row, the `[ssh-config]` tag for a config row. For each connection,
/// the field yielding the highest `fuzzy_match` score is used for the highlight
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
            ("folder", row_prefix(conn).unwrap_or("")),
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

/// The word a row carries in its bracketed prefix when it came from
/// `~/.ssh/config` rather than from sshm's store.
///
/// One constant because two things have to agree on it: the frame draws it, and
/// the user types it — searching `ssh-config` is how the config hosts are listed
/// on their own. It is a constant and not a string a user could type into the
/// Folder field, because a stored row wearing this word would claim to be
/// something sshm must not let it edit.
pub const SSH_CONFIG_TAG: &str = "ssh-config";

/// The bracketed prefix a row carries, if it carries one: the `[ssh-config]`
/// tag for a config row, otherwise its folder.
///
/// Provenance outranks the folder rather than sitting next to it. A config row
/// has no folder to show — folders are sshm's own data, and a stanza has nothing
/// to fill one in from — and the tag is the fact the row is being shown for, so
/// a row that is both would have to draw one of them anyway. The rule is written
/// once here so the drawn row, the row's geometry and the matcher's haystack
/// cannot disagree about which word a row leads with.
///
/// `None` means no brackets at all: a stored row with no folder, which is the
/// common case and stays compact.
pub fn row_prefix(conn: &Connection) -> Option<&str> {
    if conn.source == Source::SshConfig {
        return Some(SSH_CONFIG_TAG);
    }
    conn.folder.as_deref().filter(|f| !f.is_empty())
}

/// The display text of a frame row (without the rail or cursor) — the single
/// source of truth for what a row says.
///
/// Format:
///   `[folder] alias (user@host:port)` when folder is present,
///   `alias (user@host:port)` when it is not.
///
/// The brackets hold [`row_prefix`], so a row merged from `~/.ssh/config` reads
/// `[ssh-config] alias (user@host:port)` — the same slot a folder occupies,
/// filled from provenance instead.
///
/// [`compute_field_offsets`] indexes into this string and [`row_line`] draws
/// its content spans as slices of it, so the text the matcher scores and the
/// text the user reads are the same bytes. A row is not formatted anywhere
/// else.
pub fn build_row_text(conn: &Connection) -> String {
    let prefix = row_prefix(conn).unwrap_or("");
    if prefix.is_empty() {
        format!("{} ({}@{}:{})", conn.alias, conn.user, conn.host, conn.port)
    } else {
        format!(
            "[{}] {} ({}@{}:{})",
            prefix, conn.alias, conn.user, conn.host, conn.port
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
    /// `[folder]`, or `[ssh-config]` on a config row — absent when the row
    /// carries neither ([`row_prefix`]).
    folder: Option<Range<usize>>,
    /// The alias.
    alias: Range<usize>,
    /// `(user@host:port)`.
    meta: Range<usize>,
}

/// The row's geometry: where each drawn segment sits inside
/// [`build_row_text`]'s output.
///
/// One function owns the `[prefix] alias (user@host:port)` layout.
/// [`compute_field_offsets`] derives the matcher's field offsets from these
/// ranges and [`row_line`] cuts its spans out of the row text at them, so the
/// text the highlight indices count into and the text the frame draws are the
/// same bytes. Before this, `row_line` formatted the row a second time by
/// hand and nothing joined the two — see the note on [`row_line`].
fn row_segments(conn: &Connection) -> RowSegments {
    let folder = row_prefix(conn);
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
///
/// The `"folder"` field is the bracketed prefix, so on a config row it indexes
/// the `[ssh-config]` tag: the tag is searchable exactly the way a folder is,
/// which is what makes typing `ssh-config` list the config hosts.
pub fn compute_field_offsets(conn: &Connection) -> Vec<(&str, usize, usize)> {
    let segs = row_segments(conn);
    let folder = row_prefix(conn).unwrap_or("");
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
    build_frame_with_flow(
        connections,
        query,
        selection,
        mode,
        canvas,
        &FrameFlow::default(),
    )
}

/// [`build_frame`] with the manage flow layered on.
///
/// The flow is what #36 adds to the frame: the inline delete confirm's
/// question in the header, the `■` line the confirm leaves when it is
/// answered, and the dim `◇` note the last action left above the rows. A
/// default `FrameFlow` is no flow at all, which is what the pick frame and
/// every pre-#36 caller pass — so this stays *the* frame seam, with one
/// argument that says where the interaction has got to.
///
/// The flow lines are drawn **inside** the row budget, never on top of it.
/// The frame's height is the invariant #34's settle-collapse counts on, and
/// a note that pushed a row off the bottom of the frame would break it. When
/// the terminal is too short to hold the flow and a row, the settled line is
/// dropped before the note: the note is the thing worth reading.
pub fn build_frame_with_flow(
    connections: &[Connection],
    query: &str,
    selection: usize,
    mode: FrameMode,
    canvas: Canvas,
    flow: &FrameFlow,
) -> Frame {
    build_frame_with_note(connections, query, selection, mode, canvas, flow, None)
}

/// [`build_frame_with_flow`] with the cached update note layered on (#39).
///
/// The note is a line the frame emits *above* the header — `◆ update
/// available: v… — run sshm update` — read once at the edge from a
/// previous run's cache and handed in here as data. The frame never opens
/// the cache file itself: `note` is a plain `Option<&str>`, so the code
/// path that paints the frame performs no I/O and cannot block on a
/// network round-trip before first paint.
///
/// **Design decision (option a).** The note is part of the `Frame` value,
/// not a line the command runner prints above it. The inline viewport is
/// opened at a fixed height and ratatui 0.30 cannot resize it while it is
/// live, so the frame's height must be decided exactly once, before the
/// viewport opens. Folding the note into the frame lets that decision happen
/// here, in one place: the note is drawn first and *displaces one list row*
/// rather than adding a line, so the frame's total height is identical
/// whether or not a note is present. A note printed by the runner above the
/// frame (option b) would sit outside the height the driver counts, and the
/// settle-collapse would either leave it stranded or have to learn about
/// it — a second place that has to know the note exists. Keeping it in the
/// frame means the note cannot appear, vanish, or change while the frame is
/// active: it is read once at the edge and frozen into the value.
pub fn build_frame_with_note(
    connections: &[Connection],
    query: &str,
    selection: usize,
    mode: FrameMode,
    canvas: Canvas,
    flow: &FrameFlow,
    note: Option<&str>,
) -> Frame {
    let t = Theme::clack().resolve(canvas.support);
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(connections, &matcher, query);

    // The flow is a manage-frame idea. A pick frame has no confirm to ask
    // and no action to leave a note about, so it renders with none.
    let empty = FrameFlow::default();
    let flow = match mode {
        FrameMode::Manage => flow,
        FrameMode::Pick => &empty,
    };

    let state = if connections.is_empty() {
        FrameState::Empty
    } else if matches.is_empty() {
        FrameState::NoMatch
    } else {
        FrameState::Rows
    };

    let matched: Vec<usize> = matches.iter().map(|(conn_idx, _, _)| *conn_idx).collect();
    let selection = selection.min(matched.len().saturating_sub(1));

    // The update note adds a line above the header; it does not displace a
    // list row. The frame's total height grows by one line when the note is
    // present, decided once before the viewport opens. The visible rows are
    // what the terminal leaves after chrome, prompt, and the note line
    // itself — so 8 rows are kept with and without the note at full height,
    // and the budget saturates (never overflows) at very small heights.
    let note_line = note.map(|v| update_note_line(v, &t, canvas.fit_width()));
    let note_rows = usize::from(note_line.is_some());
    // One expression, because there is only one quantity here: `Canvas::visible_rows()`
    // *is* `fit_visible_rows(canvas.height)`, so the no-note case is this case
    // with `note_rows` of zero. A branch between two spellings of the same
    // arithmetic is a branch no test can tell apart.
    let visible = fit_visible_rows(canvas.height.saturating_sub(note_rows)).unwrap_or(0);

    let mut lines = Vec::new();
    if let Some(line) = note_line {
        lines.push(line);
    }
    lines.push(header_line(mode, flow, &t));

    // The flow lines come out of the row budget, not on top of it.
    let above = flow_lines(flow, visible, &t);
    let rows = visible.saturating_sub(above.len());
    lines.extend(above);

    match state {
        FrameState::Empty => {
            lines.push(state_line(empty_message(mode), &t));
            pad_rows(&mut lines, rows.saturating_sub(1), &t);
        }
        FrameState::NoMatch => {
            lines.push(state_line(&format!("No matches for {query:?}"), &t));
            pad_rows(&mut lines, rows.saturating_sub(1), &t);
        }
        FrameState::Rows => {
            // The list area is the window, not the list: the frame emits the
            // `rows` lines the window covers and pads the rest with bare
            // rail, so narrowing the query changes which rows are there but
            // never how many lines the frame takes.
            let window = visible_window(matches.len(), selection, rows);
            for (i, (conn_idx, _score, hits)) in matches.iter().enumerate() {
                if window.contains(&i) {
                    lines.push(row_line(&connections[*conn_idx], i == selection, hits, &t));
                }
            }
            pad_rows(&mut lines, rows.saturating_sub(window.len()), &t);
        }
    }

    lines.push(hint_rail_line(mode, flow, canvas.fit_width(), &t));
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
/// Both arms name something that works today. The manage arm names the
/// chord: as of #37 `Ctrl+A` opens the form map and writes the
/// Connection, so an empty manage frame — the exact place a new user
/// starts — can point straight at the action instead of at a command
/// outside the frame.
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

/// The dim hint line: the escape hatch first, then Enter, then movement.
///
/// The order is the contract, not a stylistic choice — on a narrow terminal the
/// tail gets dropped, so what is listed first is what survives (user story 23).
///
/// **The rail lists only keys that actually work in this build.** That rule
/// cut `Ctrl+A add` and `Ctrl+E edit` from the manage rail: `Ctrl+A`
/// answered its own chord with "not built yet", and `Ctrl+E` left the frame
/// byte-identical to Enter, so each advertised an action that does not
/// happen. The chords stay wired — `manage::step` still routes both, and
/// `Ctrl+A` still answers visibly — they are just not *hinted* until #37
/// gives them the behaviour their labels promise. `Ctrl+X delete` stays
/// because it really deletes.
///
/// The rule is the frame's own and it cuts both ways: `assert_advertised_
/// chords_are_live` in `frame_test.rs` fails if a chord is hinted without a
/// handler, and `the_manage_rail_advertises_only_the_keys_that_work` in
/// `manage_frame_test.rs` fails if a chord is hinted whose named action is
/// not the one that happens.
///
/// While the delete confirm is open the rail changes to the answers that
/// confirm actually reads, in the same order: the way out first.
///
/// The rail is fitted with the shared drop-from-the-end rule
/// ([`fit_hints_with`], the same one [`fit_hints`] is built on), against the
/// width left after the gutter. That is what turns "82 columns of hints in an
/// 80-column terminal" from a mid-word clip into a clean loss of the
/// least-needed segment.
fn hint_rail_line(mode: FrameMode, flow: &FrameFlow, width: usize, t: &Theme) -> Line<'static> {
    // Declared out here so the borrow outlives the `match` that picks one.
    const FORM_FIELD: &[&str] = &["Esc back", "Enter next", "↑↓ move", "Ctrl+C quit"];
    const FORM_SUBMIT: &[&str] = &["Esc back", "Enter save", "↑↓ move", "Ctrl+C quit"];
    const CONFIRMING: &[&str] = &["Esc back", "y confirm", "N abort", "Ctrl+C quit"];

    let hints: &[&str] = if flow.confirming.is_some() {
        CONFIRMING
    } else if let Some(form) = &flow.form {
        // The rail names what Enter means on the row the cursor is on:
        // `next` on a field, `save` on the ▶ row. Hinting one meaning for
        // both keys would be lying about one of them.
        //
        // `↑↓ move` is on it because the arrows here move the *form cursor*,
        // not the list cursor — and with the list hidden behind the map,
        // nothing else on screen says what they do.
        if form.submit_focused {
            FORM_SUBMIT
        } else {
            FORM_FIELD
        }
    } else {
        match mode {
            FrameMode::Pick => &[
                "Esc cancel",
                "Enter select",
                "↑↓ navigate",
                // Already last, which under drop-from-the-end ordering means it is
                // the first thing a narrow terminal loses — the ordering #39 asks
                // for, free here because the line was written in that order.
                "sshm manage to add or edit",
            ],
            // `Ctrl+A add` and `Ctrl+E edit` are both on the rail (#37):
            // each chord does the thing its label names. Both open the same
            // six-field map; `Ctrl+A` starts it blank, `Ctrl+E` starts it
            // from the selected Connection.
            //
            // `Enter edit` is off it. Six hints do not fit the 75 columns
            // an 80-column terminal leaves the rail, and drop-from-the-end
            // would cut `Ctrl+E` — the one hint this ticket exists to
            // make discoverable — first. Between the two keys that claim
            // "edit", the rail keeps the one that keeps its promise:
            // `Ctrl+E` changes `connections.json`, while Enter only
            // leaves the frame with the selection routed to the emit
            // path, which writes nothing. Enter still works; it is just
            // not what the manage frame's user is here to do.
            FrameMode::Manage => &[
                "Esc cancel",
                "↑↓ navigate",
                "Ctrl+X delete",
                "Ctrl+A add",
                "Ctrl+E edit",
            ],
        }
    };

    let dim = Style::default().fg(t.fg_muted);
    let fitted = fit_hints_with(hints, width.saturating_sub(GUTTER), HINT_SEP);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(format!("{GUTTER_PAD}{fitted}"), dim),
    ])
}

/// The `◆ <question>` line that opens every frame.
///
/// While the delete confirm is open the question *is* the header — that is
/// the Clack grammar for a confirm: one line, one ask. The folder prefix and
/// the `(y/N)` answer hint are dim and the alias is bold, exactly as they
/// are in the row the confirm is about, so the two read as the same
/// Connection.
fn header_line(mode: FrameMode, flow: &FrameFlow, t: &Theme) -> Line<'static> {
    if let Some(target) = &flow.confirming {
        let dim = Style::default().fg(t.fg_muted);
        let bold = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled("Delete ", bold),
        ];

        if let Some(folder) = target.folder.as_deref().filter(|f| !f.is_empty()) {
            spans.push(Span::styled(format!("[{folder}] "), dim));
        }

        spans.push(Span::styled(target.alias.clone(), bold));
        spans.push(Span::styled("? ", bold));
        spans.push(Span::styled("(y/N)", dim));

        return Line::from(spans);
    }

    // The form's header names the *ask*, not the field. The old add header
    // wore the field word because the field was the whole question; on the
    // map every field is on screen at once, so the question at the top of
    // the frame is the one the map cannot answer for itself: am I adding,
    // and if I am editing, of which Connection.
    //
    // Same grammar as the delete confirm's header — `&#9670; Edit`, dim folder prefix,
    // bold alias — so the chords that change a Connection read as the same
    // kind of thing.
    if let Some(form) = &flow.form {
        return Line::from(vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled(
                form.header.clone(),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    let title = match mode {
        FrameMode::Pick => "Select a Connection",
        FrameMode::Manage => "Manage Connections",
    };

    Line::from(vec![
        Span::styled("◆", Style::default().fg(t.accent)),
        Span::raw(" "),
        Span::styled(
            title,
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
    ])
}

/// The caret the add map draws for itself.
///
/// The frame hides the hardware cursor for its whole life — it has to,
/// or a redraw would leave a blinking cell in the middle of a frame that
/// is trying not to look like a window. A text field still has to show
/// where it is being typed into, so the frame draws its own. `_` because
/// it is ASCII: every font that renders this frame's box-drawing renders
/// it, and a caret that turns into tofu is worse than no caret at all.
const CARET: &str = "_";

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
    // The meta recedes by *token*, not by attribute. `DIM` used to be stacked on
    // `fg_muted` here on the theory that a colour alone "does not recede at
    // all" — but Windows Terminal ignores SGR 2 entirely
    // (microsoft/terminal#6703), and on the scheme behind issue #42 `fg_muted`
    // as `DarkGray` was the *same colour* as the body text, so neither spelling
    // receded. The recession now lives in `fg_muted` itself: `Indexed(245)` at
    // 4.67:1 against body `Gray` at 7.57:1.
    let meta_style = Style::default().fg(t.fg_muted);
    let alias_style = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);
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
