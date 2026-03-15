mod app;
mod config;

use app::App;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::process;

fn main() -> io::Result<()> {
    struct RatatuiGuard;

    impl Drop for RatatuiGuard {
        fn drop(&mut self) {
            ratatui::restore();
        }
    }

    ratatui::init();
    let _guard = RatatuiGuard;

    let terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let mut app = App::new();

    let should_connect = app.run(terminal)?;

    if should_connect {
        if let Some(conn) = app.should_connect {
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

            let mut args: Vec<std::ffi::OsString> = vec![];

            if let Some(ref key) = conn.key_path {
                args.push(std::ffi::OsStr::new("-i").to_os_string());
                args.push(std::ffi::OsStr::new(key).to_os_string());
            }

            if conn.port != 22 {
                args.push(std::ffi::OsStr::new("-p").to_os_string());
                args.push(std::ffi::OsStr::new(&conn.port.to_string()).to_os_string());
            }

            let user_host = if conn.user.is_empty() {
                conn.host.clone()
            } else {
                format!("{}@{}", conn.user, conn.host)
            };
            args.push(std::ffi::OsStr::new(&user_host).to_os_string());

            let status = process::Command::new("ssh").args(&args).status();

            let code = status.map(|s| s.code().unwrap_or(1)).unwrap_or(1);
            process::exit(code);
        }
    }

    Ok(())
}
