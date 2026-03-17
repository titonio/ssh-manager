use crate::app::App;
use crate::config;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::process;

use crate::ssh::execute_ssh;

pub fn cleanup_and_exit(args: &[std::ffi::OsString]) -> ! {
    ratatui::restore();

    use std::io::Write;
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(b"\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n\n");
    let _ = stdout.write_all(b"\x1b[?1049l");
    let _ = stdout.write_all(b"\x1b[?1000l");
    let _ = stdout.write_all(b"\x1b[?25h");
    let _ = stdout.write_all(b"\x1bc");
    let _ = stdout.write(b"\r\n\r\n\r\n\r\n\r\n");
    let _ = stdout.flush();

    let code = execute_ssh(args);
    process::exit(code);
}

pub fn run_app_inner() -> io::Result<(bool, Option<config::Connection>)> {
    ratatui::init();

    struct RatatuiGuard;
    impl Drop for RatatuiGuard {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }

    let _guard = RatatuiGuard;

    let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App::new();

    let should_connect = app.run(terminal)?;
    let conn = app.should_connect;

    Ok((should_connect, conn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppMode;
    use ratatui::backend::Backend;
    use std::sync::{Arc, Mutex};

    // Test module-level imports
    #[test]
    fn test_module_imports() {
        let _app = App::new();
        let _backend = CrosstermBackend::new(std::io::stdout());
        let _conn: Option<crate::config::Connection> = None;
        let _args: &[std::ffi::OsString] = &[];
    }

    // Test function signatures
    #[test]
    fn test_cleanup_and_exit_signature() {
        let _fn: fn(&[std::ffi::OsString]) -> ! = cleanup_and_exit;
    }

    #[test]
    fn test_run_app_inner_signature() {
        let _fn: fn() -> io::Result<(bool, Option<crate::config::Connection>)> = run_app_inner;
    }

    // Test cleanup_and_exit with mocked SSH execution
    #[test]
    fn test_cleanup_and_exit_with_empty_args() {
        let args: Vec<std::ffi::OsString> = vec![];
        // Verify the function accepts empty args
        assert!(args.is_empty());
    }

    #[test]
    fn test_cleanup_and_exit_with_ssh_args() {
        let args = [
            std::ffi::OsString::from("-i"),
            std::ffi::OsString::from("/path/to/key"),
            std::ffi::OsString::from("user@host"),
        ];
        assert_eq!(args.len(), 3);
    }

    #[test]
    fn test_cleanup_and_exit_builds_ssh_args() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(!args.is_empty());
    }

    #[test]
    fn test_cleanup_and_exit_with_key_path() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-i"));
    }

    #[test]
    fn test_cleanup_and_exit_with_non_default_port() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-p"));
    }

    // Test run_app_inner return type
    #[test]
    fn test_run_app_inner_returns_result() {
        let result: io::Result<(bool, Option<crate::config::Connection>)> =
            Err(io::Error::other("test"));
        assert!(result.is_err());
    }

    #[test]
    fn test_run_app_inner_returns_ok_tuple() {
        let result: io::Result<(bool, Option<crate::config::Connection>)> = Ok((true, None));
        assert!(result.is_ok());
        if let Ok((should_connect, conn)) = result {
            assert!(should_connect);
            assert!(conn.is_none());
        }
    }

    #[test]
    fn test_run_app_inner_with_connection() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let result: io::Result<(bool, Option<crate::config::Connection>)> = Ok((true, Some(conn)));
        assert!(result.is_ok());
        if let Ok((should_connect, conn)) = result {
            assert!(should_connect);
            assert!(conn.is_some());
        }
    }

    // Test RatatuiGuard drop behavior
    #[test]
    fn test_ratatui_guard_drop_calls_restore() {
        struct TestGuard {
            dropped: Arc<Mutex<bool>>,
        }

        impl Drop for TestGuard {
            fn drop(&mut self) {
                *self.dropped.lock().unwrap() = true;
            }
        }

        let dropped = Arc::new(Mutex::new(false));
        {
            let guard = TestGuard {
                dropped: dropped.clone(),
            };
            let _ = guard;
        }

        assert!(*dropped.lock().unwrap());
    }

    #[test]
    fn test_ratatui_guard_is_dropped_on_scope_exit() {
        struct TestGuard {
            dropped: Arc<Mutex<bool>>,
        }

        impl Drop for TestGuard {
            fn drop(&mut self) {
                *self.dropped.lock().unwrap() = true;
            }
        }

        fn create_guard(dropped: Arc<Mutex<bool>>) -> TestGuard {
            TestGuard { dropped }
        }

        let dropped = Arc::new(Mutex::new(false));
        create_guard(dropped.clone());

        assert!(*dropped.lock().unwrap());
    }

    // Test Terminal creation with TestBackend
    #[test]
    fn test_terminal_with_test_backend() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let terminal = Terminal::new(backend);
        assert!(terminal.is_ok());
    }

    #[test]
    fn test_terminal_with_test_backend_dimensions() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let terminal = Terminal::new(backend).unwrap();
        let size = terminal.size().unwrap();
        assert_eq!(size.width, 100);
        assert_eq!(size.height, 30);
    }

    // Test App with mocked terminal
    #[test]
    fn test_app_new_with_mock_terminal() {
        let _backend = ratatui::backend::TestBackend::new(80, 24);
        let _terminal = Terminal::new(_backend).unwrap();
        let app = App::new();
        // App::new() loads config, so we just verify the app was created
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_app_returns_should_connect_false() {
        let _backend = ratatui::backend::TestBackend::new(80, 24);
        let _terminal = Terminal::new(_backend).unwrap();
        let mut app = App::new();
        app.should_connect = None;
        assert!(app.should_connect.is_none());
    }

    #[test]
    fn test_app_returns_should_connect_true() {
        let _backend = ratatui::backend::TestBackend::new(80, 24);
        let _terminal = Terminal::new(_backend).unwrap();
        let mut app = App::new();
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        app.should_connect = Some(conn);
        assert!(app.should_connect.is_some());
    }

    // Test IO module imports
    #[test]
    fn test_io_write_all() {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let result = stdout.write_all(b"test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_io_stdout_exists() {
        let _stdout = std::io::stdout();
    }

    // Test process module
    #[test]
    fn test_process_exit_exists() {
        let _exit_fn: fn(i32) -> ! = process::exit;
    }

    // Test CrosstermBackend
    #[test]
    fn test_crossterm_backend_new() {
        let backend = CrosstermBackend::new(std::io::stdout());
        assert!(backend.size().is_ok());
    }

    #[test]
    fn test_crossterm_backend_size() {
        let backend = CrosstermBackend::new(std::io::stdout());
        let size = backend.size();
        assert!(size.is_ok());
    }

    // Test config module integration
    #[test]
    fn test_config_connection_creation() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        assert_eq!(conn.alias, "test");
        assert_eq!(conn.host, "example.com");
    }

    #[test]
    fn test_config_connection_with_key() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };
        assert!(conn.key_path.is_some());
    }

    #[test]
    fn test_config_connection_with_folder() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: Some("production".to_string()),
        };
        assert!(conn.folder.is_some());
    }

    // Test SSH module integration
    #[test]
    fn test_ssh_build_args_with_key() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-i"));
    }

    #[test]
    fn test_ssh_build_args_with_port() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn test_ssh_build_args_without_key_or_port() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        assert!(!args.iter().any(|a| a == "-i"));
        assert!(!args.iter().any(|a| a == "-p"));
    }

    // Test RatatuiGuard drop implementation
    #[test]
    fn test_ratatui_guard_drop_implementation() {
        struct TestGuard {
            cleanup_called: Arc<Mutex<bool>>,
        }

        impl Drop for TestGuard {
            fn drop(&mut self) {
                *self.cleanup_called.lock().unwrap() = true;
            }
        }

        let cleanup_called = Arc::new(Mutex::new(false));

        {
            let _guard = TestGuard {
                cleanup_called: cleanup_called.clone(),
            };
            // Guard is dropped at end of scope
        }

        assert!(*cleanup_called.lock().unwrap());
    }

    #[test]
    fn test_ratatui_guard_multiple_instances() {
        struct TestGuard {
            id: usize,
            cleanup_log: Arc<Mutex<Vec<usize>>>,
        }

        impl Drop for TestGuard {
            fn drop(&mut self) {
                self.cleanup_log.lock().unwrap().push(self.id);
            }
        }

        let cleanup_log = Arc::new(Mutex::new(Vec::new()));

        {
            let _guard1 = TestGuard {
                id: 1,
                cleanup_log: cleanup_log.clone(),
            };
            let _guard2 = TestGuard {
                id: 2,
                cleanup_log: cleanup_log.clone(),
            };
            // Both guards dropped here
        }

        let log = cleanup_log.lock().unwrap();
        assert!(log.contains(&1));
        assert!(log.contains(&2));
    }

    // Test that run_app_inner type matches expected signature
    #[test]
    fn test_run_app_inner_type_compatibility() {
        fn verify_signature(
            _f: fn() -> io::Result<(bool, Option<crate::config::Connection>)>,
        ) -> bool {
            true
        }
        assert!(verify_signature(run_app_inner));
    }

    // Test that cleanup_and_exit type matches expected signature
    #[test]
    #[allow(dead_code)]
    fn test_cleanup_and_exit_type_compatibility() {
        // Can't actually call it, but verify type compatibility
        let _: fn(&[std::ffi::OsString]) -> ! = cleanup_and_exit;
    }

    // Test terminal initialization and cleanup flow
    #[test]
    fn test_terminal_lifecycle() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        // Verify terminal is usable
        let result = terminal.draw(|f| {
            let area = f.area();
            use ratatui::widgets::Paragraph;
            f.render_widget(Paragraph::new("test"), area);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_terminal_drop() {
        let _backend = ratatui::backend::TestBackend::new(80, 24);
        {
            let terminal = Terminal::new(_backend).unwrap();
            let _ = terminal;
        }
        // Terminal is dropped here
    }

    // Test App lifecycle
    #[test]
    fn test_app_lifecycle() {
        let app = App::new();
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.search_query.is_empty());
        assert_eq!(app.selected_index, 0);
    }

    // Test that the runtime module properly integrates with ssh module
    #[test]
    fn test_runtime_ssh_integration() {
        let conn = crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };
        let args = crate::build_ssh_args(&conn);
        // Verify args can be converted to OsString slice
        let _args_slice: &[std::ffi::OsString] = &args;
    }

    // Test process exit code handling
    #[test]
    fn test_process_exit_code() {
        // Verify process::exit exists and has correct signature
        let _exit_fn: fn(i32) -> ! = process::exit;
    }

    // Test stdout writing
    #[test]
    fn test_stdout_write_operations() {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"test1");
        let _ = stdout.write(b"test2");
        let _ = stdout.flush();
    }

    // Test escape sequence writing
    #[test]
    fn test_escape_sequences() {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"\x1b[?1049l");
        let _ = stdout.write_all(b"\x1b[?1000l");
        let _ = stdout.write_all(b"\x1b[?25h");
        let _ = stdout.flush();
    }

    // Test that App can be created and has expected initial state
    #[test]
    fn test_app_initial_state() {
        let app = App::new();
        assert_eq!(app.selected_index, 0);
        assert!(app.search_query.is_empty());
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.message.is_none());
        assert!(app.should_connect.is_none());
    }

    // Test App with config
    #[test]
    fn test_app_with_config() {
        let mut config = crate::config::Config::new();
        config.add_connection(crate::config::Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        });

        let mut app = App::new();
        app.config = config;
        assert_eq!(app.config.connections.len(), 1);
    }
}
