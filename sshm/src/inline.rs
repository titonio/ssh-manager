//! The inline render + settle-collapse mechanics (#34).
//!
//! [`crate::frame`] turns a `(connections, query, selection, mode, canvas)`
//! into a `Frame`: a value. This module is what makes that value *live* in
//! somebody's shell without wrecking it — and, because terminal mechanics are
//! where bugs hide from unit tests, it is split into the two halves that can be
//! proved in different ways:
//!
//! * **The model** ([`diff_rows`], [`settle_trace`], [`plan_resize`],
//!   [`fit_visible_rows`] via the canvas) is pure: rows in, instructions out.
//!   It decides *what* has to happen and is tested with no terminal at all.
//! * **The driver** ([`LiveFrame`]) is thin on purpose. Every decision is
//!   already made by the model; what is left is cursor arithmetic, so it is
//!   kept to a handful of escape sequences and proved by running it.
//!
//! Four rules bind the whole module:
//!
//! 1. **Never the alternate screen.** The frame lives inside the user's own
//!    scrollback, so it must never ask for `?1049`. This path does not build a
//!    ratatui `Terminal` at all — there is no object here that owns a viewport,
//!    which is what makes "never" structural rather than a promise.
//! 2. **Constant height while active.** The frame's shape comes from the
//!    canvas ([`crate::frame::FRAME_LINES`]); the driver never lets the list
//!    change how many lines are on screen.
//! 3. **Leave the line clean.** Every exit collapses the live rows away — the
//!    rows that changed are rewritten, the rows that are gone are deleted — so
//!    no orphan row or rail fragment survives the frame. "Clean" is counted in
//!    *physical* rows, because a narrowing terminal re-wraps the glass before
//!    it says so; [`LiveFrame::note_width`] is what stops the drawn count and
//!    the physical count from drifting apart.
//! 4. **One stream, both directions.** Everything the frame writes — rows,
//!    cursor hide, cursor restore — goes to the writer the caller was given,
//!    never to a hardcoded `stdout`. Under `result=$(sshm pick …)` stdout is
//!    a pipe: a control sequence there corrupts the emitted selection, and the
//!    real terminal keeps whatever state the restore was meant to undo.

use std::io::{self, Write};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Connection;
use crate::connections::{ConnectionDraft, Store};
use crate::frame::{
    build_frame_with_note, fit_line, fit_visible_rows, Canvas, Frame, FrameFlow, FrameMode,
};
use crate::manage::{self, Effect, ManageState};
use crate::theme::{self, Theme};

// ─────────────────────────────────────────────────────────────────────────────
// The model
// ─────────────────────────────────────────────────────────────────────────────

/// What the live frame collapses to when it closes.
///
/// Both arms leave a trace, because both are the user's last sight of the
/// frame: a pick says what was chosen, a cancel says the frame was left on
/// purpose rather than having vanished.
///
/// This is the **one** settle line, for all three commands. The `--emit`
/// axis decides what happens to the pick *after* the frame is gone
/// (execute / insert / edit); it must not print a second one. Story 13
/// asks for a single settle line, and under `sshm manage` the runner used
/// to add `◆ editing` on top of the frame's own `◆ picked` — one
/// keystroke reading as two events (#35).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// A Connection was chosen.
    Picked(Connection),
    /// The user left with `Esc`/`Ctrl+C`, or the frame could not be preserved.
    Cancelled,
    /// An action the store refused.
    ///
    /// The `error` arm of #31's `Settled` set, which story 34 requires be
    /// designed. A failed delete used to escape `run_inline` with the frame
    /// still on screen — no collapse, no trace, and a live list of
    /// Connections the store had just proved it could not vouch for. The
    /// frame now collapses to this line and the process exits non-zero, so
    /// the failure is both readable and loud.
    Error {
        /// The Connection the failed action was about.
        connection: Connection,
        /// The store's own reason, carried through verbatim.
        message: String,
    },
    /// An add the store refused, named by the draft the user finished
    /// typing rather than by a Connection that does not exist.
    ///
    /// The distinction matters because the whole point of the trace is
    /// what it claims about `connections.json`: there is no Connection
    /// here to name, only a draft that never became one. The line it
    /// renders is the `◆ error` line all the same — the failure reads the
    /// same shape whatever it was about.
    AddFailed {
        /// The draft that was refused.
        draft: ConnectionDraft,
        /// The store's own reason, carried through verbatim.
        message: String,
    },
    /// An import the store refused (#38).
    ///
    /// The store refused the write; the frame collapses rather than
    /// showing a list it cannot vouch for — the same contract
    /// [`Settle::Error`] and [`Settle::AddFailed`] hold, for the same
    /// reason. There is no Connection to name here: the ask was about a
    /// file, not a row, and a line that invented one would claim an
    /// object the set never received. So the line carries the store's
    /// reason alone — `◆ import failed: <reason>` — dim where the other
    /// failure traces are dim, the `◆` carrying the state.
    ImportFailed {
        /// The store's own reason, carried through verbatim.
        message: String,
    },
}

/// One row of the frame, as an instruction to the driver.
///
/// The three instructions are the whole of the row-diff: a row that did not
/// change is left alone, a row that changed is rewritten in place, a row that
/// no longer exists is deleted so the rows under it pull up.
#[derive(Debug, Clone, PartialEq)]
pub enum RowOp {
    /// The row already shows what it should. Move past it.
    Keep,
    /// Replace this row with `line`. Move past it.
    Rewrite(Line<'static>),
    /// This row is gone. Delete it and stay on the row that took its place.
    Erase,
}

/// What is currently on screen under the live frame — the two facts a resize
/// decision needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Live {
    /// How many rows the frame occupies right now.
    pub rows: usize,
    /// The display width of the widest row drawn.
    ///
    /// This is what makes preservation decidable: a live row wider than the
    /// new terminal has wrapped, and a wrapped row cannot be counted, so
    /// erasing "the frame" would leave an orphan behind.
    pub widest: usize,
}

/// What to do when the terminal changes size mid-frame (user story 37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizePlan {
    /// Tear the frame down and re-open it at the new size. The query and the
    /// selection are the caller's, so carrying them over costs nothing.
    Reopen {
        /// How many list rows the new terminal leaves the frame.
        visible_rows: usize,
    },
    /// The live rows cannot be erased cleanly at the new size, or the new
    /// terminal cannot hold a frame at all: cancel and give the screen back.
    CancelAndRestore,
}

/// Decide what a resize means for a live frame.
///
/// Growing is always safe. Narrowing is safe only while nothing already drawn
/// has to wrap: the terminal reflows those rows before we are told, so the
/// row count we are holding stops matching the rows on the glass. Anything
/// else falls back to a clean cancel-and-restore rather than guessing.
pub fn plan_resize(live: Live, new_width: usize, new_height: usize) -> ResizePlan {
    if live.widest > new_width {
        return ResizePlan::CancelAndRestore;
    }

    match fit_visible_rows(new_height) {
        Some(visible_rows) => ResizePlan::Reopen { visible_rows },
        None => ResizePlan::CancelAndRestore,
    }
}

/// Row-diff the live frame against what it should become.
///
/// Positional, not LCS: the frame is a fixed-height stack of rows, so row *i*
/// of the old frame and row *i* of the new one are the same row. That is what
/// lets the collapse be "rewrite the first line, delete the rest" instead of
/// a matching problem, and it is why a keystroke costs the two rows that
/// actually moved rather than the whole frame.
pub fn diff_rows(prev: &[Line<'static>], next: &[Line<'static>]) -> Vec<RowOp> {
    (0..prev.len().max(next.len()))
        .map(|i| match (prev.get(i), next.get(i)) {
            (Some(before), Some(after)) if before == after => RowOp::Keep,
            (_, Some(after)) => RowOp::Rewrite(after.clone()),
            (Some(_), None) => RowOp::Erase,
            (None, None) => RowOp::Keep,
        })
        .collect()
}

/// The one line a closed frame leaves behind.
///
/// `◆ picked  web-01  (deploy@10.0.0.4:22)` is the spec's worked example
/// (user story 13) and is emitted verbatim for a Connection that has no
/// folder. A Connection that *does* have one gets it back: story 8 says the
/// folder prefix shows only when a Connection has one, story 27 keeps it in
/// a settle trace (`◇ added [prod] web-01`), and `config.rs` imposes no
/// alias-uniqueness across folders — so dropping it would leave
/// `[prod] web-01` and a folder-less `web-01` with byte-identical traces
/// and no way to tell which one was actually picked. A cancel leaves
/// `◆ cancelled`.
///
/// The emphasis is the frame's own grammar — accent step icon, bold alias,
/// dim meta — so the trace reads as the same design system as the list it
/// replaced, and carries its meaning with the colour turned off.
pub fn settle_trace(settle: &Settle, canvas: Canvas) -> Vec<Line<'static>> {
    let t = Theme::clack().resolve(canvas.support);
    // `◆` is the accent glyph in `tokens.md`. Both traces wear that role: the
    // cancel icon is state ("this step was abandoned"), and `border` is
    // documented as structural chrome that carries none.
    let icon = |role: ratatui::style::Color| Span::styled("◆", Style::default().fg(role));
    let dim = Style::default().fg(t.fg_muted);

    match settle {
        Settle::Picked(conn) => vec![connection_trace("picked", &TraceSubject::from(conn), &t)],
        Settle::Cancelled => vec![Line::from(vec![
            icon(t.accent),
            Span::styled(" cancelled", dim),
        ])],
        Settle::Error {
            connection,
            message,
        } => {
            // Built through `connection_trace` so the verb is read by the
            // `#31 Settled set` gate in `design_system_test.rs` rather
            // than sneaking past it.
            let mut line = connection_trace("error", &TraceSubject::from(connection), &t);
            push_reason(&mut line, message, canvas.fit_width(), &t);
            vec![fit_line(line, canvas.fit_width())]
        }
        Settle::AddFailed { draft, message } => {
            // The same `◆ error` line, with the draft in place of the
            // Connection: the failure is the same failure, and a second
            // shape for it would be a second grammar.
            let mut line = connection_trace("error", &TraceSubject::from(draft), &t);
            push_reason(&mut line, message, canvas.fit_width(), &t);
            vec![fit_line(line, canvas.fit_width())]
        }
        Settle::ImportFailed { message } => {
            // No `connection_trace` here: an import is about a file,
            // not a row, and there is no Connection to name. The line
            // keeps the failure grammar the other traces use — accent
            // glyph, dim text, the store's own reason cut to the
            // canvas — so a refused import reads as the same class of
            // failure even though it names nothing.
            let mut line = Line::from(vec![icon(t.accent), Span::styled(" import failed: ", dim)]);
            let reason = fit_reason(message, line.width(), canvas.fit_width());
            if !reason.is_empty() {
                line.spans.push(Span::styled(reason, dim));
            }
            vec![fit_line(line, canvas.fit_width())]
        }
    }
}

/// Append the store's reason to an error trace, cut to the room left over.
fn push_reason(line: &mut Line<'static>, message: &str, budget: usize, t: &Theme) {
    // The reason is the only part of this line that comes from
    // outside the program, so it is the only part that can arrive
    // too long. It is cut to the room left over — with an ellipsis,
    // because a message that simply stops reads as a message that
    // finished — and the whole line is still fitted as a last
    // defence: the collapse counts one settle line as one physical
    // row, and a wrapped row strands half a message on the glass.
    let reason = fit_reason(message, line.width(), budget);
    if !reason.is_empty() {
        let dim = Style::default().fg(t.fg_muted);
        line.spans.push(Span::styled(format!("  — {reason}"), dim));
    }
}

/// Cut a settle-trace reason down to the room the canvas leaves.
///
/// `used` is what the line already spends before the reason; `budget` is
/// the canvas's own fit width. A reason that fits comes back untouched; one
/// that does not comes back with its last column spent on `…`, so the
/// reader can see the cut rather than mistaking it for the end of the
/// sentence. With no room at all the reason is dropped — `◆ error` plus the
/// Connection still says what the frame needs to say, and the full text is
/// on stderr regardless.
fn fit_reason(message: &str, used: usize, budget: usize) -> String {
    const SEP: usize = 4; // "  — "
    if used + SEP + message.chars().count() <= budget {
        return message.to_string();
    }
    let room = budget.saturating_sub(used + SEP);
    if room == 0 {
        return String::new();
    }
    let mut kept: String = message.chars().take(room - 1).collect();
    kept.push('…');
    kept
}

/// What a settle trace is about, in the five fields the line renders.
///
/// A trace used to be able to name only a stored `Connection`. An add the
/// store refused has no Connection to name — only the draft that never
/// became one — and inventing a `Connection` value for the failure would
/// put a fake id on the glass's most permanent line. So the trace names
/// the *fields*, and both a `Connection` and a `ConnectionDraft` know how
/// to answer for them.
struct TraceSubject<'a> {
    folder: Option<&'a str>,
    alias: &'a str,
    user: &'a str,
    host: &'a str,
    port: String,
}

impl<'a> From<&'a Connection> for TraceSubject<'a> {
    fn from(conn: &'a Connection) -> Self {
        Self {
            folder: conn.folder.as_deref(),
            alias: &conn.alias,
            user: &conn.user,
            host: &conn.host,
            port: conn.port.to_string(),
        }
    }
}

impl<'a> From<&'a ConnectionDraft> for TraceSubject<'a> {
    fn from(draft: &'a ConnectionDraft) -> Self {
        Self {
            folder: none_if_blank(&draft.folder),
            alias: &draft.alias,
            user: &draft.user,
            host: &draft.host,
            port: draft.port.clone(),
        }
    }
}

fn none_if_blank(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// One `◆ <verb>  [folder] alias  (user@host:port)` line.
///
/// The single shape every settle trace takes, so two verbs can never drift
/// apart in grammar. The verb is a state from #31's `Settled` set
/// (`picked | cancelled | added | edited | deleted | error`): the gate in
/// `tests/design_system_test.rs` reads the verbs out of this call site, so
/// a verb the spec does not have fails the build instead of shipping a
/// scrollback that claims something the command never did.
fn connection_trace(verb: &str, subject: &TraceSubject<'_>, t: &Theme) -> Line<'static> {
    let icon = |role: ratatui::style::Color| Span::styled("◆", Style::default().fg(role));
    let dim = Style::default().fg(t.fg_muted);
    let bold = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);

    let mut spans = vec![icon(t.accent), Span::raw(format!(" {verb}  "))];

    if let Some(folder) = subject.folder.filter(|f| !f.is_empty()) {
        spans.push(Span::styled(format!("[{folder}] "), dim));
    }

    spans.push(Span::styled(subject.alias.to_string(), bold));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("({}@{}:{})", subject.user, subject.host, subject.port),
        dim,
    ));

    Line::from(spans)
}

// ─────────────────────────────────────────────────────────────────────────────
// The driver
// ─────────────────────────────────────────────────────────────────────────────

/// Move the cursor to the top of the live frame, at column zero.
fn up(rows: usize) -> String {
    format!("\x1b[{rows}A\r")
}

/// Erase the row the cursor is on.
const ERASE_ROW: &[u8] = b"\x1b[2K";

/// Delete the row the cursor is on, pulling the rows below it up.
const DELETE_ROW: &[u8] = b"\x1b[M";

/// A live frame: the frame's lines, on the glass, in the user's own scrollback.
///
/// The driver tracks how many rows it drew, how wide the widest one was, and
/// how many *physical* rows those rows currently occupy — which is what the
/// next redraw and the next resize need. It never reads the terminal and never
/// owns a viewport: it writes, and remembers what it wrote.
#[derive(Debug)]
pub struct LiveFrame<'w, W: Write> {
    out: &'w mut W,
    drawn: Vec<Line<'static>>,
    widest: usize,
    /// Physical rows the drawn rows currently stand on.
    ///
    /// Equal to `drawn.len()` while nothing has wrapped, and larger once a
    /// narrowing has re-wrapped rows into two. Every `up(N)` and `\x1b[M`
    /// the driver issues counts *this*, not the line count, because that is
    /// what the terminal counts.
    occupied: usize,
}

impl<'w, W: Write> LiveFrame<'w, W> {
    /// A driver writing to `out`, with nothing drawn yet.
    pub fn new(out: &'w mut W) -> Self {
        Self {
            out,
            drawn: Vec::new(),
            widest: 0,
            occupied: 0,
        }
    }

    /// What is on screen right now, for [`plan_resize`] to decide against.
    pub fn geometry(&self) -> Live {
        Live {
            rows: self.drawn.len(),
            widest: self.widest,
        }
    }

    /// How many physical rows the drawn rows occupy at `width` columns.
    ///
    /// The frame's rows are hard-newlined, so each is its own logical line
    /// and a terminal `width` columns wide re-wraps it into
    /// [`physical_rows`] of its own. Summing that is the only honest way to
    /// count what has to be erased after a narrowing.
    pub fn physical_rows(&self, width: usize) -> usize {
        self.drawn
            .iter()
            .map(|line| crate::frame::physical_rows(line.width(), width))
            .sum()
    }

    /// Tell the driver the terminal is now `width` columns wide.
    ///
    /// The terminal re-wraps the rows already on the glass *before* it says
    /// anything, so a narrowing silently turns N drawn rows into more than N
    /// physical rows. This recomputes the count the collapse has to use.
    /// Call it on every resize, before acting on the plan — otherwise the
    /// rewind lands short, the trace prints mid-frame, and the wrapped tails
    /// survive above it.
    pub fn note_width(&mut self, width: usize) {
        self.occupied = self.physical_rows(width);
    }

    /// Open the frame on the line *below* the cursor.
    ///
    /// The leading newline is the whole of "renders below the prompt": the
    /// frame never lands on the line the user is typing on, and the prompt
    /// stays exactly where it was.
    pub fn open(&mut self, frame: &Frame) -> io::Result<()> {
        self.out.write_all(b"\r\n")?;
        self.paint(frame.lines())
    }

    /// Open straight into a settle trace, without ever drawing a live frame.
    ///
    /// The path a terminal too short to host the frame takes: it still lands
    /// below the prompt and still leaves a trace, so a cancel reads the same
    /// whether or not a frame ever drew. The alternative — opening a frame
    /// whose list area collapsed to zero rows — leaves the user staring at
    /// three lines of chrome with nothing in it.
    pub fn open_settled(&mut self, trace: &[Line<'static>]) -> io::Result<()> {
        self.out.write_all(b"\r\n")?;
        self.paint(trace)
    }

    /// Redraw in place, writing only the rows the new frame changed.
    pub fn redraw(&mut self, frame: &Frame) -> io::Result<()> {
        self.rewind()?;
        let ops = diff_rows(&self.drawn, frame.lines());
        self.apply(&ops)
    }

    /// Collapse the live frame to `trace` — the settle-collapse.
    ///
    /// The first row becomes the trace and every row below it is deleted, so
    /// the live list is gone rather than having been painted over.
    ///
    /// The rows below are counted *physically*, not as drawn lines. After a
    /// narrowing reflow the glass holds more rows than the frame was drawn
    /// with, and diffing the drawn lines against the trace would delete the
    /// drawn surplus only — leaving the wrapped tails, and the `└`, sitting
    /// above the trace.
    pub fn collapse(&mut self, trace: &[Line<'static>]) -> io::Result<()> {
        self.rewind()?;

        let mut painted = Vec::with_capacity(trace.len());
        for line in trace {
            self.write_row(line)?;
            painted.push(line.clone());
        }

        let leftover = self.occupied.saturating_sub(painted.len());
        if leftover > 0 {
            self.out.write_all(format!("\x1b[{leftover}M").as_bytes())?;
        }

        self.commit(painted)
    }

    /// Tear the frame down and draw a new one where it stood, at whatever size
    /// the new frame is. The resize path's re-open.
    pub fn reopen(&mut self, frame: &Frame) -> io::Result<()> {
        self.rewind()?;
        self.delete_live_rows()?;
        self.paint(frame.lines())
    }

    /// Remove every live row, giving back the screen as it was before the
    /// frame opened.
    pub fn erase(&mut self) -> io::Result<()> {
        self.rewind()?;
        self.delete_live_rows()
    }

    /// Move the cursor back to the top of the live frame.
    ///
    /// Counts physical rows, not drawn lines: after a narrowing reflow the
    /// two are not the same number, and the difference is exactly how far
    /// short the rewind would otherwise land.
    fn rewind(&mut self) -> io::Result<()> {
        if self.occupied == 0 {
            return Ok(());
        }
        self.out.write_all(up(self.occupied).as_bytes())
    }

    /// Draw `lines` from where the cursor stands, one row per line.
    fn paint(&mut self, lines: &[Line<'static>]) -> io::Result<()> {
        let mut next = Vec::with_capacity(lines.len());
        for line in lines {
            self.write_row(line)?;
            next.push(line.clone());
        }
        self.commit(next)
    }

    /// Execute a row-diff from where the cursor stands.
    fn apply(&mut self, ops: &[RowOp]) -> io::Result<()> {
        let mut next: Vec<Line<'static>> = Vec::with_capacity(ops.len());

        for (i, op) in ops.iter().enumerate() {
            match op {
                RowOp::Keep => {
                    // Nothing to draw, but the row still occupies a line: step
                    // down without touching it.
                    self.out.write_all(b"\r\n")?;
                    if let Some(line) = self.drawn.get(i) {
                        next.push(line.clone());
                    }
                }
                RowOp::Rewrite(line) => {
                    self.write_row(line)?;
                    next.push(line.clone());
                }
                RowOp::Erase => {
                    // Delete without stepping down: the row underneath has
                    // just moved up into this one's place.
                    self.out.write_all(DELETE_ROW)?;
                }
            }
        }

        self.commit(next)
    }

    /// Write one row: clear the line, draw it, step down.
    fn write_row(&mut self, line: &Line<'static>) -> io::Result<()> {
        self.out.write_all(ERASE_ROW)?;
        self.out
            .write_all(theme::ansi::line_to_ansi(line).as_bytes())?;
        self.out.write_all(b"\r\n")
    }

    /// Delete the rows the frame currently occupies.
    fn delete_live_rows(&mut self) -> io::Result<()> {
        let rows = self.occupied;
        if rows > 0 {
            self.out.write_all(format!("\x1b[{rows}M").as_bytes())?;
        }
        self.commit(Vec::new())
    }

    /// Record what is now on screen.
    ///
    /// `widest` is measured in display columns, not characters: a CJK alias
    /// counts one per character and spends two per column, and the resize
    /// decision is made against what the terminal actually ran out of.
    fn commit(&mut self, drawn: Vec<Line<'static>>) -> io::Result<()> {
        self.widest = drawn.iter().map(|line| line.width()).max().unwrap_or(0);
        self.drawn = drawn;
        // Whatever was just painted was painted at the width we are standing
        // at, so it occupies exactly as many physical rows as there are
        // lines. A later narrowing changes that; `note_width` re-reads it.
        self.occupied = self.drawn.len();
        self.out.flush()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The live loop
// ─────────────────────────────────────────────────────────────────────────────

/// What the user did with a live frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineOutcome {
    /// Enter on a row. The `Connection` is the frame's own selection, so the
    /// caller never has to re-run the filter to agree with what was shown.
    Picked(Connection),
    /// `Esc`, `Ctrl+C`, or a resize the frame could not survive.
    Cancelled,
}

/// Drive a live inline frame to its settle.
///
/// This is the mechanics, not the command: it owns the frame, the keystrokes
/// that move through it, and the collapse that closes it. What the pick *means*
/// — execute, insert, edit — is the `--emit` axis and belongs to the command
/// cut-over (#35), which is a thin caller of this function.
///
/// The frame is drawn to `out`, which is the user's terminal (or, under a
/// captured stdout, whatever the caller has redirected there). Keys come from
/// the terminal's own event stream.
///
/// In [`FrameMode::Manage`] the keystrokes are not decided here: they are
/// handed to [`crate::manage::step`], and this function performs what that
/// returns through the [`Store`] it was given. That split is the whole point
/// of #36 — the delete confirm's decisions are unit-testable because they
/// are not written in this loop, and what is left here is the part that
/// genuinely needs a terminal.
///
/// This entry point opens the frame on a query-seeded list. A caller that
/// needs the frame to open on some other step — the first-run import offer,
/// which is a phase the command decides on before any keystroke exists —
/// goes through [`run_inline_with_state`] instead.
pub fn run_inline<W: Write>(
    out: &mut W,
    store: &mut dyn Store,
    initial_query: String,
    mode: FrameMode,
    note: Option<String>,
) -> io::Result<InlineOutcome> {
    run_inline_with_state(
        out,
        store,
        ManageState::with_query(initial_query),
        mode,
        note,
    )
}

/// [`run_inline`], opened on a caller-supplied [`ManageState`] (#38).
///
/// The first-run import offer is not a keystroke: it is a step the frame
/// opens *on*, decided by the command before the driver starts. Seeding
/// it through this entry point is what lets the offer travel as the same
/// `ManageState` every later keystroke produces — the driver does not grow
/// a flag to re-derive, and `run_inline` keeps its own meaning. The
/// behaviour from here is identical to [`run_inline`]'s: the state is
/// where the two differ, and only at the start.
///
/// `note` is the cached update note (#39), read once by the command at the
/// edge before the driver starts and handed down here as data. The driver
/// passes it to the frame on every build but never re-reads it: the note is
/// frozen at the value the edge read, so it cannot appear, vanish, or change
/// while the frame is live. The frame displaces a row for it, so the
/// viewport height is fixed before `open` and never moves.
pub fn run_inline_with_state<W: Write>(
    out: &mut W,
    store: &mut dyn Store,
    initial: ManageState,
    mode: FrameMode,
    note: Option<String>,
) -> io::Result<InlineOutcome> {
    let (width, height) = crossterm::terminal::size()?;
    let paint = Paint {
        mode,
        note: note.as_deref(),
        canvas: Canvas::detect(width as usize, height as usize),
    };
    open_and_drive(
        &mut CrosstermEvents,
        &mut CrosstermRawMode,
        out,
        store,
        paint,
        initial,
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// The driver
//
// Everything below the entry point is the part of the command that does not
// need a terminal to be worth proving: which keystroke means which step, which
// step asks the store for which write, and what the frame leaves on the glass
// when it closes. The two things it cannot do without a terminal — reading keys
// and switching the terminal's own input mode — arrive as the [`Events`] and
// [`RawMode`] seams, so the driver proper is driven from a script.
// ─────────────────────────────────────────────────────────────────────────────

/// What the frame is painted for, and on what: the emit mode that decides
/// which keys it has, the update note frozen at the edge (#39), and the canvas
/// the terminal offers.
///
/// They travel together because they are one fact about the frame — the frame
/// is a function of these three and of the state — and only the canvas ever
/// changes, when a resize re-reads the terminal.
struct Paint<'a> {
    mode: FrameMode,
    note: Option<&'a str>,
    canvas: Canvas,
}

/// The keystroke stream, as the driver sees it.
///
/// The driver's only source of keystrokes, and the seam that lets a test hand
/// it a script instead of a terminal. Everything above it — the routing of a
/// key to a step, the performing of a step's effects — is then assertable
/// without a pty, which is the difference between a driver whose complexity is
/// tested and one whose complexity is only hoped for.
pub(crate) trait Events {
    /// The next event the terminal sent.
    fn event(&mut self) -> io::Result<Event>;
}

/// The switch that puts the terminal into raw mode for the life of the frame.
///
/// Separate from [`Events`] so a test can drive a whole frame without putting
/// its own stdin into raw mode and losing the ability to report a failure.
///
/// Only *entering* is a seam. Leaving stays the [`RawModeGuard`]'s job on the
/// real terminal, because the order is load-bearing and belongs fixed in one
/// place: the cursor is restored before raw mode goes off, which is what the
/// guard's declaration order buys every exit path — including the ones that
/// leave by `?`, where an explicit `leave()` would be skipped.
pub(crate) trait RawMode {
    /// Enter raw mode. The caller leaves it again with the [`RawModeGuard`] it
    /// keeps for the life of the frame.
    fn enter(&mut self) -> io::Result<()>;
}

/// The terminal's own keystroke stream.
pub(crate) struct CrosstermEvents;

impl Events for CrosstermEvents {
    fn event(&mut self) -> io::Result<Event> {
        crossterm::event::read()
    }
}

/// The terminal's own raw-mode switch.
pub(crate) struct CrosstermRawMode;

impl RawMode for CrosstermRawMode {
    fn enter(&mut self) -> io::Result<()> {
        crossterm::terminal::enable_raw_mode()
    }
}

/// Paint the frame and drive it to its settle: keystrokes from `events`, raw
/// mode entered through `raw`.
///
/// Both are injected because everything past here is the part of the command a
/// test can prove. What stays here is the ordering no test should get to
/// re-decide: a terminal too short for a frame never gets one, and never gets
/// raw mode or a hidden cursor either.
fn open_and_drive<W: Write, E: Events, R: RawMode>(
    events: &mut E,
    raw: &mut R,
    out: &mut W,
    store: &mut dyn Store,
    mut paint: Paint<'_>,
    initial: ManageState,
) -> io::Result<InlineOutcome> {
    // A terminal too short to host the frame never gets one. This is the
    // same cancel-and-restore the resize path falls back to, applied at open:
    // without it `build_frame` emits three lines of chrome wrapped around a
    // list area that fitted zero rows, which is a degenerate frame rather
    // than a clean refusal.
    if paint.canvas.visible_rows().is_none() {
        let mut live = LiveFrame::new(out);
        return settle_before_open(&mut live, paint.canvas);
    }

    // The frame is assembled before the terminal is touched, in this order
    // because it has always been in this order: a build that cannot be done
    // must not leave the terminal in raw mode with a hidden cursor to explain.
    let mut state = initial;
    let mut frame = build(store, &mut state, &paint);

    raw.enter()?;
    let _raw = RawModeGuard;
    let mut cursor = CursorGuard::new(out);
    let mut live = LiveFrame::new(cursor.writer());
    live.open(&frame)?;

    drive(events, &mut live, store, &mut state, &mut frame, &mut paint)
}

/// Read events until one of them ends the frame, repainting after every one
/// that does not.
///
/// The loop is the whole of the driver's own behaviour, and deliberately
/// nothing more: an event arrives, one of the handlers below decides what it
/// means, and `Some` is the frame's answer. Every path out of here goes
/// through a settle, so the glass is never left mid-frame.
fn drive<W: Write, E: Events>(
    events: &mut E,
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &mut Paint<'_>,
) -> io::Result<InlineOutcome> {
    loop {
        let answer = match events.event()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                keystroke(live, store, state, frame, &*paint, key)
            }
            Event::Resize(width, height) => resize(
                live,
                store,
                state,
                frame,
                paint,
                usize::from(width),
                usize::from(height),
            ),
            // A release, a repeat, a mouse: nothing the frame has a use for,
            // and nothing that repaints.
            _ => Ok(None),
        }?;

        if let Some(outcome) = answer {
            return Ok(outcome);
        }
    }
}

/// One keystroke, routed by the frame's mode: manage asks [`manage::step`]
/// what it means, the pick frame decides for itself.
fn keystroke<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &Paint<'_>,
    key: KeyEvent,
) -> io::Result<Option<InlineOutcome>> {
    match paint.mode {
        FrameMode::Manage => manage_key(live, store, state, frame, paint, key),
        FrameMode::Pick => pick_key(live, store, state, frame, paint, key),
    }
}

/// One keystroke in the manage frame: ask [`manage::step`] what it means,
/// perform what it asks for through the store, fold every answer straight
/// back into the state, and repaint.
///
/// `Some` is the frame's final answer, already collapsed; `None` means the
/// frame is still open and freshly painted. The keystrokes are not decided
/// here — that is [`manage::step`], which is a pure function and therefore
/// testable — only performed.
fn manage_key<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &Paint<'_>,
    key: KeyEvent,
) -> io::Result<Option<InlineOutcome>> {
    let selected = frame
        .selected_connection_index()
        .map(|index| store.all()[index].clone());
    let step = manage::step(state, key, selected.as_ref());
    *state = step.state;

    for effect in step.effects {
        if let Some(outcome) = perform(live, store, state, effect, paint)? {
            return Ok(Some(outcome));
        }
    }

    repaint(live, store, state, frame, paint)
}

/// Perform one effect the manage step asked for, through the store, and fold
/// the store's answer straight back into the state.
///
/// Every arm keeps the same discipline, and it is stated once here rather than
/// in each of them: the effect is performed here, its result is folded back
/// through the matching `manage::settle_*`, and so the note the frame paints
/// reports what the store did rather than what was asked for. A store that
/// refuses collapses the frame to `◆ error …` and the process leaves non-zero
/// — a painted frame the user cannot read the failure in, over a list the
/// store just proved unreliable, is the state this replaces.
///
/// `Some` is the frame's final answer.
fn perform<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    effect: Effect,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    match effect {
        Effect::Exit(outcome) => settle(live, paint.canvas, outcome).map(Some),
        Effect::Delete { target } => perform_delete(live, store, state, target, paint),
        Effect::Add { draft } => perform_add(live, store, state, draft, paint),
        Effect::Update { target, draft } => {
            perform_update(live, store, state, target, draft, paint)
        }
        Effect::Import { path } => perform_import(live, store, state, path, paint),
    }
}

/// The settled line records the answer the user gave; whether anything was
/// removed is the store's to tell. `Ok(None)` in particular means nothing was,
/// and the frame says so.
fn perform_delete<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    target: Connection,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    let outcome = manage::DeleteOutcome::from_remove(store.remove(&target.id));
    match manage::settle_delete(state, &target, outcome) {
        Ok(next) => {
            *state = next;
            Ok(None)
        }
        Err(message) => settle_error(live, paint.canvas, &target, message).map(Some),
    }
}

/// [`Store::add`] is the only implementation of "add a Connection": the frame
/// does not get its own, so the `◇ added` note can only ever report what the
/// store answered.
fn perform_add<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    draft: ConnectionDraft,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    let added = store.add(&draft);
    match manage::settle_add(state, store.all(), added) {
        Ok(next) => {
            *state = next;
            Ok(None)
        }
        Err(message) => settle_add_error(live, paint.canvas, &draft, message).map(Some),
    }
}

/// `Ok(None)` means the Connection was not there to change, and the frame says
/// so instead of claiming an edit the editor never landed.
fn perform_update<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    target: Connection,
    draft: ConnectionDraft,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    let outcome = manage::EditOutcome::from_update(store.update(&target.id, &draft));
    match manage::settle_edit(state, store.all(), &target, outcome) {
        Ok(next) => {
            *state = next;
            Ok(None)
        }
        Err(message) => settle_error(live, paint.canvas, &target, message).map(Some),
    }
}

/// An import is about a file, not a row: the `◇ imported` note reports how
/// many stanzas the store wrote, not how many the scan promised.
fn perform_import<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &mut dyn Store,
    state: &mut ManageState,
    path: String,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    let outcome = manage::ImportOutcome::from_report(store.import_ssh_config(&path));
    match manage::settle_import(state, outcome) {
        Ok(next) => {
            *state = next;
            Ok(None)
        }
        Err(message) => settle_import_error(live, paint.canvas, message).map(Some),
    }
}

/// One keystroke in the pick frame: the arrows and `j`/`k` move, typing
/// filters, Enter takes the row under the cursor, and `Esc` or `Ctrl+C`
/// leaves.
fn pick_key<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &Paint<'_>,
    key: KeyEvent,
) -> io::Result<Option<InlineOutcome>> {
    if is_cancel(key) {
        return settle(live, paint.canvas, InlineOutcome::Cancelled).map(Some);
    }

    if let Some(outcome) = pick_enter(live, store, frame, paint.canvas, key)? {
        return Ok(Some(outcome));
    }

    // A key the frame has no use for leaves the glass alone: no rebuild, no
    // redraw, exactly as it always did.
    if !pick_motion(state, key) {
        return Ok(None);
    }

    repaint(live, store, state, frame, paint)
}

/// The two keystrokes that mean "not today": `Esc`, and the `Ctrl+C` every
/// terminal user reaches for when `Esc` is not within reach.
fn is_cancel(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

/// Enter in the pick frame takes the row under the cursor and leaves with it.
/// Over nothing there is nothing to take: the frame stays open and repaints,
/// which is what it did before.
fn pick_enter<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &dyn Store,
    frame: &Frame,
    canvas: Canvas,
    key: KeyEvent,
) -> io::Result<Option<InlineOutcome>> {
    if key.code != KeyCode::Enter {
        return Ok(None);
    }

    let Some(index) = frame.selected_connection_index() else {
        return Ok(None);
    };

    settle(
        live,
        canvas,
        InlineOutcome::Picked(store.all()[index].clone()),
    )
    .map(Some)
}

/// The pick frame's own keystrokes: the arrows and `j`/`k` move the cursor,
/// typing filters the list, Backspace shortens the filter. `false` means the
/// key was not the frame's, and nothing repaints.
fn pick_motion(state: &mut ManageState, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => state.selection = state.selection.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => state.selection = state.selection.saturating_add(1),
        // Enter over nothing: the frame stays open and repaints unchanged.
        KeyCode::Enter => {}
        KeyCode::Backspace => {
            state.query.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.query.push(c);
        }
        _ => return false,
    }

    true
}

/// The terminal changed size: recount what fits, then either rebuild the frame
/// at the new size or collapse it the way a cancel does.
fn resize<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &mut Paint<'_>,
    width: usize,
    height: usize,
) -> io::Result<Option<InlineOutcome>> {
    // The terminal re-wrapped the rows on the glass before it told us.
    // Recount before anything acts on the plan, so a collapse that follows
    // erases the rows that are really there rather than the ones we remember
    // drawing.
    live.note_width(width);

    match plan_resize(live.geometry(), width, height) {
        ResizePlan::Reopen { .. } => {
            paint.canvas = Canvas::detect(width, height);
            *frame = build(store, state, paint);
            live.reopen(frame)?;
            Ok(None)
        }
        ResizePlan::CancelAndRestore => {
            // A cancel is a cancel: it leaves the same trace the `Esc` path
            // leaves, so the user is never left wondering why the frame
            // disappeared.
            settle(live, paint.canvas, InlineOutcome::Cancelled).map(Some)
        }
    }
}

/// One place assembles the inputs a manage frame is built from, and one place
/// re-syncs the selection to whatever the frame clamped it to. Without it
/// every call site re-typed the quintuple and had to remember that the
/// selection it passed in is not necessarily the one it gets. The update note
/// rides along unchanged on every build — it is read once at the edge and
/// never re-read here, so the paint path touches no cache file and no network
/// (#39).
fn build(store: &dyn Store, state: &mut ManageState, paint: &Paint<'_>) -> Frame {
    let frame = build_frame_with_note(
        store.all(),
        &state.query,
        state.selection,
        paint.mode,
        paint.canvas,
        &FrameFlow::from(&*state),
        paint.note,
    );
    state.selection = frame.selection();
    frame
}

/// Rebuild and repaint: what the user sees is always the frame [`build`] just
/// made, never a remembered one.
fn repaint<W: Write>(
    live: &mut LiveFrame<'_, W>,
    store: &dyn Store,
    state: &mut ManageState,
    frame: &mut Frame,
    paint: &Paint<'_>,
) -> io::Result<Option<InlineOutcome>> {
    *frame = build(store, state, paint);
    live.redraw(frame)?;
    Ok(None)
}

/// The answer a terminal too short for a frame gets: the cancel trace, drawn
/// once below the prompt, with no live frame ever painted.
fn settle_before_open<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
) -> io::Result<InlineOutcome> {
    live.open_settled(&settle_trace(&Settle::Cancelled, canvas))?;
    Ok(InlineOutcome::Cancelled)
}
/// Collapse the frame to the trace this outcome leaves and hand the shell back.
fn settle<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
    outcome: InlineOutcome,
) -> io::Result<InlineOutcome> {
    let trace = match &outcome {
        InlineOutcome::Picked(conn) => Settle::Picked(conn.clone()),
        InlineOutcome::Cancelled => Settle::Cancelled,
    };
    live.collapse(&settle_trace(&trace, canvas))?;
    Ok(outcome)
}

/// Collapse the frame on a failed action and hand the shell back **with an
/// error**, so the failure is readable on the glass and loud in the exit
/// code.
///
/// The collapse happens before the `Err` leaves, which is the whole point:
/// the frame must never be abandoned mid-paint. The trace is written
/// through the same settle path every other exit uses, so a failed delete
/// leaves the shell exactly as clean as a cancel does — one line, no rail,
/// no corner — and the line says what broke and to which Connection.
fn settle_error<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
    connection: &Connection,
    message: String,
) -> io::Result<InlineOutcome> {
    let trace = Settle::Error {
        connection: connection.clone(),
        message: message.clone(),
    };
    live.collapse(&settle_trace(&trace, canvas))?;
    Err(io::Error::other(message))
}

/// Collapse the frame on a refused **add** and hand the shell back with an
/// error, mirroring [`settle_error`].
///
/// The trace names the draft, not a Connection: the add failed, so there
/// is no Connection to point at, and a line that invented one would be
/// claiming an object the file never received.
fn settle_add_error<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
    draft: &ConnectionDraft,
    message: String,
) -> io::Result<InlineOutcome> {
    let trace = Settle::AddFailed {
        draft: draft.clone(),
        message: message.clone(),
    };
    live.collapse(&settle_trace(&trace, canvas))?;
    Err(io::Error::other(message))
}

/// Collapse the frame on a refused **import** and hand the shell back with
/// an error, mirroring [`settle_error`] and [`settle_add_error`].
///
/// The trace names no Connection: the offer was about a file, and the
/// store refused the write, so there is nothing in the set to point at
/// and nothing on disk changed. The frame collapses rather than showing a
/// list it cannot vouch for — the same reason a refused delete collapses,
/// with more force here, because the whole list the user is looking at
/// would have been the import's supposed result.
fn settle_import_error<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
    message: String,
) -> io::Result<InlineOutcome> {
    let trace = Settle::ImportFailed {
        message: message.clone(),
    };
    live.collapse(&settle_trace(&trace, canvas))?;
    Err(io::Error::other(message))
}

/// Hide the cursor while the frame is live.
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";

/// Show the cursor again.
const SHOW_CURSOR: &[u8] = b"\x1b[?25h";

/// Hides the cursor on `out` for its life and puts it back on the *same*
/// writer.
///
/// The two halves have to agree on the stream. The hide goes to the caller's
/// writer because that is the stream the frame is drawn on; restoring on a
/// hardcoded `stdout` would, under `result=$(sshm pick …)`, land the
/// show-cursor SGR in the *pipe* — corrupting the emitted selection — while
/// the real terminal's cursor stayed hidden. Owning the writer for the whole
/// life of the guard is what keeps that split honest, which is the same split
/// #35's captured-stdout contract rests on.
pub struct CursorGuard<'w, W: Write> {
    out: &'w mut W,
}

impl<'w, W: Write> CursorGuard<'w, W> {
    /// Hide the cursor on `out`.
    pub fn new(out: &'w mut W) -> Self {
        let _ = out.write_all(HIDE_CURSOR);
        Self { out }
    }

    /// The writer, borrowed for as long as the guard holds it.
    fn writer(&mut self) -> &mut W {
        self.out
    }
}

impl<W: Write> Drop for CursorGuard<'_, W> {
    fn drop(&mut self) {
        let _ = self.out.write_all(SHOW_CURSOR);
        let _ = self.out.flush();
    }
}

/// Leaves raw mode whatever the exit path.
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ImportReport;
    use crate::theme::ColorSupport;
    use std::collections::VecDeque;

    // The driver reads its keystrokes and switches the terminal's input mode
    // through two seams, so a test can hand it a script and a lie instead of a
    // pty. Everything asserted below is what the frame *did*: what it wrote,
    // what it asked the store to do, and what it answered with.

    // ── The fakes the two seams accept ─────────────────────────────────────

    fn canvas() -> Canvas {
        Canvas::new(80, 24, ColorSupport::Truecolor)
    }

    fn web01() -> Connection {
        Connection {
            id: "1".into(),
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
            alias: "web-02".into(),
            ..web01()
        }
    }

    fn draft() -> ConnectionDraft {
        ConnectionDraft {
            alias: "db-01".into(),
            host: "10.0.0.9".into(),
            user: "postgres".into(),
            port: "5432".into(),
            key_path: String::new(),
            folder: "prod".into(),
        }
    }

    /// A store that answers from a Vec and remembers what it was asked to do.
    /// `refuse` turns every write into a failure, which is how the collapse
    /// paths get tested without a disk that misbehaves.
    struct Ledger {
        connections: Vec<Connection>,
        asked: Vec<String>,
        refuse: Option<String>,
        imported: usize,
    }

    fn ledger(connections: Vec<Connection>) -> Ledger {
        Ledger {
            connections,
            asked: Vec::new(),
            refuse: None,
            imported: 0,
        }
    }

    impl Store for Ledger {
        fn all(&self) -> &[Connection] {
            &self.connections
        }

        fn remove(&mut self, id: &str) -> Result<Option<Connection>, String> {
            self.asked.push(format!("remove {id}"));
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            let at = self.connections.iter().position(|c| c.id == id);
            Ok(at.map(|i| self.connections.remove(i)))
        }

        fn add(&mut self, draft: &ConnectionDraft) -> Result<Connection, String> {
            self.asked.push(format!("add {}", draft.alias));
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            let conn = Connection {
                id: format!("minted-{}", self.connections.len()),
                alias: draft.alias.clone(),
                host: draft.host.clone(),
                user: draft.user.clone(),
                port: draft.port.parse().unwrap_or(22),
                key_path: none_if_blank(&draft.key_path).map(str::to_string),
                folder: none_if_blank(&draft.folder).map(str::to_string),
            };
            self.connections.push(conn.clone());
            Ok(conn)
        }

        fn update(
            &mut self,
            existing_id: &str,
            draft: &ConnectionDraft,
        ) -> Result<Option<Connection>, String> {
            self.asked.push(format!("update {existing_id}"));
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            let Some(at) = self.connections.iter().position(|c| c.id == existing_id) else {
                return Ok(None);
            };
            self.connections[at].alias = draft.alias.clone();
            self.connections[at].host = draft.host.clone();
            Ok(Some(self.connections[at].clone()))
        }

        fn import_ssh_config(&mut self, path: &str) -> Result<ImportReport, String> {
            self.asked.push(format!("import {path}"));
            if let Some(reason) = &self.refuse {
                return Err(reason.clone());
            }
            Ok(ImportReport {
                imported: self.imported,
                failures: Vec::new(),
            })
        }
    }

    /// The events a test drives the frame with, in order. Running out is a
    /// failure rather than a hang: every script ends with a key that closes
    /// the frame.
    struct Scripted {
        events: VecDeque<Event>,
    }

    impl Events for Scripted {
        fn event(&mut self) -> io::Result<Event> {
            self.events
                .pop_front()
                .ok_or_else(|| io::Error::other("the script ran out"))
        }
    }

    /// Records that the frame asked for raw mode, without putting the test
    /// process's own terminal into it.
    #[derive(Default)]
    struct RawSwitch {
        entered: bool,
    }

    impl RawMode for RawSwitch {
        fn enter(&mut self) -> io::Result<()> {
            self.entered = true;
            Ok(())
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ctrl(ch: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL))
    }

    /// What a driven frame left behind: its answer, everything it wrote, and
    /// whether it asked for raw mode.
    struct Driven {
        outcome: io::Result<InlineOutcome>,
        glass: String,
        raw: bool,
    }

    impl Driven {
        /// The answer, expecting the frame to have closed cleanly.
        fn settled(&self) -> &InlineOutcome {
            self.outcome
                .as_ref()
                .expect("the frame closed the way a frame closes")
        }
    }

    fn drive_script(
        events: Vec<Event>,
        store: &mut Ledger,
        canvas: Canvas,
        initial: ManageState,
        mode: FrameMode,
    ) -> Driven {
        let mut out: Vec<u8> = Vec::new();
        let mut scripted = Scripted {
            events: events.into(),
        };
        let mut raw = RawSwitch::default();
        let paint = Paint {
            mode,
            note: None,
            canvas,
        };
        let outcome = open_and_drive(&mut scripted, &mut raw, &mut out, store, paint, initial);
        Driven {
            outcome,
            glass: String::from_utf8(out).expect("the frame writes text"),
            raw: raw.entered,
        }
    }

    /// The glass with the control sequences taken out: what the user reads.
    fn plain(glass: &str) -> String {
        let mut out = String::new();
        let mut chars = glass.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            // A CSI sequence is its introducer, its parameter bytes, and a
            // final byte in `@`..`~`. Skipping to the final byte is what
            // leaves the text the user reads.
            if chars.peek() == Some(&'[') {
                chars.next();
            }
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        }
        out
    }

    // ── Opening ────────────────────────────────────────────────────────────

    /// A terminal too short to host a frame never gets one, and never gets raw
    /// mode or a hidden cursor either: it gets the cancel trace and its prompt
    /// back, which is the whole difference between a refusal and a degenerate
    /// frame.
    #[test]
    fn a_terminal_too_short_for_a_frame_never_gets_one() {
        let mut store = ledger(vec![web01()]);
        let short = Canvas::new(80, 4, ColorSupport::Monochrome);
        let driven = drive_script(
            vec![],
            &mut store,
            short,
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(
            !driven.raw,
            "a terminal that never got a frame never got raw mode either"
        );
        let glass = plain(&driven.glass);
        assert!(
            glass.contains("cancelled"),
            "the settle line is still the whole answer: {glass}"
        );
        assert!(
            !glass.contains('│'),
            "no frame chrome was ever drawn: {glass}"
        );
    }

    // ── The pick frame ─────────────────────────────────────────────────────

    #[test]
    fn escape_in_the_pick_frame_collapses_it_and_leaves_cancelled() {
        let mut store = ledger(vec![web01()]);
        let driven = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(driven.raw, "a live frame does ask for raw mode");
        let glass = plain(&driven.glass);
        assert!(
            glass.trim_end().ends_with("◆ cancelled"),
            "the settle line is the last thing the frame wrote: {glass}"
        );
    }

    #[test]
    fn ctrl_c_in_the_pick_frame_leaves_the_way_escape_does() {
        let mut store = ledger(vec![web01()]);
        let escape = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );
        let mut store = ledger(vec![web01()]);
        let interrupted = drive_script(
            vec![ctrl('c')],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(interrupted.settled(), &InlineOutcome::Cancelled);
        assert_eq!(
            plain(&escape.glass),
            plain(&interrupted.glass),
            "one cancel, however it was typed, leaves one trace"
        );
    }

    #[test]
    fn typing_filters_and_enter_takes_the_row_under_the_cursor() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![key(KeyCode::Char('w')), key(KeyCode::Enter)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Picked(web01()),
            "Enter takes the row under the cursor"
        );
        let glass = plain(&driven.glass);
        assert!(glass.contains("picked"), "got: {glass}");
        assert!(store.asked.is_empty(), "picking writes nothing");
    }

    #[test]
    fn enter_over_nothing_keeps_the_frame_open() {
        let mut store = ledger(Vec::new());
        let driven = drive_script(
            vec![key(KeyCode::Enter), key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Cancelled,
            "Enter over an empty list is not a pick"
        );
    }

    #[test]
    fn an_unbound_key_in_the_pick_frame_writes_nothing() {
        let mut store = ledger(vec![web01()]);
        let alone = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );
        let mut store = ledger(vec![web01()]);
        let paged = drive_script(
            vec![key(KeyCode::PageDown), key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            plain(&alone.glass),
            plain(&paged.glass),
            "a key the frame has no use for leaves the glass alone"
        );
    }

    #[test]
    fn a_key_that_is_not_a_press_is_not_a_keystroke() {
        let mut store = ledger(vec![web01()]);
        let release = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('w'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        let driven = drive_script(
            vec![release, key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
    }

    // ── The manage frame ───────────────────────────────────────────────────

    #[test]
    fn escape_in_the_manage_frame_leaves_the_frame_cancelled() {
        let mut store = ledger(vec![web01()]);
        let driven = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Manage,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(plain(&driven.glass).contains("cancelled"));
    }

    /// The delete travels as an effect, is performed through the store, and
    /// the note the frame paints afterwards reports what the store did.
    #[test]
    fn a_confirmed_delete_is_performed_and_the_frame_says_so() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![ctrl('x'), key(KeyCode::Char('y')), key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Manage,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert_eq!(store.asked, vec!["remove 1".to_string()]);
        assert_eq!(store.connections.len(), 1, "the store really lost the row");
        let glass = plain(&driven.glass);
        assert!(glass.contains("deleted"), "the note names the act: {glass}");
    }

    /// A store that refuses the delete does not keep the frame painting a
    /// list it cannot vouch for: the frame collapses and the command leaves
    /// non-zero.
    #[test]
    fn a_refused_delete_collapses_the_frame_and_leaves_an_error() {
        let mut store = ledger(vec![web01()]);
        store.refuse = Some("disk on fire".into());
        let driven = drive_script(
            vec![ctrl('x'), key(KeyCode::Char('y'))],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Manage,
        );

        let err = driven
            .outcome
            .expect_err("a refused write is a failed command");
        assert!(format!("{err}").contains("disk on fire"), "got: {err}");
        assert!(
            plain(&driven.glass).contains("error"),
            "the glass says so too"
        );
    }

    // ── The effects, one at a time ──────────────────────────────────────────

    /// Perform one effect against a store and hand back what the frame
    /// answered, the state it settled into, and what it wrote.
    fn perform_effect(
        effect: Effect,
        store: &mut Ledger,
    ) -> (io::Result<Option<InlineOutcome>>, ManageState, String) {
        let mut out: Vec<u8> = Vec::new();
        let mut live = LiveFrame::new(&mut out);
        let mut state = ManageState::new();
        let paint = Paint {
            mode: FrameMode::Manage,
            note: None,
            canvas: canvas(),
        };
        let answer = perform(&mut live, store, &mut state, effect, &paint);
        (
            answer,
            state,
            String::from_utf8(out).expect("the frame writes text"),
        )
    }

    #[test]
    fn an_add_effect_writes_through_the_store_and_names_the_new_row() {
        let mut store = ledger(vec![web01()]);
        let (answer, state, glass) = perform_effect(Effect::Add { draft: draft() }, &mut store);

        assert!(answer.expect("the store took the add").is_none());
        assert_eq!(store.asked, vec!["add db-01".to_string()]);
        assert!(plain(&glass).is_empty(), "an add repaints nothing here");
        let _ = state;
    }

    #[test]
    fn a_refused_add_collapses_the_frame_and_leaves_an_error() {
        let mut store = ledger(vec![web01()]);
        store.refuse = Some("read-only file system".into());
        let (answer, _, glass) = perform_effect(Effect::Add { draft: draft() }, &mut store);

        let err = answer.expect_err("a refused add is a failed command");
        assert!(format!("{err}").contains("read-only file system"));
        assert!(plain(&glass).contains("error"), "the glass says so too");
    }

    #[test]
    fn an_update_effect_keeps_the_id_and_reports_the_change() {
        let mut store = ledger(vec![web01()]);
        let (answer, _, glass) = perform_effect(
            Effect::Update {
                target: web01(),
                draft: ConnectionDraft {
                    alias: "web-99".into(),
                    ..ConnectionDraft::from_connection(&web01())
                },
            },
            &mut store,
        );

        assert!(answer.expect("the store took the edit").is_none());
        assert_eq!(store.asked, vec!["update 1".to_string()]);
        assert_eq!(store.connections[0].alias, "web-99");
        assert!(plain(&glass).is_empty());
    }

    #[test]
    fn a_refused_update_collapses_the_frame_and_leaves_an_error() {
        let mut store = ledger(vec![web01()]);
        store.refuse = Some("no space left".into());
        let (answer, _, glass) = perform_effect(
            Effect::Update {
                target: web01(),
                draft: ConnectionDraft::from_connection(&web01()),
            },
            &mut store,
        );

        let err = answer.expect_err("a refused edit is a failed command");
        assert!(format!("{err}").contains("no space left"));
        assert!(plain(&glass).contains("error"));
    }

    #[test]
    fn an_import_effect_writes_the_stanzas_and_reports_the_count() {
        let mut store = ledger(Vec::new());
        store.imported = 3;
        let (answer, state, _) = perform_effect(
            Effect::Import {
                path: "/home/dev/.ssh/config".into(),
            },
            &mut store,
        );

        assert!(answer.expect("the store took the import").is_none());
        assert_eq!(
            store.asked,
            vec!["import /home/dev/.ssh/config".to_string()]
        );
        assert!(
            matches!(
                state.trace,
                Some(manage::Trace::Imported { imported: 3, .. })
            ),
            "the note reports what the store wrote"
        );
    }

    #[test]
    fn a_refused_import_collapses_the_frame_and_leaves_an_error() {
        let mut store = ledger(Vec::new());
        store.refuse = Some("permission denied".into());
        let (answer, _, glass) = perform_effect(
            Effect::Import {
                path: "/home/dev/.ssh/config".into(),
            },
            &mut store,
        );

        let err = answer.expect_err("a refused import is a failed command");
        assert!(format!("{err}").contains("permission denied"));
        assert!(
            plain(&glass).contains("import failed"),
            "the glass says what failed: {glass}"
        );
    }

    #[test]
    fn an_exit_effect_settles_the_frame_with_its_outcome() {
        let mut store = ledger(vec![web01()]);
        let (answer, _, glass) =
            perform_effect(Effect::Exit(InlineOutcome::Picked(web01())), &mut store);

        assert_eq!(
            answer.expect("an exit is not a failure"),
            Some(InlineOutcome::Picked(web01()))
        );
        assert!(plain(&glass).contains("picked"));
    }

    // ── Resize ─────────────────────────────────────────────────────────────

    #[test]
    fn a_narrower_terminal_rebuilds_the_frame_where_it_stood() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![
                Event::Resize(60, 24),
                key(KeyCode::Char('w')),
                key(KeyCode::Esc),
            ],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(plain(&driven.glass).contains("cancelled"));
    }

    /// A terminal that shrinks past what a frame needs is a cancel, and it
    /// leaves the same trace `Esc` leaves: the user is never left wondering
    /// why the frame disappeared.
    #[test]
    fn a_terminal_too_short_after_a_resize_cancels_the_frame() {
        let mut store = ledger(vec![web01()]);
        let driven = drive_script(
            vec![Event::Resize(80, 2)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(plain(&driven.glass).contains("cancelled"));
    }

    // ── The pick frame's own keystrokes ────────────────────────────────────
    //
    // The pick frame decides its keys for itself rather than asking
    // `manage::step`, so nothing above this line proves they work. Each one is
    // asserted by what the frame *does* with the key: which row the cursor
    // ends on, what the filter holds, and whether the frame is still open.

    /// Typing narrows the list, which is the whole reason the frame is open.
    /// Asserted by what `Enter` takes: if the keystroke never reached the
    /// filter, `Enter` picks the first row of the *unfiltered* list and looks
    /// exactly like success.
    #[test]
    fn typing_narrows_the_list_to_what_matches() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![key(KeyCode::Char('2')), key(KeyCode::Enter)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Picked(web02()),
            "`2` matches only web-02, so that is the row Enter takes"
        );
    }

    /// A chord is not a letter. `Ctrl+A` means *add* in the manage frame and
    /// nothing in the pick frame, but it must never mean *type an `a`*: a chord
    /// swallowed as text filters the list to something the user never typed,
    /// and the frame is now lying about the query.
    #[test]
    fn a_chord_is_not_a_letter() {
        let mut store = ledger(vec![web01()]);
        let alone = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );
        let mut store = ledger(vec![web01()]);
        let chorded = drive_script(
            vec![ctrl('a'), key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            plain(&chorded.glass),
            plain(&alone.glass),
            "Ctrl+A is not the letter a: the frame must not repaint as if it were"
        );
    }

    /// The cursor moves with the arrows and with `j`/`k`, and both ways: a
    /// frame that could only go down would strand the first row, and one that
    /// could only go up would strand the last.
    #[test]
    fn the_cursor_walks_both_ways_with_arrows_and_with_j_k() {
        let picked = |keys: Vec<Event>| {
            let mut store = ledger(vec![web01(), web02()]);
            let mut script = keys;
            script.push(key(KeyCode::Enter));
            let driven = drive_script(
                script,
                &mut store,
                canvas(),
                ManageState::new(),
                FrameMode::Pick,
            );
            driven.settled().clone()
        };

        assert_eq!(
            picked(vec![key(KeyCode::Down)]),
            InlineOutcome::Picked(web02()),
            "Down moves the cursor to the next row"
        );
        assert_eq!(
            picked(vec![key(KeyCode::Char('j'))]),
            InlineOutcome::Picked(web02()),
            "`j` is Down"
        );
        assert_eq!(
            picked(vec![key(KeyCode::Down), key(KeyCode::Up)]),
            InlineOutcome::Picked(web01()),
            "Up moves the cursor back"
        );
        assert_eq!(
            picked(vec![key(KeyCode::Char('j')), key(KeyCode::Char('k'))]),
            InlineOutcome::Picked(web01()),
            "`k` is Up"
        );
    }

    /// Backspace shortens the filter, so the rows that were filtered away come
    /// back. Asserted by what is under the cursor afterwards, because a
    /// Backspace that did nothing would still leave a frame that looks fine.
    #[test]
    fn backspace_shortens_the_filter_and_brings_the_rows_back() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![
                key(KeyCode::Char('2')),
                key(KeyCode::Backspace),
                key(KeyCode::Enter),
            ],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Picked(web01()),
            "the filter is empty again, so the first row of the whole list is under the cursor"
        );
    }

    /// `Ctrl+C` is the cancel chord; not every chord is. `Ctrl+Backspace` is
    /// still a Backspace, and a frame that read every chord as *give up* would
    /// throw away the work of a filter every time a user reaches for the
    /// word-delete they use everywhere else.
    #[test]
    fn a_chord_over_the_filter_is_not_a_cancel() {
        let mut store = ledger(vec![web01(), web02()]);
        let driven = drive_script(
            vec![
                key(KeyCode::Char('2')),
                Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
                key(KeyCode::Enter),
            ],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Picked(web01()),
            "Ctrl+Backspace shortens the filter; only Ctrl+C leaves the frame"
        );
    }

    /// Enter with nothing to take leaves the frame open and repainted: the key
    /// was the frame's, it just had nothing to act on. A frame that swallowed
    /// it silently would look frozen, which is how a user decides the program
    /// has hung.
    #[test]
    fn enter_over_nothing_answers_with_a_repaint() {
        let mut store = ledger(Vec::new());
        let alone = drive_script(
            vec![key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );
        let mut store = ledger(Vec::new());
        let pressed = drive_script(
            vec![key(KeyCode::Enter), key(KeyCode::Esc)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(pressed.settled(), &InlineOutcome::Cancelled);
        assert!(
            plain(&pressed.glass).len() > plain(&alone.glass).len(),
            "Enter over nothing still answers: the frame repaints rather than ignoring the key"
        );
    }

    /// A key *released* is not a key pressed. Terminals in kitty-keyboard mode
    /// report releases, and a release read as a press would move the cursor
    /// twice for one keystroke.
    #[test]
    fn a_released_key_is_not_a_keystroke() {
        let mut store = ledger(vec![web01(), web02()]);
        let release = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        let driven = drive_script(
            vec![release, key(KeyCode::Enter)],
            &mut store,
            canvas(),
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(
            driven.settled(),
            &InlineOutcome::Picked(web01()),
            "the cursor never moved, so Enter still takes the first row"
        );
    }

    /// A refusal writes a refusal and nothing else. There are no live rows to
    /// delete on this path, and asking for zero of them is not harmless:
    /// `ESC [ 0 M` means *one line* to a terminal that reads a zero parameter
    /// as the ECMA-48 default, so the frame could eat the user's prompt on its
    /// way out of a terminal it never drew in.
    #[test]
    fn a_refusal_deletes_no_rows_it_never_drew() {
        let mut store = ledger(vec![web01()]);
        let short = Canvas::new(80, 4, ColorSupport::Monochrome);
        let driven = drive_script(
            vec![],
            &mut store,
            short,
            ManageState::new(),
            FrameMode::Pick,
        );

        assert_eq!(driven.settled(), &InlineOutcome::Cancelled);
        assert!(
            !driven.glass.contains("\u{1b}[0M"),
            "the refusal must not ask to delete rows it never drew: {:?}",
            driven.glass
        );
    }

    // ── The two small helpers the settle lines are built from ──────────────

    /// A blank is not a value: `key_path: ""` must not be written to the file
    /// as an empty path, and a path of any length must survive untouched.
    #[test]
    fn a_blank_field_holds_no_value_at_all() {
        assert_eq!(none_if_blank(""), None, "empty is not a value");
        assert_eq!(
            none_if_blank("web-01"),
            Some("web-01"),
            "a value comes back as it was given"
        );
    }

    /// A reason that fits is the reason; one that does not is cut with the
    /// ellipsis that tells the reader there was more; one with no room at all
    /// is dropped, because `◆ error  web-01` already says what happened and
    /// the full text is on stderr.
    #[test]
    fn a_reason_is_kept_whole_cut_with_an_ellipsis_or_dropped() {
        assert_eq!(fit_reason("disk on fire", 10, 40), "disk on fire");
        assert_eq!(
            fit_reason("disk on fire", 25, 40),
            "disk on fi…",
            "the cut is marked, so a truncated reason is not mistaken for the whole one"
        );
        assert_eq!(
            fit_reason("disk on fire", 36, 40),
            "",
            "with no room for even one character the reason is dropped, not smeared"
        );
    }
}
