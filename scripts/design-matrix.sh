#!/usr/bin/env bash
#
# Terminal capability matrix.
#
# The TUI equivalent of checking a web page at several viewport widths. Runs the
# design detector once per colour mode. No injection is needed: ColorSupport::detect()
# reads COLORTERM / TERM / NO_COLOR, so the environment *is* the matrix.
#
# Each pass also dumps real ANSI frames, so the same run that proves the palette
# arithmetic also produces something a human can look at:
#
#   cat sshm/target/design-frames/picker-Truecolor.ansi
#
# Usage: ./scripts/design-matrix.sh

set -euo pipefail
cd "$(dirname "$0")/../sshm"

pass() {
  local label="$1"
  shift
  printf '\n\033[1m── %s ──\033[0m\n' "$label"
  # `env VAR=` sets an empty value, which detect() treats as "not truecolour".
  env "$@" SSHM_DUMP_FRAMES=1 cargo test --test design_system_test 2>&1 | tail -22
}

# -u NO_COLOR is not optional. A non-interactive shell (CI, a harness, a pipe)
# usually carries NO_COLOR=1 and TERM=dumb from wherever it was spawned, and
# detect() honours those first — so without the unsets every "colour" pass silently
# runs monochrome and the matrix proves nothing.
pass "truecolour (COLORTERM=truecolor)" -u NO_COLOR COLORTERM=truecolor TERM=xterm-256color
pass "256 colour  (TERM=xterm-256color, no COLORTERM)" -u NO_COLOR COLORTERM= TERM=xterm-256color
pass "16 colour   (TERM=xterm)" -u NO_COLOR COLORTERM= TERM=xterm
pass "no colour   (NO_COLOR=1)" NO_COLOR=1 TERM=xterm-256color

printf '\n\033[1mframes written:\033[0m\n'
ls -1 target/design-frames/ 2>/dev/null | sed 's/^/  /' || echo "  (none)"
