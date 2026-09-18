#!/usr/bin/env bash
# Render the README hero GIF (demo/hero.tape).
#
# Why a wrapper rather than `vhs demo/hero.tape`: the reel drives the real
# `sshm` binary, and that binary reads $HOME/.ssh/connections.json — and
# bare `sshm` *executes* ssh on Enter. So before anything is recorded:
#
#   1. the crate is built (with the real HOME — cargo needs it),
#   2. a throwaway sandbox is reset from demo/fixtures/connections.json,
#   3. `sshm` is put on PATH inside that sandbox so the reel types `sshm`,
#      not an env-var placeholder (vhs echoes the literal keystrokes; the
#      shell only expands at execution, so a $VAR in a typed command shows
#      up in the GIF as $VAR),
#   4. HOME is pointed at the sandbox, so the user's own store is never
#      opened and every render starts from the same fixture.
#
#   scripts/render-hero.sh                    # -> demo/hero.gif
#   scripts/render-hero.sh /tmp/hero.gif     # -> anywhere
#   SSHM_HERO_SANDBOX=/tmp/other scripts/render-hero.sh
#
# Requires: cargo (rustup shims are fine) and vhs
#           https://github.com/charmbracelet/vhs

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$REPO/demo/hero.gif}"
SANDBOX="${SSHM_HERO_SANDBOX:-/tmp/sshm-hero-home}"
FIXTURE="$REPO/demo/fixtures/connections.json"
BIN="$REPO/sshm/target/debug/sshm"

export PATH="$HOME/.cargo/bin:$PATH"

echo "==> building sshm"
( cd "$REPO/sshm" && cargo build )

echo "==> resetting sandbox at $SANDBOX"
rm -rf "$SANDBOX"
mkdir -p "$SANDBOX/.ssh" "$SANDBOX/bin"
cp "$FIXTURE" "$SANDBOX/.ssh/connections.json"

# Decor for the opening `ls ~/.ssh`: the two files the frame manages, so the
# scrollback names the store right before the tool that owns it appears.
printf '# demo ssh config\n' > "$SANDBOX/.ssh/config"
printf 'demo\n' > "$SANDBOX/.ssh/id_ed25519"

ln -sf "$BIN" "$SANDBOX/bin/sshm"

# The inner shell inherits these, so the reel types a plain `sshm` and every
# connection it touches lands in the sandbox rather than in ~/.ssh.
export PATH="$SANDBOX/bin:$PATH"
export HOME="$SANDBOX"

if [ "$(command -v sshm)" != "$SANDBOX/bin/sshm" ]; then
  echo "!! sandbox sshm is not first on PATH" >&2
  exit 1
fi

echo "==> recording $OUT"
vhs -q -o "$OUT" "$REPO/demo/hero.tape"

# The reel must end exactly where the fixture began: web-03 added, web-03
# deleted. If the counts disagree, a beat fired out of order and the GIF is
# lying about the app — fail loudly rather than ship it.
before=$(python3 -c "import json;print(len(json.load(open('$FIXTURE'))['connections']))")
after=$(python3 -c "import json;print(len(json.load(open('$SANDBOX/.ssh/connections.json'))['connections']))")
if [ "$before" != "$after" ]; then
  echo "!! round trip broke: fixture has $before, sandbox ended with $after" >&2
  exit 1
fi

size=$(du -h "$OUT" | cut -f1)
echo "==> ok: $OUT ($size), store round-tripped $before -> $after"
