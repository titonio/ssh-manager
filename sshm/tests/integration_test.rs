use insta::assert_snapshot;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use sshm::app::{App, AppMode, InputBuffer};
use sshm::config::{Config, Connection};

fn create_test_app() -> App {
    let mut config = Config::new();
    config.add_connection(Connection {
        id: "1".to_string(),
        alias: "prod-server".to_string(),
        host: "192.168.1.10".to_string(),
        user: "admin".to_string(),
        port: 22,
        key_path: None,
        folder: Some("production".to_string()),
    });
    config.add_connection(Connection {
        id: "2".to_string(),
        alias: "dev-server".to_string(),
        host: "192.168.1.20".to_string(),
        user: "developer".to_string(),
        port: 2222,
        key_path: None,
        folder: Some("development".to_string()),
    });
    config.add_connection(Connection {
        id: "3".to_string(),
        alias: "web-server".to_string(),
        host: "example.com".to_string(),
        user: "www".to_string(),
        port: 22,
        key_path: None,
        folder: None,
    });

    App {
        config,
        selected_index: 0,
        search_query: String::new(),
        mode: AppMode::Normal,
        matcher: fuzzy_matcher::skim::SkimMatcherV2::default(),
        filtered_indices: vec![0, 1, 2],
        message: None,
        input_buffer: InputBuffer::default(),
        input_field: 0,
        should_connect: None,
        ctrl_c_count: 0,
    }
}

#[test]
fn test_main_screen_layout() {
    let app = create_test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_main_screen_with_header_footer() {
    let app = create_test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    let buffer = backend.buffer();

    // Check that the header contains SSH
    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(content.contains("SSH"));
    assert!(content.contains("Connection Manager"));
}

#[test]
fn test_connection_list_shows_all_connections() {
    let app = create_test_app();
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_selected_item_highlighted() {
    let mut app = create_test_app();
    app.selected_index = 1;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_search_mode_shows_search_bar() {
    let mut app = create_test_app();
    app.mode = AppMode::Search;
    app.search_query = "prod".to_string();

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_help_mode_shows_help_popup() {
    let mut app = create_test_app();
    app.mode = AppMode::Help;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_add_connection_popup() {
    let mut app = create_test_app();
    app.mode = AppMode::Add;

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render_input(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_edit_connection_popup() {
    let mut app = create_test_app();
    app.mode = AppMode::Edit;
    app.selected_index = 0;
    app.input_buffer = InputBuffer::from_connection(app.config.connections.first().unwrap());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render_input(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_empty_connections_message() {
    let config = Config::new();
    let app = App {
        config,
        selected_index: 0,
        search_query: String::new(),
        mode: AppMode::Normal,
        matcher: fuzzy_matcher::skim::SkimMatcherV2::default(),
        filtered_indices: vec![],
        message: None,
        input_buffer: InputBuffer::default(),
        input_field: 0,
        should_connect: None,
        ctrl_c_count: 0,
    };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_search_filters_connections() {
    let mut app = create_test_app();
    app.search_query = "prod".to_string();
    app.update_filter();

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_no_matches_shows_message() {
    let mut app = create_test_app();
    app.search_query = "nonexistent".to_string();
    app.update_filter();

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_footer_help_text_normal_mode() {
    let app = create_test_app();

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    let buffer = backend.buffer();

    let content: String = buffer.content.iter().map(|c| c.symbol()).collect();
    assert!(content.contains("Navigate"));
    assert!(content.contains("Connect"));
}

#[test]
fn test_different_terminal_sizes() {
    let app = create_test_app();

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_message_popup() {
    let mut app = create_test_app();
    app.message = Some("Test message".to_string());

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            app.render(f);
            app.render_popup(f, "Test message");
        })
        .unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_input_field_navigation() {
    let mut app = create_test_app();
    app.mode = AppMode::Add;
    app.input_field = 3;
    app.input_buffer.alias = "test".to_string();
    app.input_buffer.host = "example.com".to_string();
    app.input_buffer.user = "user".to_string();

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render_input(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_connection_with_custom_port() {
    let mut config = Config::new();
    config.add_connection(Connection {
        id: "1".to_string(),
        alias: "custom-port".to_string(),
        host: "192.168.1.100".to_string(),
        user: "admin".to_string(),
        port: 2222,
        key_path: Some("/home/user/.ssh/id_rsa".to_string()),
        folder: Some("prod".to_string()),
    });

    let app = App {
        config,
        selected_index: 0,
        search_query: String::new(),
        mode: AppMode::Normal,
        matcher: fuzzy_matcher::skim::SkimMatcherV2::default(),
        filtered_indices: vec![0],
        message: None,
        input_buffer: InputBuffer::default(),
        input_field: 0,
        should_connect: None,
        ctrl_c_count: 0,
    };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}

#[test]
fn test_folder_grouping_display() {
    let mut config = Config::new();
    config.add_connection(Connection {
        id: "1".to_string(),
        alias: "server1".to_string(),
        host: "192.168.1.1".to_string(),
        user: "user".to_string(),
        port: 22,
        key_path: None,
        folder: Some("folder1".to_string()),
    });
    config.add_connection(Connection {
        id: "2".to_string(),
        alias: "server2".to_string(),
        host: "192.168.1.2".to_string(),
        user: "user".to_string(),
        port: 22,
        key_path: None,
        folder: Some("folder1".to_string()),
    });
    config.add_connection(Connection {
        id: "3".to_string(),
        alias: "server3".to_string(),
        host: "192.168.1.3".to_string(),
        user: "user".to_string(),
        port: 22,
        key_path: None,
        folder: Some("folder2".to_string()),
    });

    let app = App {
        config,
        selected_index: 0,
        search_query: String::new(),
        mode: AppMode::Normal,
        matcher: fuzzy_matcher::skim::SkimMatcherV2::default(),
        filtered_indices: vec![0, 1, 2],
        message: None,
        input_buffer: InputBuffer::default(),
        input_field: 0,
        should_connect: None,
        ctrl_c_count: 0,
    };

    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| app.render(f)).unwrap();

    let backend = terminal.backend();
    assert_snapshot!(backend);
}
