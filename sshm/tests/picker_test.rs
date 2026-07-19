use ratatui::backend::{Backend, TestBackend};
use ratatui::Terminal;
use sshm::config::Connection;
use sshm::picker::{build_ssh_command, render_picker_frame, PickerOutcome};

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

/// Render the inline picker UI onto a TestBackend using the shared
/// `render_picker_frame` function, so tests exercise the same code path
/// as production `run_pick`.
fn render_picker(connections: &[Connection], selected_index: usize) -> TestBackend {
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| render_picker_frame(f, connections, selected_index))
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
    assert_eq!(
        build_ssh_command(&conn),
        "ssh -i /path/to/key admin@example.com"
    );
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
    assert_eq!(
        build_ssh_command(&conn),
        "ssh -i /key -p 2222 admin@example.com"
    );
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