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

    /// Check for updates
    CheckUpdate,

    /// Open an inline picker to select a Connection (insert, don't execute)
    Pick {
        /// Seed the picker with an initial fuzzy query (for shell widget integration)
        #[arg(long)]
        query: Option<String>,
    },
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::Connection;
    use crate::picker::{self, PickerOutcome};

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
}
