#!/usr/bin/env bash
#
# Smoke test for install.sh, driven entirely by fixtures.
#
# install.sh is served from `master`, so a bad merge is live for every user
# before anyone can release their way out of it — and until `sshm update`
# ships, it is the only update path that exists at all. The interesting
# behaviour is also the untestable-against-production behaviour: what happens
# when an older binary is already present, shadowing, symlinked, or sitting in
# a directory this user cannot write.
#
# So: a fake `curl` serves a stubbed /releases/latest response and a local
# tarball, and a fake `sshm` binary reports a version on demand. No network,
# no real release, every branch reachable.
#
#   ./tests/install/smoke.sh
#
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
INSTALL_SH="$ROOT/install.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

PASS=0
FAIL=0
FAILED_NAMES=()

ok() { PASS=$((PASS + 1)); printf '    ok    %s\n' "$1"; }
bad() {
    FAIL=$((FAIL + 1))
    FAILED_NAMES+=("$1")
    printf '    FAIL  %s\n' "$1"
    if [ -n "${2:-}" ]; then printf '          %s\n' "$2"; fi
}

assert_contains() {
    local needle="$1" hay="$2" what="$3"
    case "$hay" in
        *"$needle"*) ok "$what" ;;
        *) bad "$what" "expected to contain: $needle" ;;
    esac
}

assert_not_contains() {
    local needle="$1" hay="$2" what="$3"
    case "$hay" in
        *"$needle"*) bad "$what" "expected NOT to contain: $needle" ;;
        *) ok "$what" ;;
    esac
}

assert_eq() {
    local want="$1" got="$2" what="$3"
    if [ "$want" = "$got" ]; then
        ok "$what"
    else
        bad "$what" "expected [$want] got [$got]"
    fi
}

assert_file_version() {
    # $1 = path, $2 = expected version, $3 = what
    local got=""
    if [ -x "$1" ]; then got=$("$1" --version 2>/dev/null || true); fi
    assert_eq "sshm $2" "$got" "$3"
}

# ---------------------------------------------------------------------------
# Platform triple, matching install.sh
# ---------------------------------------------------------------------------

case "$(uname -s)" in
    Linux) PLATFORM="x86_64-unknown-linux-musl" ;;
    Darwin)
        if [ "$(uname -m)" = "arm64" ]; then
            PLATFORM="aarch64-apple-darwin"
        else
            PLATFORM="x86_64-apple-darwin"
        fi
        ;;
    *) printf 'unsupported platform for smoke test: %s\n' "$(uname -s)" >&2; exit 1 ;;
esac

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        return 1
    fi
}

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

# make_release <version> [sha_override]
#
# Builds a tarball holding a stand-in `sshm` that answers --version, and a
# GitHub-shaped /releases/latest JSON pointing at it. The JSON is
# pretty-printed with a space after each colon and emits `digest` before
# `browser_download_url` inside the asset object, because that is the shape
# install.sh's parsers are written against.
make_release() {
    local ver="$1" sha="${2:-}"
    local stage="$WORK/stage-$ver"
    rm -rf "$stage"
    mkdir -p "$stage"

    cat >"$stage/sshm" <<FAKE
#!/usr/bin/env bash
case "\${1:-}" in
    --version) echo "sshm $ver" ;;
    completions) echo "# fake sshm $ver completions for \$2" ;;
    *) echo "fake sshm $ver" ;;
esac
FAKE
    chmod +x "$stage/sshm"
    tar -czf "$WORK/sshm-$ver.tar.gz" -C "$stage" sshm

    if [ -z "$sha" ]; then
        sha=$(sha256_of "$WORK/sshm-$ver.tar.gz")
    fi

    cat >"$WORK/release.json" <<JSON
{
  "url": "https://api.github.com/repos/titonio/ssh-manager/releases/1",
  "html_url": "https://github.com/titonio/ssh-manager/releases/tag/v$ver",
  "tag_name": "v$ver",
  "name": "v$ver",
  "draft": false,
  "prerelease": false,
  "assets": [
    {
      "name": "sshm-v$ver-$PLATFORM.tar.gz",
      "label": null,
      "content_type": "application/gzip",
      "size": 1234,
      "digest": "sha256:$sha",
      "browser_download_url": "https://github.com/titonio/ssh-manager/releases/download/v$ver/sshm-v$ver-$PLATFORM.tar.gz"
    }
  ]
}
JSON
    export FIXTURE_JSON="$WORK/release.json"
    export FIXTURE_TARBALL="$WORK/sshm-$ver.tar.gz"
}

# A fake curl: serves the fixtures, and records every URL it was asked for
# so a test can assert that nothing was downloaded.
make_fake_curl() {
    mkdir -p "$WORK/fakebin"
    cat >"$WORK/fakebin/curl" <<'FAKECURL'
#!/usr/bin/env bash
out=""
want_code=0
url=""
args=("$@")
i=0
while [ "$i" -lt "${#args[@]}" ]; do
    a="${args[$i]}"
    case "$a" in
        -o) i=$((i + 1)); out="${args[$i]}" ;;
        -w) want_code=1 ;;
        http* | file*) url="$a" ;;
        *) ;;
    esac
    i=$((i + 1))
done
printf '%s\n' "$url" >>"${FAKE_CURL_LOG:?}"
case "$url" in
    *releases/latest*)
        cp "$FIXTURE_JSON" "$out"
        [ "$want_code" -eq 1 ] && printf '200'
        exit 0
        ;;
    *releases/download*)
        cp "$FIXTURE_TARBALL" "$out"
        [ "$want_code" -eq 1 ] && printf '200'
        exit 0
        ;;
    *)
        printf 'fake curl: unexpected url: %s\n' "$url" >&2
        exit 6
        ;;
esac
FAKECURL
    chmod +x "$WORK/fakebin/curl"
}

make_fake_curl

# run_install <case-dir> [extra env assignments are made by the caller]
#
# Runs install.sh with the fake curl first on PATH, a throwaway HOME, and no
# controlling terminal — so `tty_available` inside the script answers the
# same way a non-interactive `curl | bash` would, and a sudo branch cannot
# quietly block waiting for a password nobody is there to type.
OUT="$WORK/out.txt"
ERR="$WORK/err.txt"
LOG="$WORK/curl.log"

run_install() {
    : >"$OUT"
    : >"$ERR"
    : >"$LOG"
    export FAKE_CURL_LOG="$LOG"
    local run_path="$WORK/fakebin:$PATH"
    if [ "${NO_SUDO:-0}" = "1" ]; then
        # A sudo that always fails, shadowing the real one. Deterministic, and
        # it leaves the rest of PATH intact — removing the real sudo's directory
        # from PATH took `env bash` down with it.
        mkdir -p "$WORK/nosudo"
        printf '#!/bin/sh\nexit 1\n' >"$WORK/nosudo/sudo"
        chmod +x "$WORK/nosudo/sudo"
        run_path="$WORK/nosudo:$run_path"
    fi
    local rc=0
    (
        export PATH="$run_path"
        if command -v setsid >/dev/null 2>&1; then
            setsid -w bash "$INSTALL_SH" "${ARGS[@]}" </dev/null >"$OUT" 2>"$ERR"
        else
            bash "$INSTALL_SH" "${ARGS[@]}" </dev/null >"$OUT" 2>"$ERR"
        fi
    ) || rc=$?
    RC=$rc
    ALL="$(cat "$OUT" "$ERR")"
}

ARGS=()

# A fresh sandbox: throwaway HOME plus the directories the script probes.
new_env() {
    local case_dir="$1"
    export HOME="$case_dir/home"
    mkdir -p "$HOME"
    USER_BIN="$HOME/.local/bin"
    BREW_BIN="$case_dir/opt/homebrew/bin"
    LOCAL_BIN="$case_dir/usr/local/bin"
    export SSHM_PROBE_ORDER="$USER_BIN $BREW_BIN $LOCAL_BIN"
    export SSHM_INSTALL_LADDER="$LOCAL_BIN $USER_BIN"
    export SHELL="/bin/bash"
}

section() { printf '\n  %s\n' "$1"; }

# ===========================================================================
printf '%s\n' "install.sh smoke test  ($PLATFORM)"
# ===========================================================================

# ---------------------------------------------------------------------------
section "fresh install, nothing present"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-fresh"
rm -rf "$D"
new_env "$D"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_contains "Installing sshm 0.2.0" "$ALL" "announces a fresh install"
assert_file_version "$LOCAL_BIN/sshm" "0.2.0" "binary landed in the first writable ladder dir"

# ---------------------------------------------------------------------------
section "update over an older install replaces that same path"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-update"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 0.1.9"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_contains "sshm 0.1.9 → 0.2.0" "$ALL" "prints the transition line"
assert_not_contains "Installing sshm" "$ALL" "does not call an update an install"
assert_file_version "$USER_BIN/sshm" "0.2.0" "the old path is the one that changed"

# ---------------------------------------------------------------------------
section "already on the latest release"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-latest"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 0.2.0"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_contains "sshm 0.2.0 is already the latest release." "$ALL" "says so in the words sshm update uses"
assert_not_contains "releases/download" "$(cat "$LOG")" "downloaded nothing"

# ---------------------------------------------------------------------------
section "--force reinstalls the same version"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-force"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 0.2.0"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
ARGS=(--force)
run_install
assert_eq "0" "$RC" "exits 0"
assert_contains "--force was given" "$ALL" "says why it is reinstalling"
assert_contains "releases/download" "$(cat "$LOG")" "actually downloaded"
assert_file_version "$USER_BIN/sshm" "0.2.0" "binary is in place"

# ---------------------------------------------------------------------------
section "shadowed installs: the one a PATH would find first wins"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-shadow"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN" "$LOCAL_BIN"
printf '#!/bin/sh\necho "sshm 0.1.4"\n' >"$USER_BIN/sshm"
printf '#!/bin/sh\necho "sshm 0.1.7"\n' >"$LOCAL_BIN/sshm"
chmod +x "$USER_BIN/sshm" "$LOCAL_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_file_version "$USER_BIN/sshm" "0.2.0" "the ~/.local/bin copy was replaced"
assert_file_version "$LOCAL_BIN/sshm" "0.1.7" "the /usr/local/bin copy was left alone"
assert_contains "left untouched" "$ALL" "reports the other install"
assert_contains "0.1.7" "$ALL" "names the other install's version"

# ---------------------------------------------------------------------------
section "symlinked install: one level resolved, the link survives"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-symlink"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN" "$D/realdir"
printf '#!/bin/sh\necho "sshm 0.1.9"\n' >"$D/realdir/sshm"
chmod +x "$D/realdir/sshm"
ln -s "$D/realdir/sshm" "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_file_version "$D/realdir/sshm" "0.2.0" "the file behind the link was replaced"
if [ -L "$USER_BIN/sshm" ]; then ok "the symlink is still a symlink"; else bad "the symlink is still a symlink" "it was replaced by a regular file"; fi

# ---------------------------------------------------------------------------
section "relative symlink target is anchored to the link, not the cwd"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-relsymlink"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN" "$HOME/lib"
printf '#!/bin/sh\necho "sshm 0.1.9"\n' >"$HOME/lib/sshm"
chmod +x "$HOME/lib/sshm"
ln -s "../../lib/sshm" "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0"
assert_file_version "$HOME/lib/sshm" "0.2.0" "the relative target was resolved against the link's directory"

# ---------------------------------------------------------------------------
section "unwritable target: nothing downloaded, the escalation is printed"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-unwritable"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 0.1.9"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
chmod 555 "$USER_BIN"
NO_SUDO=1
ARGS=()
run_install
NO_SUDO=0
chmod 755 "$USER_BIN"
assert_eq "1" "$RC" "exits non-zero"
assert_contains "is not writable by this user" "$ALL" "names the directory"
assert_contains "Nothing was downloaded and nothing was changed." "$ALL" "promises nothing happened"
assert_not_contains "releases/download" "$(cat "$LOG")" "and it is true: no download"
assert_contains "sudo bash" "$ALL" "prints the exact command to re-run"

# ---------------------------------------------------------------------------
section "installed version newer than the release: downgrades, and says so"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-downgrade"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 9.9.9"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "exits 0 — install.sh is the explicit route"
assert_contains "downgrade" "$ALL" "says the word"
assert_file_version "$USER_BIN/sshm" "0.2.0" "the release won"

# ---------------------------------------------------------------------------
section "unreadable installed version: proceeds, stays quiet about the from"
# ---------------------------------------------------------------------------
make_release "0.2.0"
D="$WORK/c-unknown"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\nexit 3\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
ARGS=()
run_install
assert_eq "0" "$RC" "a binary that cannot answer is not a reason to stop"
assert_contains "Updating sshm to 0.2.0" "$ALL" "worded without a from-version"
assert_file_version "$USER_BIN/sshm" "0.2.0" "still installed"

# ---------------------------------------------------------------------------
section "checksum mismatch: refuses before touching the target"
# ---------------------------------------------------------------------------
make_release "0.2.0" "0000000000000000000000000000000000000000000000000000000000000000"
D="$WORK/c-corrupt"
rm -rf "$D"
new_env "$D"
mkdir -p "$USER_BIN"
printf '#!/bin/sh\necho "sshm 0.1.9"\n' >"$USER_BIN/sshm"
chmod +x "$USER_BIN/sshm"
BEFORE=$(sha256_of "$USER_BIN/sshm")
ARGS=()
run_install
AFTER=$(sha256_of "$USER_BIN/sshm")
assert_eq "1" "$RC" "exits non-zero"
assert_contains "Checksum mismatch" "$ALL" "says why"
assert_eq "$BEFORE" "$AFTER" "the existing binary is byte-identical"

# ---------------------------------------------------------------------------
section "no asset for this platform: distinct from a missing release"
# ---------------------------------------------------------------------------
make_release "0.2.0"
sed -i.bak "s/$PLATFORM/some-other-triple/" "$WORK/release.json" 2>/dev/null ||
    sed "s/$PLATFORM/some-other-triple/" "$WORK/release.json.bak" >"$WORK/release.json"
D="$WORK/c-noasset"
rm -rf "$D"
new_env "$D"
ARGS=()
run_install
assert_eq "1" "$RC" "exits non-zero"
assert_contains "No release asset matches this platform" "$ALL" "names the platform, not the release"
assert_not_contains "releases/download" "$(cat "$LOG")" "downloaded nothing"

# ---------------------------------------------------------------------------
section "--help works and documents the pipe form"
# ---------------------------------------------------------------------------
ARGS=(--help)
run_install
assert_eq "0" "$RC" "exits 0"
assert_contains "bash -s -- --force" "$ALL" "shows how to pass a flag through a pipe"

# ---------------------------------------------------------------------------
section "unknown option is refused"
# ---------------------------------------------------------------------------
ARGS=(--nonsense)
run_install
assert_eq "2" "$RC" "exits 2"
assert_contains "unknown option" "$ALL" "says which"

# ===========================================================================
printf '\n%s\n' "----------------------------------------"
printf '%d passed, %d failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -gt 0 ]; then
    printf 'failed: %s\n' "${FAILED_NAMES[*]}"
    exit 1
fi
printf 'all green\n'
