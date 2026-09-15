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
terminal.
"""

import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(REPO, "sshm", "target", "debug", "examples", "inline_demo")

SCROLLBACK = [
    "prior scrollback: cargo build --release",
    "prior scrollback: git log --oneline -3",
    "prior scrollback: ls demo docs",
]

DOWN = b"\x1b[B"
UP = b"\x1b[A"
ENTER = b"\r"
ESC = b"\x1b"
BACKSPACE = b"\x7f"


class Screen:
    """A minimal VT screen: rows x cols of characters, with a cursor."""

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
        if self.c < self.cols:
            self.buf[self.r][self.c] = ch
            self.c += 1

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

        Real terminals anchor the *cursor* across a vertical resize: on a
        shrink the viewport keeps the cursor visible and content above it
        scrolls out (into scrollback this model does not track); on a grow
        the cursor keeps its row and blank rows appear at the bottom.
        Horizontal resize truncates here; real terminals reflow, which is
        exactly why the driver's `widest` check cancels rather than trusting
        its row count across a narrowing.
        """
        s = Screen(rows, cols)
        if rows < len(self.buf):
            # Keep the window that contains the cursor, anchored at its bottom.
            keep_from = max(0, min(self.r - (rows - 1), len(self.buf) - rows))
            src_slice = self.buf[keep_from:]
            s.r = self.r - keep_from
        else:
            src_slice = self.buf
            s.r = self.r
        for i in range(min(rows, len(src_slice))):
            src = src_slice[i]
            for j in range(min(cols, len(src))):
                s.buf[i][j] = src[j]
        s.c = min(self.c, cols - 1)
        s.alt_screen = self.alt_screen
        return s

    def nonblank_height(self):
        rows = self.text()
        last = -1
        for i, r in enumerate(rows):
            if r.strip():
                last = i
        return last + 1


class Demo:
    def __init__(self, args, rows=24, cols=80):
        if not os.path.exists(BIN):
            raise SystemExit(f"build the demo first: {BIN}")
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
            script = "".join(f"printf '%s\\n' {line!r};" for line in SCROLLBACK)
            script += "printf '$ ';"
            script += f" exec {BIN} {' '.join(args)}"
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

    def dump(self, label):
        rows = self.screen.text()
        print(f"\n=== {label}  ({self.cols}x{self.rows}) ===")
        for i, r in enumerate(rows):
            marker = "·" if not r.strip() else " "
            print(f"{i:2d}{marker}| {r}")
        print(f"    non-blank rows on screen: {self.screen.nonblank_height()}"
              f"   alternate screen: {self.screen.alt_screen}")


def scenario_pick():
    d = Demo(["pick"])
    d.pump(0.6)
    d.dump("A. frame opened (below the prompt, prior scrollback intact)")

    for _ in range(6):
        d.send(DOWN, 0.12)
    d.dump("B. selection 6 of 14 — window slid to 2..10, selection centred")

    for _ in range(8):
        d.send(DOWN, 0.12)
    d.dump("C. selection pinned at the end — window 6..14, height unchanged")

    for ch in "web":
        d.send(ch.encode(), 0.2)
    d.dump("D. typed 'web' — narrowed to 2 rows, frame still 11 lines")

    d.send(BACKSPACE, 0.2)
    d.send(BACKSPACE, 0.2)
    d.dump("E. backspaced to 'w' — list widened again, height unchanged")

    d.send(ENTER, 0.5)
    d.dump("F. Enter — live list erased, settle trace only")


def scenario_cancel():
    d = Demo(["pick"])
    d.pump(0.6)
    d.send(b"sta", 0.4)
    d.dump("G. typed 'sta' before cancelling")
    d.send(ESC, 0.5)
    d.dump("H. Esc — cancel trace, nothing left behind")


def scenario_resize():
    d = Demo(["pick"])
    d.pump(0.6)
    d.send(b"web", 0.4)
    d.send(DOWN, 0.2)
    d.dump("I. before resize: query 'web', selection on web-02")

    d.resize(30, 100)
    d.dump("J. 100x30 — re-opened, query and selection kept")

    d.resize(11, 100)
    d.dump("K. 100x11 — re-opened SMALLER (7 visible rows), query and selection kept")

    d.resize(24, 100)
    d.dump("L. back to 100x24 — re-opened at 8 visible rows again")

    d.resize(24, 60)
    d.dump("M. 60x24 — a live row would have to wrap: cancel-and-restore")


def scenario_short_terminal():
    d = Demo(["pick"], rows=11, cols=80)
    d.pump(0.6)
    d.dump("N. a short terminal: 7 visible rows, one constant height")
    d.send(b"w", 0.4)
    d.dump("O. same short terminal after typing — height unchanged")


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else "all"
    scenarios = {
        "pick": scenario_pick,
        "cancel": scenario_cancel,
        "resize": scenario_resize,
        "short": scenario_short_terminal,
    }
    if which == "all":
        for fn in scenarios.values():
            fn()
    else:
        scenarios[which]()


if __name__ == "__main__":
    main()
