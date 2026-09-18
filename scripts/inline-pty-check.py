#!/usr/bin/env python3
"""Drive the #34 inline frame under a real PTY and print what the screen holds.

The unit tests model the terminal in Rust; this drives the real thing. It
spawns the inline demo inside a pty behind a few lines of fake scrollback,
feeds it keystrokes, resizes the pty mid-frame, and prints the screen buffer
after every step so the frame can be *looked at* — which is the rule this
repo holds a UI change to.

    cargo build --example inline_demo
    python3 scripts/inline-pty-check.py

The screen model understands the sequences the inline driver emits (CUU/CUD,
EL, DL, SGR, the cursor private modes) plus CR/LF/printables. Ambiguous-width
glyphs (◆ │ ❯ └) are counted as one cell, as they render in a monospace
terminal; East-Asian Wide/Fullwidth glyphs are counted as two, which is what
makes the wide-character scenario mean something.

Horizontal resize **reflows**, like a real terminal: every logical line is
re-wrapped at the new width, so a row drawn at 80 columns occupies more
physical rows at 60 than it was drawn as. The earlier version of this model
truncated instead — which meant the harness never once exercised the case
the driver's cancel-and-restore fallback exists for, and a test that cannot
fail is not a test.

Checks are printed as PASS/FAIL next to each dump and the script exits
non-zero if any of them fails, so this is a gate as well as a look.
"""

import fcntl
import hashlib
import json
import os
import pty
import select
import struct
import sys
import tempfile
import termios
import time
import unicodedata

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "sshm", "target", "debug", "examples", "inline_demo")
SSH_BIN = os.path.join(REPO, "sshm", "target", "debug", "sshm")

SCROLLBACK = [
    "prior scrollback: cargo build --release",
    "prior scrollback: git log --oneline -3",
    "prior scrollback: ls demo docs",
]

# The row the demo's prompt sits on, and the row the frame opens on.
PROMPT_ROW = len(SCROLLBACK)
FRAME_ROW = PROMPT_ROW + 1

DOWN = b"\x1b[B"
UP = b"\x1b[A"
RIGHT = b"\x1b[C"
LEFT = b"\x1b[D"
ENTER = b"\r"
ESC = b"\x1b"
BACKSPACE = b"\x7f"
TAB = b"\t"

# The form map's own geometry, in rows below the header. Six fields, then
# the rule row (which the error line takes over), then the `▶` row — the
# eight rows `VISIBLE_ROWS` is spent on.
FIELD_ORDER = ["Alias", "Host", "User", "Port", "Key", "Folder"]
RULE_OFFSET = 7
SUBMIT_OFFSET = 8

CHECKS = []


def check(label, ok, detail=""):
    """Record a verdict so the run can both be looked at and be gated."""
    CHECKS.append((label, ok))
    print(f"    [{'PASS' if ok else 'FAIL'}] {label}" + (f" — {detail}" if detail else ""))
    return ok


def cell_width(ch):
    """Cells a character occupies in a monospace terminal.

    East Asian Wide and Fullwidth take two. Combining marks take none.
    Ambiguous (`◆ │ ❯ └ ·`) take one, which is how they render in the
    monospace terminals this project targets.
    """
    if unicodedata.combining(ch):
        return 0
    if unicodedata.east_asian_width(ch) in ("W", "F"):
        return 2
    return 1


def display_width(text):
    return sum(cell_width(c) for c in text)


def wrap_logical(text, cols):
    """Re-wrap one logical line into physical rows of at most `cols` cells."""
    if cols <= 0:
        return [text]
    chunks, cur, used = [], [], 0
    for ch in text:
        w = cell_width(ch)
        if used + w > cols and cur:
            chunks.append("".join(cur))
            cur, used = [], 0
        cur.append(ch)
        used += w
    chunks.append("".join(cur))
    return chunks


class Screen:
    """A minimal VT screen: rows x cols of cells, with a cursor."""

    def __init__(self, rows, cols):
        self.rows = rows
        self.cols = cols
        self.buf = [[" "] * cols for _ in range(rows)]
        # The SGR run each cell was painted under, kept alongside the glyph.
        # The text model ignores colour; the *checks* cannot. "`▶` is lit"
        # and "`▶` is dim" are the same characters and nothing else, so a
        # screen that throws the escape away cannot tell a ready submit row
        # from a dead one — and a check that cannot fail is not a check.
        self.style = [[""] * cols for _ in range(rows)]
        self.r = 0
        self.c = 0
        self.alt_screen = False
        self.sgr = ""

    def feed(self, s):
        i = 0
        n = len(s)
        while i < n:
            ch = s[i]
            if ch == "\x1b":
                i = self._escape(s, i)
            elif ch == "\r":
                self.c = 0
                i += 1
            elif ch == "\n":
                self._lf()
                i += 1
            elif ch == "\x08":
                self.c = max(0, self.c - 1)
                i += 1
            elif ch == "\t":
                self.c = min(self.cols - 1, self.c + 8)
                i += 1
            else:
                self._put(ch)
                i += 1

    def _escape(self, s, i):
        if i + 1 >= len(s):
            return i + 1
        if s[i + 1] != "[":
            return i + 2
        j = i + 2
        params = ""
        while j < len(s) and (s[j].isdigit() or s[j] in ";?<>"):
            params += s[j]
            j += 1
        if j >= len(s):
            return j
        cmd = s[j]
        self._csi(cmd, params)
        return j + 1

    def _csi(self, cmd, params):
        nums = [int(p) if p else 1 for p in params.lstrip("?").split(";")] if params.lstrip("?") else []

        def arg(idx, default=1):
            return nums[idx] if idx < len(nums) else default

        if cmd == "A":
            self.r = max(0, self.r - arg(0))
        elif cmd == "B":
            self.r = min(self.rows - 1, self.r + arg(0))
        elif cmd == "C":
            self.c = min(self.cols - 1, self.c + arg(0))
        elif cmd == "D":
            self.c = max(0, self.c - arg(0))
        elif cmd == "E":
            self._lf()
            self.c = 0
        elif cmd == "F":
            self.r = max(0, self.r - 1)
            self.c = 0
        elif cmd == "G":
            self.c = arg(0) - 1
        elif cmd == "d":
            self.r = arg(0) - 1
        elif cmd in ("H", "f"):
            self.r = arg(0) - 1
            self.c = arg(1) - 1
        elif cmd == "J":
            self._erase_display(arg(0))
        elif cmd == "K":
            self._erase_line(arg(0))
        elif cmd == "M":
            # DL: the rows below pull up, a blank row appears at the bottom.
            for _ in range(arg(0)):
                if self.r < len(self.buf):
                    del self.buf[self.r]
                    self.buf.append([" "] * self.cols)
        elif cmd == "@":
            pass
        elif cmd == "m":
            # Track the active SGR run so a check can ask what colour a
            # given cell was painted in. `0` / empty is a full reset;
            # anything else replaces the run, which is close enough to how
            # the emitters here work — they end every run with a reset.
            self.sgr = "" if params in ("", "0", ";") else params
        elif cmd in ("h", "l"):
            if "1049" in params or "1047" in params:
                self.alt_screen = True
        else:
            raise SystemExit(f"unmodelled CSI: {cmd!r} {params!r}")

    def _lf(self):
        if self.r == self.rows - 1:
            self.buf.pop(0)
            self.buf.append([" "] * self.cols)
        else:
            self.r += 1

    def _put(self, ch):
        while len(self.buf) <= self.r:
            self.buf.append([" "] * self.cols)
            self.style.append([""] * self.cols)
        w = cell_width(ch)
        if self.c < self.cols:
            self.buf[self.r][self.c] = ch
            self.style[self.r][self.c] = self.sgr
            # A double-width glyph owns the cell after it as well; blanking it
            # keeps the column arithmetic honest.
            for k in range(1, w):
                if self.c + k < self.cols:
                    self.buf[self.r][self.c + k] = ""
                    self.style[self.r][self.c + k] = self.sgr
            self.c += w

    def _erase_line(self, mode):
        if self.r >= len(self.buf):
            return
        if mode == 2:
            self.buf[self.r] = [" "] * self.cols
            self.style[self.r] = [""] * self.cols
        elif mode == 0:
            for c in range(self.c, self.cols):
                self.buf[self.r][c] = " "
                self.style[self.r][c] = ""
        elif mode == 1:
            for c in range(0, min(self.c + 1, self.cols)):
                self.buf[self.r][c] = " "
                self.style[self.r][c] = ""

    def _erase_display(self, mode):
        if mode == 2:
            self.buf = [[" "] * self.cols for _ in range(self.rows)]
            self.style = [[""] * self.cols for _ in range(self.rows)]
            self.r = 0
            self.c = 0

    def text(self, upto=None):
        rows = self.buf if upto is None else self.buf[:upto]
        return ["".join(r).rstrip() for r in rows]

    def resized(self, rows, cols):
        """The same screen after the window changed shape.

        Vertical: the viewport keeps the cursor visible. On a shrink the
        content above it scrolls out (into scrollback this model does not
        track); on a grow blank rows appear at the bottom.

        Horizontal: **reflow**. Every logical line is re-wrapped at the new
        width, so a row drawn at 80 columns becomes two physical rows at 40
        and the frame occupies more of the glass than it was drawn as. The
        inline driver hard-newlines every row and never soft-wraps, so each
        row of this buffer is one logical line. The cursor rides on its own
        logical line through the re-wrap.
        """
        logical = ["".join(r).rstrip() for r in self.buf]
        # The cursor may sit one row past the drawn content — real space,
        # but no content of its own.
        past_end = self.r >= len(logical)

        reflowed = []
        cursor = 0
        for i, line in enumerate(logical):
            chunks = wrap_logical(line, cols)
            if i == self.r:
                chunk = min(self.c // max(cols, 1), len(chunks) - 1)
                cursor = len(reflowed) + chunk
            reflowed.extend(chunks)
        if past_end:
            cursor = len(reflowed)

        s = Screen(rows, cols)
        start = max(0, min(cursor - (rows - 1), max(0, len(reflowed) - rows)))
        for i in range(rows):
            src = start + i
            if src < len(reflowed):
                for j, ch in enumerate(reflowed[src][:cols]):
                    s.buf[i][j] = ch
        s.r = cursor - start
        s.alt_screen = self.alt_screen
        return s

    def nonblank_height(self):
        rows = self.text()
        last = -1
        for i, r in enumerate(rows):
            if r.strip():
                last = i
        return last + 1

    def frame_rows(self):
        """Non-blank rows from the frame's own region down.

        The demo reports its outcome on stderr (`[demo] Picked(...)`), which
        lands on the glass right after the settle trace. It is harness
        output, not frame output, so it is not counted here.
        """
        return [
            r
            for r in self.text()[FRAME_ROW:]
            if r.strip() and not r.startswith("[demo]")
        ]


class Demo:
    def __init__(self, args, rows=24, cols=80, bin=None, home=None, env=None):
        bin = bin or BIN
        if not os.path.exists(bin):
            raise SystemExit(f"build the demo first: {bin}")
        master, slave = pty.openpty()
        self._set_size(slave, rows, cols)
        pid = os.fork()
        if pid == 0:
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
            os.dup2(slave, 0)
            os.dup2(slave, 1)
            os.dup2(slave, 2)
            os.close(master)
            os.close(slave)
            script = ""
            if home:
                # The manage scenario runs the real binary against a throwaway
                # HOME so deletes land on a real connections.json we can hash.
                script += f"export HOME={home!r}; "
            for k, v in (env or {}).items():
                script += f"export {k}={v!r}; "
            script += "".join(f"printf '%s\\n' {line!r};" for line in SCROLLBACK)
            script += "printf '$ ';"
            script += f" exec {bin} {' '.join(args)}"
            os.execvp("/bin/bash", ["bash", "-c", script])
            os._exit(127)
        os.close(slave)
        self.master = master
        self.pid = pid
        self.screen = Screen(rows, cols)
        self.rows = rows
        self.cols = cols

    @staticmethod
    def _set_size(fd, rows, cols):
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    def pump(self, seconds=0.35):
        deadline = time.time() + seconds
        while time.time() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                if not data:
                    break
                self.screen.feed(data.decode("utf-8", "replace"))

    def send(self, data, settle=0.35):
        os.write(self.master, data)
        self.pump(settle)

    def resize(self, rows, cols, settle=0.5):
        # TIOCSWINSZ on the master resizes the slave and raises SIGWINCH in
        # the foreground process group — the same path a real resize takes.
        self._set_size(self.master, rows, cols)
        self.rows, self.cols = rows, cols
        self.screen = self.screen.resized(rows, cols)
        self.pump(settle)

    def reflow_only(self, rows, cols):
        """Resize and reflow the model, but do not let the driver answer yet.

        The terminal re-wraps the glass the instant the window changes; the
        driver finds out afterwards, via SIGWINCH. Splitting the two is what
        lets the reflow itself be looked at before the fallback erases it —
        otherwise every dump after a narrowing shows the aftermath and the
        thing under test is never visible at all.
        """
        self._set_size(self.master, rows, cols)
        self.rows, self.cols = rows, cols
        self.screen = self.screen.resized(rows, cols)

    def dump(self, label):
        rows = self.screen.text()
        print(f"\n=== {label}  ({self.cols}x{self.rows}) ===")
        for i, r in enumerate(rows):
            marker = "·" if not r.strip() else " "
            print(f"{i:2d}{marker}| {r}")
        print(f"    non-blank rows on screen: {self.screen.nonblank_height()}"
              f"   alternate screen: {self.screen.alt_screen}")

    def exit_code(self, timeout=3.0):
        """The child's exit status, or None if it is still running."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                pid, status = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                return None
            if pid:
                if os.WIFEXITED(status):
                    return os.WEXITSTATUS(status)
                return None
            time.sleep(0.05)
        return None


def scenario_pick():
    d = Demo(["pick"])
    d.pump(0.6)
    d.dump("A. frame opened (below the prompt, prior scrollback intact)")
    check("no alternate screen", not d.screen.alt_screen)
    check("frame opens below the prompt line",
          d.screen.text()[PROMPT_ROW] == "$", repr(d.screen.text()[PROMPT_ROW]))

    for _ in range(6):
        d.send(DOWN, 0.12)
    d.dump("B. selection 6 of 15 — window slid to 2..10, selection centred")

    for _ in range(8):
        d.send(DOWN, 0.12)
    d.dump("C. selection pinned at the end — window 7..15, height unchanged")

    height = len(d.screen.frame_rows())
    for ch in "web":
        d.send(ch.encode(), 0.2)
    d.dump("D. typed 'web' — narrowed to 2 rows, frame still 11 lines")
    check("height constant across narrowing", len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")

    d.send(BACKSPACE, 0.2)
    d.send(BACKSPACE, 0.2)
    d.dump("E. backspaced to 'w' — list widened again, height unchanged")
    check("height constant across widening", len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")

    d.send(ENTER, 0.5)
    d.dump("F. Enter — live list erased, settle trace only")
    check("submit leaves exactly one frame row", len(d.screen.frame_rows()) == 1,
          f"{len(d.screen.frame_rows())} rows: {d.screen.frame_rows()}")
    check("the trace carries the folder prefix",
          d.screen.frame_rows()[0].startswith("◆ picked  ["),
          d.screen.frame_rows()[0])


def scenario_cancel():
    d = Demo(["pick"])
    d.pump(0.6)
    d.send(b"sta", 0.4)
    d.dump("G. typed 'sta' before cancelling")
    d.send(ESC, 0.5)
    d.dump("H. Esc — cancel trace, nothing left behind")
    check("cancel leaves exactly one frame row", len(d.screen.frame_rows()) == 1,
          f"{d.screen.frame_rows()}")
    check("cancel trace is '◆ cancelled'",
          d.screen.frame_rows()[0] == "◆ cancelled", d.screen.frame_rows()[0])


def scenario_resize():
    d = Demo(["pick"])
    d.pump(0.6)
    d.send(b"web", 0.4)
    d.send(DOWN, 0.2)
    d.dump("I. before resize: query 'web', selection on web-02")

    d.resize(30, 100)
    d.dump("J. 100x30 — re-opened, query and selection kept")
    check("re-opened frame keeps the query", "web" in "\n".join(d.screen.frame_rows()),
          "query survived the resize")

    d.resize(11, 100)
    d.dump("K. 100x11 — re-opened SMALLER (7 visible rows), query and selection kept")

    d.resize(24, 100)
    d.dump("L. back to 100x24 — re-opened at 8 visible rows again")


def scenario_reflow():
    """The case the cancel-and-restore fallback exists for.

    The terminal narrows *after* the frame is drawn. The rows that no longer
    fit are re-wrapped by the terminal into more physical rows before the
    driver is told, so a collapse that counts the rows it drew erases too
    few and leaves the frame's tail sitting above the trace.
    """
    d = Demo(["pick"])
    d.pump(0.6)
    before = d.screen.nonblank_height()
    d.dump("M. 80x24 before the narrowing")

    d.reflow_only(24, 60)
    mid = d.screen.nonblank_height()
    d.dump("N. narrowed to 60 — the hint rail re-wrapped, so the frame now "
           "occupies MORE physical rows than it was drawn as")
    check("the narrowing really did reflow (frame grew on the glass)",
          mid > before, f"{before} -> {mid} non-blank rows")

    d.pump(0.8)
    d.dump("O. the driver's cancel-and-restore after the reflow")
    trace = d.screen.text()[FRAME_ROW]
    check("the trace landed exactly on the frame's own first row",
          trace == "◆ cancelled", repr(trace))
    check("clean cancel after a reflowing narrowing — no surviving rows",
          "│" not in "\n".join(d.screen.text()[PROMPT_ROW:])
          and "└" not in "\n".join(d.screen.text()[PROMPT_ROW:]),
          "no rail fragment and no corner left below the prompt")
    check("nothing survived above the trace",
          all(not r.strip() for r in d.screen.text()[FRAME_ROW + 2:]),
          "the rows under the settle are empty")


def scenario_wide_chars():
    """A CJK alias must not break the collapse.

    A wide glyph is one `char` and two columns. Measuring rows in characters
    under-counts them, the row wraps, and the `up(N)` / `\\x1b[M` counts
    stop matching the glass.
    """
    d = Demo(["pick"])
    d.pump(0.6)
    d.send("服务器".encode("utf-8"), 0.4)
    d.dump("P. typed a CJK query — the wide-character row is listed")
    check("the wide-character row matched", "服务器-01" in "\n".join(d.screen.frame_rows()),
          "CJK alias visible in the frame")

    d.send(ENTER, 0.5)
    d.dump("Q. Enter on the CJK alias — collapse must still be one clean line")
    check("wide-character settle leaves one frame row", len(d.screen.frame_rows()) == 1,
          f"{d.screen.frame_rows()}")
    check("wide-character trace names the CJK folder and alias",
          d.screen.frame_rows()[0] == "◆ picked  [生产] 服务器-01  (ops@10.9.9.9:22)",
          d.screen.frame_rows()[0])


def scenario_short_terminal():
    d = Demo(["pick"], rows=11, cols=80)
    d.pump(0.6)
    d.dump("R. a short terminal: 7 visible rows, one constant height")
    d.send(b"w", 0.4)
    d.dump("S. same short terminal after typing — height unchanged")


def scenario_too_short_at_open():
    """A terminal too short to host the frame must cancel cleanly at open.

    Before the guard, `fit_visible_rows` returned `None`, `build_frame`
    substituted zero visible rows, and the frame opened as three lines of
    chrome with no list in it.
    """
    d = Demo(["pick"], rows=4, cols=80)
    d.pump(0.8)
    d.dump("T. a 4-row terminal: too short for a frame at all")
    screen = "\n".join(d.screen.text())
    check("no degenerate frame was drawn", "│" not in screen and "└" not in screen,
          "no rail and no corner on screen")
    # FRAME_ROW is meaningless here: a 4-row terminal scrolled the fake
    # scrollback off the top, so look for the trace anywhere on the glass.
    traces = [r for r in d.screen.text() if r.startswith("◆")]
    check("the too-short open leaves the cancel trace and nothing else",
          traces == ["◆ cancelled"], f"◆ lines on screen: {traces}")
    check("no alternate screen on the too-short path", not d.screen.alt_screen)


def scenario_manage():
    """#36: in-list delete with an inline (y/N) confirm, on the real binary.

    Runs `sshm manage` against a throwaway HOME holding a seeded
    `~/.ssh/connections.json`, so every verdict about persistence is a
    sha256 of the real file, not a claim. The chord bytes are the raw
    control characters a terminal sends for Ctrl+X (0x18) and Ctrl+C (0x03).
    """
    home = tempfile.mkdtemp(prefix="sshm-pty-manage-")
    cfg = os.path.join(home, ".ssh", "connections.json")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)
    with open(cfg, "w") as f:
        json.dump(
            {
                "connections": [
                    {"id": "id-web-01", "alias": "web-01", "host": "10.0.0.4",
                     "user": "deploy", "port": 22, "folder": "prod"},
                    {"id": "id-web-02", "alias": "web-02", "host": "10.0.0.5",
                     "user": "deploy", "port": 22, "folder": "prod"},
                ]
            },
            f,
            indent=2,
        )

    def sha():
        with open(cfg, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()

    h0 = sha()
    print(f"    seeded connections.json sha256: {h0}")

    CTRL_X, CTRL_C = b"\x18", b"\x03"

    # ── run 1: Ctrl+X raises the confirm; N declines; hash unchanged ────
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.dump("U. `sshm manage` — frame open over two Connections")
    height = len(d.screen.frame_rows())
    check("both Connections listed",
          "10.0.0.4" in "\n".join(d.screen.frame_rows())
          and "10.0.0.5" in "\n".join(d.screen.frame_rows()))

    d.send(CTRL_X, 0.4)
    d.dump("V. Ctrl+X — the inline confirm is raised in the header")
    check("confirm header is the ticket's string, verbatim",
          d.screen.text()[FRAME_ROW] == "◆ Delete [prod] web-01? (y/N)",
          repr(d.screen.text()[FRAME_ROW]))
    check("the confirm does not grow the frame",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the confirm rail names the answers it reads",
          "y confirm" in rail and "N abort" in rail and "Esc back" in rail,
          repr(rail))

    d.send(b"N", 0.4)
    d.dump("W. N at the confirm — declined, settled with ■, nothing deleted")
    joined = "\n".join(d.screen.frame_rows())
    check("the declined confirm settles with ■",
          "│   ■ Delete [prod] web-01? No" in joined, joined)
    check("nothing claims a delete happened", "deleted" not in joined, joined)
    check("web-01 is still in the list", "10.0.0.4" in joined)
    d.send(ESC, 0.5)
    d.dump("X. Esc leaves the declined run")
    h1 = sha()
    check("N left connections.json byte-identical", h1 == h0, f"{h0} -> {h1}")

    # ── run 1b: Esc at the confirm declines it too — never deletes ─────
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(CTRL_X, 0.4)
    d.send(ESC, 0.4)
    d.dump("X2. Esc at the confirm — backs out of the step, still no delete")
    joined = "\n".join(d.screen.frame_rows())
    check("Esc closed the confirm without a Yes",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections"
          and "Yes" not in joined, joined)
    check("the decline still leaves its ■ No trace",
          "■ Delete [prod] web-01? No" in joined, joined)
    check("web-01 still on the glass", "10.0.0.4" in joined)
    d.send(ESC, 0.5)
    d.dump("X3. second Esc exits the frame")
    check("the second Esc leaves the cancel trace",
          d.screen.text()[FRAME_ROW] == "◆ cancelled",
          repr(d.screen.text()[FRAME_ROW]))
    check("Esc at the confirm deleted nothing", sha() == h0)

    # ── run 2: y deletes exactly one, settles ■, note ◇, hash changes ──
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(CTRL_X, 0.4)
    d.send(b"y", 0.4)
    d.dump("Y. y at the confirm — settled ■ Yes, dim ◇ note, list returns")
    joined = "\n".join(d.screen.frame_rows())
    check("the answered confirm settles with ■ Yes",
          "│   ■ Delete [prod] web-01? Yes" in joined, joined)
    check("the dim note names the deleted Connection",
          "◇ deleted [prod] web-01" in joined, joined)
    check("exactly one Connection left the list (web-01's host gone)",
          "10.0.0.4" not in joined, joined)
    check("the neighbour survives in the list (web-02's host present)",
          "10.0.0.5" in joined, joined)
    d.send(ESC, 0.5)
    d.dump("Z. Esc after the delete")
    h2 = sha()
    check("y changed connections.json", h2 != h0, f"{h0} -> {h2}")
    on_disk = open(cfg).read()
    check("web-01 is gone from the file", "web-01" not in on_disk, on_disk)
    check("web-02 survives in the file", "web-02" in on_disk)

    # ── run 3: a printable filters and never deletes; Ctrl+C cancels ────
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(b"x", 0.4)
    d.dump("AA. printable 'x' — the chord letter typed into the filter")
    joined = "\n".join(d.screen.frame_rows())
    check("typing 'x' searched instead of deleting (no-match state)",
          'No matches for "x"' in joined, joined)
    check("the printable left the file untouched", sha() == h2)
    d.send(CTRL_C, 0.5)
    d.dump("AB. Ctrl+C — the frame cancels clean")
    check("Ctrl+C leaves the cancel trace",
          d.screen.text()[FRAME_ROW] == "◆ cancelled",
          repr(d.screen.text()[FRAME_ROW]))
    check("Ctrl+C deleted nothing", sha() == h2)
    check("no alternate screen on any manage path", not d.screen.alt_screen)

    # ── run 4: the rail lists only keys that work (#36 review, #37 update) ──
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.dump("AC. the manage rail, as painted")
    rail = d.screen.text()[FRAME_ROW + len(d.screen.frame_rows()) - 2]
    check("the rail still names the chord that really deletes",
          "Ctrl+X delete" in rail, repr(rail))
    # `Ctrl+A add` is back on the rail (#37): the chord now walks the
    # five-step sequence and writes the Connection, which is the thing the
    # label promises. The #36-review check that demanded its absence was
    # written when the chord answered "not built yet" — that excuse died
    # with the checkpoint this scenario replaces.
    check("the rail advertises the chord that now adds",
          "Ctrl+A add" in rail, repr(rail))
    # `Ctrl+E edit` is on the rail (#37): the chord now opens the in-place
    # single-field editor and writes the Connection it captured, which is
    # what the label promises. The #36-review check that demanded its
    # absence was written when the chord only routed to the emit path —
    # that excuse died with the editor this ticket added.
    check("the rail advertises the chord that now edits",
          "Ctrl+E edit" in rail, repr(rail))
    check("the rail keeps the escape hatch and movement",
          "Esc cancel" in rail and "↑↓ navigate" in rail, repr(rail))
    d.send(ESC, 0.4)

    # ── run 5: `jakarta` — every printable reaches the filter ──────────
    #
    # `j` and `k` were bound to movement, carried over from the pick path,
    # which made a search containing either untypeable. The frame echoes the
    # query back in its no-match line, so the exact string on the glass is
    # the proof: one missing or reordered character and it does not match.
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(b"jakarta", 0.6)
    d.dump("AD. typed `jakarta` — j and k are filter text, not movement")
    joined = "\n".join(d.screen.frame_rows())
    check('the whole word reached the filter: No matches for "jakarta"',
          'No matches for "jakarta"' in joined, joined)
    check("typing it deleted nothing", sha() == h2)

    # ...and it backspaces out to the full list again, so the word really
    # was the only thing separating the user from their Connections.
    for _ in range(7):
        d.send(BACKSPACE, 0.12)
    d.dump("AE. backspaced the word away — the list is whole again")
    joined = "\n".join(d.screen.frame_rows())
    check("the list came back whole after clearing the filter",
          "10.0.0.5" in joined and "No matches" not in joined, joined)
    check("clearing the filter wrote nothing", sha() == h2)
    d.send(ESC, 0.4)


def scenario_manage_error():
    """#36 review: a failed delete collapses the frame; it does not abandon it.

    The Connections file is made read-only, so `Store::remove` removes the
    Connection from the in-memory set and then fails to write it back. The
    frame must settle to `◆ error …` with the store's own reason, leave no
    rail or corner behind, and exit non-zero — not sit there painted with
    eleven rows and no trace, which is what this path used to do.
    """
    home = tempfile.mkdtemp(prefix="sshm-pty-fail-")
    os.makedirs(os.path.join(home, ".ssh"), exist_ok=True)
    cfg = os.path.join(home, ".ssh", "connections.json")
    with open(cfg, "w") as f:
        json.dump(
            {"connections": [
                {"id": "id-web-01", "alias": "web-01", "host": "10.0.0.4",
                 "user": "deploy", "port": 22, "folder": "prod"}
            ]},
            f, indent=2)
    os.chmod(cfg, 0o444)

    CTRL_X = b"\x18"
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.8)
    d.dump("AF. frame open over a Connections file that cannot be written")
    d.send(CTRL_X, 0.4)
    d.send(b"y", 1.0)
    d.dump("AG. y on a delete the store cannot persist")

    text = d.screen.text()
    errors = [r for r in text if r.startswith("◆ error")]
    check("the failed delete settles to the `◆ error` trace",
          len(errors) == 1, f"{errors}")
    if errors:
        check("the error trace names the Connection the action was about",
              "web-01" in errors[0], errors[0])
        check("the error trace carries the store's own reason",
              "denied" in errors[0].lower(), errors[0])
    below = "\n".join(text[PROMPT_ROW:])
    check("no live frame was abandoned: no rail below the prompt", "│" not in below,
          repr(below))
    check("no live frame was abandoned: no corner left behind", "└" not in below,
          repr(below))
    code = d.exit_code()
    check("the process exits non-zero so the failure is loud as well as visible",
          code not in (None, 0), f"exit code: {code}")
    check("no alternate screen on the error path", not d.screen.alt_screen)

    os.chmod(cfg, 0o644)


def scenario_manage_add():
    """#41: the `Ctrl+A` form map, driven on the real binary.

    Same throwaway-HOME discipline as the delete scenario: every verdict
    about persistence is a sha256 of the real `connections.json`, not a
    claim. The chord byte is 0x01 — what a terminal sends for Ctrl+A.

    The map replaces the old stepped walk. All six fields sit on the glass
    at once, the cursor is a *row* rather than a step, and the only gate
    left in the whole form is the `▶` row. So the walk covers what that
    actually changed:

    * the map opens whole, with `◆` on exactly one row;
    * Enter on a required field **advances** — it no longer refuses,
      because the row's own `○`/`!` glyph already says what is wrong;
    * the one refusal left lives on the `▶` row and names every field
      that is wrong, in map order;
    * the typed **User reaches `connections.json`** — the bug this map
      exists to fix, proven against the file rather than asserted;
    * nothing is written until `▶` is accepted, so Esc and Ctrl+C leave
      the file byte-identical.

    Colour is switched on for this scenario (`TERM=xterm-256color`,
    `NO_COLOR` cleared). The harness otherwise runs monochrome, and
    "`▶` is lit" versus "`▶` is dim" is carried by colour and nothing
    else — under `NO_COLOR` a ready button and a dead one are the same
    bytes, and a check that cannot fail is not a check.
    """
    home = tempfile.mkdtemp(prefix="sshm-pty-add-")
    cfg = os.path.join(home, ".ssh", "connections.json")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)

    def seed():
        with open(cfg, "w") as f:
            json.dump(
                {
                    "connections": [
                        {"id": "id-web-01", "alias": "web-01", "host": "10.0.0.4",
                         "user": "deploy", "port": 22, "folder": "prod"},
                        {"id": "id-web-02", "alias": "web-02", "host": "10.0.0.5",
                         "user": "deploy", "port": 22, "folder": "prod"},
                    ]
                },
                f,
                indent=2,
            )

    def sha():
        with open(cfg, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()

    def on_disk():
        with open(cfg) as f:
            return json.load(f)["connections"]

    CTRL_A, CTRL_C = b"\x01", b"\x03"
    COLOUR = {"TERM": "xterm-256color", "NO_COLOR": ""}

    # ── map readers: every claim below is read off the glass ────────────
    def fields(d):
        """The six field rows of the map, as painted."""
        return d.screen.text()[FRAME_ROW + 1:FRAME_ROW + 7]

    def labels(d):
        return [r[6:12].strip() for r in fields(d)]

    def glyphs(d):
        return [r[4] if len(r) > 4 else "" for r in fields(d)]

    def field_row(d, name):
        for r in fields(d):
            if r[6:12].strip() == name:
                return r
        return None

    def field_glyph(d, name):
        r = field_row(d, name)
        return r[4] if r and len(r) > 4 else None

    def field_value(d, name):
        """The row's text, with the focused row's caret taken off.

        The frame paints `_` after the value on the row the cursor is on.
        The caret is chrome, not content, so a check that wants the value
        asks for it stripped; a check that wants the caret asks for it by
        name. Both are read off the same row.
        """
        r = field_row(d, name)
        if not r or len(r) <= 13:
            return None
        v = r[13:]
        if r[4] == "◆" and v.endswith("_"):
            v = v[:-1]
        return v

    def caret(d, name):
        """Does the focused row show its caret?"""
        r = field_row(d, name)
        return bool(r) and r[4] == "◆" and r.endswith("_")

    def rule_row(d):
        return d.screen.text()[FRAME_ROW + RULE_OFFSET]

    def submit_row(d):
        return d.screen.text()[FRAME_ROW + SUBMIT_OFFSET]

    def submit_sgr(d):
        row = FRAME_ROW + SUBMIT_OFFSET
        for c, ch in enumerate(d.screen.buf[row]):
            if ch == "▶":
                return d.screen.style[row][c] or ""
        return None

    def submit_lit(d):
        """Is `▶` painted in the accent rather than the muted tier?

        `fg_muted` is `38;5;245`; the accent is `36`. A ready submit row
        carries the accent, a not-ready one never does.
        """
        return "36" in (submit_sgr(d) or "")

    def open_map():
        d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
        d.pump(0.7)
        d.send(CTRL_A, 0.4)
        return d

    def check_shape(d, where):
        rows = fields(d)
        check(f"{where}: all six field rows are on the glass at once",
              len(rows) == 6 and labels(d) == FIELD_ORDER,
              f"{len(rows)} rows, labels {labels(d)}")
        g = glyphs(d)
        check(f"{where}: `◆` sits on exactly one row",
              g.count("◆") == 1, f"glyphs {g}")

    # ── run 1: the map opens whole and Enter advances ─────────────────
    seed()
    h0 = sha()
    print(f"    seeded connections.json sha256: {h0}")

    d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
    d.pump(0.7)
    d.dump("CA. manage frame open — Ctrl+A on the rail")
    height = len(d.screen.frame_rows())

    d.send(CTRL_A, 0.4)
    d.dump("CB. Ctrl+A — the whole map opens, ◆ on Alias")
    check("the header names the ask",
          d.screen.text()[FRAME_ROW] == "◆ New connection",
          repr(d.screen.text()[FRAME_ROW]))
    check_shape(d, "the map at open")
    check("the cursor opens on Alias", field_glyph(d, "Alias") == "◆",
          f"Alias row: {field_row(d, 'Alias')!r}")
    check("a required field that is still empty says `<required>`, not `<optional>`",
          field_value(d, "Alias") == "<required>"
          and field_value(d, "Host") == "<required>",
          f"Alias {field_value(d, 'Alias')!r} Host {field_value(d, 'Host')!r}")
    check("an optional empty field says `<optional>`",
          field_value(d, "Key") == "<optional>"
          and field_value(d, "Folder") == "<optional>",
          f"Key {field_value(d, 'Key')!r} Folder {field_value(d, 'Folder')!r}")
    check("an empty required row wears `○`, an empty optional one `·`",
          field_glyph(d, "Host") == "○" and field_glyph(d, "User") == "·",
          f"Host {field_glyph(d, 'Host')!r} User {field_glyph(d, 'User')!r}")
    check("with nothing filled the `▶` row is dim, not lit",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the rail names what Enter means on a field row",
          "Esc back" in rail and "Enter next" in rail and "Ctrl+C quit" in rail,
          repr(rail))
    check("the map did not grow the frame",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")

    d.send(ENTER, 0.4)
    d.dump("CC. Enter on an empty required Alias — it ADVANCES, it does not refuse")
    joined = "\n".join(d.screen.frame_rows())
    check("Enter moved the cursor off Alias onto Host",
          field_glyph(d, "Host") == "◆" and field_glyph(d, "Alias") == "○",
          f"Alias {field_glyph(d, 'Alias')!r} Host {field_glyph(d, 'Host')!r}")
    check("advancing is not a refusal: no `!` line was raised",
          not rule_row(d).startswith("│   !"), repr(rule_row(d)))
    check("the left-behind Alias still reads as blocked (`○`)",
          field_glyph(d, "Alias") == "○", repr(field_row(d, "Alias")))
    check("the `▶` row is still dim — nothing became ready",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    check("moving off an empty required field wrote nothing", sha() == h0)

    d.send(b"db-01", 0.3)
    d.dump("CD. typing lands on the focused row, not the filter behind it")
    check("the focused row echoes what is typed",
          field_value(d, "Host") == "db-01" and caret(d, "Host"),
          repr(field_row(d, "Host")))
    check("the keystrokes did not filter the list",
          "No matches" not in "\n".join(d.screen.frame_rows()),
          "the Connections behind the map are still listed")
    d.send(UP, 0.3)
    check("Up walks the cursor back to Alias", field_glyph(d, "Alias") == "◆",
          repr(field_row(d, "Alias")))
    d.send(DOWN, 0.3)
    check("Down walks it forward again", field_glyph(d, "Host") == "◆",
          repr(field_row(d, "Host")))

    # Fill Alias, Host, User and Port by advancing with Enter, the way the
    # map is meant to be used: type, Enter, type, Enter. The Host row is
    # cleared first — it is still holding the `db-01` typed above, and a
    # fill that appends to a leftover is not a fill.
    for _ in range(5):
        d.send(BACKSPACE, 0.1)   # clear the Host row back to empty
    d.send(UP, 0.25)             # Host -> Alias
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.25)          # Alias -> Host
    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.25)          # Host -> User
    d.send(b"ops", 0.3)
    d.send(ENTER, 0.25)          # User -> Port
    d.send(b"2222", 0.3)
    d.dump("CE. Alias, Host, User and Port filled by advancing with Enter")
    check("Alias settled valid", field_glyph(d, "Alias") == "✓",
          repr(field_row(d, "Alias")))
    check("Host settled valid", field_glyph(d, "Host") == "✓",
          repr(field_row(d, "Host")))
    check("the Host holds the host, not a mash of the earlier typing",
          field_value(d, "Host") == "10.1.1.7", repr(field_row(d, "Host")))
    check("the typed User is on the glass", field_value(d, "User") == "ops",
          repr(field_row(d, "User")))
    check("with the required fields filled the `▶` row is lit",
          submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    check("filling the form wrote nothing — `▶` is the only write",
          sha() == h0)

    d.send(ENTER, 0.25)          # Port -> Key
    d.send(ENTER, 0.25)          # Key -> Folder
    d.send(b"staging", 0.3)      # the Folder value
    d.send(ENTER, 0.25)          # Folder -> Submit
    d.dump("CF. cursor on the `▶` row")
    check("the cursor reached the `▶` row",
          submit_row(d).startswith("│   ▶ Add connection"), repr(submit_row(d)))
    check("the `▶` row is lit and bold once the form is ready",
          submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the rail switches to what Enter means on the `▶` row",
          "Enter save" in rail and "Enter next" not in rail, repr(rail))

    d.send(ENTER, 0.6)
    d.dump("CG. Enter on `▶` — one Connection written, the user landed")
    joined = "\n".join(d.screen.frame_rows())
    check("the frame is back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the dim note names the added Connection with its folder",
          "│   ◇ added [staging] db-01" in joined, joined)
    check("the new Connection is on the glass with the typed user",
          "[staging] db-01 (ops@10.1.1.7:2222)" in joined, joined)
    check("the whole map kept the frame at its constant height",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    h1 = sha()
    check("the completed add changed connections.json", h1 != h0, f"{h0} -> {h1}")
    saved = on_disk()
    added = [c for c in saved if c["alias"] == "db-01"]
    check("exactly one Connection was added", len(saved) == 3 and len(added) == 1,
          f"{len(saved)} connections, {len(added)} named db-01")
    check("THE FIX: the typed User is in connections.json",
          bool(added) and added[0].get("user") == "ops",
          json.dumps(added[0] if added else {}))
    check("the settled Port landed as typed",
          bool(added) and added[0].get("port") == 2222,
          json.dumps(added[0] if added else {}))
    check("the Folder landed",
          bool(added) and added[0].get("folder") == "staging",
          json.dumps(added[0] if added else {}))
    check("the empty Key stayed absent — no empty key_path was written",
          '"key_path": ""' not in open(cfg).read(), open(cfg).read())
    d.send(ESC, 0.4)

    # ── run 1b: the `▶` row focused while the form is still invalid ───
    # Fresh baseline. Run 1 legitimately wrote a Connection, so `h0` is
    # stale by here; this run asks only that *its own* refused `▶` added
    # nothing, which is a comparison against the file as run 1 left it.
    h1b = sha()
    d = open_map()
    for _ in range(6):
        d.send(DOWN, 0.2)        # Down saturates on the `▶` row
    d.dump("CF2. cursor walked onto `▶` with the form still empty")
    check("Down saturated the cursor onto the `▶` row",
          "Enter save" in d.screen.text()[FRAME_ROW + height - 2],
          repr(d.screen.text()[FRAME_ROW + height - 2]))
    check("focused does not mean ready: `▶` carries no accent",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    check("and no field row is focused while `▶` is",
          "◆" not in "".join(glyphs(d)), f"glyphs {glyphs(d)}")
    d.send(ENTER, 0.5)
    d.dump("CF3. Enter on `▶` with every field empty — refused, naming both required fields")
    check("the refusal names every field that is wrong, in map order",
          rule_row(d) == "│   ! alias is required \u00b7 host is required",
          repr(rule_row(d)))
    check("the refusal kept the map at eight rows",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    check("an empty-form refusal wrote nothing", sha() == h1b)
    d.send(ESC, 0.4)

    # ── run 2: an invalid Port advances, and only `▶` stops the write ──
    seed()
    h2 = sha()
    d = open_map()
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.25)          # -> Host
    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.25)          # -> User
    d.send(b"ops", 0.3)
    d.send(ENTER, 0.25)          # -> Port
    d.send(b"abc", 0.3)
    d.send(ENTER, 0.4)           # Port -> Key, even though Port is garbage
    d.dump("CG. an invalid Port ADVANCES; the row wears `!` and `▶` stays dim")
    check("Enter on the garbage Port moved off it",
          field_glyph(d, "Key") == "◆", repr(field_row(d, "Key")))
    check("the invalid Port row wears `!`",
          field_glyph(d, "Port") == "!", repr(field_row(d, "Port")))
    check("the bad value is kept on the row for fixing",
          field_value(d, "Port") == "abc", repr(field_row(d, "Port")))
    check("with a bad Port the `▶` row stays dim",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    check("an invalid Port wrote nothing", sha() == h2)

    d.send(ENTER, 0.25)          # Key -> Folder
    d.send(b"staging", 0.3)
    d.send(ENTER, 0.25)          # Folder -> Submit
    d.dump("CH. cursor on `▶` with the bad Port still in the form")
    check("the `▶` row is still dim with the Port invalid",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    d.send(ENTER, 0.5)
    d.dump("CI. Enter on `▶` while invalid — refused, naming the field")
    joined = "\n".join(d.screen.frame_rows())
    check("the refusal states the range and names the field",
          rule_row(d) == "│   ! port must be 1–65535 (e.g. 22)",
          repr(rule_row(d)))
    check("the refusal took the rule row, so the map is still eight rows",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    check("the frame stayed on the form — it did not fall back to the list",
          d.screen.text()[FRAME_ROW] == "◆ New connection",
          repr(d.screen.text()[FRAME_ROW]))
    check("a refused `▶` wrote nothing", sha() == h2)

    d.send(UP, 0.25)
    d.send(UP, 0.25)
    d.send(UP, 0.25)             # Submit -> Folder -> Key -> Port
    d.dump("CJ. walked back to the Port to fix it")
    check("the cursor is back on the Port", field_glyph(d, "Port") == "◆",
          repr(field_row(d, "Port")))
    for _ in range(3):
        d.send(BACKSPACE, 0.12)
    d.dump("CK. the first Backspace stops reporting the old mistake")
    check("the refusal is gone once the user is fixing it",
          not rule_row(d).startswith("│   !"), repr(rule_row(d)))
    d.send(b"2222", 0.3)
    d.dump("CL. a valid Port typed — the `▶` row lights again")
    check("the Port row holds the fixed value",
          field_value(d, "Port") == "2222" and caret(d, "Port"),
          repr(field_row(d, "Port")))
    check("with the form valid the `▶` row is lit",
          submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    d.send(DOWN, 0.25)           # Port -> Key
    # The focused row always wears `◆`, so the Port's own glyph is only
    # readable once the cursor has left it.
    check("the fixed Port row reports `✓` once the cursor leaves it",
          field_glyph(d, "Port") == "✓", repr(field_row(d, "Port")))
    d.send(DOWN, 0.25)           # Key -> Folder
    d.send(DOWN, 0.25)           # Folder -> Submit
    d.send(ENTER, 0.6)
    d.dump("CM. Enter on `▶` now — the write happens")
    joined = "\n".join(d.screen.frame_rows())
    h3 = sha()
    check("the fixed form changed connections.json", h3 != h2, f"{h2} -> {h3}")
    saved = [c for c in on_disk() if c["alias"] == "db-01"]
    check("the fixed Port landed as 2222",
          bool(saved) and saved[0].get("port") == 2222,
          json.dumps(saved[0] if saved else {}))
    check("the typed User landed too",
          bool(saved) and saved[0].get("user") == "ops",
          json.dumps(saved[0] if saved else {}))
    check("the note names the added Connection",
          "│   ◇ added [staging] db-01" in joined, joined)
    d.send(ESC, 0.4)

    # ── run 3: Esc off the top row abandons; the file never hears ─────
    seed()
    h4 = sha()
    d = open_map()
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"ops", 0.3)
    d.dump("CN. half-filled map, cursor on Port")
    check("three rows are filled on the glass",
          field_value(d, "Alias") == "db-01"
          and field_value(d, "Host") == "10.1.1.7"
          and field_value(d, "User") == "ops",
          f"{[field_value(d, n) for n in ('Alias', 'Host', 'User')]}")
    check("a half-filled map has written nothing", sha() == h4)
    d.send(UP, 0.2)
    d.send(UP, 0.2)
    d.send(UP, 0.2)              # Port -> User -> Host -> Alias
    check("the cursor is on the top row", field_glyph(d, "Alias") == "◆",
          repr(field_row(d, "Alias")))
    d.send(ESC, 0.5)
    d.dump("CO. Esc off the top row abandons the map — nothing saved")
    joined = "\n".join(d.screen.frame_rows())
    check("the frame is back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the note answers the only question the user has",
          "│   ◇ add abandoned — nothing saved" in joined, joined)
    check("no ◇ added note — the write never happened",
          "added" not in joined, joined)
    h5 = sha()
    check("abandoning the map left connections.json byte-identical",
          h5 == h4, f"{h4} -> {h5}")

    # ── run 4: Ctrl+C mid-form cancels and writes nothing ─────────────
    d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
    d.pump(0.7)
    d.send(CTRL_A, 0.4)
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"half-typed", 0.3)
    d.send(CTRL_C, 0.5)
    d.dump("CP. Ctrl+C mid-form — the frame cancels clean")
    check("Ctrl+C leaves the cancel trace",
          d.screen.text()[FRAME_ROW] == "◆ cancelled",
          repr(d.screen.text()[FRAME_ROW]))
    check("Ctrl+C mid-form wrote nothing", sha() == h4)
    check("no alternate screen on any add path", not d.screen.alt_screen)

    # ── run 5: adding under a filter must still SHOW the new row ──────
    seed()
    h6 = sha()
    d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
    d.pump(0.7)
    d.send(b"web", 0.4)
    d.dump("CQ. filtered to `web` — two rows match, the new one would not")
    joined = "\n".join(d.screen.frame_rows())
    check("the filter is live and shows the web rows",
          "web-01" in joined and "db-01" not in joined, joined)
    d.send(CTRL_A, 0.4)
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"ops", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"2222", 0.3)
    d.send(ENTER, 0.25)
    d.send(ENTER, 0.25)
    d.send(b"staging", 0.3)
    d.send(ENTER, 0.25)          # Folder -> Submit
    d.dump("CR. the map filled under a `web` filter, cursor on `▶`")
    check("the map covers the filtered list — the filter is not visible",
          "No matches" not in "\n".join(d.screen.frame_rows())
          and "web-01" not in "\n".join(fields(d)),
          "\n".join(d.screen.frame_rows()))
    d.send(ENTER, 0.6)
    d.dump("CS. added db-01 under a `web` filter — the filter clears")
    joined = "\n".join(d.screen.frame_rows())
    check("the frame is back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the note names the added Connection",
          "│   ◇ added [staging] db-01" in joined, joined)
    check("the new Connection is actually on the glass — the filter cleared",
          "[staging] db-01 (ops@10.1.1.7:2222)" in joined, joined)
    check("the cursor is on the new row, not stranded where the filter left it",
          "❯ [staging] db-01" in joined, joined)
    check("the add really landed on disk", sha() != h6)
    saved = [c for c in on_disk() if c["alias"] == "db-01"]
    check("and the user typed under a filter is on disk too",
          bool(saved) and saved[0].get("user") == "ops",
          json.dumps(saved[0] if saved else {}))
    d.send(ESC, 0.4)


def scenario_manage_edit():
    """#41: the `Ctrl+E` form map, seeded from a live Connection.

    Same throwaway-HOME discipline as the add and delete scenarios: every
    verdict about persistence is a sha256 of the real
    `connections.json`, not a claim. The chord byte is 0x05 — what a
    terminal sends for Ctrl+E.

    The edit map is the add map with a baseline under it, and that one
    difference is what the walk has to prove:

    * it opens **seeded** — every row holds the stored value, and nothing
      is `●` yet, because nothing has been changed yet;
    * a row the user actually changes turns `●`, and only that row;
    * Enter on `▶` performs **exactly one write** carrying **both**
      changes, against the Connection the chord captured;
    * Enter on `▶` with nothing changed refuses with `nothing to save`
      rather than silently rewriting the file;
    * Esc off the top row and Ctrl+C both leave the file byte-identical.

    Colour is switched on for the same reason the add scenario needs it:
    `●`-against-`✓` and a lit-against-dim `▶` are the whole of the
    signal, and under `NO_COLOR` they are invisible.
    """
    home = tempfile.mkdtemp(prefix="sshm-pty-edit-")
    cfg = os.path.join(home, ".ssh", "connections.json")
    os.makedirs(os.path.dirname(cfg), exist_ok=True)

    def seed():
        with open(cfg, "w") as f:
            json.dump(
                {
                    "connections": [
                        {"id": "id-web-01", "alias": "web-01", "host": "10.0.0.4",
                         "user": "deploy", "port": 22, "folder": "prod"},
                        {"id": "id-web-02", "alias": "web-02", "host": "10.0.0.5",
                         "user": "deploy", "port": 22, "folder": "prod"},
                    ]
                },
                f,
                indent=2,
            )

    def sha():
        with open(cfg, "rb") as f:
            return hashlib.sha256(f.read()).hexdigest()

    def on_disk():
        with open(cfg) as f:
            return json.load(f)["connections"]

    CTRL_E, CTRL_C = b"\x05", b"\x03"
    COLOUR = {"TERM": "xterm-256color", "NO_COLOR": ""}

    def fields(d):
        return d.screen.text()[FRAME_ROW + 1:FRAME_ROW + 7]

    def labels(d):
        return [r[6:12].strip() for r in fields(d)]

    def glyphs(d):
        return [r[4] if len(r) > 4 else "" for r in fields(d)]

    def field_row(d, name):
        for r in fields(d):
            if r[6:12].strip() == name:
                return r
        return None

    def field_glyph(d, name):
        r = field_row(d, name)
        return r[4] if r and len(r) > 4 else None

    def field_value(d, name):
        """The row's text, with the focused row's caret taken off.

        The frame paints `_` after the value on the row the cursor is on.
        The caret is chrome, not content, so a check that wants the value
        asks for it stripped; a check that wants the caret asks for it by
        name. Both are read off the same row.
        """
        r = field_row(d, name)
        if not r or len(r) <= 13:
            return None
        v = r[13:]
        if r[4] == "◆" and v.endswith("_"):
            v = v[:-1]
        return v

    def caret(d, name):
        """Does the focused row show its caret?"""
        r = field_row(d, name)
        return bool(r) and r[4] == "◆" and r.endswith("_")

    def rule_row(d):
        return d.screen.text()[FRAME_ROW + RULE_OFFSET]

    def submit_row(d):
        return d.screen.text()[FRAME_ROW + SUBMIT_OFFSET]

    def submit_sgr(d):
        row = FRAME_ROW + SUBMIT_OFFSET
        for c, ch in enumerate(d.screen.buf[row]):
            if ch == "▶":
                return d.screen.style[row][c] or ""
        return None

    def submit_lit(d):
        return "36" in (submit_sgr(d) or "")

    def open_edit():
        d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
        d.pump(0.7)
        d.send(CTRL_E, 0.4)
        return d

    # ── run 1: the map opens seeded, and nothing is `●` ───────────────
    seed()
    h0 = sha()
    print(f"    seeded connections.json sha256: {h0}")

    d = Demo(["manage"], bin=SSH_BIN, home=home, env=COLOUR)
    d.pump(0.7)
    d.dump("EA. manage frame open — Ctrl+E on the rail")
    height = len(d.screen.frame_rows())
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("Ctrl+E is on the rail because it now does what it says",
          "Ctrl+E edit" in rail, repr(rail))

    d.send(CTRL_E, 0.4)
    d.dump("EB. Ctrl+E — the map opens seeded from the selected Connection")
    check("the header names the Connection being edited",
          d.screen.text()[FRAME_ROW] == "◆ Edit [prod] web-01",
          repr(d.screen.text()[FRAME_ROW]))
    check("all six field rows are on the glass at once",
          len(fields(d)) == 6 and labels(d) == FIELD_ORDER,
          f"{len(fields(d))} rows, labels {labels(d)}")
    check("the cursor opens on Alias", field_glyph(d, "Alias") == "◆",
          repr(field_row(d, "Alias")))
    check("every row arrived holding the stored value",
          field_value(d, "Alias") == "web-01"
          and field_value(d, "Host") == "10.0.0.4"
          and field_value(d, "User") == "deploy"
          and field_value(d, "Port") == "22"
          and field_value(d, "Folder") == "prod",
          f"{[field_value(d, n) for n in FIELD_ORDER]}")
    check("the seeded value arrives on the line, caret and all",
          caret(d, "Alias"), repr(field_row(d, "Alias")))
    check("NOTHING shows as `●` — nothing has been changed yet",
          "●" not in "".join(glyphs(d)), f"glyphs {glyphs(d)}")
    check("the one field the store does not hold reads `<not set>`, not `<optional>`",
          field_value(d, "Key") == "<not set>", repr(field_row(d, "Key")))
    check("the `▶` row is dim — there is nothing to save yet",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    check("the `▶` row names the Connection the write is about",
          submit_row(d) == "│   ▶ Save changes to web-01", repr(submit_row(d)))
    check("the edit map did not grow the frame",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    check("opening the editor wrote nothing", sha() == h0)

    d.send(b"x", 0.3)
    d.dump("EC. typing turns exactly one row `●`")
    check("the focused row echoes the keystroke",
          field_value(d, "Alias") == "web-01x" and caret(d, "Alias"),
          repr(field_row(d, "Alias")))
    check("with a change pending the `▶` row is lit",
          submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    d.send(DOWN, 0.25)           # Alias -> Host, so Alias can show its own glyph
    check("the changed row reports `●` once the cursor leaves it",
          field_glyph(d, "Alias") == "●", repr(field_row(d, "Alias")))
    check("and only that row — the untouched rows keep `✓`",
          [g for n, g in (("Host", field_glyph(d, "Host")),
                         ("User", field_glyph(d, "User")),
                         ("Folder", field_glyph(d, "Folder"))) if g == "●"] == [],
          f"glyphs {glyphs(d)}")
    check("the keystroke did not filter the list",
          "No matches" not in "\n".join(d.screen.frame_rows()),
          "the Connections behind the map are still listed")
    check("typing still wrote nothing — `▶` is the write", sha() == h0)

    # ── run 2: two changes, one write ─────────────────────────────────
    seed()
    h1 = sha()
    d = open_edit()
    d.send(b"-live", 0.3)        # Alias -> web-01-live
    d.send(ENTER, 0.25)         # -> Host
    d.send(ENTER, 0.25)         # -> User
    d.send(ENTER, 0.25)         # -> Port
    for _ in range(2):
        d.send(BACKSPACE, 0.12)  # clear the stored "22"
    d.send(b"2222", 0.3)        # Port -> 2222
    d.dump("ED. two rows changed — Alias and Port")
    check("the Port row holds the new port with the caret on it",
          field_value(d, "Port") == "2222" and caret(d, "Port"),
          repr(field_row(d, "Port")))
    check("the Alias row still reports its own change",
          field_glyph(d, "Alias") == "●", repr(field_row(d, "Alias")))
    d.send(DOWN, 0.25)          # Port -> Key, so the Port can show its own glyph
    d.dump("ED2. cursor one row below — both changed rows now readable")
    check("the changed Alias wears `●`", field_glyph(d, "Alias") == "●",
          repr(field_row(d, "Alias")))
    check("the changed Port wears `●`", field_glyph(d, "Port") == "●",
          repr(field_row(d, "Port")))
    check("the rows nobody touched are NOT `●`",
          field_glyph(d, "Host") == "✓" and field_glyph(d, "User") == "✓"
          and field_glyph(d, "Folder") == "✓",
          f"glyphs {glyphs(d)}")
    check("exactly two rows report a change",
          glyphs(d).count("●") == 2, f"glyphs {glyphs(d)}")
    check("two pending changes have still written nothing", sha() == h1)

    d.send(ENTER, 0.25)         # Key -> Folder
    d.send(ENTER, 0.25)         # Folder -> Submit
    d.dump("EE. cursor on `▶` with both changes pending")
    check("the `▶` row is lit", submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the rail says `Enter save` on the `▶` row",
          "Enter save" in rail and "Enter next" not in rail, repr(rail))
    d.send(ENTER, 0.6)
    d.dump("EF. Enter on `▶` — ONE write carrying BOTH changes")
    joined = "\n".join(d.screen.frame_rows())
    h2 = sha()
    check("the edit changed connections.json", h2 != h1, f"{h1} -> {h2}")
    saved = on_disk()
    check("exactly one write: the file still holds two Connections",
          len(saved) == 2, f"{len(saved)} connections")
    check("both changes are in the file",
          saved[0]["alias"] == "web-01-live" and saved[0]["port"] == 2222,
          json.dumps(saved[0]))
    check("the fields nobody changed came through untouched",
          saved[0]["host"] == "10.0.0.4" and saved[0]["user"] == "deploy"
          and saved[0]["folder"] == "prod",
          json.dumps(saved[0]))
    check("the id survived the edit — no orphaned row",
          saved[0]["id"] == "id-web-01", json.dumps(saved[0]))
    check("the neighbour is untouched",
          saved[1]["alias"] == "web-02" and saved[1]["port"] == 22,
          json.dumps(saved[1]))
    check("the note names the Connection the store actually wrote",
          "│   ◇ edited [prod] web-01-live" in joined, joined)
    check("the list is refreshed and shows the changed Connection",
          "[prod] web-01-live (deploy@10.0.0.4:2222)" in joined, joined)
    d.send(ESC, 0.4)

    # ── run 3: Enter on `▶` with nothing changed refuses ──────────────
    seed()
    h3 = sha()
    d = open_edit()
    for _ in range(6):
        d.send(ENTER, 0.2)      # walk Alias -> Submit without touching a row
    d.dump("EG. walked to `▶` with nothing changed")
    check("no row wears `●` after a walk that changed nothing",
          "●" not in "".join(glyphs(d)), f"glyphs {glyphs(d)}")
    check("the `▶` row stays dim with nothing changed",
          not submit_lit(d), f"▶ sgr {submit_sgr(d)!r}")
    d.send(ENTER, 0.5)
    d.dump("EH. Enter on `▶` with nothing changed — `nothing to save`")
    check("the refusal says `nothing to save`",
          rule_row(d) == "│   ! nothing to save", repr(rule_row(d)))
    check("the frame stayed on the edit map",
          d.screen.text()[FRAME_ROW] == "◆ Edit [prod] web-01",
          repr(d.screen.text()[FRAME_ROW]))
    check("the refusal took the rule row, so the height never moved",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    check("a `nothing to save` refusal wrote nothing", sha() == h3)
    d.send(ESC, 0.4)

    # ── run 4: movement never moves the target onto a neighbour ───────
    #
    # The cursor is walked the length of the map, wrapped round with Tab,
    # and parked on the Port — with the cursor's position asserted at every
    # step. Without those step checks a miscount would quietly write the
    # Port's digits into whichever row the cursor had drifted onto, and the
    # scenario would still "pass" while proving nothing about the target.
    seed()
    h4 = sha()
    d = open_edit()
    check("the edit opens with the cursor on Alias",
          field_glyph(d, "Alias") == "◆", f"glyphs {glyphs(d)}")
    for _ in range(7):
        d.send(DOWN, 0.15)      # Down saturates at Submit; the 7th changes nothing
    d.dump("EI. Down walked the map and saturated at the `▶` row")
    check("Down saturated on the `▶` row, not past it",
          "Enter save" in d.screen.text()[FRAME_ROW + height - 2],
          repr(d.screen.text()[FRAME_ROW + height - 2]))
    check("no field row carries the cursor once it is on `▶`",
          "◆" not in "".join(glyphs(d)), f"glyphs {glyphs(d)}")
    check("the header still names the Connection captured at the chord",
          d.screen.text()[FRAME_ROW] == "◆ Edit [prod] web-01",
          repr(d.screen.text()[FRAME_ROW]))

    d.send(TAB, 0.2)            # Submit wraps to Alias
    check("Tab wrapped the cursor from `▶` back to Alias",
          field_glyph(d, "Alias") == "◆", f"glyphs {glyphs(d)}")
    d.send(TAB, 0.2)            # -> Host
    check("Tab landed on Host", field_glyph(d, "Host") == "◆",
          f"glyphs {glyphs(d)}")
    d.send(TAB, 0.2)            # -> User
    check("Tab landed on User", field_glyph(d, "User") == "◆",
          f"glyphs {glyphs(d)}")
    d.send(TAB, 0.2)            # -> Port
    d.dump("EJ. Tab walked down to the Port row — still editing web-01")
    check("Tab landed on Port", field_glyph(d, "Port") == "◆",
          repr(field_row(d, "Port")))
    check("the header still names web-01 after the wrap",
          d.screen.text()[FRAME_ROW] == "◆ Edit [prod] web-01",
          repr(d.screen.text()[FRAME_ROW]))

    for _ in range(2):
        d.send(BACKSPACE, 0.12)  # clear the stored "22"
    d.send(b"2200", 0.3)
    check("the Port row took the new port",
          field_value(d, "Port") == "2200" and caret(d, "Port"),
          repr(field_row(d, "Port")))
    d.send(ENTER, 0.25)         # Port -> Key
    check("the cursor left the Port, which now reports `●`",
          field_glyph(d, "Port") == "●" and field_glyph(d, "Key") == "◆",
          f"glyphs {glyphs(d)}")
    d.send(ENTER, 0.25)         # Key -> Folder
    d.send(ENTER, 0.25)         # Folder -> Submit
    check("the cursor is back on the `▶` row",
          "Enter save" in d.screen.text()[FRAME_ROW + height - 2],
          repr(d.screen.text()[FRAME_ROW + height - 2]))
    d.send(ENTER, 0.6)
    d.dump("EK. the write landed on the captured Connection, not a neighbour")
    saved = on_disk()
    check("the captured Connection took the change",
          saved[0]["id"] == "id-web-01" and saved[0]["port"] == 2200,
          json.dumps(saved[0]))
    check("no stray value landed in another field of the target",
          saved[0]["alias"] == "web-01" and saved[0]["host"] == "10.0.0.4"
          and saved[0]["user"] == "deploy" and saved[0]["folder"] == "prod",
          json.dumps(saved[0]))
    check("the neighbour kept its own port",
          saved[1]["alias"] == "web-02" and saved[1]["port"] == 22,
          json.dumps(saved[1]))
    check("and only the two Connections we started with exist",
          len(saved) == 2, f"{len(saved)} connections")
    d.send(ESC, 0.4)

    # ── run 5: Esc off the top row after edits abandons ───────────────
    seed()
    h5 = sha()
    d = open_edit()
    d.send(b"ZZZ", 0.2)
    d.send(ENTER, 0.25)
    d.send(b"notstored", 0.3)
    d.dump("EL. two rows edited, cursor on Host")
    check("both edited rows report `●`",
          field_glyph(d, "Alias") == "●" and field_glyph(d, "Host") == "◆",
          f"glyphs {glyphs(d)}")
    check("edited-but-unsubmitted rows have written nothing", sha() == h5)
    d.send(UP, 0.2)
    check("Up walks back to the top row", field_glyph(d, "Alias") == "◆",
          repr(field_row(d, "Alias")))
    d.send(ESC, 0.5)
    d.dump("EM. Esc off the top row — the edit is abandoned")
    joined = "\n".join(d.screen.frame_rows())
    check("back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the note answers whether the half-typed fields were saved",
          "│   ◇ edit abandoned — nothing saved" in joined, joined)
    check("no ◇ edited note — the write never happened",
          "edited" not in joined, joined)
    check("abandoning left connections.json byte-identical", sha() == h5,
          f"{h5} -> {sha()}")

    # ── run 6: Ctrl+C inside the map cancels and writes nothing ───────
    seed()
    h6 = sha()
    d = open_edit()
    d.send(b"half-typed", 0.3)
    d.send(ENTER, 0.25)
    d.send(b"and-more", 0.3)
    d.send(CTRL_C, 0.5)
    d.dump("EN. Ctrl+C inside the edit map — the frame cancels clean")
    check("Ctrl+C leaves the cancel trace",
          d.screen.text()[FRAME_ROW] == "◆ cancelled",
          repr(d.screen.text()[FRAME_ROW]))
    check("Ctrl+C inside the edit map wrote nothing", sha() == h6)
    check("no alternate screen on any edit path", not d.screen.alt_screen)


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    scenarios = {
        "pick": scenario_pick,
        "cancel": scenario_cancel,
        "resize": scenario_resize,
        "reflow": scenario_reflow,
        "wide": scenario_wide_chars,
        "short": scenario_short_terminal,
        "too-short": scenario_too_short_at_open,
        "manage": scenario_manage,
        "manage-error": scenario_manage_error,
        "manage-add": scenario_manage_add,
        "manage-edit": scenario_manage_edit,
    }
    if which == "all":
        for fn in scenarios.values():
            fn()
    else:
        scenarios[which]()

    failed = [label for label, ok in CHECKS if not ok]
    print(f"\n{'=' * 60}")
    print(f"checks: {len(CHECKS) - len(failed)}/{len(CHECKS)} passed")
    for label in failed:
        print(f"  FAILED: {label}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
