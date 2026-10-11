#!/usr/bin/env python3
"""Drive the frame in a real pseudo-terminal and report what the terminal says.

Run by `tests/pty_test.rs`; it takes the path to the binary and nothing else.
It exists because three things about the frame are facts about a terminal and
cannot be seen from a pipe: the frame only draws when it has a terminal to
draw on, it takes the terminal out of cooked mode while it is open, and it
puts the terminal back on the way out. A test that never had a terminal would
assert none of that, which is why those three survived mutation for as long as
they did.

The parent watches the master side of the pty. On Linux the line discipline
belongs to the slave, so `tcgetattr` on the master reports the settings the
child is running under — the check is what the terminal itself says, not what
the program claims.

Exits 0 when every check holds, 1 with the failures listed. Every wait has a
deadline: a mutant that wedges the binary must fail the run, not hang it.
"""

import fcntl
import json
import os
import pathlib
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

# Rows and columns the frame needs to host itself. A fresh pty has no size at
# all, and a terminal with no rows is one the frame correctly refuses to draw
# in — so the size is set here rather than left to chance.
ROWS, COLS = 24, 80

# How long to wait, at most, for each stage. Generous for a loaded CI runner
# and short enough that a wedged binary still reports within a minute.
OPEN_TIMEOUT = 15.0
EXIT_TIMEOUT = 15.0

COOKED = termios.ECHO | termios.ICANON

CURSOR_HIDE = b"\x1b[?25l"
CURSOR_RESTORE = b"\x1b[?25h"


def lflag(fd):
    """The terminal's local flags, as the terminal reports them."""
    return termios.tcgetattr(fd)[3]


def spawn(binary, home):
    """The binary, alone in its own session with the pty as its controlling
    terminal — which is what `/dev/tty` resolves to, and the frame opens
    `/dev/tty` for itself when its stdout is not the terminal."""
    master, slave = pty.openpty()
    fcntl.ioctl(
        slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0)
    )

    def become_leader():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    env = dict(os.environ, HOME=home, TERM="xterm-256color")
    child = subprocess.Popen(
        [binary, "pick"],
        stdin=slave,
        stdout=slave,
        stderr=slave,
        env=env,
        preexec_fn=become_leader,
    )
    return master, slave, child


def pump(master, wanted, deadline):
    """Read until `wanted` appears or the deadline passes.

    Returns everything read. Reading continuously is not optional: the pty
    buffer is small and a frame repaints enough to fill it.
    """
    seen = b""
    while time.monotonic() < deadline:
        ready, _, _ = select.select([master], [], [], 0.02)
        if not ready:
            continue
        try:
            chunk = os.read(master, 65536)
        except OSError:
            break
        if not chunk:
            break
        seen += chunk
        if wanted(seen):
            break
    return seen


def main():
    if len(sys.argv) != 2:
        print("usage: pty_frame_check.py <path to sshm>", file=sys.stderr)
        return 2

    home = tempfile.mkdtemp()
    pathlib.Path(home, ".ssh").mkdir()
    pathlib.Path(home, ".ssh", "connections.json").write_text(
        json.dumps(
            {
                "connections": [
                    {
                        "id": "id-web-01",
                        "alias": "web-01",
                        "host": "10.0.0.7",
                        "user": "deploy",
                        "port": 22,
                        "key_path": None,
                        "folder": None,
                    }
                ]
            }
        )
    )

    master, slave, child = spawn(sys.argv[1], home)
    failures = []

    drawn = pump(master, lambda b: CURSOR_HIDE in b, time.monotonic() + OPEN_TIMEOUT)
    if not drawn:
        failures.append("the frame drew nothing at all and the command is over")
    elif CURSOR_HIDE not in drawn:
        failures.append(
            "the frame printed without hiding the cursor, so it is not a frame: %r"
            % drawn[:200]
        )
    elif child.poll() is not None:
        failures.append(
            "the frame drew and left without waiting for an answer (exit %s)"
            % child.returncode
        )

    if not failures:
        # The frame is open and waiting. What does the terminal say about
        # itself right now?
        if lflag(master) & COOKED == COOKED:
            failures.append(
                "the frame is open but the terminal is still cooked: keys are "
                "being echoed and buffered by the line discipline"
            )

        os.write(master, b"\x1b")  # Escape: cancel
        drained = pump(
            master, lambda b: CURSOR_RESTORE in b, time.monotonic() + EXIT_TIMEOUT
        )
        drawn += drained

        try:
            child.wait(timeout=EXIT_TIMEOUT)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
            failures.append("the frame never came back from Escape")

        if child.returncode != 130:
            failures.append(
                "a cancelled frame left %s, not the 130 a cancelled command leaves"
                % child.returncode
            )
        if CURSOR_RESTORE not in drawn:
            failures.append("the cursor was never given back")

    # The terminal itself, after the frame is gone: handed back as it was
    # found, or left raw for whatever runs next in this shell.
    if lflag(master) & COOKED != COOKED:
        failures.append(
            "the terminal was left out of cooked mode: the next command in "
            "this shell inherits a terminal that does not echo"
        )

    os.close(slave)
    os.close(master)

    for failure in failures:
        print(failure, file=sys.stderr)
    if failures:
        return 1
    print("frame drawn, terminal taken, terminal handed back")
    return 0


if __name__ == "__main__":
    sys.exit(main())
