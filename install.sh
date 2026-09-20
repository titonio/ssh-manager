#!/usr/bin/env bash
#
# sshm — installer and updater
#
# Installs the latest release of sshm. When an older sshm is already on the
# machine this replaces *that* binary rather than laying a second copy down in
# front of it — the same rule `sshm update` follows (docs/adr/0002). A second
# copy is exactly how "I updated and nothing changed" happens: the new binary
# lands somewhere the user was never running the old one from.
#
# Run:
#   curl -sL https://raw.githubusercontent.com/titonio/ssh-manager/master/install.sh | bash
#   curl -sL .../install.sh | bash -s -- --force      # reinstall the same version
#
# Three properties hold throughout:
#   * Nothing is downloaded until the target is known and known to be writable.
#   * The swap is an atomic rename on the target's own filesystem.
#   * No shell rc file is ever edited. Where the user must act, the exact
#     command or line to add is printed instead.

set -euo pipefail

REPO_OWNER="titonio"
REPO_NAME="ssh-manager"
BIN_NAME="sshm"
SELF_URL="https://raw.githubusercontent.com/$REPO_OWNER/$REPO_NAME/master/install.sh"
API_URL="https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"

if [ -z "${HOME:-}" ]; then
    printf 'error: HOME is not set; cannot determine where to install.\n' >&2
    exit 1
fi

# Where an existing install is looked for, in the order a typical interactive
# PATH would find it — `~/.local/bin` first, because on Debian and Ubuntu it
# precedes `/usr/local/bin` and therefore wins.
#
# Both lists are overridable so the smoke test in tests/install/smoke.sh can
# drive every branch (shadowed installs, symlinks, unwritable targets) against
# a fixture instead of a live release. The defaults are the real paths.
read -r -a PROBE_ORDER <<<"${SSHM_PROBE_ORDER:-$HOME/.local/bin /opt/homebrew/bin /usr/local/bin}"
read -r -a INSTALL_LADDER <<<"${SSHM_INSTALL_LADDER:-/usr/local/bin $HOME/.local/bin}"

FORCE=0

usage() {
    cat <<EOF
Installs the latest release of $BIN_NAME, or replaces an existing install in place.

Usage:
  install.sh [--force]

  --force     Reinstall even when the installed version is already the latest.
  -h, --help  Show this help.

Through a pipe, pass flags after \`bash -s --\`:

  curl -sL $SELF_URL | bash -s -- --force
EOF
}

log() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --force) FORCE=1 ;;
        -h | -help | --help)
            usage
            exit 0
            ;;
        *)
            printf 'error: unknown option: %s\n' "$1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

# ---------------------------------------------------------------------------
# Platform
# ---------------------------------------------------------------------------

detect_platform() {
    case "$(uname -s)" in
        Linux)
            # The kernel name alone does not say which binary runs — the
            # Darwin branch beside this one has always known that. Termux
            # reports `Linux` with `uname -m` = `aarch64`, and answering
            # it with the x86_64 triple shipped an unrunnable binary whose
            # only symptom was a swallowed `Exec format error` (#48).
            # Every Linux ships as static musl; only the arch varies.
            case "$(uname -m)" in
                x86_64) printf 'x86_64-unknown-linux-musl\n' ;;
                aarch64) printf 'aarch64-unknown-linux-musl\n' ;;
                *)
                    die "Unsupported architecture: $(uname -m). sshm ships x86_64 and aarch64 Linux binaries."
                    ;;
            esac
            ;;
        Darwin)
            case "$(uname -m)" in
                arm64) printf 'aarch64-apple-darwin\n' ;;
                *) printf 'x86_64-apple-darwin\n' ;;
            esac
            ;;
        MINGW* | MSYS* | CYGWIN* | Windows*)
            die "Windows is not supported by this installer yet. See https://github.com/$REPO_OWNER/$REPO_NAME/releases."
            ;;
        *) die "Unsupported platform: $(uname -s)" ;;
    esac
}

# ---------------------------------------------------------------------------
# Small portable helpers
# ---------------------------------------------------------------------------

# `timeout` is coreutils and is not on macOS by default; `gtimeout` is the
# Homebrew name. Without either, run the command bare — the timeout is a guard
# against a hung old binary, never a correctness requirement.
run_with_timeout() {
    local secs="$1"
    shift
    if command -v timeout >/dev/null 2>&1; then
        timeout "$secs" "$@"
    elif command -v gtimeout >/dev/null 2>&1; then
        gtimeout "$secs" "$@"
    else
        "$@"
    fi
}

tty_available() { (exec >/dev/tty) 2>/dev/null; }

# Empty, or "sudo". Every write to the target goes through run() so the
# privilege decision is made once, in one place.
SUDO=""
run() {
    if [ -n "$SUDO" ]; then
        "$SUDO" "$@"
    else
        "$@"
    fi
}

# Decide how to write into $1. Succeeds if the directory is already writable
# (no escalation — the old script reached for sudo even here, which breaks a
# box that owns its own /usr/local/bin), or if sudo is present and can either
# run without a password or prompt for one on a real tty.
choose_privilege() {
    local dir="$1"
    if [ -w "$dir" ]; then
        SUDO=""
        return 0
    fi
    if command -v sudo >/dev/null 2>&1; then
        if sudo -n true 2>/dev/null || tty_available; then
            SUDO="sudo"
            return 0
        fi
    fi
    return 1
}

# The version a binary reports, or empty. Running an old binary as part of
# replacing it is a read that may hang or fail, so it is bounded and its
# failure is ordinary: an unknown installed version changes the wording of
# the message, never whether the install proceeds.
#
# $2 (optional): where the binary's stderr goes — /dev/null unless the
# caller wants it. Probes of *old* binaries want none; their failure is
# ordinary and the text is noise. The pre-swap check of the *fresh* binary
# asks for it: there the stderr is the reason. On Termux/ARM64 the
# wrong-arch download answers the version probe with nothing on stdout and
# `Exec format error` on stderr, and discarding that left users with a
# bare "did not report a version" and no way to know why (#48).
read_installed_version() {
    local bin="$1" err_file="${2:-/dev/null}" out
    out=$(run_with_timeout 10 "$bin" --version 2>"$err_file" || true)
    printf '%s\n' "$out" | awk '{
        for (i = 1; i <= NF; i++) {
            if ($i ~ /^v?[0-9]+\.[0-9]+\.[0-9]+$/) { sub(/^v/, "", $i); print $i; exit }
        }
    }'
}

# Prints 1, 0 or -1 for $1 vs $2, or 2 when either side is unparseable.
# Unparseable is its own answer so callers can stay quiet rather than invent
# an ordering — the same rule `is_newer` applies in sshm/src/update.rs.
version_cmp() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        if (!(a ~ /^v?[0-9]+\.[0-9]+\.[0-9]+$/)) { print 2; exit }
        if (!(b ~ /^v?[0-9]+\.[0-9]+\.[0-9]+$/)) { print 2; exit }
        sub(/^v/, "", a); sub(/^v/, "", b)
        split(a, A, "."); split(b, B, ".")
        for (i = 1; i <= 3; i++) {
            x = A[i] + 0; y = B[i] + 0
            if (x > y) { print 1; exit }
            if (x < y) { print -1; exit }
        }
        print 0
    }'
}

# Collapse "." and ".." by name so a diagnostic prints one path instead of a
# walk through it. Not canonicalize(): that resolves every symlink and insists
# the file exists, which is more than this script is allowed to assume.
collapse_path() {
    printf '%s\n' "$1" | awk '{
        n = 0
        m = split($0, P, "/")
        for (i = 1; i <= m; i++) {
            if (P[i] == "" || P[i] == ".") continue
            if (P[i] == "..") { if (n > 0) n--; continue }
            out[++n] = P[i]
        }
        s = ""
        for (i = 1; i <= n; i++) s = s "/" out[i]
        print s
    }'
}

# Resolve ONE symlink level, anchored — deliberately the same single level
# `resolved_target()` in sshm/src/update.rs resolves, so the two paths that
# perform this act agree on which file is at risk. A stored target of
# "../lib/sshm" is relative to the link, not to the process.
resolve_one_level() {
    local p="$1" stored
    if [ -L "$p" ]; then
        stored=$(readlink "$p" || true)
        if [ -z "$stored" ]; then
            printf '%s\n' "$p"
            return
        fi
        case "$stored" in
            /*) collapse_path "$stored" ;;
            *) collapse_path "$(dirname "$p")/$stored" ;;
        esac
    else
        printf '%s\n' "$p"
    fi
}

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
# Locate any existing install
# ---------------------------------------------------------------------------

CANDIDATES=()

add_candidate() {
    local p="$1" existing
    [ -n "$p" ] || return 0
    # bash 3.2 (the macOS default) treats an empty array as an error under
    # set -u, hence the ${arr[@]+x} guard.
    for existing in "${CANDIDATES[@]+${CANDIDATES[@]}}"; do
        [ "$existing" = "$p" ] && return 0
    done
    CANDIDATES+=("$p")
}

find_existing() {
    local d p which_path
    for d in "${PROBE_ORDER[@]}"; do
        p="$d/$BIN_NAME"
        if [ -e "$p" ] || [ -L "$p" ]; then
            add_candidate "$p"
        fi
    done

    # `command -v` answers "what does this shell actually run", which covers an
    # install in a directory outside the probe list. It is appended, not
    # preferred: inside `curl | bash` the shell is non-interactive, so
    # ~/.bashrc was never sourced and ~/.local/bin may be missing from PATH
    # even though it is the install the user runs every day.
    which_path=$(command -v "$BIN_NAME" 2>/dev/null || true)
    if [ -n "$which_path" ]; then
        add_candidate "$which_path"
    fi
}

# ---------------------------------------------------------------------------
# Release metadata
# ---------------------------------------------------------------------------

# GitHub pretty-prints this response with a space after each colon today, but
# every parse below tolerates either shape. A literal space in the pattern is
# one upstream formatting change away from "Failed to get latest release
# version" on a perfectly good release.
json_scalar() {
    local file="$1" key="$2"
    awk -v key="$key" '$0 ~ "\"" key "\":" {
        s = $0
        sub(".*\"" key "\":[ \t]*\"", "", s)
        sub(/".*/, "", s)
        print s
        exit
    }' "$file"
}

# The digest and the URL of the one asset we want. Within an asset object
# GitHub emits `digest` before `browser_download_url`, so remembering the last
# digest seen and pairing it at the matching URL is exact — and needs no jq,
# which is not a safe assumption on a stranger's machine.
asset_pair() {
    awk -v plat="$1" '
        $0 ~ /"digest":/ {
            d = $0
            sub(/.*"digest":[ \t]*"/, "", d)
            sub(/".*/, "", d)
            last = d
        }
        $0 ~ /"browser_download_url":/ && index($0, plat) > 0 {
            u = $0
            sub(/.*"browser_download_url":[ \t]*"/, "", u)
            sub(/".*/, "", u)
            print last "\t" u
        }
    ' "$JSON_FILE"
}

# ---------------------------------------------------------------------------
# Install
# ---------------------------------------------------------------------------

PLATFORM=$(detect_platform)
log "Platform: $PLATFORM"

find_existing

TARGET=""
HAD_EXISTING=0
if [ "${#CANDIDATES[@]}" -gt 0 ]; then
    HAD_EXISTING=1
    TARGET=$(resolve_one_level "${CANDIDATES[0]}")
fi

INSTALLED_VERSION=""
if [ -n "$TARGET" ]; then
    INSTALLED_VERSION=$(read_installed_version "$TARGET" || true)
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

JSON_FILE="$TMP_DIR/release.json"
HTTP_CODE=$(curl -sL -o "$JSON_FILE" -w '%{http_code}' "$API_URL" 2>/dev/null || echo "000")

case "$HTTP_CODE" in
    200) : ;;
    403 | 401)
        die "GitHub's release API refused the request (HTTP $HTTP_CODE). This is usually the unauthenticated rate limit of 60 requests per hour per IP. Nothing was downloaded and nothing was changed; try again later."
        ;;
    404)
        die "GitHub has no latest release for $REPO_OWNER/$REPO_NAME (HTTP 404). Nothing was downloaded and nothing was changed."
        ;;
    *)
        die "Could not reach GitHub's release API (HTTP $HTTP_CODE). Nothing was downloaded and nothing was changed."
        ;;
esac

VERSION=$(json_scalar "$JSON_FILE" "tag_name")
if [ -z "$VERSION" ]; then
    die "Could not read a release tag from GitHub's response. Nothing was downloaded and nothing was changed."
fi
# Tags are `v0.2.0`; `sshm --version` prints `0.2.0`. Normalise once here so
# every comparison and every printed line speaks the same form as the binary.
VERSION="${VERSION#v}"

MATCHING_URLS=$(awk '
    $0 ~ /"browser_download_url":/ {
        s = $0
        sub(/.*"browser_download_url":[ \t]*"/, "", s)
        sub(/".*/, "", s)
        if (index(s, plat) > 0) print s
    }
' plat="$PLATFORM" "$JSON_FILE")

MATCH_COUNT=$(printf '%s\n' "$MATCHING_URLS" | grep -c . || true)

if [ "$MATCH_COUNT" -eq 0 ]; then
    log "No release asset matches this platform ($PLATFORM)."
    log "Assets on $VERSION:"
    awk '/"browser_download_url":/ { s=$0; sub(/.*"browser_download_url":[ \t]*"/,"",s); sub(/".*/,"",s); print "  " s }' "$JSON_FILE" >&2
    die "Nothing was downloaded and nothing was changed."
fi
if [ "$MATCH_COUNT" -gt 1 ]; then
    log "More than one release asset matches this platform ($PLATFORM); refusing to guess."
    printf '%s\n' "$MATCHING_URLS" >&2
    die "Nothing was downloaded and nothing was changed."
fi

DOWNLOAD_URL=$(printf '%s\n' "$MATCHING_URLS" | head -n 1)
EXPECTED_SHA=$(asset_pair "$PLATFORM" | head -n 1 | cut -f1 | sed 's/^sha256://')

# ---------------------------------------------------------------------------
# Decide the target and how to write it — before downloading anything
# ---------------------------------------------------------------------------

if [ -n "$TARGET" ]; then
    TARGET_DIR=$(dirname "$TARGET")
    if ! choose_privilege "$TARGET_DIR"; then
        log "sshm cannot be replaced: $TARGET_DIR is not writable by this user, and sudo is unavailable."
        log "Nothing was downloaded and nothing was changed."
        log "Re-run this installer with privileges:"
        log "  curl -sL $SELF_URL | sudo bash"
        exit 1
    fi
else
    TARGET_DIR=""
    for d in "${INSTALL_LADDER[@]}"; do
        # Try as ourselves before reaching for sudo. An absent directory we can
        # create is cheaper than a privilege we do not need, and escalating to
        # create one leaves root-owned files behind in paths the user then has
        # to sudo to manage.
        if [ -d "$d" ] && [ -w "$d" ]; then
            TARGET_DIR="$d"
            SUDO=""
            break
        fi
        if [ ! -e "$d" ] && mkdir -p "$d" 2>/dev/null && [ -w "$d" ]; then
            TARGET_DIR="$d"
            SUDO=""
            break
        fi
        if choose_privilege "$d"; then
            TARGET_DIR="$d"
            break
        fi
    done
    if [ -z "$TARGET_DIR" ]; then
        TARGET_DIR="$HOME/.local/bin"
        SUDO=""
        mkdir -p "$TARGET_DIR" 2>/dev/null || true
    fi
    TARGET="$TARGET_DIR/$BIN_NAME"
fi

# ---------------------------------------------------------------------------
# What is about to happen
# ---------------------------------------------------------------------------

if [ -n "$INSTALLED_VERSION" ]; then
    case "$(version_cmp "$VERSION" "$INSTALLED_VERSION")" in
        0)
            if [ "$FORCE" -eq 1 ]; then
                log "sshm $VERSION is already the latest release. Reinstalling because --force was given."
            else
                log "sshm $VERSION is already the latest release."
                exit 0
            fi
            ;;
        -1)
            # Deliberate: install.sh is the explicit route. Someone who just
            # piped a script into a shell has already opted into "make this
            # the release". The downgrade is shown, not blocked — this is the
            # point where it differs from `sshm update`, whose strict-greater
            # rule exists precisely because that command is not explicit.
            log "sshm $INSTALLED_VERSION → $VERSION (this is a downgrade from the version installed)"
            ;;
        1) log "sshm $INSTALLED_VERSION → $VERSION" ;;
        *) log "Updating sshm to $VERSION (the installed version could not be read)" ;;
    esac
elif [ "$HAD_EXISTING" -eq 1 ]; then
    # Something is installed but would not say what. The version is an
    # optimisation for the message, never a gate on the install.
    log "Updating sshm to $VERSION (the installed version could not be read)"
else
    log "Installing sshm $VERSION"
fi

log "Target: $TARGET"

# ---------------------------------------------------------------------------
# Download, verify, extract
# ---------------------------------------------------------------------------

ARCHIVE="$TMP_DIR/$BIN_NAME.tar.gz"
log "Downloading $DOWNLOAD_URL"
if ! curl -fL --retry 3 --progress-bar "$DOWNLOAD_URL" -o "$ARCHIVE"; then
    die "Download failed. Nothing was installed."
fi

# GitHub computes a sha256 for every release asset and returns it in the same
# response used to pick the asset, so this costs no extra request and nothing
# new needs publishing. A truncated or corrupt download is caught here,
# before a single byte of it reaches the target directory.
if [ -n "$EXPECTED_SHA" ]; then
    ACTUAL_SHA=$(sha256_of "$ARCHIVE" || echo "")
    if [ -z "$ACTUAL_SHA" ]; then
        warn "no sha256 tool found (sha256sum or shasum); the download was not verified."
    elif [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
        die "Checksum mismatch: expected $EXPECTED_SHA, got $ACTUAL_SHA. The download is corrupt or not the asset it claims to be. Nothing was installed."
    else
        log "Checksum verified."
    fi
else
    warn "GitHub published no digest for this asset; the download was not verified."
fi

log "Extracting..."
if ! tar -xzf "$ARCHIVE" -C "$TMP_DIR"; then
    die "Could not unpack the download. Nothing was installed."
fi

EXTRACTED="$TMP_DIR/$BIN_NAME"
if [ ! -f "$EXTRACTED" ]; then
    EXTRACTED=$(find "$TMP_DIR" -maxdepth 2 -type f -name "$BIN_NAME" -print -quit || true)
fi
if [ -z "$EXTRACTED" ] || [ ! -f "$EXTRACTED" ]; then
    die "The archive did not contain a $BIN_NAME binary. Nothing was installed."
fi

# Pre-swap verification: the artifact is checked while it is still harmless,
# so a wrong-asset or corrupt-archive case never reaches the target at all.
#
# This is the one call site that keeps the binary's stderr: a fresh
# download that cannot answer `--version` is not an ordinary failure like
# an unreadable old install — it is the wrong binary for this machine, and
# the binary itself says so. Carrying that text into the error message is
# what makes the #48 class of report self-diagnosing.
EXTRACTED_ERR="$TMP_DIR/extracted.stderr"
EXTRACTED_VERSION=$(read_installed_version "$EXTRACTED" "$EXTRACTED_ERR" || true)
if [ -z "$EXTRACTED_VERSION" ]; then
    BINARY_REASON=$(head -n 3 "$EXTRACTED_ERR" 2>/dev/null | tr '\n' ' ' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//' || true)
    if [ -n "$BINARY_REASON" ]; then
        BINARY_REASON="${BINARY_REASON%.}"
        die "The downloaded binary did not report a version. It said: $BINARY_REASON. Nothing was installed."
    fi
    die "The downloaded binary did not report a version. Nothing was installed."
fi
if [ "$EXTRACTED_VERSION" != "$VERSION" ]; then
    die "The downloaded binary reports $EXTRACTED_VERSION but the release is $VERSION. Nothing was installed."
fi

# ---------------------------------------------------------------------------
# The swap: stage beside the target, then rename
# ---------------------------------------------------------------------------
#
# Staging on the target's own filesystem makes the final move a rename(2):
# atomic, no partial-file window, and no ETXTBSY from copying into a binary
# that is currently running. `mv` from /tmp is a cross-device copy and gets
# all three of those wrong.

mkdir_p_target() {
    if [ ! -d "$TARGET_DIR" ]; then
        run mkdir -p "$TARGET_DIR"
    fi
}

mkdir_p_target

STAGED="$TARGET_DIR/.$BIN_NAME.new.$$"
if ! run cp -f "$EXTRACTED" "$STAGED"; then
    die "Could not stage the new binary in $TARGET_DIR. Nothing was installed."
fi
run chmod 0755 "$STAGED"

if ! run mv -f "$STAGED" "$TARGET"; then
    run rm -f "$STAGED" 2>/dev/null || true
    die "Could not replace $TARGET. Nothing was installed."
fi

# ---------------------------------------------------------------------------
# Post-swap verification
# ---------------------------------------------------------------------------

FINAL_VERSION=$(read_installed_version "$TARGET" || true)
if [ "$FINAL_VERSION" != "$VERSION" ]; then
    die "$TARGET reports '${FINAL_VERSION:-nothing}' but the release is $VERSION. The swap did not land where this shell will find it."
fi

log "Installed sshm $FINAL_VERSION at $TARGET"

# The single diagnostic that ends this whole class of report: the path we
# wrote and the path this shell resolves are not the same file.
RESOLVED_NOW=$(command -v "$BIN_NAME" 2>/dev/null || true)
if [ -n "$RESOLVED_NOW" ] && [ "$RESOLVED_NOW" != "$TARGET" ]; then
    warn "\`command -v $BIN_NAME\` resolves to $RESOLVED_NOW, which is not the binary just updated."
    warn "Open a new shell, or put $TARGET_DIR ahead of it in PATH."
fi

# ---------------------------------------------------------------------------
# PATH
# ---------------------------------------------------------------------------
#
# Never edit a shell rc file from a piped script. The user gets the exact line
# and owns the decision.

case ":$PATH:" in
    *":$TARGET_DIR:"*) : ;;
    *)
        log ""
        log "$TARGET_DIR is not on PATH for this shell. Add this line to your shell config:"
        log "  export PATH=\"$TARGET_DIR:\$PATH\""
        ;;
esac

# ---------------------------------------------------------------------------
# Completions — generated by the binary just installed, never by a PATH lookup
# ---------------------------------------------------------------------------

install_completions() {
    local target="$1" shell="${SHELL:-}"
    shell="${shell##*/}"
    local file

    case "$shell" in
        bash)
            file="$HOME/.bash_completion.d/$BIN_NAME"
            mkdir -p "$HOME/.bash_completion.d"
            ;;
        zsh)
            file="$HOME/.zsh/_$BIN_NAME"
            mkdir -p "$HOME/.zsh"
            ;;
        fish)
            file="$HOME/.config/fish/completions/$BIN_NAME.fish"
            mkdir -p "$HOME/.config/fish/completions"
            ;;
        *)
            return 0
            ;;
    esac

    if run_with_timeout 15 "$target" completions "$shell" >"$file" 2>/dev/null; then
        log "$shell completions installed to $file"
    else
        rm -f "$file" 2>/dev/null || true
        warn "could not generate $shell completions from $target; the binary itself is installed regardless."
    fi
}

install_completions "$TARGET"

# ---------------------------------------------------------------------------
# Other installs left untouched
# ---------------------------------------------------------------------------

if [ "${#CANDIDATES[@]}" -gt 1 ]; then
    log ""
    log "Other $BIN_NAME binaries were found and left untouched:"
    i=1
    while [ "$i" -lt "${#CANDIDATES[@]}" ]; do
        other="${CANDIDATES[$i]}"
        other_resolved=$(resolve_one_level "$other")
        other_version=$(read_installed_version "$other" || true)
        if [ "$other_resolved" != "$other" ]; then
            log "  $other -> $other_resolved${other_version:+  (sshm $other_version)}"
        else
            log "  $other${other_version:+  (sshm $other_version)}"
        fi
        i=$((i + 1))
    done
    log "Only $TARGET was replaced."
fi
