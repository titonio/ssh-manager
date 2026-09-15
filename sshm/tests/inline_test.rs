//! The inline render + settle-collapse mechanics (#34).
//!
//! Two seams, tested differently because they are different kinds of thing:
//!
//! * **The pure model** (`diff_rows`, `settle_trace`, `plan_resize`) is a
//!   value-to-value function. It is tested the same way `build_frame` is: no
//!   terminal, no PTY, expectations taken from the spec's worked examples.
//! * **The driver** (`LiveFrame`) is the only thing in the inline path that
//!   moves a cursor, so it is tested against a *writer that models the
//!   screen*. The assertion is about what ends up visible — the prompt still
//!   there, the live rows gone, no orphan rail — not about which escape
//!   sequence produced it.
//!
//! The driver is deliberately thin: every decision it makes was already made
//! by the pure model above. What is left to prove is that the cursor math
//! lands where it claims, which is exactly what the screen model checks.

use std::cell::RefCell;
use std::io::{self, Write};
use std::rc::Rc;

use ratatui::style::Modifier;
use ratatui::text::Line;
use sshm::config::Connection;
use sshm::frame::{build_frame, fit_visible_rows, Canvas, FrameMode, FRAME_LINES, VISIBLE_ROWS};
use sshm::inline::{
    diff_rows, plan_resize, settle_trace, Live, LiveFrame, ResizePlan, RowOp, Settle,
};
use sshm::theme::ColorSupport;

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn canvas() -> Canvas {
    Canvas::new(80, 24, ColorSupport::Truecolor)
}

/// The spec's own worked example: `◆ picked  web-01  (deploy@10.0.0.4:22)`.
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

/// `c-00..c-{n}` so a window slice can be read straight out of the row text.
fn many(n: usize) -> Vec<Connection> {
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

fn lines(texts: &[&str]) -> Vec<Line<'static>> {
    texts.iter().map(|t| Line::from(t.to_string())).collect()
}

fn line(text: &str) -> Line<'static> {
    Line::from(text.to_string())
}

fn line_text(line: &Line<'static>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

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
// Seam: the height budget
// ─────────────────────────────────────────────────────────────────────────────

/// A normal terminal gets the full window; a short one gets a smaller window;
/// a terminal with no room for a row at all gets `None`, which is the answer
/// that makes the resize path cancel rather than draw something broken.
#[test]
fn a_terminal_reports_how_many_list_rows_it_can_hold() {
    assert_eq!(fit_visible_rows(24), Some(VISIBLE_ROWS));
    assert_eq!(fit_visible_rows(12), Some(VISIBLE_ROWS));
    assert_eq!(fit_visible_rows(11), Some(7));
    assert_eq!(fit_visible_rows(5), Some(1));
    assert_eq!(
        fit_visible_rows(4),
        None,
        "no room for a row above the chrome"
    );
    assert_eq!(fit_visible_rows(0), None);
}

// ─────────────────────────────────────────────────────────────────────────────
// Seam: the row diff
// ─────────────────────────────────────────────────────────────────────────────

/// A frame redrawn against itself writes nothing: the row diff is what keeps a
/// live frame from repainting rows that have not changed.
#[test]
fn an_unchanged_frame_diffs_to_nothing_to_write() {
    let frame = lines(&["◆ header", "│ ❯ row", "│   row", "└"]);
    let ops = diff_rows(&frame, &frame);

    assert_eq!(ops.len(), 4);
    assert!(
        ops.iter().all(|op| *op == RowOp::Keep),
        "identical rows must not be rewritten: {ops:?}"
    );
}

/// Only the rows whose content actually moved get rewritten. This is what
/// makes a keystroke cost two row writes instead of eleven.
#[test]
fn only_the_rows_that_changed_are_rewritten() {
    let before = lines(&["a", "b", "c", "d"]);
    let after = lines(&["a", "B", "c", "D"]);

    assert_eq!(
        diff_rows(&before, &after),
        vec![
            RowOp::Keep,
            RowOp::Rewrite(line("B")),
            RowOp::Keep,
            RowOp::Rewrite(line("D")),
        ]
    );
}

/// The settle-collapse: an eleven-line live frame becomes one settle line.
/// The first row is rewritten with the trace and every row below it is
/// deleted, so the live list leaves no orphan rows behind.
#[test]
fn collapsing_an_eleven_line_frame_rewrites_one_row_and_erases_the_rest() {
    let live: Vec<Line<'static>> = (0..FRAME_LINES)
        .map(|i| line(&format!("live {i}")))
        .collect();
    let trace = lines(&["◆ picked  web-01  (deploy@10.0.0.4:22)"]);

    let ops = diff_rows(&live, &trace);

    assert_eq!(ops.len(), FRAME_LINES);
    assert_eq!(
        ops[0],
        RowOp::Rewrite(line("◆ picked  web-01  (deploy@10.0.0.4:22)"))
    );
    assert_eq!(
        ops[1..],
        vec![RowOp::Erase; FRAME_LINES - 1],
        "every row the settle trace does not fill must be erased"
    );
}

/// A frame that grows (a taller terminal after a resize) writes the extra rows
/// below the ones already on screen rather than dropping them.
#[test]
fn rows_beyond_the_live_frame_are_written_not_dropped() {
    let before = lines(&["a"]);
    let after = lines(&["a", "b", "c"]);

    assert_eq!(
        diff_rows(&before, &after),
        vec![
            RowOp::Keep,
            RowOp::Rewrite(line("b")),
            RowOp::Rewrite(line("c"))
        ]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Seam: the settle trace
// ─────────────────────────────────────────────────────────────────────────────

/// The picked trace is the exact line the spec names, and it is one line — the
/// whole live frame collapses to it.
#[test]
fn a_picked_connection_settles_to_the_trace_the_spec_names() {
    let trace = settle_trace(&Settle::Picked(web01()), canvas());

    assert_eq!(trace.len(), 1, "the settle trace is a single line");
    assert_eq!(
        line_text(&trace[0]),
        "◆ picked  web-01  (deploy@10.0.0.4:22)",
        "the trace must be the spec's worked example, verbatim"
    );
}

/// The trace keeps the frame's emphasis grammar: the step icon carries the
/// accent, the alias is bold, the host meta is dim. It reads as the same
/// design system as the frame it replaced.
#[test]
fn the_picked_trace_keeps_the_frames_emphasis_grammar() {
    let trace = settle_trace(&Settle::Picked(web01()), canvas());
    let spans = &trace[0].spans;

    assert_eq!(
        spans[0].content.as_ref(),
        "◆",
        "the trace opens with the Clack step icon"
    );
    assert!(
        span_with(spans, "web-01")
            .style
            .add_modifier
            .contains(Modifier::BOLD),
        "the alias stays bold, as it is in a row"
    );
    let meta = span_with(spans, "(deploy@10.0.0.4:22)");
    assert!(
        meta.style.add_modifier.contains(Modifier::DIM),
        "the host meta stays dim: {meta:?}"
    );
}

/// A cancel leaves a trace too: the user sees that they left, rather than the
/// frame vanishing without a word.
#[test]
fn a_cancel_settles_to_a_cancel_trace() {
    let trace = settle_trace(&Settle::Cancelled, canvas());

    assert_eq!(trace.len(), 1);
    assert_eq!(line_text(&trace[0]), "◆ cancelled");
    assert_eq!(trace[0].spans[0].content.as_ref(), "◆");
}

/// The settle trace is part of the user's scrollback now, so it obeys the
/// frame's transparency rule: no span paints a background.
#[test]
fn the_settle_trace_paints_no_background() {
    for settle in [Settle::Picked(web01()), Settle::Cancelled] {
        for line in settle_trace(&settle, canvas()) {
            for span in &line.spans {
                assert_eq!(
                    span.style.bg, None,
                    "{settle:?}: span {:?} paints a background",
                    span.content
                );
            }
        }
    }
}

/// A monochrome canvas settles in monochrome: the trace carries its meaning in
/// the words and the glyph, not the colour.
#[test]
fn the_settle_trace_degrades_with_the_canvas() {
    let mono = Canvas::new(80, 24, ColorSupport::Monochrome);
    let ansi = settle_trace(&Settle::Picked(web01()), mono)
        .iter()
        .map(|l| sshm::theme::ansi::line_to_ansi(l))
        .collect::<String>();

    assert!(ansi.contains("picked") && ansi.contains("web-01"));
    let coloured: Vec<String> = sgr_params(&ansi)
        .into_iter()
        .filter(|p| !matches!(p.as_str(), "0" | "1" | "2" | "39" | "49"))
        .collect();
    assert!(
        coloured.is_empty(),
        "monochrome settle trace emitted colour: {coloured:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Seam: the resize decision
// ─────────────────────────────────────────────────────────────────────────────

/// A wider or taller terminal re-opens the frame with more room. The query and
/// the selection are the caller's, so preservation is just "re-open, do not
/// cancel" (user story 37).
#[test]
fn growing_the_terminal_reopens_the_frame() {
    let live = Live {
        rows: 11,
        widest: 40,
    };

    assert_eq!(
        plan_resize(live, 120, 40),
        ResizePlan::Reopen {
            visible_rows: VISIBLE_ROWS
        }
    );
}

/// Narrowing is safe only while nothing already drawn has to wrap. A row that
/// no longer fits would have wrapped before we were told, and a wrapped row
/// cannot be counted — erasing eleven logical rows would leave an orphan.
#[test]
fn narrowing_is_safe_only_while_no_live_row_has_to_wrap() {
    let live = Live {
        rows: 11,
        widest: 40,
    };

    assert_eq!(
        plan_resize(live, 80, 24),
        ResizePlan::Reopen {
            visible_rows: VISIBLE_ROWS
        }
    );
    assert_eq!(
        plan_resize(live, 39, 24),
        ResizePlan::CancelAndRestore,
        "a 40-column live row cannot survive a 39-column terminal"
    );
}

/// Shrinking the terminal below what a frame needs cancels rather than drawing
/// a frame with no room for a row.
#[test]
fn a_terminal_too_short_to_hold_the_frame_cancels() {
    let live = Live {
        rows: 11,
        widest: 40,
    };

    assert_eq!(plan_resize(live, 80, 4), ResizePlan::CancelAndRestore);
}

/// A resize that shrinks the window but still fits re-opens at the smaller
/// window — the frame follows the terminal, it does not cancel on a squeeze.
#[test]
fn a_squeezed_but_viable_terminal_reopens_smaller() {
    let live = Live {
        rows: 11,
        widest: 40,
    };

    assert_eq!(
        plan_resize(live, 80, 11),
        ResizePlan::Reopen { visible_rows: 7 }
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Seam: the driver
//
// `LiveFrame` is the only thing in the inline path that moves a cursor. These
// tests run it against a writer that models the screen, so the assertion is
// about what the user would see — not about which escape sequence was used.
// ─────────────────────────────────────────────────────────────────────────────

/// The frame is drawn below the prompt line and never over it.
#[test]
fn the_frame_opens_below_the_prompt_and_leaves_it_alone() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let frame = build_frame(&[web01()], "", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();

    assert_eq!(
        view.borrow().row(0),
        "$ sshm ",
        "the user's prompt line must be untouched"
    );
    assert_eq!(
        view.borrow().row(1),
        "◆ Select a Connection",
        "the frame starts on the line below the prompt"
    );
    assert_eq!(view.borrow().height(), 1 + FRAME_LINES);
}

/// The driver never enters the alternate screen: the user's own scrollback is
/// still where they left it, and the frame is part of it.
#[test]
fn the_driver_never_enters_the_alternate_screen() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let frame = build_frame(&[web01()], "", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();
    live.redraw(&build_frame(
        &[web01()],
        "web",
        0,
        FrameMode::Pick,
        canvas(),
    ))
    .unwrap();
    live.collapse(&settle_trace(&Settle::Picked(web01()), canvas()))
        .unwrap();

    assert!(
        !view.borrow().entered_alternate_screen,
        "the inline driver switched to the alternate screen — the user's \
         scrollback must stay exactly where it was"
    );
}

/// After a submit the live frame is gone: the whole eleven-line frame
/// collapses to the single settle line the spec asks for, sitting on the line
/// below the user's untouched prompt. No orphan rows, no leftover rail
/// fragments (user story 13).
#[test]
fn a_submit_collapses_the_live_frame_to_one_clean_line() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let frame = build_frame(&many(20), "", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();
    assert_eq!(view.borrow().height(), 1 + FRAME_LINES);

    live.collapse(&settle_trace(&Settle::Picked(web01()), canvas()))
        .unwrap();

    assert_eq!(
        view.borrow().rows(),
        vec![
            "$ sshm ".to_string(),
            "◆ picked  web-01  (deploy@10.0.0.4:22)".to_string(),
        ],
        "the live list must be fully erased, leaving the trace and nothing else"
    );
}

/// Same for a cancel: the frame collapses to the cancel trace and the screen
/// is clean.
#[test]
fn a_cancel_collapses_the_live_frame_to_one_clean_line() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let frame = build_frame(&many(20), "web", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();
    live.collapse(&settle_trace(&Settle::Cancelled, canvas()))
        .unwrap();

    assert_eq!(
        view.borrow().rows(),
        vec!["$ sshm ".to_string(), "◆ cancelled".to_string()],
        "a cancel leaves the cancel trace and nothing behind"
    );
}

/// A live redraw keeps the frame the same height on screen: narrowing the
/// query changes which rows are drawn, never how many lines exist.
#[test]
fn a_live_redraw_never_changes_the_height_on_screen() {
    let (mut sink, view) = shared_screen("$ ");
    let conns = many(20);
    let mut live = LiveFrame::new(&mut sink);

    live.open(&build_frame(&conns, "", 0, FrameMode::Pick, canvas()))
        .unwrap();
    let before = view.borrow().height();

    for query in ["c", "c-", "c-0", "c-03", "zzz", ""] {
        live.redraw(&build_frame(&conns, query, 0, FrameMode::Pick, canvas()))
            .unwrap();
        assert_eq!(
            view.borrow().height(),
            before,
            "query {query:?} changed the on-screen height: {:?}",
            view.borrow().rows()
        );
    }
}

/// Erasing the live frame leaves the screen exactly as it was before the frame
/// opened — the primitive the resize fallback is built from.
#[test]
fn erasing_the_live_frame_restores_the_screen_to_what_was_there_before() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let frame = build_frame(&many(20), "", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();
    live.erase().unwrap();

    assert_eq!(view.borrow().rows(), vec!["$ sshm ".to_string()]);
}

/// Re-opening at a new size replaces the live rows in place: the frame stays
/// where it was, at the new height, with the prompt still above it.
#[test]
fn reopening_at_a_new_size_replaces_the_live_rows_in_place() {
    let (mut sink, view) = shared_screen("$ sshm ");
    let conns = many(20);
    let small_canvas = Canvas::new(80, 11, ColorSupport::Truecolor);

    let mut live = LiveFrame::new(&mut sink);
    live.open(&build_frame(&conns, "", 0, FrameMode::Pick, canvas()))
        .unwrap();
    live.reopen(&build_frame(
        &conns,
        "c-0",
        3,
        FrameMode::Pick,
        small_canvas,
    ))
    .unwrap();

    assert_eq!(view.borrow().row(0), "$ sshm ");
    assert_eq!(view.borrow().row(1), "◆ Select a Connection");
    assert_eq!(
        view.borrow().height(),
        1 + (1 + 7 + 1 + 1),
        "the frame re-opened at the smaller terminal's height, not the old one: {:?}",
        view.borrow().rows()
    );
    assert_eq!(
        view.borrow().rows().last().map(String::as_str),
        Some("└"),
        "the re-opened frame closes on its own corner, with nothing left under it"
    );
}

/// The driver reports what it has on screen, which is what the resize plan
/// reads before deciding whether preservation is even possible.
#[test]
fn the_driver_reports_the_live_geometry_it_drew() {
    let (mut sink, _view) = shared_screen("$ ");
    let frame = build_frame(&many(20), "", 0, FrameMode::Pick, canvas());

    let mut live = LiveFrame::new(&mut sink);
    live.open(&frame).unwrap();

    let geometry = live.geometry();
    assert_eq!(geometry.rows, FRAME_LINES);
    assert!(
        geometry.widest >= 30,
        "the widest live row should be the longest row text, got {}",
        geometry.widest
    );
    assert!(
        geometry.widest <= 80,
        "the frame fitted its hint rail to the canvas, got {}",
        geometry.widest
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// A screen model, so the driver can be tested on what it leaves visible
//
// It understands exactly the sequences the driver emits: CR, LF, CUU (`A`),
// CUD (`B`), EL (`2K`), DL (`M`), the cursor show/hide private modes, and
// SGR (`m`, consumed as styling this model does not care about). Anything
// else is a panic: a sequence showing up here that the model cannot interpret
// means the driver grew a new trick nobody modelled.
// ─────────────────────────────────────────────────────────────────────────────

/// A local handle around the modelled screen. The orphan rule will not let a
/// test implement `io::Write` for `Rc` directly, and the driver needs a writer
/// the test can still read while the frame is live — so the shared handle is a
/// local type, and cloning it is the read handle.
#[derive(Clone)]
struct Shared(Rc<RefCell<Screen>>);

impl std::ops::Deref for Shared {
    type Target = RefCell<Screen>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A write handle for the driver plus a read handle for the assertions, so a
/// test can look at the screen while the frame is still live.
fn shared_screen(prompt: &str) -> (Shared, Shared) {
    let screen = Shared(Rc::new(RefCell::new(Screen::with_prompt(prompt))));
    (screen.clone(), screen)
}

#[derive(Debug)]
struct Screen {
    rows: Vec<String>,
    row: usize,
    col: usize,
    entered_alternate_screen: bool,
}

impl Screen {
    fn with_prompt(prompt: &str) -> Self {
        Self {
            rows: vec![prompt.to_string()],
            row: 0,
            col: prompt.chars().count(),
            entered_alternate_screen: false,
        }
    }

    fn rows(&self) -> Vec<String> {
        self.rows.clone()
    }

    fn row(&self, i: usize) -> String {
        self.rows.get(i).cloned().unwrap_or_default()
    }

    fn height(&self) -> usize {
        self.rows.len()
    }

    /// The terminal always has a row under the cursor; this model grows lazily.
    fn ensure_row(&mut self) {
        while self.rows.len() <= self.row {
            self.rows.push(String::new());
        }
    }

    fn put(&mut self, c: char) {
        self.ensure_row();
        if self.col < self.rows[self.row].chars().count() {
            let mut chars: Vec<char> = self.rows[self.row].chars().collect();
            chars[self.col] = c;
            self.rows[self.row] = chars.into_iter().collect();
        } else {
            self.rows[self.row].push(c);
        }
        self.col += 1;
    }

    fn consume(&mut self, s: &str) {
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    assert_eq!(
                        chars.next(),
                        Some('['),
                        "unexpected escape sequence in driver output"
                    );
                    let mut params = String::new();
                    let mut command = None;
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            command = Some(c);
                            break;
                        }
                        params.push(c);
                    }
                    let n: usize = params.trim_start_matches('?').parse().unwrap_or(1);
                    match command {
                        Some('A') => self.row = self.row.saturating_sub(n),
                        Some('B') => self.row += n,
                        Some('K') => {
                            self.ensure_row();
                            self.rows[self.row] = String::new();
                        }
                        Some('M') => {
                            for _ in 0..n {
                                if self.row < self.rows.len() {
                                    self.rows.remove(self.row);
                                }
                            }
                        }
                        Some('m') => {} // SGR: styling, invisible to this model
                        Some('h') | Some('l') => {
                            // A private-mode set/clear. The only one that
                            // matters here is the alternate screen: if the
                            // inline driver ever emits one, the user's
                            // scrollback is gone, and the test that reads
                            // this flag fails.
                            if params.contains("1049") || params.contains("1047") {
                                self.entered_alternate_screen = true;
                            }
                        }
                        other => panic!("unmodelled escape command {other:?} in {params:?}"),
                    }
                }
                '\r' => self.col = 0,
                '\n' => self.row += 1,
                c => self.put(c),
            }
        }
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text =
            std::str::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.borrow_mut().consume(text);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
