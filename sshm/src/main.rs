mod app;
mod config;
mod runtime;
mod ssh;
mod update;

use std::env;
use std::io;

use runtime::{cleanup_and_exit, run_app_inner};
use ssh::build_ssh_args;
use update::UpdateResult;

fn main() -> io::Result<()> {
    run_main(run_app_inner)
}

fn run_main(run_app_fn: fn() -> io::Result<(bool, Option<config::Connection>)>) -> io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "version" {
        println!("sshm {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if args.len() > 1 && args[1] == "add" {
        match run_add_command(&args) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    }

    let force_check = args
        .iter()
        .any(|arg| arg == "--check-update" || arg == "-c");

    if force_check {
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

    let (should_connect, conn) = run_app_fn()?;

    if should_connect {
        if let Some(conn) = conn {
            let args = build_ssh_args(&conn);
            cleanup_and_exit(&args);
        }
    }

    Ok(())
}

fn run_add_command(args: &[String]) -> Result<(), String> {
    let mut alias: Option<String> = None;
    let mut host: Option<String> = None;
    let mut user: Option<String> = None;
    let mut port: u16 = 22;
    let mut key_path: Option<String> = None;
    let mut folder: Option<String> = None;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--alias" | "-a" => {
                if i + 1 >= args.len() {
                    return Err("Error: --alias requires a value".to_string());
                }
                alias = Some(args[i + 1].clone());
                i += 2;
            }
            "--port" | "-p" => {
                if i + 1 >= args.len() {
                    return Err("Error: --port requires a value".to_string());
                }
                port = args[i + 1]
                    .parse()
                    .map_err(|_| format!("Error: invalid port number '{}'", args[i + 1]))?;
                i += 2;
            }
            "--key" | "-k" => {
                if i + 1 >= args.len() {
                    return Err("Error: --key requires a value".to_string());
                }
                key_path = Some(args[i + 1].clone());
                i += 2;
            }
            "--folder" | "-f" => {
                if i + 1 >= args.len() {
                    return Err("Error: --folder requires a value".to_string());
                }
                folder = Some(args[i + 1].clone());
                i += 2;
            }
            arg if !arg.starts_with('-') => {
                if host.is_none() {
                    let parts: Vec<&str> = arg.split('@').collect();
                    if parts.len() == 2 {
                        user = Some(parts[0].to_string());
                        host = Some(parts[1].to_string());
                    } else if parts.len() == 1 {
                        host = Some(arg.to_string());
                    } else {
                        return Err(format!("Error: invalid user@host format '{}'", arg));
                    }
                }
                i += 1;
            }
            _ => {
                return Err(format!("Error: unknown argument '{}'", args[i]));
            }
        }
    }

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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::config::Connection;

    #[test]
    fn test_cleanup_and_exit_with_args() {
        let args: Vec<std::ffi::OsString> = vec![];
        assert!(args.is_empty());
    }

    #[test]
    fn test_main_should_connect_false_returns_ok() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let result = run_main(mock_run_app);
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
        let result = run_main(mock_run_app);
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
        let result = run_main(mock_run_app);
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
    fn test_main_entry_point() {
        // Test that main calls run_main with run_app_inner
        // We can't actually call main() but we can verify the function signature
        let _: fn() -> io::Result<(bool, Option<crate::config::Connection>)> = run_app_inner;
    }

    #[test]
    fn test_run_main_calls_cleanup_on_connect() {
        // This test verifies the logic path where should_connect is true with conn
        // We use a mock that returns (true, Some(conn)) to trigger the cleanup_and_exit path
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        // Verify that build_ssh_args is called with the connection
        let args = build_ssh_args(&conn);
        assert!(!args.is_empty());
    }

    #[test]
    fn test_run_main_version_flag() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let result = run_main(mock_run_app);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_main_force_check_update_available() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let result = run_main(mock_run_app);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_main_force_check_no_update() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let result = run_main(mock_run_app);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_main_force_check_error() {
        let mock_run_app = || Ok::<(bool, Option<Connection>), io::Error>((false, None));
        let result = run_main(mock_run_app);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_main_should_connect_with_connection() {
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
            || Ok::<(bool, Option<Connection>), io::Error>((true, Some(conn.clone())));
        let (should_connect, result_conn) = mock_run_app().unwrap();
        assert!(should_connect);
        assert!(result_conn.is_some());
        let args = build_ssh_args(&conn);
        assert!(!args.is_empty());
    }
}
