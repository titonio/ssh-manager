use crate::config::Connection;
use crate::ssh::build_ssh_args;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use std::io;

/// Outcome of running the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// User accepted a Connection.
    Selected(Connection),
    /// User cancelled (Esc or Ctrl-C).
    Cancel,
}

/// How many lines the inline picker occupies by default.
const DEFAULT_HEIGHT: u16 = 15;

/// Run the inline picker, returning a `PickerOutcome`.
///
/// This is the injectable seam: callers can pass a mock runner for testing.
pub fn run_pick(connections: Vec<Connection>) -> io::Result<PickerOutcome> {
    if connections.is_empty() {
        return Ok(PickerOutcome::Cancel);
    }

    // We do NOT call ratatui::init() (which switches to alt-screen).
    // Instead we create a Terminal directly with a CrosstermBackend so
    // the picker renders inline below the cursor.
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal =
        ratatui::Terminal::with_options(
            backend,
            ratatui::TerminalOptions { viewport: ratatui::Viewport::Inline(DEFAULT_HEIGHT) },
        )?;

    let mut selected_index = 0;
    let outcome = loop {
        terminal.draw(|f| {
            let area = f.area();
            let block = Block::default()
                .title(" Pick Connection ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(129, 161, 193)));

            f.render_widget(block, area);

            let inner = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);

            if connections.is_empty() {
                let empty =
                    ratatui::widgets::Paragraph::new("No Connections").alignment(Alignment::Center);
                f.render_widget(empty, inner);
                return;
            }

            let items: Vec<ListItem> = connections
                .iter()
                .enumerate()
                .map(|(i, conn)| {
                    let display = format!("{} ({})", conn.alias, conn.host);
                    let is_selected = i == selected_index;
                    let style = if is_selected {
                        Style::default()
                            .fg(Color::Rgb(235, 203, 139))
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(216, 222, 233))
                    };
                    ListItem::new(display).style(style)
                })
                .collect();

            let list = List::new(items);
            f.render_widget(list, inner);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if selected_index > 0 {
                        selected_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected_index < connections.len().saturating_sub(1) {
                        selected_index += 1;
                    }
                }
                KeyCode::Enter => {
                    break PickerOutcome::Selected(connections[selected_index].clone());
                }
                KeyCode::Esc | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break PickerOutcome::Cancel;
                }
                _ => {}
            }
        }
    };

    // Restore terminal state
    ratatui::restore();
    Ok(outcome)
}

/// Build the literal `ssh` command string for a Connection.
///
/// Reuses the same field logic as `build_ssh_args`:
/// `-i <key>` when a key path is present,
/// `-p <port>` when port is not 22,
/// `user@host` when user is non-empty else bare `host`.
pub fn build_ssh_command(conn: &Connection) -> String {
    let args = build_ssh_args(conn);
    let mut parts = vec!["ssh".to_string()];
    for arg in args {
        parts.push(arg.to_string_lossy().to_string());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_connection(user: &str, host: &str, port: u16, key: Option<&str>) -> Connection {
        Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            key_path: key.map(|s| s.to_string()),
            folder: None,
        }
    }

    #[test]
    fn test_build_ssh_command_basic() {
        let conn = make_connection("admin", "example.com", 22, None);
        assert_eq!(build_ssh_command(&conn), "ssh admin@example.com");
    }

    #[test]
    fn test_build_ssh_command_with_key() {
        let conn = make_connection("admin", "example.com", 22, Some("/path/to/key"));
        assert_eq!(build_ssh_command(&conn), "ssh -i /path/to/key admin@example.com");
    }

    #[test]
    fn test_build_ssh_command_with_port() {
        let conn = make_connection("admin", "example.com", 2222, None);
        assert_eq!(build_ssh_command(&conn), "ssh -p 2222 admin@example.com");
    }

    #[test]
    fn test_build_ssh_command_empty_user() {
        let conn = make_connection("", "example.com", 22, None);
        assert_eq!(build_ssh_command(&conn), "ssh example.com");
    }

    #[test]
    fn test_build_ssh_command_full() {
        let conn = make_connection("admin", "example.com", 2222, Some("/key"));
        assert_eq!(build_ssh_command(&conn), "ssh -i /key -p 2222 admin@example.com");
    }

    #[test]
    fn test_picker_outcome_variants() {
        let cancel = PickerOutcome::Cancel;
        assert_eq!(cancel, PickerOutcome::Cancel);

        let conn = make_connection("u", "h", 22, None);
        let selected = PickerOutcome::Selected(conn.clone());
        assert_eq!(selected, PickerOutcome::Selected(conn));
        assert_ne!(selected, PickerOutcome::Cancel);
    }

    #[test]
    fn test_run_pick_empty_connections_returns_cancel() {
        // When there are no connections, run_pick returns Cancel immediately
        // without ever opening a terminal.
        let result = run_pick(vec![]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), PickerOutcome::Cancel);
    }
}
