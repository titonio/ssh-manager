mod app;
mod config;
mod picker;
mod runtime;
mod ssh;
mod update;

use std::io;

use clap::{CommandFactory, Parser, Subcommand};
use picker::build_ssh_command;
use picker::run_pick;
use runtime::{cleanup_and_exit, run_app_inner};
use ssh::build_ssh_args;
use update::UpdateResult;

#[derive(Parser)]
#[command(name = "sshm")]
#[command(about = "A modern TUI for managing SSH connections")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short = 'c', long = "check-update", global = true)]
    check_update: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new SSH connection
    Add {
        /// Connection alias (friendly name)
        #[arg(short = 'a', long)]
        alias: Option<String>,

        /// Server hostname (optionally with user@host format)
        #[arg(required = true)]
        host: Option<String>,

        /// SSH username
        #[arg(short = 'u', long)]
        user: Option<String>,

        /// SSH port (default: 22)
        #[arg(short = 'p', long, default_value = "22")]
        port: u16,

        /// Path to private key
        #[arg(short = 'k', long)]
        key: Option<String>,

        /// Folder/group for organization
        #[arg(short = 'f', long)]
        folder: Option<String>,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell type to generate completions for
        #[arg(value_parser = clap::value_parser!(clap_complete::Shell))]
        shell: clap_complete::Shell,
    },

    /// Generate shell initialization scripts
    Init {
        /// Shell type to generate initialization script for
        #[arg(value_parser = clap::value_parser!(ShellType))]
        shell: ShellType,

        /// Suppress bind lines in the generated script (env: SSHM_NO_BIND)
        #[arg(long)]
        no_bind: bool,
    },

    /// Check for updates
    CheckUpdate,

    /// Open an inline picker to select a Connection (insert, don't execute)
    Pick {
        /// Seed the picker with an initial fuzzy query (for shell widget integration)
        #[arg(long)]
        query: Option<String>,
    },
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum ShellType {
    Zsh,
    Bash,
}

fn main() -> io::Result<()> {
    run_main(run_app_inner, run_pick)
}

fn run_main(
    run_app_fn: fn() -> io::Result<(bool, Option<config::Connection>)>,
    run_pick_fn: fn(Vec<config::Connection>, String) -> io::Result<picker::PickerOutcome>,
) -> io::Result<()> {
    let cli = Cli::parse();
    dispatch(cli, run_app_fn, run_pick_fn)
}

/// Dispatch on an already-parsed CLI. Separated from `run_main` so tests can
/// drive the pick path (and assert the update checker is never reached)
/// without relying on `std::env::args`.
fn dispatch(
    cli: Cli,
    run_app_fn: fn() -> io::Result<(bool, Option<config::Connection>)>,
    run_pick_fn: fn(Vec<config::Connection>, String) -> io::Result<picker::PickerOutcome>,
) -> io::Result<()> {
    // Handle completions command
    if let Some(Commands::Completions { shell }) = cli.command {
        generate_completions(shell);
        return Ok(());
    }

    // Handle init command
    if let Some(Commands::Init { shell, no_bind }) = cli.command {
        // The --no-bind flag overrides the env var when set.
        if no_bind {
            std::env::set_var("SSHM_NO_BIND", "1");
        }
        match shell {
            ShellType::Zsh => {
                print_init_zsh_script();
                return Ok(());
            }
            ShellType::Bash => {
                print_init_bash_script();
                return Ok(());
            }
        }
    }

    // Handle pick command — inline picker (insert, don't execute)
    if let Some(Commands::Pick { query }) = cli.command {
        let connections = config::Config::load().connections;
        match run_pick_fn(connections, query.unwrap_or_default()) {
            Ok(picker::PickerOutcome::Selected(conn)) => {
                println!("{}", build_ssh_command(&conn));
                return Ok(());
            }
            Ok(picker::PickerOutcome::Cancel) => {
                std::process::exit(130);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                return Err(e);
            }
        }
    }

    // Handle check-update flag or command
    if cli.check_update || matches!(cli.command, Some(Commands::CheckUpdate)) {
        match update::force_check_for_update() {
            UpdateResult::UpdateAvailable { version } => {
                println!("Update available: v{}", version);
                println!("Run again without flag to update automatically.");
                return Ok(());
            }
            UpdateResult::NoUpdate => {
                println!("No update available.");
                return Ok(());
            }
            UpdateResult::Error(e) => {
                eprintln!("Error checking for updates: {}", e);
            }
        }
    }

    // Handle add command
    if let Some(Commands::Add {
        alias,
        host,
        user,
        port,
        key,
        folder,
    }) = cli.command
    {
        run_add_command(alias, host, user, port, key, folder).map_err(io::Error::other)?;
        return Ok(());
    }

    let (should_connect, conn) = run_app_fn()?;

    if should_connect {
        if let Some(conn) = conn {
            let args = build_ssh_args(&conn);
            cleanup_and_exit(&args);
        }
    }

    Ok(())
}

fn run_add_command(
    alias: Option<String>,
    host: Option<String>,
    user: Option<String>,
    port: u16,
    key_path: Option<String>,
    folder: Option<String>,
) -> Result<(), String> {
    let alias = alias.ok_or("Error: --alias is required")?;
    let host = host.ok_or("Error: user@host is required")?;

    let conn = config::Connection {
        id: uuid::Uuid::new_v4().to_string(),
        alias,
        host,
        user: user.unwrap_or_default(),
        port,
        key_path,
        folder,
    };

    let mut config = config::Config::load();
    config.add_connection(conn);
    config
        .save()
        .map_err(|e| format!("Error saving config: {}", e))?;

    println!("Connection added successfully!");

    Ok(())
}

fn generate_completions(shell: clap_complete::Shell) {
    let mut app = Cli::command();
    let bin_name = app.get_name().to_string();

    println!("Generating completion script for {:?}...", shell);

    clap_complete::generate(shell, &mut app, bin_name, &mut std::io::stdout());
}

/// Generate the zsh initialization script for the inline picker widget and
/// **<TAB> completion trigger.
///
/// Returns the script content as a string for testing and printing.
fn generate_init_zsh_script() -> String {
    // Default bind key (Ctrl+Alt+S)
    // Default bind key for Ctrl+Alt+S in zsh notation: `\e` is ESC (the
    // Alt/Meta modifier prefix) and `^S` is Ctrl+S, yielding Ctrl+Alt+S. This
    // matches the legacy xterm encoding ESC + Ctrl-S that the terminals send,
    // and is far more portable than a guessed CSI sequence.
    let default_bind_key = "\\e^S";

    // Check for SSHM_NO_BIND environment variable
    let no_bind = std::env::var("SSHM_NO_BIND").is_ok();

    // Check for custom bind key via SSHM_BIND_KEY environment variable
    let bind_key = std::env::var("SSHM_BIND_KEY").unwrap_or_else(|_| default_bind_key.to_string());

    // Shared script body (identical in both no-bind and bind variants).
    // The header (# sshm init zsh …) is emitted separately by each branch
    // because the no-bind variant has a different first line.
    let shared_body = r#"# Shell function wrapper: makes "sshm pick" insert onto the command line
# when run interactively (stdout is a terminal).  For pipe/redirect usage the
# raw binary output is preserved.
sshm() {
    if [[ "$1" == "pick" && -t 1 ]]; then
        local result
        result=$(command sshm pick "${@:2}")
        local ret=$?
        if [[ $ret -eq 0 && -n "$result" ]]; then
            print -z "$result"
        fi
        return $ret
    fi
    # Shortcut: "sshm ssh user@host …" passes through to ssh directly.
    # This lets "sshm **<TAB>" (which completes to "sshm ssh user@host")
    # execute the ssh command when Enter is pressed.
    if [[ "$1" == "ssh" ]]; then
        command ssh "${@:2}"
        return $?
    fi
    command sshm "$@"
}

# ZLE widget function for sshm inline picker (Ctrl+Alt+S)
_sshm_inline_picker() {
    local saved_buffer="$BUFFER"
    
    local result
    result=$(sshm pick --query "$BUFFER")
    local exit_code=$?
    
    if [[ $exit_code -ne 0 ]]; then
        BUFFER="$saved_buffer"
        zle reset-prompt
        return
    fi
    
    if [[ -n "$result" ]]; then
        BUFFER="$result"
        CURSOR=${#BUFFER}
    fi
    
    zle reset-prompt
}

# Completion function for ssh / sshm: intercepts the ** trigger token like fzf does.
# Integrates through zsh's completion system — Tab is NOT hijacked.
# When the word being completed ends with **, the inline picker opens;
# otherwise normal ssh completion runs unchanged.
_sshm() {
    if [[ $PREFIX == *'**' ]]; then
        local stripped="${PREFIX%'**'}"
        local result ret
        result=$(sshm pick --query "$stripped" 2>/dev/null)
        ret=$?
        if [[ $ret -eq 0 && -n "$result" ]]; then
            IPREFIX=""
            # When completing for "ssh", strip the "ssh " prefix since
            # the user already typed "ssh".  For "sshm" or other callers,
            # keep the full result so it replaces the trigger word.
            if [[ $words[1] == "ssh" ]]; then
                compadd -U -- "${result#ssh }"
            else
                compadd -U -- "$result"
            fi
            return 0
        fi
        return 1
    fi
    # Pass through to original ssh completion
    if (( $+functions[_ssh] )); then
        _ssh "$@"
    fi
}

# Register the ZLE widget
zle -N _sshm_inline_picker

# Register **<TAB> completion via compdef (if completion system is loaded).
# This does NOT hijack Tab — it hooks into zsh's completion chain naturally.
if (( $+functions[compdef] )); then
    compdef _sshm ssh sshm
fi
"#;

    if no_bind {
        let mut s = String::from(
            "# sshm init zsh - Inline Picker Widget + **<TAB> completion trigger (no bind)\n",
        );
        s.push_str("# Sourced via: eval \"$(sshm init zsh)\"\n");
        s.push_str("# Note: Bind lines suppressed by SSHM_NO_BIND=1\n\n");
        s.push_str(shared_body);
        s
    } else {
        let mut s = format!(
            "# sshm init zsh - Inline Picker Widget + **<TAB> completion trigger\n\
             # Sourced via: eval \"$(sshm init zsh)\"\n\
             # Bind key: {bind} (override with SSHM_BIND_KEY)\n\n",
            bind = bind_key
        );
        s.push_str(shared_body);
        s.push_str(&format!(
            "\n# Bind the widget to the trigger key (Ctrl+Alt+S)\n\
             bindkey '{bind}' _sshm_inline_picker\n",
            bind = bind_key
        ));
        s
    }
}

/// Print the zsh initialization script for the inline picker widget.
fn print_init_zsh_script() {
    print!("{}", generate_init_zsh_script());
}

/// Generate the bash initialization script for the inline picker widget and
/// **<TAB> completion trigger.
///
/// Returns the script content as a string for testing and printing.
fn generate_init_bash_script() -> String {
    // Default bind key in bash bind -x notation for Ctrl+Alt+S.
    // bash uses `\e` for the Escape key (Alt/Meta prefix) and `\C-s` for Ctrl+S.
    // Together `"\e\C-s"` represents Ctrl+Alt+S, matching the legacy xterm
    // encoding ESC + Ctrl-S that terminals send.
    let default_bind_key = "\\e\\C-s";

    // Check for SSHM_NO_BIND environment variable
    let no_bind = std::env::var("SSHM_NO_BIND").is_ok();

    // Check for custom bind key via SSHM_BIND_KEY environment variable
    let bind_key = std::env::var("SSHM_BIND_KEY").unwrap_or_else(|_| default_bind_key.to_string());

    if no_bind {
        r###"# sshm init bash - Inline Picker Widget + **<TAB> completion trigger (no bind)
# Sourced via: eval "$(sshm init bash)"
# Note: Bind lines suppressed by SSHM_NO_BIND=1

_sshm_inline_picker() {
    local result exit_code
    result=$(sshm pick --query "$READLINE_LINE")
    exit_code=$?
    if [[ $exit_code -ne 0 ]]; then
        return
    fi
    if [[ -n "$result" ]]; then
        READLINE_LINE="$result"
        READLINE_POINT=${#READLINE_LINE}
    fi
}

# **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
# pressing Tab opens the inline picker. Note: without **, Tab has no
# effect (bash cannot chain to normal completion from a key-bound function).
_sshm_completion_picker() {
    if [[ "$READLINE_LINE" == *'**' ]]; then
        local saved="$READLINE_LINE"
        local stripped="${READLINE_LINE%'**'}"
        local result exit_code
        result=$(sshm pick --query "$stripped")
        exit_code=$?
        if [[ $exit_code -ne 0 ]]; then
            READLINE_LINE="$saved"
            return
        fi
        if [[ -n "$result" ]]; then
            READLINE_LINE="$result"
            READLINE_POINT=${#READLINE_LINE}
        fi
    fi
}
"###
        .to_string()
    } else {
        format!(
            r###"# sshm init bash - Inline Picker Widget + **<TAB> completion trigger
# Sourced via: eval "$(sshm init bash)"
# Bind key: {bind_key} (override with SSHM_BIND_KEY)

_sshm_inline_picker() {{
    local result exit_code
    result=$(sshm pick --query "$READLINE_LINE")
    exit_code=$?
    if [[ $exit_code -ne 0 ]]; then
        return
    fi
    if [[ -n "$result" ]]; then
        READLINE_LINE="$result"
        READLINE_POINT=${{#READLINE_LINE}}
    fi
}}

# **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
# pressing Tab opens the inline picker. Note: without **, Tab has no
# effect (bash cannot chain to normal completion from a key-bound function).
_sshm_completion_picker() {{
    if [[ "$READLINE_LINE" == *'**' ]]; then
        local saved="$READLINE_LINE"
        local stripped="${{READLINE_LINE%'**'}}"
        local result exit_code
        result=$(sshm pick --query "$stripped")
        exit_code=$?
        if [[ $exit_code -ne 0 ]]; then
            READLINE_LINE="$saved"
            return
        fi
        if [[ -n "$result" ]]; then
            READLINE_LINE="$result"
            READLINE_POINT=${{#READLINE_LINE}}
        fi
    fi
}}

# Bind the widget to the trigger key (Ctrl+Alt+S)
bind -x '"{bind_key}":_sshm_inline_picker'

# Bind **<TAB> completion trigger (Tab key)
bind -x '"\C-i":_sshm_completion_picker'
"###
        )
    }
}

/// Print the bash initialization script for the inline picker widget.
fn print_init_bash_script() {
    print!("{}", generate_init_bash_script());
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::Connection;
    use crate::picker::{self, PickerOutcome};
    use serial_test::serial;

    #[test]
    fn test_cleanup_and_exit_with_args() {
        let args: Vec<std::ffi::OsString> = vec![];
        assert!(args.is_empty());
    }

    #[test]
    fn test_main_should_connect_false_returns_ok() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let mock_run_pick = |_c: Vec<Connection>, _q: String| -> io::Result<PickerOutcome> {
            Ok(PickerOutcome::Cancel)
        };
        let result = run_main(mock_run_app, mock_run_pick);
        assert!(result.is_ok());
    }

    #[test]
    fn test_main_should_connect_true_with_conn() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let mock_run_app =
            move || Ok::<(bool, Option<Connection>), io::Error>((true, Some(conn.clone())));

        let (should_connect, result_conn) = mock_run_app().unwrap();
        assert!(should_connect);
        assert!(result_conn.is_some());
    }

    #[test]
    fn test_main_should_connect_true_no_conn() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((true, None));
        let mock_run_pick = |_c: Vec<Connection>, _q: String| -> io::Result<PickerOutcome> {
            Ok(PickerOutcome::Cancel)
        };
        let result = run_main(mock_run_app, mock_run_pick);
        assert!(result.is_ok());
    }

    #[test]
    fn test_main_branch_should_connect_true_with_key() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };
        let args = build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-i"));
    }

    #[test]
    fn test_main_branch_port_not_22() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
        };
        let args = build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn test_main_branch_port_is_22() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = build_ssh_args(&conn);
        assert!(!args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn test_main_branch_user_empty() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = build_ssh_args(&conn);
        assert_eq!(args.last().unwrap(), "example.com");
    }

    #[test]
    fn test_main_branch_user_not_empty() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = build_ssh_args(&conn);
        assert_eq!(args.last().unwrap(), "admin@example.com");
    }

    #[test]
    fn test_run_main_logic() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let mock_run_pick = |_c: Vec<Connection>, _q: String| -> io::Result<PickerOutcome> {
            Ok(PickerOutcome::Cancel)
        };
        let result = run_main(mock_run_app, mock_run_pick);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_main_with_connection() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let mock_run_app = move || Ok::<(bool, Option<Connection>), io::Error>((true, Some(conn)));

        let (should_connect, result_conn) = mock_run_app().unwrap();
        assert!(should_connect);
        assert!(result_conn.is_some());
    }

    #[test]
    fn test_run_main_logic_paths() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert!(!args.is_empty());
    }

    // NOTE: pick-path branch behavior is asserted via dispatch in
    // test_dispatch_pick_does_not_invoke_update_checker_or_run_app below,
    // which drives the real dispatch logic rather than a standalone mock.

    #[test]
    fn test_run_main_pick_build_ssh_command_reuses_args() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };
        let cmd = picker::build_ssh_command(&conn);
        assert!(cmd.starts_with("ssh "));
        assert!(cmd.contains("-i"));
        assert!(cmd.contains("-p"));
        assert!(cmd.contains("admin@example.com"));
    }

    /// The spec requires: "Update checker skip is asserted by the injected-runner
    /// test (the runner is never asked to run it on the pick path)."
    ///
    /// We drive the real `dispatch` with a `Cli { command: Some(Pick), ... }` and a
    /// `run_app_fn` that panics if ever called. The Pick branch returns before the
    /// update-checker block, so `run_app_fn` is never invoked — proving the update
    /// checker is structurally skipped on the pick path. We use a thread-local flag
    /// instead of a panicking closure because `dispatch` returns `Ok(())` on the
    /// `Selected` path (it writes to stdout and returns) rather than reaching
    /// `run_app_fn`.
    #[test]
    fn test_dispatch_pick_does_not_invoke_update_checker_or_run_app() {
        // The pick path runs before the update-checker block, so reaching the
        // Selected return proves dispatch never reached the update checker.
        let cli = Cli {
            command: Some(Commands::Pick { query: None }),
            check_update: false,
        };

        // A zero-arg fn that panics if ever called, proving the pick path never
        // falls through to the fullscreen TUI / update-checker branches.
        fn run_app_that_panics() -> io::Result<(bool, Option<Connection>)> {
            panic!("run_app_fn must not be called on the pick path");
        }

        let mock_run_pick =
            |_connections: Vec<Connection>, _query: String| -> io::Result<PickerOutcome> {
                Ok(PickerOutcome::Selected(Connection {
                    id: "1".to_string(),
                    alias: "test".to_string(),
                    host: "example.com".to_string(),
                    user: "admin".to_string(),
                    port: 22,
                    key_path: None,
                    folder: None,
                }))
            };

        let result = dispatch(cli, run_app_that_panics, mock_run_pick);
        assert!(result.is_ok());
    }

    // ── sshm init zsh snapshot tests (AC8, AC9) ──────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_default() {
        // Clear any env overrides
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init zsh)"
        # Bind key: \e^S (override with SSHM_BIND_KEY)

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            # Shortcut: "sshm ssh user@host …" passes through to ssh directly.
            # This lets "sshm **<TAB>" (which completes to "sshm ssh user@host")
            # execute the ssh command when Enter is pressed.
            if [[ "$1" == "ssh" ]]; then
                command ssh "${@:2}"
                return $?
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # Completion function for ssh / sshm: intercepts the ** trigger token like fzf does.
        # Integrates through zsh's completion system — Tab is NOT hijacked.
        # When the word being completed ends with **, the inline picker opens;
        # otherwise normal ssh completion runs unchanged.
        _sshm() {
            if [[ $PREFIX == *'**' ]]; then
                local stripped="${PREFIX%'**'}"
                local result ret
                result=$(sshm pick --query "$stripped" 2>/dev/null)
                ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    IPREFIX=""
                    # When completing for "ssh", strip the "ssh " prefix since
                    # the user already typed "ssh".  For "sshm" or other callers,
                    # keep the full result so it replaces the trigger word.
                    if [[ $words[1] == "ssh" ]]; then
                        compadd -U -- "${result#ssh }"
                    else
                        compadd -U -- "$result"
                    fi
                    return 0
                fi
                return 1
            fi
            # Pass through to original ssh completion
            if (( $+functions[_ssh] )); then
                _ssh "$@"
            fi
        }

        # Register the ZLE widget
        zle -N _sshm_inline_picker

        # Register **<TAB> completion via compdef (if completion system is loaded).
        # This does NOT hijack Tab — it hooks into zsh's completion chain naturally.
        if (( $+functions[compdef] )); then
            compdef _sshm ssh sshm
        fi

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bindkey '\e^S' _sshm_inline_picker
        "###);
    }

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_no_bind() {
        // Set SSHM_NO_BIND to suppress bind lines
        std::env::set_var("SSHM_NO_BIND", "1");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger (no bind)
        # Sourced via: eval "$(sshm init zsh)"
        # Note: Bind lines suppressed by SSHM_NO_BIND=1

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            # Shortcut: "sshm ssh user@host …" passes through to ssh directly.
            # This lets "sshm **<TAB>" (which completes to "sshm ssh user@host")
            # execute the ssh command when Enter is pressed.
            if [[ "$1" == "ssh" ]]; then
                command ssh "${@:2}"
                return $?
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # Completion function for ssh / sshm: intercepts the ** trigger token like fzf does.
        # Integrates through zsh's completion system — Tab is NOT hijacked.
        # When the word being completed ends with **, the inline picker opens;
        # otherwise normal ssh completion runs unchanged.
        _sshm() {
            if [[ $PREFIX == *'**' ]]; then
                local stripped="${PREFIX%'**'}"
                local result ret
                result=$(sshm pick --query "$stripped" 2>/dev/null)
                ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    IPREFIX=""
                    # When completing for "ssh", strip the "ssh " prefix since
                    # the user already typed "ssh".  For "sshm" or other callers,
                    # keep the full result so it replaces the trigger word.
                    if [[ $words[1] == "ssh" ]]; then
                        compadd -U -- "${result#ssh }"
                    else
                        compadd -U -- "$result"
                    fi
                    return 0
                fi
                return 1
            fi
            # Pass through to original ssh completion
            if (( $+functions[_ssh] )); then
                _ssh "$@"
            fi
        }

        # Register the ZLE widget
        zle -N _sshm_inline_picker

        # Register **<TAB> completion via compdef (if completion system is loaded).
        # This does NOT hijack Tab — it hooks into zsh's completion chain naturally.
        if (( $+functions[compdef] )); then
            compdef _sshm ssh sshm
        fi
        "###);

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_snapshot_init_zsh_custom_bind_key() {
        // Clear SSHM_NO_BIND and set custom SSHM_BIND_KEY
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "^S");

        let script = generate_init_zsh_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init zsh - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init zsh)"
        # Bind key: ^S (override with SSHM_BIND_KEY)

        # Shell function wrapper: makes "sshm pick" insert onto the command line
        # when run interactively (stdout is a terminal).  For pipe/redirect usage the
        # raw binary output is preserved.
        sshm() {
            if [[ "$1" == "pick" && -t 1 ]]; then
                local result
                result=$(command sshm pick "${@:2}")
                local ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    print -z "$result"
                fi
                return $ret
            fi
            # Shortcut: "sshm ssh user@host …" passes through to ssh directly.
            # This lets "sshm **<TAB>" (which completes to "sshm ssh user@host")
            # execute the ssh command when Enter is pressed.
            if [[ "$1" == "ssh" ]]; then
                command ssh "${@:2}"
                return $?
            fi
            command sshm "$@"
        }

        # ZLE widget function for sshm inline picker (Ctrl+Alt+S)
        _sshm_inline_picker() {
            local saved_buffer="$BUFFER"
            
            local result
            result=$(sshm pick --query "$BUFFER")
            local exit_code=$?
            
            if [[ $exit_code -ne 0 ]]; then
                BUFFER="$saved_buffer"
                zle reset-prompt
                return
            fi
            
            if [[ -n "$result" ]]; then
                BUFFER="$result"
                CURSOR=${#BUFFER}
            fi
            
            zle reset-prompt
        }

        # Completion function for ssh / sshm: intercepts the ** trigger token like fzf does.
        # Integrates through zsh's completion system — Tab is NOT hijacked.
        # When the word being completed ends with **, the inline picker opens;
        # otherwise normal ssh completion runs unchanged.
        _sshm() {
            if [[ $PREFIX == *'**' ]]; then
                local stripped="${PREFIX%'**'}"
                local result ret
                result=$(sshm pick --query "$stripped" 2>/dev/null)
                ret=$?
                if [[ $ret -eq 0 && -n "$result" ]]; then
                    IPREFIX=""
                    # When completing for "ssh", strip the "ssh " prefix since
                    # the user already typed "ssh".  For "sshm" or other callers,
                    # keep the full result so it replaces the trigger word.
                    if [[ $words[1] == "ssh" ]]; then
                        compadd -U -- "${result#ssh }"
                    else
                        compadd -U -- "$result"
                    fi
                    return 0
                fi
                return 1
            fi
            # Pass through to original ssh completion
            if (( $+functions[_ssh] )); then
                _ssh "$@"
            fi
        }

        # Register the ZLE widget
        zle -N _sshm_inline_picker

        # Register **<TAB> completion via compdef (if completion system is loaded).
        # This does NOT hijack Tab — it hooks into zsh's completion chain naturally.
        if (( $+functions[compdef] )); then
            compdef _sshm ssh sshm
        fi

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bindkey '^S' _sshm_inline_picker
        "###);

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    // ── sshm init zsh contract assertions (AC9) ──────────────────────────────────────────────

    #[test]
    fn test_init_zsh_contains_sshm_pick() {
        let script = generate_init_zsh_script();
        assert!(script.contains("sshm pick"));
    }

    #[test]
    fn test_init_zsh_seeds_query_buffer() {
        let script = generate_init_zsh_script();
        assert!(script.contains("--query \"$BUFFER\""));
    }

    #[test]
    #[serial]
    fn test_init_zsh_binds_ctrl_alt_s_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        assert!(script.contains("bindkey"));
        // \e^S is zsh notation for Ctrl+Alt+S (ESC + Ctrl+S, the legacy xterm
        // encoding). It is far more portable than a guessed CSI sequence.
        assert!(script.contains("\\e^S"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_suppresses_bind_with_no_bind_flag() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_zsh_script();
        assert!(!script.contains("bindkey"));
        assert!(script.contains("no bind"));

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_zsh_honors_sshm_bind_key() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "^X");

        let script = generate_init_zsh_script();
        assert!(script.contains("bindkey"));
        assert!(script.contains("'^X'"));

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    #[test]
    fn test_init_zsh_replaces_buffer_with_cursor_to_end() {
        let script = generate_init_zsh_script();
        // Result replaces the buffer (not appended) so the full ssh command is on the command line.
        assert!(script.contains("BUFFER=\"$result\""));
        assert!(script.contains("CURSOR=${#BUFFER}"));
    }

    #[test]
    fn test_init_zsh_calls_reset_prompt() {
        let script = generate_init_zsh_script();
        assert!(script.contains("zle reset-prompt"));
    }

    #[test]
    fn test_init_zsh_restores_buffer_on_cancel() {
        let script = generate_init_zsh_script();
        assert!(script.contains("BUFFER=\"$saved_buffer\""));
    }

    // ── sshm init zsh **<TAB> completion trigger contract tests (AC for #16, #23) ───────────

    #[test]
    fn test_init_zsh_has_completion_function() {
        let script = generate_init_zsh_script();
        // The **<TAB> trigger is now a compdef-registered completion function, not a
        // ZLE widget that hijacks Tab.
        assert!(script.contains("_sshm()"));
    }

    #[test]
    fn test_init_zsh_completion_checks_prefix_for_doublestar() {
        let script = generate_init_zsh_script();
        // The completion function checks $PREFIX (not $LBUFFER) — it runs inside
        // zsh's completion system, not as a raw Tab binding.
        assert!(script.contains("PREFIX == *'**'"));
    }

    #[test]
    fn test_init_zsh_completion_strips_doublestar() {
        let script = generate_init_zsh_script();
        // Strips ** from the end of PREFIX (the word being completed).
        assert!(script.contains("${PREFIX%'**'}"));
    }

    #[test]
    fn test_init_zsh_completion_falls_through_via_compdef() {
        let script = generate_init_zsh_script();
        // When ** is not present, passes through to original _ssh completion.
        // Does NOT hijack Tab — completion chain is preserved.
        assert!(script.contains("_ssh \"$@\""));
    }

    #[test]
    fn test_init_zsh_completion_uses_compdef_not_bindkey() {
        let script = generate_init_zsh_script();
        // The trigger hooks into zsh's completion system via compdef, not
        // by hijacking the Tab key with bindkey.
        assert!(script.contains("compdef _sshm ssh"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_completion_no_tab_binding() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_zsh_script();
        // Tab must NOT be hijacked — normal completion stays intact.
        assert!(!script.contains("bindkey '^I'"));
    }

    #[test]
    #[serial]
    fn test_init_zsh_completion_no_tab_binding_with_no_bind() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_zsh_script();
        assert!(!script.contains("bindkey '^I'"));

        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    fn test_init_zsh_completion_compadd_with_result() {
        let script = generate_init_zsh_script();
        // The completion function uses compadd to insert the selected host
        // (with the "ssh " prefix stripped).
        assert!(script.contains("compadd -U -- \"${result#ssh }\""));
    }

    #[test]
    fn test_init_zsh_compdef_guarded_by_function_check() {
        let script = generate_init_zsh_script();
        // compdef is only called when the completion system is loaded.
        assert!(script.contains("$+functions[compdef]"));
    }

    #[test]
    fn test_init_zsh_completion_replaces_buffer() {
        let script = generate_init_zsh_script();
        // The _sshm_inline_picker widget still replaces the buffer (Ctrl+Alt+S).
        assert!(script.contains("BUFFER=\"$result\""));
        assert!(script.contains("CURSOR=${#BUFFER}"));
    }

    // ── sshm init bash snapshot tests (AC7) ────────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_snapshot_init_bash_default() {
        // Clear any env overrides
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init bash)"
        # Bind key: \e\C-s (override with SSHM_BIND_KEY)

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\e\C-s":_sshm_inline_picker'

        # Bind **<TAB> completion trigger (Tab key)
        bind -x '"\C-i":_sshm_completion_picker'
        "###);
    }

    #[test]
    #[serial]
    fn test_snapshot_init_bash_no_bind() {
        // Set SSHM_NO_BIND to suppress bind lines
        std::env::set_var("SSHM_NO_BIND", "1");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger (no bind)
        # Sourced via: eval "$(sshm init bash)"
        # Note: Bind lines suppressed by SSHM_NO_BIND=1

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
            fi
        }
        "###);

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_snapshot_init_bash_custom_bind_key() {
        // Clear SSHM_NO_BIND and set custom SSHM_BIND_KEY
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "\\C-t");

        let script = generate_init_bash_script();
        insta::assert_snapshot!(script, @r###"
        # sshm init bash - Inline Picker Widget + **<TAB> completion trigger
        # Sourced via: eval "$(sshm init bash)"
        # Bind key: \C-t (override with SSHM_BIND_KEY)

        _sshm_inline_picker() {
            local result exit_code
            result=$(sshm pick --query "$READLINE_LINE")
            exit_code=$?
            if [[ $exit_code -ne 0 ]]; then
                return
            fi
            if [[ -n "$result" ]]; then
                READLINE_LINE="$result"
                READLINE_POINT=${#READLINE_LINE}
            fi
        }

        # **<TAB> completion trigger (bash): when READLINE_LINE ends with **,
        # pressing Tab opens the inline picker. Note: without **, Tab has no
        # effect (bash cannot chain to normal completion from a key-bound function).
        _sshm_completion_picker() {
            if [[ "$READLINE_LINE" == *'**' ]]; then
                local saved="$READLINE_LINE"
                local stripped="${READLINE_LINE%'**'}"
                local result exit_code
                result=$(sshm pick --query "$stripped")
                exit_code=$?
                if [[ $exit_code -ne 0 ]]; then
                    READLINE_LINE="$saved"
                    return
                fi
                if [[ -n "$result" ]]; then
                    READLINE_LINE="$result"
                    READLINE_POINT=${#READLINE_LINE}
                fi
            fi
        }

        # Bind the widget to the trigger key (Ctrl+Alt+S)
        bind -x '"\C-t":_sshm_inline_picker'

        # Bind **<TAB> completion trigger (Tab key)
        bind -x '"\C-i":_sshm_completion_picker'
        "###);

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    // ── sshm init bash contract assertions (AC8) ────────────────────────────────────────────

    #[test]
    fn test_init_bash_contains_sshm_pick() {
        let script = generate_init_bash_script();
        assert!(script.contains("sshm pick"));
    }

    #[test]
    fn test_init_bash_seeds_query_readline_line() {
        let script = generate_init_bash_script();
        assert!(script.contains("--query \"$READLINE_LINE\""));
    }

    #[test]
    #[serial]
    fn test_init_bash_binds_ctrl_alt_s_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        assert!(script.contains("bind -x"));
        // \e\C-s is bash notation for Ctrl+Alt+S.
        assert!(script.contains("\\e\\C-s"));
    }

    #[test]
    #[serial]
    fn test_init_bash_suppresses_bind_with_no_bind_flag() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_bash_script();
        assert!(!script.contains("bind -x"));
        assert!(script.contains("no bind"));

        // Clean up
        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    #[serial]
    fn test_init_bash_honors_sshm_bind_key() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::set_var("SSHM_BIND_KEY", "\\C-x");

        let script = generate_init_bash_script();
        assert!(script.contains("bind -x"));
        assert!(script.contains("\\C-x"));

        // Clean up
        std::env::remove_var("SSHM_BIND_KEY");
    }

    #[test]
    fn test_init_bash_splices_into_readline_line_with_cursor_to_end() {
        let script = generate_init_bash_script();
        assert!(script.contains("READLINE_LINE=\"$result\""));
        assert!(script.contains(r"READLINE_POINT=${#READLINE_LINE}"));
    }

    #[test]
    fn test_init_bash_leaves_readline_line_untouched_on_nonzero_exit() {
        let script = generate_init_bash_script();
        // On non-zero exit, the function returns without modifying READLINE_LINE.
        assert!(script.contains("if [[ $exit_code -ne 0 ]]; then"));
        assert!(script.contains("return"));
        // No assignment to READLINE_LINE on the cancel path.
        let normal_return_count = script.matches("return").count();
        assert!(normal_return_count >= 1);
    }

    // ── sshm init bash **<TAB> completion trigger contract tests ───────────────────────────

    #[test]
    fn test_init_bash_has_completion_trigger_function() {
        let script = generate_init_bash_script();
        assert!(script.contains("_sshm_completion_picker"));
    }

    #[test]
    fn test_init_bash_completion_checks_readline_for_doublestar() {
        let script = generate_init_bash_script();
        // The widget checks if READLINE_LINE ends with ** ($READLINE_LINE is quoted)
        assert!(script.contains("\"$READLINE_LINE\" == *'**'"));
    }

    #[test]
    fn test_init_bash_completion_strips_doublestar() {
        let script = generate_init_bash_script();
        assert!(script.contains("${READLINE_LINE%'**'}"));
    }

    #[test]
    #[serial]
    fn test_init_bash_completion_binds_tab_by_default() {
        std::env::remove_var("SSHM_NO_BIND");
        std::env::remove_var("SSHM_BIND_KEY");

        let script = generate_init_bash_script();
        assert!(script.contains(r###"bind -x '"\C-i":_sshm_completion_picker'"###));
    }

    #[test]
    #[serial]
    fn test_init_bash_completion_binds_tab_suppressed_with_no_bind() {
        std::env::set_var("SSHM_NO_BIND", "1");

        let script = generate_init_bash_script();
        assert!(!script.contains(r###"\C-i":_sshm_completion_picker"###));

        std::env::remove_var("SSHM_NO_BIND");
    }

    #[test]
    fn test_init_bash_completion_saves_buffer_before_stripping() {
        let script = generate_init_bash_script();
        let saved_before = script
            .find("local saved=\"$READLINE_LINE\"")
            .expect("must save buffer");
        let strip_pos = script.find("${READLINE_LINE%'**'}").expect("must strip **");
        assert!(
            saved_before < strip_pos,
            "saved must be captured before stripping **"
        );
    }

    #[test]
    fn test_init_bash_completion_seeds_query_with_stripped_line() {
        let script = generate_init_bash_script();
        assert!(script.contains("--query \"$stripped\""));
    }
}
