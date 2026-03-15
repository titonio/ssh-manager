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
