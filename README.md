# SSH Manager (sshm)

A modern Terminal User Interface (TUI) for managing SSH connections, built with Rust and Ratatui.

## Features

- **Connection Management**: Add, edit, and remove SSH server connections
- **Fuzzy Search**: Quickly find connections using fuzzy matching
- **Import from SSH Config**: Automatically import existing connections from `~/.ssh/config`
- **Organized by Folders**: Group connections into folders for better organization
- **Custom SSH Keys**: Support for custom private key paths
- **Non-Standard Ports**: Configure custom SSH ports (default: 22)
- **Automatic Updates**: Built-in update checker with GitHub release integration
- **Nord Theme**: Beautiful Nordic-inspired color scheme

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
│   │   ├── main.rs          # Entry point
│   │   ├── app.rs           # TUI application logic
│   │   ├── config.rs        # Configuration management
│   │   ├── runtime.rs       # Runtime and cleanup
│   │   ├── ssh.rs           # SSH connection handling
│   │   └── update.rs        # Update checking
│   ├── Cargo.toml           # Rust dependencies
│   └── tests/               # Unit tests
├── .github/
│   └── workflows/
│       └── ci.yml           # CI/CD pipeline
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
- **self-github-update-enhanced** - Auto-update from GitHub

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

- **0.1.5** - Current version with update functionality
- **0.1.0** - Initial release

## License

This project is open source. See the repository for license details.

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.

## Support

For issues and feature requests, please use the [GitHub Issues](https://github.com/titonio/ssh-manager/issues) page.
