#!/usr/bin/env bash
# Build the binary, then run the E2E suite against exactly that binary.
#
# One script, called the same way from a laptop and from CI, which is the only
# reason the two cannot drift: the suite tests a binary, so *which* binary it
# tests has to be decided in one place. The build is a plain debug build — the
# suite is about behaviour at a terminal, not about codegen.
#
#   ./sshm-e2e/run-e2e.sh                 # the whole suite
#   ./sshm-e2e/run-e2e.sh picker_test     # one test target
#   ./sshm-e2e/run-e2e.sh -- --nocapture  # past flags on to the test harness
#
# Nothing here reaches the network beyond the loopback interface, and nothing
# here retries: a flake is a bug, not a hiccup.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(dirname "$here")"

# Where the built binary goes. Honour an existing CARGO_TARGET_DIR so this
# reuses whatever cache the caller is already paying for, and so CI and a
# developer with a shared target dir get the same path resolution.
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  target_dir="$CARGO_TARGET_DIR"
else
  target_dir="$repo/sshm/target"
fi

echo "building sshm…"
cargo build --manifest-path "$repo/sshm/Cargo.toml"

export SSHM_E2E_BIN="$target_dir/debug/sshm"
if [ ! -x "$SSHM_E2E_BIN" ]; then
  echo "expected the built binary at $SSHM_E2E_BIN and found nothing executable there" >&2
  exit 1
fi
echo "testing $SSHM_E2E_BIN"

# nextest when it is installed: per-test process isolation and a per-test
# timeout mean one hung PTY child fails one named test instead of eating the
# job. Plain libtest is the fallback so a fresh laptop needs nothing extra.
# The config is named explicitly rather than left to discovery: nextest looks for
# it beside the *current* directory, and this script is called from wherever the
# caller happens to be standing.
if cargo nextest --version >/dev/null 2>&1; then
  exec cargo nextest run --manifest-path "$here/Cargo.toml" \
    --config-file "$here/.config/nextest.toml" "$@"
fi
exec cargo test --manifest-path "$here/Cargo.toml" "$@"
