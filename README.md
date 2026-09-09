# SSH Manager (sshm)

A modern Terminal User Interface (TUI) for managing SSH connections, built with Rust and Ratatui.

## Features

- **Connection Management**: Add, edit, and remove SSH server connections
- **Fuzzy Search**: Quickly find connections using fuzzy matching
- **Import from SSH Config**: Automatically import existing connections from `~/.ssh/config`
- **Organized by Folders**: Group connections into folders for better organization
- **Custom SSH Keys**: Support for custom private key paths
- **Non-Standard Ports**: Configure custom SSH ports (default: 22)
- **Inline Picker**: Filter-as-you-type selector triggered from the shell (Ctrl+Alt+S or `**<TAB>`)
- **Shell Integration**: `sshm init zsh|bash` emits ZLE/widget scripts with trigger bindings
- **Automatic Updates**: Built-in update checker with GitHub release integration
- **Clack-style UI**: Inline prompt frames (◆ header, dim │ rail, ❯ cursor) modeled on @clack/prompts and vercel-labs skills — no boxes, no background colors, foreground-only palette

## Installation

### One-Line Installation

```bash
curl -sL https://raw.githubusercontent.com/titonio/ssh-manager/master/install.sh | bash
```

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

### Command Line Options

- `-c, --check-update`: Check for updates without running the TUI

### Subcommands

- `add`: Add a new SSH connection
- `completions`: Generate shell completion scripts
- `check-update`: Check for updates
- `init`: Generate shell initialization scripts (zsh, bash) for the inline picker widget
- `pick`: Open an inline fuzzy picker to select a Connection (inserts `ssh` command, doesn't execute)

### Shell Integration (Inline Picker)

Source the init script to bind an inline fuzzy picker that lets you select an SSH
connection and insert its `ssh` command onto the command line without executing it:

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
Pick a connection → the `ssh` command is inserted at the cursor. Cancel (Esc/Ctrl-C)
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

Type `**` at the end of the buffer and press Tab to open the picker (showing all
connections). Pick a connection → the `ssh` command is inserted at the cursor.
When the trigger token isn't present, normal zsh completion runs unchanged.

This works alongside Ctrl+Alt+S — the bound key remains the primary entry,
`**<TAB>` is the completion-style alternative.

The Tab binding is suppressed under `SSHM_NO_BIND=1`.

#### Bash caveat for `**<TAB>`

When sourced in bash, `**<TAB>` opens the picker only when `**` is present at the
end of `READLINE_LINE`. Without `**`, Tab has no effect (bash cannot chain from
`bind -x` into normal completion).

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

#### Fullscreen TUI

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Navigate connection list |
| `Enter` | Connect to selected server |
| `a` | Add new connection |
| `e` | Edit selected connection |
| `d` | Delete selected connection |
| `f` | Filter/search connections |
| `i` | Import from ~/.ssh/config |
| `q` | Quit application |
| `?` | Show help |

#### Shell Inline Picker

| Trigger | Action |
|---------|--------|
| `Ctrl+Alt+S` | Open picker (seeded with buffer as query) |
| `**<TAB>` (zsh) | Open picker (showing all connections) |
| `Enter` in picker | Insert selected `ssh` command at cursor |
| `Esc`/`Ctrl-C` in picker | Cancel, leave buffer untouched |
| `↑`/`↓` or `j`/`k` in picker | Navigate connections in picker |

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
├── sshm/                    # Main application
│   ├── src/
│   │   ├── main.rs          # Entry point, CLI dispatch, init script generation
│   │   ├── app.rs           # TUI application logic
│   │   ├── config.rs        # Configuration management
│   │   ├── picker.rs        # Inline fuzzy picker (ratatui) + ssh command builder
│   │   ├── runtime.rs       # Runtime and cleanup
│   │   ├── ssh.rs           # SSH connection handling
│   │   └── update.rs        # Update checking
│   ├── Cargo.toml           # Rust dependencies
│   └── tests/               # Unit tests
├── .github/
│   └── workflows/
│       └── ci.yml           # CI/CD pipeline
├── CONTEXT.md               # Domain language and glossary
├── TUI_DESIGN_GUIDELINES.md # TUI design documentation
└── README.md                # This file
```

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

## CI/CD

The project uses GitHub Actions for:
- Code formatting checks
- Clippy linting
- Unit testing
- Coverage reporting (80% threshold)
- Security auditing
- Automatic releases on version tags

## Version History

- **0.1.10** - Current version with `**<TAB>` completion trigger, bash support for `sshm init`, `--no-bind` flag, and serialized test fixes
- **0.1.9** - Inline picker, shell init scripts (zsh + bash), and `**<TAB>` completion trigger
- **0.1.5** - Update functionality
- **0.1.0** - Initial release

## License

This project is open source. See the repository for license details.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Support

For issues and feature requests, please use the [GitHub Issues](https://github.com/titonio/ssh-manager/issues) page.
