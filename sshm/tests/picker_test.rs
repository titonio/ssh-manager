use ratatui::backend::{Backend, TestBackend};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::Terminal;
use ratatui::widgets::{Block, Borders, List, ListItem};
use sshm::config::Connection;
use sshm::picker::{build_ssh_command, PickerOutcome};

fn create_test_connections() -> Vec<Connection> {
    vec![
        Connection {
            id: "1".to_string(),
            alias: "prod-server".to_string(),
            host: "192.168.1.10".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: Some("production".to_string()),
        },
        Connection {
            id: "2".to_string(),
            alias: "dev-server".to_string(),
            host: "192.168.1.20".to_string(),
            user: "developer".to_string(),
            port: 2222,
            key_path: None,
            folder: Some("development".to_string()),
        },
        Connection {
            id: "3".to_string(),
            alias: "web-server".to_string(),
            host: "example.com".to_string(),
            user: "www".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        },
    ]
}

/// Render the inline picker UI onto a TestBackend.
fn render_picker(connections: &[Connection], selected_index: usize) -> TestBackend {
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();

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
    })
    .unwrap();

    terminal.backend().clone()
}

#[test]
fn test_picker_inline_height() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, 0);
    assert_eq!(backend.size().unwrap().height, 15);
}

#[test]
fn test_picker_shows_all_connections() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("prod-server"));
    assert!(content.contains("dev-server"));
    assert!(content.contains("web-server"));
}

#[test]
fn test_picker_selected_item_highlighted() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, 1);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("dev-server"));
}

#[test]
fn test_picker_no_connections_shows_message() {
    let backend = render_picker(&[], 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("No Connections"));
}

#[test]
fn test_picker_header() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("Pick Connection"));
}

#[test]
fn test_build_ssh_command_basic() {
    let conn = Connection {
        id: "1".to_string(),
        alias: "test".to_string(),
        host: "example.com".to_string(),
        user: "admin".to_string(),
        port: 22,
        key_path: None,
        folder: None,
    };
    assert_eq!(build_ssh_command(&conn), "ssh admin@example.com");
}

#[test]
fn test_build_ssh_command_with_key() {
    let conn = Connection {
        id: "1".to_string(),
        alias: "test".to_string(),
        host: "example.com".to_string(),
        user: "admin".to_string(),
        port: 22,
        key_path: Some("/path/to/key".to_string()),
        folder: None,
    };
    assert_eq!(build_ssh_command(&conn), "ssh -i /path/to/key admin@example.com");
}

#[test]
fn test_build_ssh_command_with_port() {
    let conn = Connection {
        id: "1".to_string(),
        alias: "test".to_string(),
        host: "example.com".to_string(),
        user: "admin".to_string(),
        port: 2222,
        key_path: None,
        folder: None,
    };
    assert_eq!(build_ssh_command(&conn), "ssh -p 2222 admin@example.com");
}

#[test]
fn test_build_ssh_command_empty_user() {
    let conn = Connection {
        id: "1".to_string(),
        alias: "test".to_string(),
        host: "example.com".to_string(),
        user: "".to_string(),
        port: 22,
        key_path: None,
        folder: None,
    };
    assert_eq!(build_ssh_command(&conn), "ssh example.com");
}

#[test]
fn test_build_ssh_command_full() {
    let conn = Connection {
        id: "1".to_string(),
        alias: "test".to_string(),
        host: "example.com".to_string(),
        user: "admin".to_string(),
        port: 2222,
        key_path: Some("/key".to_string()),
        folder: None,
    };
    assert_eq!(build_ssh_command(&conn), "ssh -i /key -p 2222 admin@example.com");
}

#[test]
fn test_run_pick_empty_connections_returns_cancel() {
    let result = sshm::picker::run_pick(vec![]);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), PickerOutcome::Cancel);
}

#[test]
fn test_picker_outcome_variants() {
    let cancel = PickerOutcome::Cancel;
    assert_eq!(cancel, PickerOutcome::Cancel);

    let conn = Connection {
        id: "1".to_string(),
        alias: "u".to_string(),
        host: "h".to_string(),
        user: "".to_string(),
        port: 22,
        key_path: None,
        folder: None,
    };
    let selected = PickerOutcome::Selected(conn.clone());
    assert_eq!(selected, PickerOutcome::Selected(conn));
    assert_ne!(selected, PickerOutcome::Cancel);
}
