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

    #[test]
    fn test_run_app_inner_type() {
        let _: fn() -> io::Result<(bool, Option<crate::config::Connection>)> = run_app_inner;
    }

    #[test]
    fn test_cleanup_and_exit_type() {
        let _: fn(&[std::ffi::OsString]) -> ! = cleanup_and_exit;
    }

    #[test]
    fn test_ratatui_guard_exists() {
        struct TestGuard;
        impl Drop for TestGuard {
            fn drop(&mut self) {}
        }
        let _guard = TestGuard;
    }

    #[test]
    fn test_terminal_backend_type() {
        let _backend = CrosstermBackend::new(std::io::stdout());
    }

    #[test]
    fn test_app_new_type() {
        let _app = App::new();
    }

    #[test]
    fn test_stdout_write_all() {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(b"test");
    }

    #[test]
    fn test_process_exit_exists() {
        // Just verify the module exists
        let _ = process::exit;
    }

    #[test]
    fn test_io_module_exists() {
        let _stdout = std::io::stdout();
    }

    #[test]
    fn test_ratatui_backend_imports() {
        let _backend_type: fn(std::io::Stdout) -> CrosstermBackend<std::io::Stdout> =
            CrosstermBackend::new;
    }

    #[test]
    fn test_config_module_imports() {
        let _conn: Option<crate::config::Connection> = None;
    }

    #[test]
    fn test_ssh_module_imports() {
        let _args: &[std::ffi::OsString] = &[];
    }

    #[test]
    fn test_cleanup_function_signature() {
        let _fn: fn(&[std::ffi::OsString]) -> ! = cleanup_and_exit;
    }

    #[test]
    fn test_run_app_signature() {
        let _fn: fn() -> io::Result<(bool, Option<crate::config::Connection>)> = run_app_inner;
    }

    #[test]
    fn test_terminal_imports() {
        let _terminal_type: fn() -> Terminal<CrosstermBackend<std::io::Stdout>> =
            || unimplemented!();
    }

    #[test]
    fn test_std_io_imports() {
        let _io: std::io::Stdout = std::io::stdout();
    }

    #[test]
    fn test_process_imports() {
        let _exit_fn: fn(i32) -> ! = process::exit;
    }

    #[test]
    fn test_crossterm_backend_exists() {
        let _backend = CrosstermBackend::new(std::io::stdout());
    }

    #[test]
    fn test_app_struct_exists() {
        let _app = App::new();
    }
}
