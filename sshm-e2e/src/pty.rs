//! A real pseudo-terminal running the real binary.
//!
//! This is the seam the whole E2E suite sits on: nothing here reaches into the
//! program. A child is spawned into a fresh PTY, keystrokes go in as the bytes
//! a keyboard would send, and the answer comes back off the parsed screen grid
//! — the same way the Python harness it replaces worked, in the language the
//! rest of the suite is written in.
//!
//! Three properties hold the whole suite together, and every one of them is a
//! property of *this* file rather than of any test:
//!
//! * **Nothing sleeps.** Every wait is a check on the screen with a deadline,
//!   and a failed wait names the check it was waiting for. A wedged binary
//!   fails with a reason, not with an anonymous timeout.
//! * **The master is always being drained.** A frame repaints enough to fill a
//!   pty buffer, so a reader thread pulls bytes out continuously and feeds
//!   them to the terminal emulator; a test that only reads when it feels like
//!   it would deadlock the child.
//! * **The child cannot outlive the test.** The process group is killed on drop,
//!   so a hung child costs one failed test rather than the whole job.

use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{cmdbuilder::CommandBuilder, native_pty_system, Child, MasterPty, PtySize};

/// The canvas every E2E test runs at. Fixed, not fitted: width fitting and
/// resize planning are seam-tested at the unit level, and a PTY test that
/// varied its own size could not assert on a grid at all.
pub const ROWS: u16 = 24;
pub const COLS: u16 = 80;

/// The deadline a test gets by default. Generous for a loaded CI runner, short
/// enough that the suite stays inside its budget, and short enough that a
/// mutant which wedges the binary is reported by a named check.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);

/// How long a single blocking read on the byte channel may take before the pump
/// loop gets control back to re-check its predicate.
const PUMP_SLICE: Duration = Duration::from_millis(20);

/// How long the terminal has to stay quiet before a screen is believed.
///
/// A frame is not painted atomically. ratatui queues one write per changed run
/// of cells, so a repaint arrives as a handful of small writes and a reader
/// can catch the middle of one: the row that just lost its focus glyph has
/// been rewritten, the row that is gaining it has not been yet. A predicate
/// that reads that screen is reading a screen nobody ever saw.
///
/// The frame is event-driven — it paints when a key arrives and then stops —
/// so silence is the signal that a paint has finished. This is the only place
/// the suite waits on time rather than on content, and it is not a substitute
/// for a content wait: the content still has to be there, and the deadline
/// still applies. It is the difference between "the screen said it" and "the
/// screen said it and meant it".
const QUIET_SLICE: Duration = Duration::from_millis(30);

/// A child process running on a real pseudo-terminal, with its output parsed
/// into a screen grid as it arrives.
///
/// Build one with [`Pty::builder`], drive it with [`Pty::send`] and
/// [`Pty::wait_until`], and let it drop: the process group goes with it.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<JoinHandle<()>>,
    bytes: Receiver<Vec<u8>>,
    /// Every byte the child has ever written, in order. Some tests need the
    /// raw stream rather than the grid — a byte that never reaches the grid
    /// (a query response, an emit on stdout) is still evidence.
    raw: Vec<u8>,
    /// Where [`Pty::answer_queries`] has scanned up to in `raw`.
    scanned: usize,
    parser: vt100::Parser,
    master_fd: RawFd,
    /// Set once the reader thread has closed: the child is gone and no more
    /// bytes will ever arrive.
    eof: bool,
}

/// Assembles one PTY child: the program, its arguments, its environment, and
/// the directory it starts in.
///
/// The environment is *cleared* and rebuilt, because a test that inherits the
/// developer's `HOME`, `TERM` or `SSH_AUTH_SOCK` is a test that passes on one
/// laptop. What the child gets is exactly what [`crate::Fixture::env`] says it
/// gets, plus `TERM` and `LANG` pinned so the emulator and the program agree
/// about what the terminal is.
pub struct PtyBuilder {
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
}

impl PtyBuilder {
    /// An extra variable for the child. Later calls win, so a test can override
    /// something a [`crate::Fixture`] set.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// The directory the child starts in.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Spawn the child. Returns as soon as it is running; nothing here waits
    /// for the program to draw anything — that is what [`Pty::wait_until`] is
    /// for, so the deadline belongs to the check rather than to the spawn.
    pub fn spawn(self) -> io::Result<Pty> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(other)?;

        let mut builder = CommandBuilder::new(&self.program);
        builder.args(&self.args);
        builder.env_clear();
        for (key, value) in &self.env {
            builder.env(key, value);
        }
        // A PTY child with no `TERM` is a program that refuses to draw, and
        // `LANG` decides whether the box-drawing glyphs survive. Both are
        // pinned rather than inherited so the grid means the same thing here
        // and on CI.
        if !self.env.iter().any(|(k, _)| k == "TERM") {
            builder.env("TERM", "xterm-256color");
        }
        if !self.env.iter().any(|(k, _)| k == "LANG") {
            builder.env("LANG", "C.UTF-8");
        }
        if let Some(cwd) = &self.cwd {
            builder.cwd(cwd);
        }

        let child = pair.slave.spawn_command(builder).map_err(other)?;
        // The slave must go: while any process holds it open, a read on the
        // master never sees EOF, and a test waiting for the child to finish
        // would wait forever instead of noticing it already has.
        drop(pair.slave);

        let master_fd = pair
            .master
            .as_raw_fd()
            .ok_or_else(|| io_other("the pty master has no fd to read termios from"))?;
        let reader = pair.master.try_clone_reader().map_err(other)?;
        let writer = pair.master.take_writer().map_err(other)?;

        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || drain(reader, tx));

        Ok(Pty {
            master: pair.master,
            writer,
            child,
            reader: Some(reader),
            bytes: rx,
            raw: Vec::new(),
            scanned: 0,
            parser: vt100::Parser::new(ROWS, COLS, 0),
            master_fd,
            eof: false,
        })
    }
}

/// The two attributes a cell can carry that sshm's design uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: vt100::Color,
    pub bold: bool,
}

/// Blocks forever on `reader`, forwarding every chunk and stopping when the
/// far end is gone.
///
/// The stop condition is the interesting part: when the last slave fd closes,
/// Linux does not return `Ok(0)` from a read on the master, it returns
/// `EIO`. Treating only `Ok(0)` as EOF leaves the thread parked on a dead
/// descriptor for the rest of the run.
fn drain(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
            // EIO (and anything else fatal) means the terminal is gone.
            Err(_) => break,
        }
    }
}

impl Pty {
    /// Start building a child running `program`.
    pub fn builder(program: impl Into<PathBuf>) -> PtyBuilder {
        PtyBuilder {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
        }
    }

    /// Start a child with one argument list and the environment a fixture
    /// produced. The shorthand every simple test uses.
    pub fn spawn(
        program: impl Into<PathBuf>,
        args: &[&str],
        fixture: &crate::Fixture,
    ) -> io::Result<Pty> {
        let mut builder = Pty::builder(program);
        for arg in args {
            builder.args.push((*arg).to_string());
        }
        builder.env = fixture.env();
        builder.cwd = Some(fixture.home_dir().to_path_buf());
        builder.spawn()
    }

    /// Wait until the frame has finished drawing.
    ///
    /// Two things have to be true. The closing corner of the rail has to be on
    /// the screen, because it is the last thing a frame emits and therefore the
    /// only marker that means "everything above it is here too". And the
    /// terminal has to have gone quiet, because a repaint of an already-drawn
    /// frame leaves the old corner in place: waiting for the corner alone
    /// returns the instant a previous frame is on screen, which under a loaded
    /// runner means reading the grid while the repaint is still in flight — a
    /// failure that comes and goes, and a suite with one of those has no right
    /// to forbid retries.
    pub fn wait_for_frame(&mut self) -> io::Result<()> {
        self.wait_until_settled("the frame", DEFAULT_TIMEOUT, |screen| {
            screen.contents().contains("\u{2514}")
        })
    }

    /// Wait until a file the child is expected to write has appeared.
    ///
    /// This is a *pumping* wait, which is the only kind that works here: the
    /// child writes the file as a side effect of a keystroke, and it cannot
    /// take that keystroke while its output buffer is full. A test that polled
    /// the file without draining the terminal would be waiting for a child that
    /// is waiting for the test.
    pub fn wait_for_file(&mut self, path: &Path, timeout: Duration) -> io::Result<()> {
        self.wait_until_pumping(
            &format!("{} to be written", path.display()),
            timeout,
            || path.exists(),
        )
    }

    /// Wait for something outside the screen, pumping the terminal meanwhile.
    ///
    /// The escape hatch for assertions about the world the child leaves behind
    /// — a store file that must have changed, a shim log that must have grown.
    /// Same rule as every other wait here: a deadline, and a failure that names
    /// what it was waiting for.
    pub fn wait_until_pumping<F>(
        &mut self,
        what: &str,
        timeout: Duration,
        mut check: F,
    ) -> io::Result<()>
    where
        F: FnMut() -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            if check() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io_other(&format!(
                    "timed out after {} ms waiting for {what}\n--- screen ---\n{}",
                    timeout.as_millis(),
                    self.grid()
                )));
            }
            self.pump_slice();
        }
    }

    /// Send bytes to the child exactly as a keyboard would.
    ///
    /// These are the bytes crossterm will decode, which is the point: a chord
    /// that the terminal never delivers fails here even though every in-crate
    /// test that synthesises a `KeyEvent` stays green.
    pub fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Send one keystroke given as a byte string literal.
    pub fn key(&mut self, bytes: &[u8]) {
        self.send(bytes).expect("the pty accepts keystrokes");
    }

    /// Drain whatever has arrived, feeding the emulator, until `check` holds.
    ///
    /// The deadline is not decoration: a check that cannot fail is worse than
    /// no check, because it passes by doing nothing. On expiry the failure
    /// names what was being waited for and dumps the grid that refused to
    /// show it.
    pub fn wait_until<F>(&mut self, what: &str, timeout: Duration, check: F) -> io::Result<()>
    where
        F: Fn(&vt100::Screen) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            if check(self.parser.screen()) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(io_other(&format!(
                    "timed out after {} ms waiting for {what}\n--- screen ---\n{}--- raw tail ---\n{}",
                    timeout.as_millis(),
                    self.grid(),
                    self.raw_tail()
                )));
            }
            self.pump_slice();
        }
    }

    /// Wait until `check` holds on a screen the program has stopped painting.
    ///
    /// Same contract as [`Pty::wait_until`] with one addition: after the
    /// predicate holds, the terminal must produce nothing for [`QUIET_SLICE`].
    /// Use it wherever a predicate could be satisfied by the middle of a
    /// repaint — any check that something is *not* on the screen, or that
    /// counts rows, is exactly that — because the absence a half-painted frame
    /// shows is not the absence the test means.
    pub fn wait_until_settled<F>(
        &mut self,
        what: &str,
        timeout: Duration,
        check: F,
    ) -> io::Result<()>
    where
        F: Fn(&vt100::Screen) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            if check(self.parser.screen()) {
                match self.bytes.recv_timeout(QUIET_SLICE) {
                    Ok(chunk) => self.ingest(chunk),
                    Err(RecvTimeoutError::Timeout) => return Ok(()),
                    Err(RecvTimeoutError::Disconnected) => {
                        self.eof = true;
                        return Ok(());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(io_other(&format!(
                    "timed out after {} ms waiting for {what} to settle\n--- screen ---\n{}--- raw tail ---\n{}",
                    timeout.as_millis(),
                    self.grid(),
                    self.raw_tail()
                )));
            }
            self.pump_slice();
        }
    }

    /// Wait until `needle` appears anywhere on the grid.
    pub fn wait_for_text(&mut self, needle: &str) -> io::Result<()> {
        let owned = needle.to_string();
        self.wait_until(
            &format!("`{needle}` on the screen"),
            DEFAULT_TIMEOUT,
            move |screen| screen.contents().contains(&owned),
        )
    }

    /// Wait until the child has exited, and say how.
    ///
    /// The pump keeps running while it waits: a child that is mid-repaint when
    /// it is told to quit will fill the pty buffer, and a buffer nobody reads
    /// blocks the child that is trying to exit.
    pub fn wait_exit(&mut self, timeout: Duration) -> io::Result<ExitCode> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().map_err(other)? {
                // Let the last of the output land before handing back, so a
                // test that asserts on the final grid sees the whole frame.
                self.pump_slice();
                return Ok(ExitCode(status));
            }
            if Instant::now() >= deadline {
                return Err(io_other(&format!(
                    "the child was still running after {} ms\n--- screen ---\n{}",
                    timeout.as_millis(),
                    self.grid()
                )));
            }
            self.pump_slice();
        }
    }

    /// The grid as the terminal sees it, one `String` per row, trailing spaces
    /// kept so column positions mean something.
    pub fn grid(&mut self) -> String {
        self.drain_now();
        self.parser.screen().contents()
    }

    /// The non-empty rows of the grid, trimmed on the right — the shape most
    /// assertions and every golden want.
    pub fn visible_rows(&mut self) -> Vec<String> {
        self.drain_now();
        self.parser
            .screen()
            .contents()
            .lines()
            .map(|line| line.trim_end().to_string())
            .collect()
    }

    /// The grid as a document: every row down to the last one carrying
    /// anything, right-trimmed.
    ///
    /// This is the shape a golden holds. A golden of a 24-row terminal would
    /// spend nineteen lines saying "nothing", and the thing a reviewer needs to
    /// see is the frame's own extent — how tall it is, and where it stops.
    pub fn screen_document(&mut self) -> String {
        let rows = self.visible_rows();
        match rows.iter().rposition(|row| !row.trim().is_empty()) {
            Some(last) => rows[..=last].join("\n"),
            None => String::new(),
        }
    }

    /// The text of one grid row, right-trimmed.
    pub fn row(&mut self, row: u16) -> String {
        self.visible_rows()
            .get(row as usize)
            .cloned()
            .unwrap_or_default()
    }

    /// The colour the terminal recorded for one cell, as the emulator resolved
    /// it. Tests compare this against a `Theme` role rather than a literal.
    pub fn cell_fg(&mut self, row: u16, col: u16) -> Option<vt100::Color> {
        self.drain_now();
        self.parser.screen().cell(row, col).map(|c| c.fgcolor())
    }

    /// Whether the cell holding `needle`'s first character is bold.
    pub fn cell_bold(&mut self, row: u16, col: u16) -> Option<bool> {
        self.drain_now();
        self.parser.screen().cell(row, col).map(|c| c.bold())
    }

    /// Everything a cell's styling carries: colour and weight.
    ///
    /// Both, because the selection is carried by a glyph plus a **bold** alias
    /// rather than by a filled row — a colour-only check would miss half of how
    /// a user sees which row is theirs.
    pub fn cell_style(&mut self, row: u16, col: u16) -> Option<CellStyle> {
        self.drain_now();
        self.parser.screen().cell(row, col).map(|c| CellStyle {
            fg: c.fgcolor(),
            bold: c.bold(),
        })
    }

    /// The colour of the first cell of the first occurrence of `needle`.
    pub fn fg_of(&mut self, needle: &str) -> Option<vt100::Color> {
        self.locate(needle)
            .map(|(row, col, _)| row_col_fg(self, row, col))
    }

    /// Whether the first cell of the first occurrence of `needle` is bold.
    pub fn bold_of(&mut self, needle: &str) -> Option<bool> {
        self.locate(needle)
            .map(|(row, col, _)| self.cell_bold(row, col).unwrap_or(false))
    }

    /// Where `needle` first appears on the grid, as `(row, column, row_text)`.
    pub fn locate(&mut self, needle: &str) -> Option<(u16, u16, String)> {
        self.drain_now();
        for (idx, line) in self.parser.screen().contents().lines().enumerate() {
            if let Some(col) = line.find(needle) {
                // Columns are counted in cells, not bytes: the frame is full
                // of box-drawing glyphs, and a byte offset is not a column.
                let cells = line[..col].chars().count();
                return Some((idx as u16, cells as u16, line.trim_end().to_string()));
            }
        }
        None
    }

    /// The raw byte stream the child has written, for the assertions the grid
    /// cannot make — that a byte sequence arrived at all, or that stdout
    /// carried exactly one thing.
    pub fn raw(&mut self) -> Vec<u8> {
        self.drain_now();
        self.raw.clone()
    }

    /// Whether the terminal is cooked — echoing and line-buffered — as the
    /// terminal itself reports it.
    ///
    /// `tcgetattr` on the *master* answers for the slave on Linux, which is
    /// what makes this a fact about the terminal rather than a claim by the
    /// program about itself.
    pub fn termios_is_cooked(&self) -> io::Result<bool> {
        let flags = local_flags(self.master_fd)?;
        Ok(flags & (libc::ECHO | libc::ICANON) == (libc::ECHO | libc::ICANON))
    }

    /// Whether the terminal has flow control switched on — the `IXON` bit that
    /// turns `Ctrl+S` into a stop signal instead of a keystroke.
    pub fn termios_ixon(&self) -> io::Result<bool> {
        Ok(local_flags(self.master_fd)? & libc::IXON != 0)
    }

    /// Switch `IXON` off on the terminal.
    ///
    /// The Shell Widget binds `Ctrl+Alt+S`, and with `IXON` on the `Ctrl+S`
    /// inside it is swallowed by the line discipline as XOFF before any
    /// program sees it. A real interactive shell turns this off itself; the
    /// harness does the same so the chord is drivable at all.
    pub fn clear_ixon(&self) -> io::Result<()> {
        let mut termios = termios_of(self.master_fd)?;
        termios.c_iflag &= !(libc::IXON as libc::tcflag_t);
        set_termios(self.master_fd, &termios)
    }

    /// The terminal's window size as the kernel has it.
    pub fn winsize(&self) -> io::Result<(u16, u16)> {
        let size = self.master.get_size().map_err(other)?;
        Ok((size.rows, size.cols))
    }

    /// The child's pid, for a test that needs to know it is really a process.
    pub fn pid(&self) -> Option<u32> {
        self.child.process_id()
    }

    /// Whether the reader thread has seen the terminal close.
    pub fn terminal_closed(&self) -> bool {
        self.eof
    }

    /// Take one byte chunk if one is waiting, and answer any terminal query it
    /// contains. Never blocks.
    fn pump_now(&mut self) -> bool {
        match self.bytes.try_recv() {
            Ok(chunk) => {
                self.ingest(chunk);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.eof = true;
                false
            }
        }
    }

    /// Take one byte chunk, waiting a short slice for it to arrive.
    fn pump_slice(&mut self) {
        match self.bytes.recv_timeout(PUMP_SLICE) {
            Ok(chunk) => self.ingest(chunk),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => self.eof = true,
        }
    }

    /// Empty whatever has arrived, so an assertion sees the current state
    /// rather than the state as of the last wait.
    fn drain_now(&mut self) {
        while self.pump_now() {}
    }

    fn ingest(&mut self, chunk: Vec<u8>) {
        self.raw.extend_from_slice(&chunk);
        self.parser.process(&chunk);
        self.answer_queries();
    }

    /// Answer the terminal queries a program may block waiting for.
    ///
    /// A PTY with nothing on the other end answers nothing, and a program that
    /// asks "where is the cursor?" and gets silence will sit there until its
    /// own timeout expires. The harness answers instead, so a stall is a bug in
    /// the program rather than an artefact of the harness.
    ///
    /// Scans from where it last looked, minus enough overlap to catch a
    /// sequence that arrived split across two reads.
    fn answer_queries(&mut self) {
        let start = self.scanned.saturating_sub(8);
        self.scanned = self.raw.len();
        // Copied, not borrowed: answering a query writes to the master, and the
        // master is `self`.
        let window = self.raw[start..].to_vec();

        // DSR 6n — "report cursor position". Answered at the top-left, which
        // is where an inline frame has just put the cursor.
        if let Some(pos) = find(&window, b"\x1b[6n") {
            let _ = self.write(b"\x1b[1;1R");
            let _ = pos;
        }
        // DSR 5n — "is the terminal ready". Yes.
        if find(&window, b"\x1b[5n").is_some() {
            let _ = self.write(b"\x1b[0n");
        }
        // DA2 — device attributes II. A plain VT100 with no extensions.
        if find(&window, b"\x1b[>c").is_some() || find(&window, b"\x1b[>0c").is_some() {
            let _ = self.write(b"\x1b[>0;0;0c");
        }
        // DA1 — device attributes I. The response must not be read as the
        // `ESC [ c` request again, which is why the answers start with `?` or
        // `>` and none of the patterns above is a prefix of an answer.
        if find(&window, b"\x1b[c").is_some()
            || find(&window, b"\x1b[0c").is_some()
            || find(&window, b"\x1b[?c").is_some()
        {
            let _ = self.write(b"\x1b[?62;1;2;6;9;15;18;21;22c");
        }
        // The kitty keyboard-extension probe. Answering "no flags" is what
        // stops a modern terminal library waiting for a protocol negotiation.
        if find(&window, b"\x1b[?u").is_some() {
            let _ = self.write(b"\x1b[?0u");
        }
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    fn raw_tail(&self) -> String {
        let tail = &self.raw[self.raw.len().saturating_sub(400)..];
        String::from_utf8_lossy(tail).escape_debug().to_string()
    }
}

fn row_col_fg(pty: &mut Pty, row: u16, col: u16) -> vt100::Color {
    pty.cell_fg(row, col).unwrap_or(vt100::Color::Default)
}

/// The exit code, with the message a test wants when it is not what it hoped.
pub struct ExitCode(portable_pty::ExitStatus);

impl ExitCode {
    /// The number a shell would see. A signal-terminated child reports the
    /// signal in its `Display`, so a test never reads a meaningless `1`.
    pub fn code(&self) -> Option<u32> {
        match self.0.signal() {
            Some(_) => None,
            None => Some(self.0.exit_code()),
        }
    }

    /// Assert the code and produce a readable failure otherwise.
    pub fn assert_code(&self, expected: u32) {
        let actual = self.code();
        assert_eq!(
            actual,
            Some(expected),
            "the child exited with {self}, not {expected}"
        );
    }
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.code() {
            Some(code) => write!(f, "exit code {code}"),
            None => write!(f, "signal {:?}", self.0.signal()),
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // The whole process group, not just the child: a shell child leaves a
        // grandchild holding the terminal otherwise.
        if let Some(pid) = self.child.process_id() {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn local_flags(fd: RawFd) -> io::Result<libc::tcflag_t> {
    Ok(termios_of(fd)?.c_lflag)
}

fn termios_of(fd: RawFd) -> io::Result<libc::termios> {
    let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
    if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(termios)
}

fn set_termios(fd: RawFd, termios: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, termios) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn io_other(message: &str) -> io::Error {
    io::Error::other(message.to_string())
}

/// Anyhow-flavoured failure from portable-pty, turned into an `io::Error` so
/// every fallible step in this module reports the same way.
fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
