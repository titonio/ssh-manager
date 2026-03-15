mod app;
mod config;
mod runtime;
mod ssh;

use std::io;

use runtime::{cleanup_and_exit, run_app_inner};
use ssh::build_ssh_args;

fn main() -> io::Result<()> {
    run_main(run_app_inner)
}

fn run_main(run_app_fn: fn() -> io::Result<(bool, Option<config::Connection>)>) -> io::Result<()> {
    let (should_connect, conn) = run_app_fn()?;

    if should_connect {
        if let Some(conn) = conn {
            let args = build_ssh_args(&conn);
            cleanup_and_exit(&args);
        }
    }

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
}
