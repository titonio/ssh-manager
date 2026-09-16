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

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Connection;
use crate::connections::{ConnectionDraft, Store};
use crate::frame::{
    build_frame_with_flow, fit_line, fit_visible_rows, Canvas, Frame, FrameFlow, FrameMode,
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
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

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
        let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
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
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);
    let bold = Style::default().add_modifier(Modifier::BOLD);

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
) -> io::Result<InlineOutcome> {
    run_inline_with_state(out, store, ManageState::with_query(initial_query), mode)
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
pub fn run_inline_with_state<W: Write>(
    out: &mut W,
    store: &mut dyn Store,
    initial: ManageState,
    mode: FrameMode,
) -> io::Result<InlineOutcome> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    let (width, height) = crossterm::terminal::size()?;
    let mut canvas = Canvas::detect(width as usize, height as usize);

    // A terminal too short to host the frame never gets one. This is the
    // same cancel-and-restore the resize path falls back to, applied at open:
    // without it `build_frame` emits three lines of chrome wrapped around a
    // list area that fitted zero rows, which is a degenerate frame rather
    // than a clean refusal.
    if canvas.visible_rows().is_none() {
        let mut live = LiveFrame::new(out);
        live.open_settled(&settle_trace(&Settle::Cancelled, canvas))?;
        return Ok(InlineOutcome::Cancelled);
    }

    let mut state = initial;

    // One place assembles the inputs a manage frame is built from, and one
    // place re-syncs the selection to whatever the frame clamped it to.
    // Without it every call site re-typed the quintuple and had to remember
    // that the selection it passed in is not necessarily the one it gets.
    let build = |store: &dyn Store, state: &ManageState, canvas: Canvas| {
        let frame = build_frame_with_flow(
            store.all(),
            &state.query,
            state.selection,
            mode,
            canvas,
            &FrameFlow::from(state),
        );
        let selection = frame.selection();
        (frame, selection)
    };

    let (mut frame, synced) = build(store, &state, canvas);
    state.selection = synced;

    crossterm::terminal::enable_raw_mode()?;
    let _raw = RawModeGuard;
    let mut cursor = CursorGuard::new(out);

    let mut live = LiveFrame::new(cursor.writer());
    live.open(&frame)?;

    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if mode == FrameMode::Manage {
                    let selected = frame
                        .selected_connection_index()
                        .map(|i| store.all()[i].clone());
                    let step = manage::step(&state, key, selected.as_ref());
                    state = step.state;

                    for effect in step.effects {
                        match effect {
                            Effect::Delete { target } => {
                                // The delete is performed here and its
                                // result folded straight back through
                                // [`manage::settle_delete`], so the note
                                // the frame paints reports what the store
                                // did rather than what was asked for.
                                // `Ok(None)` in particular means nothing
                                // was removed, and the frame says so.
                                let outcome =
                                    manage::DeleteOutcome::from_remove(store.remove(&target.id));
                                match manage::settle_delete(&state, &target, outcome) {
                                    Ok(next) => state = next,
                                    // The store refused the delete. The
                                    // frame collapses to `◆ error …` and
                                    // the process exits non-zero: a
                                    // painted frame the user cannot read
                                    // the failure in, over a list the
                                    // store just proved unreliable, is
                                    // the state this replaces.
                                    Err(message) => {
                                        return settle_error(&mut live, canvas, &target, message)
                                    }
                                }
                            }
                            // The write happens here and its result folds
                            // straight back through `settle_add`, so the
                            // `◇ added` note reports what the store did
                            // rather than what the sequence finished
                            // doing. `connections::add` is the only
                            // implementation of "add a Connection"; the
                            // frame does not get its own.
                            Effect::Add { draft } => {
                                let added = store.add(&draft);
                                match manage::settle_add(&state, store.all(), added) {
                                    Ok(next) => state = next,
                                    // The store refused the add. Same
                                    // contract as a refused delete:
                                    // collapse, report, exit non-zero.
                                    Err(message) => {
                                        return settle_add_error(&mut live, canvas, &draft, message)
                                    }
                                }
                            }
                            Effect::Exit(outcome) => {
                                return settle(&mut live, canvas, outcome);
                            }
                            // The edit is performed here and its result
                            // folded straight back through `settle_edit`,
                            // so the `◇ edited` note reports what the
                            // store did rather than what the editor
                            // finished typing. `Ok(None)` in particular
                            // means the Connection was not there to
                            // change, and the frame says so instead of
                            // claiming an edit that never landed.
                            Effect::Update { target, draft } => {
                                let outcome = manage::EditOutcome::from_update(
                                    store.update(&target.id, &draft),
                                );
                                match manage::settle_edit(&state, store.all(), &target, outcome) {
                                    Ok(next) => state = next,
                                    // The store refused the edit. Same
                                    // contract as a refused add and a
                                    // refused delete: collapse, report,
                                    // exit non-zero.
                                    Err(message) => {
                                        return settle_error(&mut live, canvas, &target, message)
                                    }
                                }
                            }
                            // The import is performed here and its
                            // result folded straight back through
                            // `settle_import`, so the `◇ imported` note
                            // reports what the store wrote rather than
                            // what the scan promised — the same
                            // discipline the delete and add arms keep.
                            Effect::Import { path } => {
                                let outcome = manage::ImportOutcome::from_report(
                                    store.import_ssh_config(&path),
                                );
                                match manage::settle_import(&state, outcome) {
                                    Ok(next) => state = next,
                                    // The store refused the write. Same
                                    // contract as a refused add, delete
                                    // or edit: collapse, report, exit
                                    // non-zero — never keep painting a
                                    // list the store cannot vouch for.
                                    Err(message) => {
                                        return settle_import_error(&mut live, canvas, message)
                                    }
                                }
                            }
                        }
                    }

                    let (next, synced) = build(store, &state, canvas);
                    frame = next;
                    state.selection = synced;
                    live.redraw(&frame)?;
                    continue;
                }

                match key.code {
                    KeyCode::Esc => return settle(&mut live, canvas, InlineOutcome::Cancelled),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return settle(&mut live, canvas, InlineOutcome::Cancelled);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        state.selection = state.selection.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        state.selection = state.selection.saturating_add(1);
                    }
                    KeyCode::Enter => {
                        if let Some(index) = frame.selected_connection_index() {
                            let conn = store.all()[index].clone();
                            live.collapse(&settle_trace(&Settle::Picked(conn.clone()), canvas))?;
                            return Ok(InlineOutcome::Picked(conn));
                        }
                    }
                    KeyCode::Backspace => {
                        state.query.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        state.query.push(c);
                    }
                    _ => continue,
                }

                let (next, synced) = build(store, &state, canvas);
                frame = next;
                state.selection = synced;
                live.redraw(&frame)?;
            }
            Event::Resize(width, height) => {
                let (width, height) = (usize::from(width), usize::from(height));

                // The terminal re-wrapped the rows on the glass before it
                // told us. Recount before anything acts on the plan, so a
                // collapse that follows erases the rows that are really
                // there rather than the ones we remember drawing.
                live.note_width(width);

                match plan_resize(live.geometry(), width, height) {
                    ResizePlan::Reopen { .. } => {
                        canvas = Canvas::detect(width, height);
                        let (next, synced) = build(store, &state, canvas);
                        frame = next;
                        state.selection = synced;
                        live.reopen(&frame)?;
                    }
                    ResizePlan::CancelAndRestore => {
                        // A cancel is a cancel: it leaves the same trace the
                        // `Esc` path leaves, so the user is never left
                        // wondering why the frame disappeared.
                        return settle(&mut live, canvas, InlineOutcome::Cancelled);
                    }
                }
            }
            _ => {}
        }
    }
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
