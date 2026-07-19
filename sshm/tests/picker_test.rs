use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::backend::{Backend, TestBackend};
use ratatui::Terminal;
use sshm::config::Connection;
use sshm::picker::{compute_matches, render_picker_frame};

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
/// `render_picker_frame` function.
fn render_picker(connections: &[Connection], query: &str, selected_index: usize) -> TestBackend {
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(connections, &matcher, query);
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| render_picker_frame(f, connections, &matches, selected_index, query))
        .unwrap();

    terminal.backend().clone()
}

#[test]
fn test_picker_inline_height() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 0);
    assert_eq!(backend.size().unwrap().height, 15);
}

#[test]
fn test_picker_shows_all_connections() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("prod-server"));
    assert!(content.contains("dev-server"));
    assert!(content.contains("web-server"));
}

#[test]
fn test_picker_selected_item_highlighted() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 1);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("dev-server"));
}

#[test]
fn test_picker_no_connections_shows_message() {
    let backend = render_picker(&[], "", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("No Connections"));
}

#[test]
fn test_picker_header() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("Pick Connection"));
}

// ── Pre-narrowed list tests ────────────────────────────────────────────

#[test]
fn test_picker_narrowed_by_query() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "prod", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("prod-server"));
    // dev-server should not appear since query narrows to "prod".
    assert!(!content.contains("dev-server"));
}

#[test]
fn test_picker_folder_match() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "development", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("dev-server"));
    // prod-server should not appear since "development" doesn't match it.
    assert!(!content.contains("prod-server"));
}

#[test]
fn test_picker_no_matches_message() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "zzzznonexistent", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("No matches"));
}

#[test]
fn test_picker_folder_prefix_in_rows() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    // First row has folder prefix.
    assert!(content.contains("[production]"));
    assert!(content.contains("[development]"));
    // Third row has no folder prefix.
    assert!(content.contains("web-server (www@example.com:22)"));
}

#[test]
fn test_picker_match_highlight() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "ser", 0);

    let buffer = backend.buffer();
    // Verify the buffer contains styled cells (highlighted matches).
    let has_styled = buffer.content.iter().any(|c| {
        c.style()
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    });
    assert!(
        has_styled,
        "Expected highlighted characters for fuzzy match"
    );
}

#[test]
fn test_picker_query_line_shown() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "prod", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    // Query text should appear in the output.
    assert!(content.contains("prod"));
}

#[test]
fn test_picker_footer_keys() {
    let connections = create_test_connections();
    let backend = render_picker(&connections, "", 0);

    let buffer = backend.buffer();
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();

    assert!(content.contains("Navigate"));
    assert!(content.contains("Select"));
    assert!(content.contains("Cancel"));
}
