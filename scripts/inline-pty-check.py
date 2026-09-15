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
ENTER = b"\r"
ESC = b"\x1b"
BACKSPACE = b"\x7f"

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
        self.r = 0
        self.c = 0
        self.alt_screen = False

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
            pass
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
        w = cell_width(ch)
        if self.c < self.cols:
            self.buf[self.r][self.c] = ch
            # A double-width glyph owns the cell after it as well; blanking it
            # keeps the column arithmetic honest.
            for k in range(1, w):
                if self.c + k < self.cols:
                    self.buf[self.r][self.c + k] = ""
            self.c += w

    def _erase_line(self, mode):
        if self.r >= len(self.buf):
            return
        if mode == 2:
            self.buf[self.r] = [" "] * self.cols
        elif mode == 0:
            for c in range(self.c, self.cols):
                self.buf[self.r][c] = " "
        elif mode == 1:
            for c in range(0, min(self.c + 1, self.cols)):
                self.buf[self.r][c] = " "

    def _erase_display(self, mode):
        if mode == 2:
            self.buf = [[" "] * self.cols for _ in range(self.rows)]
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
    # `Ctrl+E edit` stays off the rail: it still leaves the frame exactly
    # as Enter does, and the in-place single-field editor is a follow-on
    # ticket, not this one. Hinting it would name an action that does not
    # happen.
    check("the rail still does not advertise Ctrl+E",
          "Ctrl+E" not in rail, repr(rail))
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
    """#37: the `Ctrl+A` add step-sequence on the real binary.

    Same throwaway-HOME discipline as the delete scenario: every verdict
    about persistence is a sha256 of the real `connections.json`, not a
    claim. The chord byte is 0x01 — what a terminal sends for Ctrl+A.

    The walk covers every way a step can answer: a required field refusing
    empty, a port refusing out-of-range, an optional field settling as
    *absent* rather than blank, the completed sequence earning its
    `◇ added` note from a write that really happened, and an abandoned
    sequence leaving the file byte-identical.
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

    CTRL_A, CTRL_C = b"\x01", b"\x03"

    # ── run 1: the full five-step walk, refusing where it should ──────
    seed()
    h0 = sha()
    print(f"    seeded connections.json sha256: {h0}")

    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.dump("CA. manage frame open — Ctrl+A on the rail")
    height = len(d.screen.frame_rows())

    d.send(CTRL_A, 0.4)
    d.dump("CB. Ctrl+A — the sequence opens on the Alias step")
    check("the first step is Alias, caret on the line",
          d.screen.text()[FRAME_ROW] == "◆ Alias  _",
          repr(d.screen.text()[FRAME_ROW]))
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the step rail names what the step reads",
          "Esc back" in rail and "Enter next" in rail and "Ctrl+C quit" in rail,
          repr(rail))
    check("the sequence did not grow the frame",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")

    d.send(ENTER, 0.4)
    d.dump("CC. Enter on an empty required Alias — refused, stays put")
    joined = "\n".join(d.screen.frame_rows())
    check("the refusal names the field",
          "│   ! alias is required" in joined, joined)
    check("still on the Alias step", d.screen.text()[FRAME_ROW] == "◆ Alias  _",
          repr(d.screen.text()[FRAME_ROW]))
    check("nothing settled behind the refusal", "◇ Alias" not in joined, joined)
    check("a refused required field wrote nothing", sha() == h0)

    d.send(b"db-01", 0.3)
    d.dump("CD. typing echoes into the live field, not the filter behind it")
    check("the field echoes what is typed",
          d.screen.text()[FRAME_ROW] == "◆ Alias  db-01_",
          repr(d.screen.text()[FRAME_ROW]))
    check("the keystrokes did not filter the list",
          "No matches" not in "\n".join(d.screen.frame_rows()),
          "the Connections behind the sequence are still listed")

    d.send(ENTER, 0.4)
    d.dump("CE. the Alias settles to ◇ and the header moves to Host")
    joined = "\n".join(d.screen.frame_rows())
    check("the settled Alias leaves a ◇ trace",
          "│   ◇ Alias   db-01" in joined, joined)
    check("the next step is Host", d.screen.text()[FRAME_ROW] == "◆ Host  _",
          repr(d.screen.text()[FRAME_ROW]))

    d.send(ENTER, 0.4)
    d.dump("CF. Enter on an empty required Host — refused, stays on Host")
    joined = "\n".join(d.screen.frame_rows())
    check("the refusal names the field", "│   ! host is required" in joined, joined)
    check("still on the Host step", d.screen.text()[FRAME_ROW] == "◆ Host  _",
          repr(d.screen.text()[FRAME_ROW]))

    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.4)
    d.dump("CG. the Host settles and the Port step opens")
    joined = "\n".join(d.screen.frame_rows())
    check("the settled Host leaves a ◇ trace",
          "│   ◇ Host    10.1.1.7" in joined, joined)
    check("the next step is Port", d.screen.text()[FRAME_ROW] == "◆ Port  _",
          repr(d.screen.text()[FRAME_ROW]))

    d.send(b"99999", 0.3)
    d.send(ENTER, 0.4)
    d.dump("CH. a port above the TCP range — refused, stays on Port")
    joined = "\n".join(d.screen.frame_rows())
    check("the refusal states the range",
          "│   ! port must be a number from 1 to 65535" in joined, joined)
    check("still on the Port step with the bad input kept for fixing",
          d.screen.text()[FRAME_ROW] == "◆ Port  99999_",
          repr(d.screen.text()[FRAME_ROW]))

    for _ in range(5):
        d.send(BACKSPACE, 0.1)
    d.send(b"2222", 0.3)
    d.send(ENTER, 0.4)
    d.dump("CI. a valid port settles; the Key step opens")
    joined = "\n".join(d.screen.frame_rows())
    check("the settled Port leaves a ◇ trace",
          "│   ◇ Port    2222" in joined, joined)
    check("the next step is Key", d.screen.text()[FRAME_ROW] == "◆ Key  _",
          repr(d.screen.text()[FRAME_ROW]))

    d.send(ENTER, 0.4)
    d.dump("CJ. the empty optional Key settles as absent, not blank")
    joined = "\n".join(d.screen.frame_rows())
    check("the absent Key is drawn as — not as nothing",
          "│   ◇ Key     —" in joined, joined)
    check("the last step is Folder", d.screen.text()[FRAME_ROW] == "◆ Folder  _",
          repr(d.screen.text()[FRAME_ROW]))
    rail = d.screen.text()[FRAME_ROW + height - 2]
    check("the last step's rail says what Enter means there",
          "Enter add" in rail and "Enter next" not in rail, repr(rail))

    d.send(b"staging", 0.3)
    d.send(ENTER, 0.5)
    d.dump("CK. Folder settles — the store wrote, the list returns with ◇ added")
    joined = "\n".join(d.screen.frame_rows())
    check("the frame is back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the dim note names the added Connection with its folder",
          "│   ◇ added [staging] db-01" in joined, joined)
    check("the new Connection is in the list",
          "[staging] db-01 (@10.1.1.7:2222)" in joined, joined)
    check("the whole sequence kept the frame at its constant height",
          len(d.screen.frame_rows()) == height,
          f"{height} -> {len(d.screen.frame_rows())}")
    h1 = sha()
    check("the completed add changed connections.json", h1 != h0, f"{h0} -> {h1}")
    on_disk = open(cfg).read()
    check("the added Connection is on disk with its settled fields",
          '"alias": "db-01"' in on_disk and '"port": 2222' in on_disk
          and '"folder": "staging"' in on_disk, on_disk)
    check("the empty Key stayed absent — no empty key_path was written",
          '"key_path": ""' not in on_disk, on_disk)
    d.send(ESC, 0.4)

    # ── run 2: walking all the way back abandons the sequence, and the
    # ── file never learns it happened ──────────────────────────────────
    seed()
    h0b = sha()
    print(f"    re-seeded connections.json sha256: {h0b}")

    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(CTRL_A, 0.4)
    d.send(b"db-01", 0.3)
    d.send(ENTER, 0.3)
    d.send(b"10.1.1.7", 0.3)
    d.send(ENTER, 0.4)
    d.dump("CL. mid-sequence at Port — two steps settled, nothing written yet")
    joined = "\n".join(d.screen.frame_rows())
    check("two steps are settled on the glass",
          "◇ Alias   db-01" in joined and "◇ Host    10.1.1.7" in joined, joined)
    check("two settled steps still wrote nothing", sha() == h0b)

    d.send(ESC, 0.3)
    d.dump("CM. Esc walks back to Host with its answer back on the line")
    check("Host is live again with its answer re-seeded",
          d.screen.text()[FRAME_ROW] == "◆ Host  10.1.1.7_",
          repr(d.screen.text()[FRAME_ROW]))
    d.send(ESC, 0.3)
    d.dump("CN. Esc walks back to Alias with its answer back on the line")
    check("Alias is live again with its answer re-seeded",
          d.screen.text()[FRAME_ROW] == "◆ Alias  db-01_",
          repr(d.screen.text()[FRAME_ROW]))
    d.send(ESC, 0.4)
    d.dump("CO. Esc off the first step abandons the sequence — nothing saved")
    joined = "\n".join(d.screen.frame_rows())
    check("the frame is back on the list",
          d.screen.text()[FRAME_ROW] == "◆ Manage Connections",
          repr(d.screen.text()[FRAME_ROW]))
    check("the note answers the only question the user has",
          "│   ◇ add abandoned — nothing saved" in joined, joined)
    check("no ◇ added note — the write never happened",
          "added" not in joined, joined)
    h2 = sha()
    check("abandoning mid-sequence left connections.json byte-identical",
          h2 == h0b, f"{h0b} -> {h2}")

    # ── run 3: Ctrl+C mid-sequence cancels the frame and writes nothing ──
    d = Demo(["manage"], bin=SSH_BIN, home=home)
    d.pump(0.7)
    d.send(CTRL_A, 0.4)
    d.send(b"half-typed", 0.3)
    d.send(CTRL_C, 0.5)
    d.dump("CP. Ctrl+C mid-sequence — the frame cancels clean")
    check("Ctrl+C leaves the cancel trace",
          d.screen.text()[FRAME_ROW] == "◆ cancelled",
          repr(d.screen.text()[FRAME_ROW]))
    check("Ctrl+C mid-sequence wrote nothing", sha() == h0b)
    check("no alternate screen on any add path", not d.screen.alt_screen)


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
