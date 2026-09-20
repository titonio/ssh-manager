# SSH Manager (sshm)

A terminal SSH connection manager that draws an **inline frame** inside your own
shell — no alternate screen, no takeover — built with Rust and Ratatui.

![sshm driven from a real shell: the inline picker opens below the prompt, filters the fleet as you type, collapses to a single settle line, then manage mode adds a connection through its field map and confirms a delete in place.](demo/hero.gif)

## Features

- **Connection Management**: Add, edit, and remove SSH server connections
- **Fuzzy Search**: Quickly find connections using fuzzy matching
- **Import from SSH Config**: Automatically import existing connections from `~/.ssh/config`
- **Organized by Folders**: Group connections into folders for better organization
- **Custom SSH Keys**: Support for custom private key paths
- **Non-Standard Ports**: Configure custom SSH ports (default: 22)
- **Inline Picker**: Filter-as-you-type selector triggered from the shell (Ctrl+Alt+S, or `**<TAB>` in zsh)
- **Shell Integration**: `sshm init zsh|bash` emits ZLE/widget scripts with trigger bindings
- **Updates You Can Apply**: `sshm check-update` asks whether a newer release exists; `sshm update` downloads it and replaces the installed binary in place
- **Transparent Frame**: A Clack-grammar frame (`◆` step, `│` rail, `❯` cursor) that borrows your terminal's own background — no alternate screen, no painted panel

## Installation

### One-Line Installation

```bash
curl -sL https://raw.githubusercontent.com/titonio/ssh-manager/master/install.sh | bash
```

Run the same command again later to update. The installer replaces the `sshm`
you are *already running* — the one your `PATH` resolves, following one level
of symlink exactly the way `sshm update` does — instead of dropping a second
copy in a directory you were never running it from, which is how "I updated
and nothing changed" happens. It prints the transition as
`sshm 0.1.12 → 0.1.13`, checks the download against the sha256 GitHub
publishes with the release, and swaps the binary with an atomic rename staged
beside the target.

If you are already on the latest release it says so and downloads nothing.
To reinstall anyway:

```bash
curl -sL https://raw.githubusercontent.com/titonio/ssh-manager/master/install.sh | bash -s -- --force
```

It never edits a shell rc file. If the install directory is not on your
`PATH`, or a second `sshm` is shadowing the one it replaced, the script names
both paths and tells you what to do about each.

### Manual Installation

#### From Source (Requires Rust)

```bash
cd sshm
cargo install --path .
```

#### From Releases

Download the pre-built binary for your platform from the [GitHub Releases](https://github.com/titonio/ssh-manager/releases) page.

## Usage

### Running the Application

```bash
sshm
```

Bare `sshm` opens the inline frame; Enter runs `ssh` against the selected
Connection.

### Command Line Options

- `-c, --check-update`: Check for updates without opening the frame

### Subcommands

Three commands open the same inline frame and differ only in what Enter means
(the `--emit` axis); the rest draw nothing.

| Command | Enter / effect |
|---------|----------------|
| `sshm` (bare) | Executes `ssh` against the selected Connection |
| `sshm pick` | Writes the chosen **alias** to stdout, for the shell to insert. Under `$(sshm pick)` the pipe carries only the alias — the frame is drawn to `/dev/tty` |
| `sshm manage` | Opens the frame with the manage header and routes the selection to the edit path (the edit itself is #36/#37; nothing is written yet) |
| `sshm add` | Add a new SSH connection without the frame |
| `sshm init zsh\|bash` | Emit the shell integration script |
| `sshm completions <shell>` | Generate shell completion scripts |
| `sshm check-update` | Ask whether a newer release exists, and name `sshm update` as the way to install it. Downloads nothing, writes nothing |
| `sshm update` | Replace the installed binary with the latest release: prints `sshm 0.1.12 → 0.1.13`, never prompts, never opens a frame. Refuses a Dev Build, and reports the `install.sh` route when the install directory is not writable |

`Esc` / `Ctrl-C` cancels any frame, leaves the shell buffer untouched, and
exits 130.

### Shell Integration (Inline Picker)

Source the init script to bind an inline fuzzy picker that lets you select an SSH
connection and insert its alias onto the command line without executing it:

```bash
# Zsh (recommended)
eval "$(sshm init zsh)"
# Now press Ctrl+Alt+S or type ** and press Tab to open the picker

# Bash
eval "$(sshm init bash)"
# Now press Ctrl+Alt+S to open the picker
```

#### Trigger key: Ctrl+Alt+S

Opens the inline picker seeded with the current buffer text as a filter query.
Pick a connection → the chosen alias is inserted at the cursor. Cancel (Esc/Ctrl-C)
→ the buffer is left untouched.

Override the bind key via the `SSHM_BIND_KEY` environment variable (zsh notation
for zsh, readline notation for bash):

```bash
# Zsh: bind to Ctrl+X instead
SSHM_BIND_KEY='^X' eval "$(sshm init zsh)"

# Bash: bind to Ctrl+\ instead
SSHM_BIND_KEY='\C-\\' eval "$(sshm init bash)"
```

Suppress all bind lines (emit only the widget function) with `SSHM_NO_BIND=1`
or the `--no-bind` flag:

```bash
eval "$(sshm init zsh --no-bind)"
# Manually bind the widget later:
# bindkey '^S' _sshm_inline_picker
```

#### Completion trigger: `**<TAB>` (zsh only)

Type `**` at the end of the buffer and press Tab to open the picker, seeded with
the buffer minus the `**`. Pick a connection → the chosen alias is inserted at
the cursor. When the trigger token isn't present, normal zsh completion runs
unchanged.

This works alongside Ctrl+Alt+S — the bound key remains the primary entry,
`**<TAB>` is the completion-style alternative.

The Tab binding is suppressed under `SSHM_NO_BIND=1`.

#### `**<TAB>` in bash: not bound

`**<TAB>` is zsh-only. In bash, `sshm init bash` never rebinds Tab: readline
cannot chain from a `bind -x` function back to normal completion, so binding
Tab would kill completion for the whole session. Bash's Tab completion is left
exactly as it was; use Ctrl+Alt+S as the entry point in bash.

### Shell Completion

Generate shell completion scripts for bash, zsh, or PowerShell:

```bash
# Bash
sshm completions bash > /etc/bash_completion.d/sshm

# Zsh
sshm completions zsh > ~/.zsh/_sshm
# Then add to ~/.zshrc: fpath=(~/.zsh $fpath) && autoload -U compinit && compinit

# Fish
sshm completions fish > ~/.config/fish/completions/sshm.fish

# PowerShell
sshm completions powershell | Out-String | Invoke-Expression
```

### Keyboard Shortcuts

There is one surface, so there is one key set. The frame reads these keys in
every mode; anything else is ignored.

| Key | Action |
|-----|--------|
| any printable character | Append to the fuzzy query (filter as you type) |
| `Backspace` | Remove the last query character |
| `↑` / `k` | Move the cursor up |
| `↓` / `j` | Move the cursor down |
| `Enter` | Resolve the selection along the command's emit axis: run `ssh`, insert the alias, or route to the edit path |
| `Esc` / `Ctrl-C` | Cancel, leave the shell buffer untouched (exit 130) |

The management chords the design calls for (`Ctrl+A` / `Ctrl+E` / `Ctrl+X`) are
#36's work — no handler reads them yet, so the frame hints nothing that names
them.

Opening the frame from the shell:

| Trigger | Action |
|---------|--------|
| `Ctrl+Alt+S` | Open the frame seeded with the current buffer as the query |
| `**<TAB>` (zsh) | Open the frame with the buffer minus `**` as the query; without `**`, normal zsh completion runs unchanged |

## Configuration

### Config File Location

The application stores its configuration in:
- **Linux**: `~/.ssh/connections.json`
- **macOS**: `~/.ssh/connections.json`
- **Windows**: `%USERPROFILE%\.ssh\connections.json`

### Config File Format

```json
{
  "connections": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "alias": "production-server",
      "host": "server.example.com",
      "user": "admin",
      "port": 22,
      "key_path": "~/.ssh/id_ed25519",
      "folder": "production"
    }
  ]
}
```

### Connection Fields

| Field | Description | Required |
|-------|-------------|----------|
| `alias` | Friendly name for the connection | Yes |
| `host` | Server hostname or IP | Yes |
| `user` | SSH username | Yes |
| `port` | SSH port (default: 22) | No |
| `key_path` | Path to private key | No |
| `folder` | Folder/group for organization | No |

## Architecture

### Project Structure

```
ssh-manager/
├── sshm/                    # The crate
│   ├── src/
│   │   ├── main.rs          # Entry point, CLI dispatch, shell init script generation
│   │   ├── lib.rs           # Crate root — the modules the tests drive
│   │   ├── frame.rs         # The inline frame view-model (Clack grammar + fit geometry)
│   │   ├── inline.rs        # The inline render + settle-collapse driver
│   │   ├── emit.rs          # The emit axis: what Enter means per command
│   │   ├── theme.rs         # Colour tokens (Clack palette, colour support, ANSI serializer)
│   │   ├── config.rs        # Configuration management
│   │   ├── connections.rs   # The Connection manager: add, edit, delete, import
│   │   ├── ssh.rs           # SSH command building and execution
│   │   └── update.rs        # Update checking
│   ├── Cargo.toml           # Rust dependencies
│   └── tests/               # Integration tests — see sshm/tests/README.md
├── demo/
│   ├── hero.tape            # The README reel (vhs)
│   ├── fixtures/            # The connections the reel is recorded against
│   └── rendered/            # Stills pulled from the design/live reels
├── scripts/
│   ├── render-hero.sh       # Records demo/hero.gif under a sandboxed HOME
│   └── design-matrix.sh    # Capability tiers (60 cols, NO_COLOR)
├── .dsh/skills/sshm-design/ # Binding UI design rules (tokens, surfaces, verification)
├── .github/
│   └── workflows/
│       └── ci.yml           # CI/CD pipeline
├── AGENTS.md                # Agent skills index
├── CONTEXT.md               # Domain language and glossary
└── README.md                # This file
```

There is no `app.rs`, `picker.rs` or `runtime.rs`: the fullscreen TUI and the
old opaque picker were deleted in the #35 cut-over, and their live
responsibilities now sit in `frame.rs` (the view-model), `inline.rs` (the
driver) and `emit.rs` (the emit axis).

### Dependencies

- **ratatui** - TUI framework
- **crossterm** - Terminal manipulation
- **serde/serde_json** - Configuration serialization
- **dirs** - Platform directory detection
- **fuzzy-matcher** - Fuzzy search functionality
- **uuid** - Unique connection identifiers
- **clap + clap_complete** - CLI parsing and shell completion generation
- **self-github-update-enhanced** - Auto-update from GitHub
- **insta** (dev) - Snapshot testing

## Testing

### Run Tests

```bash
cd sshm
cargo test
```

### Run with Coverage

```bash
cargo llvm-cov --all-features --lcov --output-path lcov.info
```

### Code Quality

```bash
# Check formatting
cargo fmt -- --check

# Run clippy lints
cargo clippy --all-targets --all-features
```

### Regenerating the README GIF

```bash
scripts/render-hero.sh            # -> demo/hero.gif
```

The GIF is recorded with [vhs](https://github.com/charmbracelet/vhs) from
`demo/hero.tape`, driving the **real** binary — not a capture of static
frames. Because bare `sshm` executes `ssh` on Enter and `sshm manage` writes
the real store, the wrapper runs the whole reel under a throwaway `HOME`
reset from `demo/fixtures/connections.json`, so your own `~/.ssh` is never
opened and every render starts from the same fixture. It then asserts the
add/delete round trip and fails if the store does not end where it began.

## CI/CD

The project uses GitHub Actions for:
- Code formatting checks
- Clippy linting
- Unit testing
- Coverage reporting (80% threshold)
- Security auditing
- Installer linting (`shellcheck`) and a fixture-driven smoke test of the update path
- Automatic releases on version tags

### Before releasing a version that touches `sshm update`

No CI job exercises the **in-app** replace path — a runner that swapped the
binary of its own checkout would prove nothing about an installed one — so
`sshm update` is verified by hand, per `docs/adr/0002`. The `install.sh`
route is the opposite case: it runs against a stubbed release API in
`tests/install/smoke.sh`, so its shadowed, symlinked, unwritable and
already-latest branches are covered in CI and need no hand run here.
Run each row, then `sshm --version`, and
paste the results into the release notes:

| Install | Run |
|---|---|
| macOS, Apple Silicon | `which sshm` → `sshm update` → `sshm --version` |
| macOS, Intel | same |
| Linux, from `/usr/local/bin` | same |
| Linux, from `~/.local/bin` | same |

In every row `which sshm` must be unchanged, `sshm --version` must report the new
version, the binary's permission bits must be the ones it started with, and no
`.sshm.__temp__*` file may be left beside it. macOS is the row to watch: the swap
copies bytes, so a byte-identical binary keeps the signature it already carries
and nothing re-signs it.

A binary at 0.1.12 or older has no working update path and cannot acquire one
in-app, so the release note must say plainly: run `install.sh` once.

## Version History

- **0.1.13** - `sshm update`: a real Apply Update. It replaces the installed binary in place; `sshm check-update` is now purely a question and downloads nothing; the update surfaces say only what is true. **A binary at 0.1.12 or older has no working update path — run `install.sh` once to get this one; no in-app update can reach it**
- **0.1.12** - fixed shell init: normal TAB completion survives `sshm init zsh` (non-dot fallthrough), and bash never rebinds TAB
- **0.1.11** - the new inline Clack-style frame (add, edit, delete, pick), a design-token theme layer, cached update notes, and a rustls security bump
- **0.1.10** - `**<TAB>` completion trigger, bash support for `sshm init`, `--no-bind` flag, and serialized test fixes
- **0.1.9** - Inline picker, shell init scripts (zsh + bash), and `**<TAB>` completion trigger
- **0.1.5** - Update functionality
- **0.1.0** - Initial release

## License

This project is open source. See the repository for license details.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Support

For issues and feature requests, please use the [GitHub Issues](https://github.com/titonio/ssh-manager/issues) page.
