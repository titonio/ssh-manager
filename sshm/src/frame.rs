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
    /// The `Ctrl+A` add sequence, while one is running (#37).
    pub add: Option<AddFlow>,
    /// The `Ctrl+E` in-place single-field editor, while one is open (#37).
    pub edit: Option<EditFlow>,
    /// The first-run import offer, while one is open (#38).
    pub import_offer: Option<ImportOfferFlow>,
}

/// One step of the add sequence that has already been answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledField {
    /// The step's word, drawn after the `◇`.
    pub label: &'static str,
    /// What it settled to, or `None` for an optional field left empty —
    /// which the frame draws as *absent* rather than as a blank.
    pub value: Option<String>,
}

/// The add sequence as far as the frame can see it (#37).
///
/// The frame does not run the sequence — [`crate::manage`] does. This is
/// the projection of one live step: the word wearing the `◆`, what is on
/// the line, what the last rejection said, and what has already settled
/// behind it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AddFlow {
    /// The step being answered now.
    pub label: &'static str,
    /// What the user has typed into it.
    pub input: String,
    /// The rejection shown under the header, if the last Enter was refused.
    pub error: Option<String>,
    /// The steps already settled, in the order they were answered.
    pub settled: Vec<SettledField>,
    /// Whether this is the last step, so the rail can say what Enter
    /// means *here* rather than guessing.
    pub last: bool,
}

impl From<&crate::manage::ManageState> for FrameFlow {
    fn from(state: &crate::manage::ManageState) -> Self {
        let (confirming, add, edit, import_offer) = match &state.phase {
            crate::manage::Phase::ConfirmDelete { target } => {
                (Some(target.clone()), None, None, None)
            }
            crate::manage::Phase::List => (None, None, None, None),
            crate::manage::Phase::Add(sequence) => (
                None,
                Some(AddFlow {
                    label: sequence.field.label(),
                    input: sequence.input.clone(),
                    error: sequence.error.clone(),
                    settled: sequence
                        .settled()
                        .into_iter()
                        .map(|(label, value)| SettledField { label, value })
                        .collect(),
                    last: sequence.field.next().is_none(),
                }),
                None,
                None,
            ),
            crate::manage::Phase::Edit(editor) => (
                None,
                None,
                Some(EditFlow {
                    target: editor.target.clone(),
                    label: editor.field.label(),
                    input: editor.input.clone(),
                    error: editor.error.clone(),
                }),
                None,
            ),
            crate::manage::Phase::ConfirmImport { count, path } => (
                None,
                None,
                None,
                Some(ImportOfferFlow {
                    count: *count,
                    path: path.clone(),
                }),
            ),
        };

        Self {
            confirming,
            trace: state.trace.clone(),
            add,
            edit,
            import_offer,
        }
    }
}

/// The in-place single-field editor as far as the frame can see it (#37).
///
/// The frame does not run the editor — [`crate::manage`] does. This is the
/// projection of the one live field: which Connection is being changed,
/// which of its fields is on the line, what is on that line, and what the
/// last rejection said.
///
/// **Why the target is carried.** The add sequence can get away with
/// naming only the field because there is nothing else in play. An edit is
/// about a specific Connection, and the frame must say which one: the
/// user is one Enter away from a write to `connections.json`, and "which
/// Connection did I just change?" must be answerable from the line at the
/// top of the frame rather than by scanning the list for a cursor that
/// could have moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditFlow {
    /// The Connection being edited.
    pub target: Connection,
    /// The field on the line now.
    pub label: &'static str,
    /// What is on the line, seeded from the field's current value.
    pub input: String,
    /// The rejection shown under the header, if the last Enter was refused.
    pub error: Option<String>,
}

/// The first-run import offer as far as the frame can see it (#38).
///
/// The frame does not run the offer — [`crate::manage`] answers it and
/// the frame runner in `main.rs` decides when an empty set with an
/// importable file deserves the ask. This is the projection of the ask
/// itself: how many Connections the scan found, and the file it found
/// them in. The frame turns that into the header question and nothing
/// else — `count` is what the scan said, not a promise about what the
/// write will return. The number the user is *then* told arrived comes
/// back through [`crate::manage::Trace::Imported`], earned from the
/// store rather than carried forward from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOfferFlow {
    /// What the scan found at `path`: the stanzas that could become
    /// Connections.
    pub count: usize,
    /// The file the offer is about — the one shown in the question and
    /// the one a `y` writes from, so the two can never disagree.
    pub path: String,
}

/// The lines the flow draws between the header and the rows, capped at the
/// room the frame can spare.
///
/// The cap is what keeps the frame's height constant (#34). When it bites,
/// the settled confirm line is dropped before the note: after a delete the
/// user needs to see *what happened*, and the settled question is the
/// redundant half.
fn flow_lines(flow: &FrameFlow, budget: usize, t: &Theme) -> Vec<Line<'static>> {
    let mut lines = match &flow.trace {
        Some(crate::manage::Trace::Deleted { connection }) => vec![
            settled_confirm_line(connection, true, t),
            note_line("deleted", connection, t),
        ],
        // The user answered yes and the store had nothing to remove. The
        // settled line still records the answer — it was given — but the
        // note says the opposite of `deleted`, because nothing was.
        Some(crate::manage::Trace::DeleteFailed { connection }) => vec![
            settled_confirm_line(connection, true, t),
            note_line(DELETE_FAILED_NOTE, connection, t),
        ],
        Some(crate::manage::Trace::Declined { connection }) => {
            vec![settled_confirm_line(connection, false, t)]
        }
        Some(crate::manage::Trace::AddAbandoned) => {
            vec![plain_note_line(ADD_ABANDONED_NOTE, t)]
        }
        // The note the ticket asks for: `◇ added [prod] web-01`. It is
        // one line, not two — there was no confirm question to settle
        // first, only the write that happened.
        Some(crate::manage::Trace::Added { connection }) => {
            vec![note_line("added", connection, t)]
        }
        // The edit's note: `◇ edited [prod] web-01`. One line, like the
        // add's — there was no confirm question to settle first, only the
        // write that happened.
        Some(crate::manage::Trace::Edited { connection }) => {
            vec![note_line("edited", connection, t)]
        }
        // Backing out of an edit. Worded to answer the one question the
        // user has: did the half-typed field get saved? It did not.
        Some(crate::manage::Trace::EditAbandoned) => {
            vec![plain_note_line(EDIT_ABANDONED_NOTE, t)]
        }
        // The user pressed Enter on a Connection the store does not have.
        // The note says the edit did not happen, because a `◇ edited`
        // over a list still showing the old row would contradict the
        // rows underneath it.
        Some(crate::manage::Trace::EditFailed { connection }) => {
            vec![note_line(EDIT_FAILED_NOTE, connection, t)]
        }
        // The import's note: `◇ imported 12 connections`. One line, like
        // the add's — the offer was asked on the header, not here, so
        // there is no settled question to record, only the write that
        // happened. The skipped count rides along when there is one, and
        // it is called *skipped*, not *failed*: the offer's number came
        // from a scan that already left those stanzas out, so they are
        // not a shortfall of what the user was promised. The live PTY
        // run for #38 caught `1 failed` reading as "one of your three
        // failed" when nothing of the three had — the skipped stanza was
        // never one of them. The word carries the difference.
        Some(crate::manage::Trace::Imported { imported, failed }) => {
            let noun = if *imported == 1 {
                "connection"
            } else {
                "connections"
            };
            let text = if *failed > 0 {
                format!("imported {imported} {noun}, {failed} skipped")
            } else {
                format!("imported {imported} {noun}")
            };
            vec![plain_note_line(&text, t)]
        }
        // The offer was heard and refused. The note is what makes the
        // empty list underneath read as *declined* rather than as a
        // screen that never asked anything.
        Some(crate::manage::Trace::ImportDeclined) => {
            vec![plain_note_line(IMPORT_DECLINED_NOTE, t)]
        }
        None => Vec::new(),
    };

    // The add sequence's own history: each answered step settled to a
    // `◇` line, and the rejection — if there is one — goes last, so a
    // terminal too short for all of them drops the oldest settled step
    // before it drops the reason the user is stuck.
    if let Some(add) = &flow.add {
        for settled in &add.settled {
            lines.push(settled_field_line(settled, t));
        }
        if let Some(error) = &add.error {
            lines.push(error_line(error, t));
        }
    }

    // The editor's own ask: the field on the line, and the rejection if
    // the last Enter was refused. The ask comes before the error for the
    // same reason the add sequence puts its reason last — when the budget
    // cuts, the reason the user is stuck on survives.
    if let Some(edit) = &flow.edit {
        lines.push(edit_field_line(edit, t));
        if let Some(error) = &edit.error {
            lines.push(error_line(error, t));
        }
    }

    if lines.len() > budget {
        lines.drain(..lines.len() - budget);
    }

    lines
}

/// The live edit field: `◆ Alias  web-01_`.
///
/// The same shape the add step wears — the field word, the text on the
/// line, the drawn caret — because it *is* the same kind of thing: one
/// field, being filled in. What differs is that the line arrives already
/// holding the value it was seeded from, so the user sees what they are
/// changing rather than an empty slot, and the header above names the
/// Connection it belongs to.
///
/// The typed text is bold and the caret dim: the text is the fact, the
/// caret is chrome. Under `NO_COLOR` both survive as themselves.
fn edit_field_line(edit: &EditFlow, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(format!("◆ {:<FIELD_LABEL_WIDTH$}  ", edit.label), dim),
        Span::styled(edit.input.clone(), bold),
        Span::styled(CARET, dim),
    ])
}

/// A settled add step: `◇ Alias  web-01`.
///
/// The label recedes with the note grammar and the value is bold, because
/// the value is the fact worth scanning back through. An optional field
/// the user left empty is drawn as `—`, not as nothing: a blank after
/// `◇ Key` reads as a step that lost its answer, not as one that
/// deliberately has none.
fn settled_field_line(settled: &SettledField, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let value = match &settled.value {
        Some(value) => Span::styled(value.clone(), bold),
        None => Span::styled(ABSENT, dim),
    };

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(GUTTER_PAD, dim),
        Span::styled(format!("◇ {:<FIELD_LABEL_WIDTH$}  ", settled.label), dim),
        value,
    ])
}

/// How a settled optional field reads when the user left it empty.
const ABSENT: &str = "—";

/// The width every settled step label is padded to, so the values line up
/// down the sequence instead of stair-stepping.
const FIELD_LABEL_WIDTH: usize = 6;

/// The step's rejection, shown under the header.
///
/// `!` plus the sentence, in the `warning` role. The role existed so a
/// warning never invents a hue; this is the first thing to draw it. The
/// state is carried by the glyph and the words as much as by the colour,
/// so it reads the same with `NO_COLOR` set.
fn error_line(message: &str, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

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
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

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
fn update_note_line(version: &str, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

    Line::from(vec![
        Span::styled("◆", Style::default().fg(t.accent)),
        Span::styled(
            format!(" update available: v{version} — run sshm update"),
            dim,
        ),
    ])
}

/// The note an abandoned add sequence leaves.
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

/// The note a declined import offer leaves.
///
/// The decline half of [`ADD_ABANDONED_NOTE`], shorter because there was
/// no half-filled form to account for: nothing was written, and the one
/// thing the note has to carry is that the ask was *heard* — the empty
/// list under it reads as refused, not as never-offered.
const IMPORT_DECLINED_NOTE: &str = "import declined";

/// The `■` line: the confirm's question, answered.
///
/// `◆` asks and `■` has been answered — the glyph is the whole of the
/// difference between a live confirm and a settled one, which is what keeps
/// the distinction readable with colour off. The line recedes with
/// `fg_muted` + `DIM`; the alias and the answer stay bold so the two facts
/// a user scans for survive the recession.
fn settled_confirm_line(conn: &Connection, answered: bool, t: &Theme) -> Line<'static> {
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

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
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

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

    // The update note comes out of the row budget, not on top of it, so the
    // frame's height is the same with or without it — the invariant the
    // inline viewport and the settle-collapse both count on. See the
    // decision note on this function.
    let note_line = note.map(|v| update_note_line(v, &t));
    let note_rows = usize::from(note_line.is_some());
    let visible = canvas.visible_rows().unwrap_or(0).saturating_sub(note_rows);

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
/// chord: as of #37 `Ctrl+A` walks the add sequence and writes the
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
/// step actually reads, in the same order: the way out first. The import
/// offer wears the same rail — same answers, same order, because it is
/// the same kind of ask with the same conservative answer set.
///
/// The rail is fitted with the shared drop-from-the-end rule
/// ([`fit_hints_with`], the same one [`fit_hints`] is built on), against the
/// width left after the gutter. That is what turns "82 columns of hints in an
/// 80-column terminal" from a mid-word clip into a clean loss of the
/// least-needed segment.
fn hint_rail_line(mode: FrameMode, flow: &FrameFlow, width: usize, t: &Theme) -> Line<'static> {
    // Declared out here so the borrow outlives the `match` that picks one.
    const ADD_MID: &[&str] = &["Esc back", "Enter next", "Ctrl+C quit"];
    const ADD_LAST: &[&str] = &["Esc back", "Enter add", "Ctrl+C quit"];
    const EDITING: &[&str] = &["Esc back", "Enter save", "←→ field", "Ctrl+C quit"];
    const CONFIRMING: &[&str] = &["Esc back", "y confirm", "N abort", "Ctrl+C quit"];

    let hints: &[&str] = if flow.confirming.is_some() || flow.import_offer.is_some() {
        CONFIRMING
    } else if let Some(add) = &flow.add {
        // The add step names what Enter means *on this step*: `next`
        // while there are steps left, `add` on the last one, where the
        // same keypress commits the Connection. A rail that said `next`
        // there would be hinting a step that does not exist.
        if add.last {
            ADD_LAST
        } else {
            ADD_MID
        }
    } else if flow.edit.is_some() {
        // The editor's rail names the two things that are true only here:
        // the arrows move the *field*, not the list cursor, and Enter
        // saves rather than advances. Without `←→ field` on the rail the
        // arrows would be a mystery — the list uses them for nothing else,
        // and nothing else on screen says they do anything.
        EDITING
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
            // each chord now does the thing its label names — `Ctrl+A`
            // walks the five-step sequence and writes the Connection,
            // `Ctrl+E` opens the in-place single-field editor and writes
            // through the store.
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

    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let fitted = fit_hints_with(hints, width.saturating_sub(GUTTER), HINT_SEP);

    Line::from(vec![
        Span::styled(RAIL, Style::default().fg(t.border)),
        Span::styled(format!("{GUTTER_PAD}{fitted}"), dim),
    ])
}

/// The `◆ <question>` line that opens every frame.
///
/// While the delete confirm is open the question *is* the header — that is
/// the Clack grammar for a step: one line, one ask. The folder prefix and
/// the `(y/N)` answer hint are dim and the alias is bold, exactly as they
/// are in the row the confirm is about, so the two read as the same
/// Connection.
fn header_line(mode: FrameMode, flow: &FrameFlow, t: &Theme) -> Line<'static> {
    if let Some(target) = &flow.confirming {
        let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);

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

    // The import offer: the question *is* the header, same position and
    // grammar as the delete confirm's — one line, one ask (#38). The
    // count and the path are the two facts the answer is about, so they
    // are bold; the words that only glue them together recede, exactly as
    // the folder prefix does in the delete question. The path is shown
    // in the form the user recognises (see [`display_import_path`]),
    // display-only: the write still goes to the raw path the phase
    // carries.
    if let Some(offer) = &flow.import_offer {
        let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);

        let home = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned));
        let shown = display_import_path(&offer.path, home.as_deref());
        // One offered Connection reads "Import 1 connection", not
        // "Import 1 connections" — the same plural the note keeps, so
        // the ask and the answer agree on the noun.
        let noun = if offer.count == 1 {
            "connection"
        } else {
            "connections"
        };

        return Line::from(vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled("Import ", bold),
            Span::styled(offer.count.to_string(), bold),
            Span::styled(format!(" {noun} from "), dim),
            Span::styled(shown, bold),
            Span::styled("? ", bold),
            Span::styled("(y/N)", dim),
        ]);
    }

    // The add step: the header *is* the step, one line, one ask — the
    // same grammar the delete confirm uses. The typed text rides on the
    // header with it because the frame hides the terminal cursor and a
    // text field with no cursor and no echo of its own is a field the
    // user cannot see themselves filling.
    if let Some(add) = &flow.add {
        let dim = Style::default().fg(t.fg_muted);

        return Line::from(vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled(
                add.label.to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(add.input.clone()),
            Span::styled(CARET, dim),
        ]);
    }

    // The edit: the header names the **target**, not the field.
    //
    // The add header wears the field word because the field is the whole
    // ask. Here the field is drawn on its own line just below, and what
    // the top of the frame must answer instead is "which Connection am I
    // about to change?" — the one question that cannot be recovered from
    // the field line, and the one that matters most before a write.
    //
    // Same shape as the delete confirm's header: `◆ Edit`, dim folder
    // prefix, bold alias. The two chords that change a Connection read as
    // the same kind of thing.
    if let Some(edit) = &flow.edit {
        let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled("◆", Style::default().fg(t.accent)),
            Span::raw(" "),
            Span::styled("Edit ", bold),
        ];

        if let Some(folder) = edit.target.folder.as_deref().filter(|f| !f.is_empty()) {
            spans.push(Span::styled(format!("[{folder}] "), dim));
        }

        spans.push(Span::styled(edit.target.alias.clone(), bold));

        return Line::from(spans);
    }

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

/// The import offer's path in the form the user recognises: a home-dir
/// prefix shown as `~/`, the way they type it (#38).
///
/// Pure on purpose: `home` is a parameter, not a call to
/// `dirs::home_dir()`, so the rule is pinned against a fake home rather
/// than the machine the test happens to run on. The header passes the
/// real home; the tests pass a made-up one.
///
/// The cut falls at a **path boundary**, never at a string prefix:
/// `/home/anyoneelse` merely starts with `/home/anyone` and is not
/// inside it, so it stays raw — mangling it would point the user at a
/// file that does not exist. With no home to explain the path, the raw
/// path is shown: a `~` standing for nothing is worse than the long
/// form. Display-only either way — the write goes to the raw path the
/// phase carries, never to the abbreviation.
pub fn display_import_path(path: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        return path.to_string();
    };

    if path == home {
        return "~".to_string();
    }

    match path.strip_prefix(home) {
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The caret the add step draws for itself.
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
