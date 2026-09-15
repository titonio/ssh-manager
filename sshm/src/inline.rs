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
//! Three rules bind the whole module:
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
//!    no orphan row or rail fragment survives the frame.

use std::io::{self, Write};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::Connection;
use crate::frame::{build_frame, fit_visible_rows, Canvas, Frame, FrameMode};
use crate::theme::{self, Theme};

// ─────────────────────────────────────────────────────────────────────────────
// The model
// ─────────────────────────────────────────────────────────────────────────────

/// What the live frame collapses to when it closes.
///
/// Both arms leave a trace, because both are the user's last sight of the
/// frame: a pick says what was chosen, a cancel says the frame was left on
/// purpose rather than having vanished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settle {
    /// A Connection was chosen.
    Picked(Connection),
    /// The user left with `Esc`/`Ctrl+C`, or the frame could not be preserved.
    Cancelled,
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
/// (user story 13) and is emitted verbatim. The folder prefix is deliberately
/// absent: the trace names the Connection the user is acting on, and the spec
/// writes that line without it. A cancel leaves `◆ cancelled`.
///
/// The emphasis is the frame's own grammar — accent step icon, bold alias, dim
/// meta — so the trace reads as the same design system as the list it
/// replaced, and carries its meaning with the colour turned off.
pub fn settle_trace(settle: &Settle, canvas: Canvas) -> Vec<Line<'static>> {
    let t = Theme::clack().resolve(canvas.support);
    let icon = |role: ratatui::style::Color| Span::styled("◆", Style::default().fg(role));
    let dim = Style::default().fg(t.fg_muted).add_modifier(Modifier::DIM);

    match settle {
        Settle::Picked(conn) => vec![Line::from(vec![
            icon(t.accent),
            Span::raw(" picked  "),
            Span::styled(
                conn.alias.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(format!("({}@{}:{})", conn.user, conn.host, conn.port), dim),
        ])],
        Settle::Cancelled => vec![Line::from(vec![
            icon(t.border),
            Span::styled(" cancelled", dim),
        ])],
    }
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
/// The driver tracks how many rows it drew and how wide the widest one was,
/// which is everything the next redraw and the next resize need. It never
/// reads the terminal and never owns a viewport: it writes, and remembers what
/// it wrote.
#[derive(Debug)]
pub struct LiveFrame<'w, W: Write> {
    out: &'w mut W,
    drawn: Vec<Line<'static>>,
    widest: usize,
}

impl<'w, W: Write> LiveFrame<'w, W> {
    /// A driver writing to `out`, with nothing drawn yet.
    pub fn new(out: &'w mut W) -> Self {
        Self {
            out,
            drawn: Vec::new(),
            widest: 0,
        }
    }

    /// What is on screen right now, for [`plan_resize`] to decide against.
    pub fn geometry(&self) -> Live {
        Live {
            rows: self.drawn.len(),
            widest: self.widest,
        }
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
    pub fn collapse(&mut self, trace: &[Line<'static>]) -> io::Result<()> {
        self.rewind()?;
        let ops = diff_rows(&self.drawn, trace);
        self.apply(&ops)
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
    fn rewind(&mut self) -> io::Result<()> {
        if self.drawn.is_empty() {
            return Ok(());
        }
        self.out.write_all(up(self.drawn.len()).as_bytes())
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
        let rows = self.drawn.len();
        if rows > 0 {
            self.out.write_all(format!("\x1b[{rows}M").as_bytes())?;
        }
        self.commit(Vec::new())
    }

    /// Record what is now on screen.
    fn commit(&mut self, drawn: Vec<Line<'static>>) -> io::Result<()> {
        self.widest = drawn
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.chars().count()).sum())
            .max()
            .unwrap_or(0);
        self.drawn = drawn;
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
pub fn run_inline<W: Write>(
    out: &mut W,
    connections: &[Connection],
    initial_query: String,
    mode: FrameMode,
) -> io::Result<InlineOutcome> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    let (width, height) = crossterm::terminal::size()?;
    let mut canvas = Canvas::detect(width as usize, height as usize);
    let mut query = initial_query;
    let mut selection = 0usize;
    let mut frame = build_frame(connections, &query, selection, mode, canvas);
    selection = frame.selection();

    crossterm::terminal::enable_raw_mode()?;
    let _raw = RawModeGuard;
    let _ = out.write_all(b"\x1b[?25l");

    let mut live = LiveFrame::new(out);
    live.open(&frame)?;

    loop {
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                match key.code {
                    KeyCode::Esc => return settle_cancel(&mut live, canvas),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return settle_cancel(&mut live, canvas);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selection = selection.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        selection = selection.saturating_add(1);
                    }
                    KeyCode::Enter => {
                        if let Some(index) = frame.selected_connection_index() {
                            let conn = connections[index].clone();
                            live.collapse(&settle_trace(&Settle::Picked(conn.clone()), canvas))?;
                            return Ok(InlineOutcome::Picked(conn));
                        }
                    }
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        query.push(c);
                    }
                    _ => continue,
                }

                frame = build_frame(connections, &query, selection, mode, canvas);
                selection = frame.selection();
                live.redraw(&frame)?;
            }
            Event::Resize(width, height) => {
                match plan_resize(live.geometry(), width as usize, height as usize) {
                    ResizePlan::Reopen { .. } => {
                        canvas = Canvas::detect(width as usize, height as usize);
                        frame = build_frame(connections, &query, selection, mode, canvas);
                        selection = frame.selection();
                        live.reopen(&frame)?;
                    }
                    ResizePlan::CancelAndRestore => {
                        // A cancel is a cancel: it leaves the same trace the
                        // `Esc` path leaves, so the user is never left
                        // wondering why the frame disappeared.
                        return settle_cancel(&mut live, canvas);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Collapse the frame to the cancel trace and hand the shell back.
fn settle_cancel<W: Write>(
    live: &mut LiveFrame<'_, W>,
    canvas: Canvas,
) -> io::Result<InlineOutcome> {
    live.collapse(&settle_trace(&Settle::Cancelled, canvas))?;
    Ok(InlineOutcome::Cancelled)
}

/// Puts the terminal back the way it was found, whatever the exit path.
struct RawModeGuard;

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = std::io::stdout().write_all(b"\x1b[?25h");
    }
}
