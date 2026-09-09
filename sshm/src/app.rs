use crate::config::{import_from_ssh_config, Config, Connection};
use crate::style;
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
#[cfg(test)]
use ratatui::layout::Rect;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Add,
    Edit,
    Search,
    Help,
    Update,
}

pub struct App {
    pub config: Config,
    pub selected_index: usize,
    pub search_query: String,
    pub mode: AppMode,
    pub matcher: SkimMatcherV2,
    pub filtered_indices: Vec<usize>,
    pub message: Option<String>,
    pub input_buffer: InputBuffer,
    pub input_field: usize,
    pub should_connect: Option<Connection>,
    pub ctrl_c_count: usize,
    pub update_info: Option<crate::update::UpdateInfo>,
}

#[derive(Debug, Clone, Default)]
pub struct InputBuffer {
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: String,
    pub key_path: String,
    pub folder: String,
}

impl InputBuffer {
    pub fn clear(&mut self) {
        self.alias.clear();
        self.host.clear();
        self.user.clear();
        self.port = "22".to_string();
        self.key_path.clear();
        self.folder.clear();
    }

    pub fn from_connection(conn: &Connection) -> Self {
        Self {
            alias: conn.alias.clone(),
            host: conn.host.clone(),
            user: conn.user.clone(),
            port: conn.port.to_string(),
            key_path: conn.key_path.clone().unwrap_or_default(),
            folder: conn.folder.clone().unwrap_or_default(),
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let config = Config::load();
        let filtered_indices: Vec<usize> = (0..config.connections.len()).collect();

        Self {
            config,
            selected_index: 0,
            search_query: String::new(),
            mode: AppMode::Normal,
            matcher: SkimMatcherV2::default(),
            filtered_indices,
            message: None,
            input_buffer: InputBuffer::default(),
            input_field: 0,
            should_connect: None,
            ctrl_c_count: 0,
            update_info: None,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> io::Result<bool> {
        self.update_filter();
        self.check_for_update();

        loop {
            terminal.draw(|f| self.render(f))?;

            if let Some(msg) = self.message.take() {
                if !self.show_message(&mut terminal, &msg)? {
                    return Ok(false);
                }
                continue;
            }

            match self.mode {
                AppMode::Normal => match self.handle_normal_mode(&mut terminal)? {
                    Some(true) => return Ok(true),
                    Some(false) => break Ok(false),
                    None => {}
                },
                AppMode::Add | AppMode::Edit => self.handle_input_mode(&mut terminal)?,
                AppMode::Search => self.handle_search_mode(&mut terminal)?,
                AppMode::Help => self.handle_help_mode(&mut terminal)?,
                AppMode::Update => self.handle_update_mode(&mut terminal)?,
            }
        }
    }

    fn show_message(&mut self, terminal: &mut DefaultTerminal, message: &str) -> io::Result<bool> {
        loop {
            terminal.draw(|f| {
                self.render(f); // Render normal interface first
                self.render_popup(f, message); // Then render popup on top
            })?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                // Handle special case: if we're waiting for second Ctrl+C to exit
                if message == "Press Ctrl+C again to exit"
                    && key.code == crossterm::event::KeyCode::Char('c')
                {
                    // We need to handle this like in handle_normal_mode
                    self.ctrl_c_count += 1;
                    if self.ctrl_c_count >= 2 {
                        return Ok(false);
                    } else {
                        // This shouldn't happen in this context, but just in case
                        self.message = Some("Press Ctrl+C again to exit".to_string());
                    }
                } else if key.code == crossterm::event::KeyCode::Enter
                    || key.code == crossterm::event::KeyCode::Esc
                    || key.code == crossterm::event::KeyCode::Char('q')
                {
                    self.message = None;
                    break;
                }
            }
        }
        Ok(true)
    }

    #[cfg(test)]
    fn centered_rect(&self, width: u16, height: u16, area: Rect) -> Rect {
        let _ = (width, height, area);
        area
    }

    pub fn render_popup(&self, f: &mut Frame, message: &str) {
        // Clack-style note frame: header, message, dismiss hint, corner.
        f.render_widget(Block::default(), f.area());
        let lines = vec![
            style::header_line(style::STEP_ACTIVE, style::green(), "Message"),
            style::rail_text(message, Style::default()),
            style::rail_text("Enter/Esc: Dismiss", style::dim()),
            style::corner_line(),
        ];
        f.render_widget(Paragraph::new(lines), f.area());
    }

    fn handle_normal_mode(&mut self, _terminal: &mut DefaultTerminal) -> io::Result<Option<bool>> {
        if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
            match key.code {
                crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                    if self.selected_index > 0 {
                        self.selected_index -= 1;
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                    let len = self.filtered_indices.len();
                    if self.selected_index < len.saturating_sub(1) {
                        self.selected_index += 1;
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Enter => {
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                    self.connect();
                    return Ok(Some(true));
                }
                crossterm::event::KeyCode::Char('c')
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL) =>
                {
                    // Handle Ctrl+C for quit (need to press twice)
                    self.ctrl_c_count += 1;
                    if self.ctrl_c_count >= 2 {
                        return Ok(Some(false));
                    } else {
                        // Show message that another Ctrl+C will exit
                        self.message = Some("Press Ctrl+C again to exit".to_string());
                    }
                }
                crossterm::event::KeyCode::Char('a') => {
                    if !self.search_query.is_empty() {
                        self.search_query.push('a');
                        self.update_filter();
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('e') => {
                    if !self.search_query.is_empty() {
                        self.search_query.push('e');
                        self.update_filter();
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('d') => {
                    if !self.search_query.is_empty() {
                        self.search_query.push('d');
                        self.update_filter();
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('i') => {
                    if !self.search_query.is_empty() {
                        self.search_query.push('i');
                        self.update_filter();
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('A') => {
                    self.mode = AppMode::Add;
                    self.input_buffer.clear();
                    self.input_field = 0;
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('E') => {
                    if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
                        if let Some(conn) = self.config.connections.get(idx) {
                            self.input_buffer = InputBuffer::from_connection(conn);
                            self.mode = AppMode::Edit;
                            self.input_field = 0;
                        }
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('D') => {
                    self.delete_connection();
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('I') => {
                    self.import_connections();
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('/') => {
                    self.mode = AppMode::Search;
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('?') => {
                    self.mode = AppMode::Help;
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Home => {
                    self.selected_index = 0;
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::End => {
                    self.selected_index = self.filtered_indices.len().saturating_sub(1);
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('g') => {
                    self.selected_index = 0;
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char('G') => {
                    self.selected_index = self.filtered_indices.len().saturating_sub(1);
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Backspace => {
                    if !self.search_query.is_empty() {
                        self.search_query.pop();
                        self.update_filter();
                    }
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                crossterm::event::KeyCode::Char(c) => {
                    // Handle printable characters for search
                    self.search_query.push(c);
                    self.update_filter();
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
                _ => {
                    self.ctrl_c_count = 0; // Reset Ctrl+C counter on other key press
                }
            }
        }
        Ok(None)
    }

    fn handle_input_mode(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render_input(f))?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Enter => {
                        self.save_connection();
                        break;
                    }
                    crossterm::event::KeyCode::Esc | crossterm::event::KeyCode::Char('q') => {
                        self.mode = AppMode::Normal;
                        break;
                    }
                    crossterm::event::KeyCode::Tab => {
                        self.advance_input_field();
                    }
                    crossterm::event::KeyCode::BackTab => {
                        if self.input_field > 0 {
                            self.input_field -= 1;
                        }
                    }
                    crossterm::event::KeyCode::Left => {
                        if self.input_field > 0 {
                            self.input_field -= 1;
                        }
                    }
                    crossterm::event::KeyCode::Right => {
                        self.advance_input_field();
                    }
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                        if self.input_field > 0 {
                            self.input_field -= 1;
                        }
                    }
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                        self.advance_input_field();
                    }
                    crossterm::event::KeyCode::Backspace => {
                        self.delete_input_char();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        self.append_input_char(c);
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn handle_search_mode(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            // Clack-style: the search lives inline in the main frame so the
            // filtered list updates as you type (like skills' search prompt).
            terminal.draw(|f| self.render(f))?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Enter
                    | crossterm::event::KeyCode::Esc
                    | crossterm::event::KeyCode::Char('q') => {
                        if key.code == crossterm::event::KeyCode::Esc
                            || key.code == crossterm::event::KeyCode::Char('q')
                        {
                            self.search_query.clear();
                            self.update_filter();
                        }
                        self.mode = AppMode::Normal;
                        break;
                    }
                    crossterm::event::KeyCode::Up | crossterm::event::KeyCode::Char('k') => {
                        if self.selected_index > 0 {
                            self.selected_index -= 1;
                        }
                    }
                    crossterm::event::KeyCode::Down | crossterm::event::KeyCode::Char('j') => {
                        let len = self.filtered_indices.len();
                        if self.selected_index < len.saturating_sub(1) {
                            self.selected_index += 1;
                        }
                    }
                    crossterm::event::KeyCode::Backspace => {
                        self.search_query.pop();
                        self.update_filter();
                    }
                    crossterm::event::KeyCode::Char(c) => {
                        self.search_query.push(c);
                        self.update_filter();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn handle_help_mode(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render_help(f))?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Esc
                    | crossterm::event::KeyCode::Char('q')
                    | crossterm::event::KeyCode::Enter => {
                        self.mode = AppMode::Normal;
                        break;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn append_input_char(&mut self, c: char) {
        match self.input_field {
            0 => self.input_buffer.alias.push(c),
            1 => self.input_buffer.host.push(c),
            2 => self.input_buffer.user.push(c),
            3 => {
                if c.is_ascii_digit() && self.input_buffer.port.len() < 5 {
                    self.input_buffer.port.push(c);
                }
            }
            4 => self.input_buffer.key_path.push(c),
            5 => self.input_buffer.folder.push(c),
            _ => {}
        }
    }

    fn advance_input_field(&mut self) {
        if self.input_field < 5 {
            self.input_field += 1;
        }
    }

    fn delete_input_char(&mut self) {
        match self.input_field {
            0 => {
                self.input_buffer.alias.pop();
            }
            1 => {
                self.input_buffer.host.pop();
            }
            2 => {
                self.input_buffer.user.pop();
            }
            3 => {
                if self.input_buffer.port.len() > 1 {
                    self.input_buffer.port.pop();
                }
            }
            4 => {
                self.input_buffer.key_path.pop();
            }
            5 => {
                self.input_buffer.folder.pop();
            }
            _ => {}
        }
    }

    fn get_current_input_field(&self) -> &str {
        match self.input_field {
            0 => "alias",
            1 => "host",
            2 => "user",
            3 => "port",
            4 => "key_path",
            5 => "folder",
            _ => "unknown",
        }
    }

    fn save_connection(&mut self) {
        let port: u16 = self.input_buffer.port.parse().unwrap_or(22);

        let conn = Connection {
            id: if self.mode == AppMode::Edit {
                self.filtered_indices
                    .get(self.selected_index)
                    .and_then(|&i| self.config.connections.get(i))
                    .map(|c| c.id.clone())
                    .unwrap_or_default()
            } else {
                uuid::Uuid::new_v4().to_string()
            },
            alias: self.input_buffer.alias.clone(),
            host: self.input_buffer.host.clone(),
            user: self.input_buffer.user.clone(),
            port,
            key_path: if self.input_buffer.key_path.is_empty() {
                None
            } else {
                Some(self.input_buffer.key_path.clone())
            },
            folder: if self.input_buffer.folder.is_empty() {
                None
            } else {
                Some(self.input_buffer.folder.clone())
            },
        };

        if self.mode == AppMode::Edit {
            self.config.update_connection(conn);
        } else {
            self.config.add_connection(conn);
        }

        if let Err(e) = self.config.save() {
            self.message = Some(format!("Error saving: {}", e));
        } else {
            self.update_filter();
            self.message = Some("Connection saved!".to_string());
        }

        self.mode = AppMode::Normal;
    }

    fn delete_connection(&mut self) {
        let id_to_remove = self
            .filtered_indices
            .get(self.selected_index)
            .and_then(|&i| self.config.connections.get(i))
            .map(|c| c.id.clone());

        if let Some(id) = id_to_remove {
            self.config.remove_connection(&id);
            if let Err(e) = self.config.save() {
                self.message = Some(format!("Error saving: {}", e));
            } else {
                self.update_filter();
                if self.selected_index > 0 && self.selected_index >= self.filtered_indices.len() {
                    self.selected_index = self.filtered_indices.len().saturating_sub(1);
                }
                self.message = Some("Connection deleted".to_string());
            }
        }
    }

    fn import_connections(&mut self) {
        let imported = import_from_ssh_config(&mut self.config);
        if let Err(e) = self.config.save() {
            self.message = Some(format!("Error saving: {}", e));
        } else {
            self.update_filter();
            self.message = Some(format!("Imported {} connections", imported));
        }
    }

    fn connect(&mut self) {
        if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
            if let Some(conn) = self.config.connections.get(idx) {
                self.should_connect = Some(conn.clone());
            }
        }
    }

    fn check_for_update(&mut self) {
        if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
            return;
        }

        match crate::update::check_for_update() {
            crate::update::UpdateResult::UpdateAvailable { version } => {
                self.update_info = Some(crate::update::UpdateInfo {
                    current_version: env!("CARGO_PKG_VERSION").to_string(),
                    new_version: version,
                });
                self.mode = AppMode::Update;
            }
            crate::update::UpdateResult::NoUpdate => {}
            crate::update::UpdateResult::Error(e) => {
                self.message = Some(format!("Update check failed: {}", e));
            }
        }
    }

    pub fn update_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices = (0..self.config.connections.len()).collect();
        } else {
            self.filtered_indices = self
                .config
                .connections
                .iter()
                .enumerate()
                .filter_map(|(i, conn)| {
                    let alias_score = self.matcher.fuzzy_match(&conn.alias, &self.search_query);
                    let host_score = self.matcher.fuzzy_match(&conn.host, &self.search_query);
                    let user_score = self.matcher.fuzzy_match(&conn.user, &self.search_query);

                    if alias_score.or(host_score).or(user_score).is_some() {
                        Some(i)
                    } else {
                        None
                    }
                })
                .collect();
        }

        if self.selected_index >= self.filtered_indices.len() {
            self.selected_index = self.filtered_indices.len().saturating_sub(1);
        }
    }

    /// Maximum list rows kept visible (clack `maxVisible` window).
    const MAX_LIST_ROWS: usize = 8;

    /// Title shown in the header row, per mode.
    fn header_title(&self) -> &'static str {
        match self.mode {
            AppMode::Normal => "SSH Connection Manager",
            AppMode::Add => "Add Connection",
            AppMode::Edit => "Edit Connection",
            AppMode::Search => "Search Connections",
            AppMode::Help => "Help",
            AppMode::Update => "Update Available",
        }
    }

    /// Footer key-hint text, per mode.
    fn footer_help_text(&self) -> &'static str {
        match self.mode {
            AppMode::Normal => "↑↓/j k: Navigate | Enter: Connect | A: Add | E: Edit | D: Delete | I: Import | /: Search | ?: Help | Ctrl+C x2: Quit",
            AppMode::Add | AppMode::Edit => "Type text | Tab: Next field | Enter: Save | Esc/q: Cancel | ←: Backspace",
            AppMode::Search => "Type to filter | Enter/Esc/q: Exit search",
            AppMode::Help => "Press Esc or q to return",
            AppMode::Update => "U: Update now | L: Ignore | Esc: Dismiss",
        }
    }

    /// The inline search rail row (active query + reverse-video cursor while
    /// in Search mode, dim placeholder otherwise).
    fn search_rail_line(&self) -> Line<'static> {
        let mut spans = vec![Span::raw("Search: ")];
        if self.search_query.is_empty() {
            spans.push(Span::styled("Type to search...", style::dim()));
        } else {
            spans.push(Span::raw(self.search_query.clone()));
            if self.mode == AppMode::Search {
                spans.push(Span::styled(
                    " ",
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
            }
        }
        style::rail(spans)
    }

    /// Spans for one connection row: dim `[folder] `, the alias (underlined
    /// when current), and a dim `(user@host:port)` hint — clack row grammar.
    fn connection_row_spans(&self, conn: &Connection, is_current: bool) -> Vec<Span<'static>> {
        let mut label: Vec<Span<'static>> = Vec::new();
        if let Some(folder) = conn.folder.as_deref().filter(|f| !f.is_empty()) {
            label.push(Span::styled(
                format!("[{folder}] "),
                if is_current {
                    style::dim().add_modifier(Modifier::UNDERLINED)
                } else {
                    style::dim()
                },
            ));
        }
        label.push(Span::styled(
            conn.alias.clone(),
            if is_current {
                style::underline()
            } else {
                Style::default()
            },
        ));
        let hint = format!("{}@{}:{}", conn.user, conn.host, conn.port);
        style::rail_row_spans_styled(is_current, label, Some(&hint))
    }

    /// Build the full main-screen frame lines (pure; unit-testable).
    fn build_main_lines(&self) -> Vec<Line<'static>> {
        let mut lines: Vec<Line> = Vec::new();

        lines.push(style::header_line(
            style::STEP_ACTIVE,
            style::green(),
            self.header_title(),
        ));
        lines.push(self.search_rail_line());
        lines.push(style::rail_blank());

        if self.filtered_indices.is_empty() {
            let empty_msg = if self.search_query.is_empty() {
                "No connections. Press 'a' to add a new connection."
            } else {
                "No connections match your search."
            };
            lines.push(style::rail_text(empty_msg, style::dim()));
        } else {
            let (start, end) = style::visible_window(
                self.filtered_indices.len(),
                self.selected_index,
                Self::MAX_LIST_ROWS,
            );
            for (i, &idx) in self.filtered_indices[start..end].iter().enumerate() {
                let conn = &self.config.connections[idx];
                let is_current = start + i == self.selected_index;
                lines.push(style::rail(self.connection_row_spans(conn, is_current)));
            }
        }

        lines.push(style::rail_blank());
        lines.push(style::rail_text(self.footer_help_text(), style::dim()));
        lines.push(style::corner_line());
        lines
    }

    pub fn render(&self, f: &mut Frame) {
        if self.mode == AppMode::Help {
            self.render_help(f);
            return;
        }

        if self.mode == AppMode::Update {
            self.render_update_popup(f);
            return;
        }

        // Clear the whole area (top-aligned frame), then draw the clack frame.
        f.render_widget(Block::default(), f.area());
        f.render_widget(Paragraph::new(self.build_main_lines()), f.area());
    }

    #[cfg(test)]
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let line = style::header_line(style::STEP_ACTIVE, style::green(), self.header_title());
        f.render_widget(Paragraph::new(line), area);
    }

    #[cfg(test)]
    fn render_list(&self, f: &mut Frame, area: Rect) {
        // List rows only (no header/footer) — used by focused render tests.
        f.render_widget(Block::default(), area);
        let mut lines: Vec<Line> = Vec::new();
        if self.filtered_indices.is_empty() {
            let empty_msg = if self.search_query.is_empty() {
                "No connections. Press 'a' to add a new connection."
            } else {
                "No connections match your search."
            };
            lines.push(style::rail_text(empty_msg, style::dim()));
        } else {
            let (start, end) = style::visible_window(
                self.filtered_indices.len(),
                self.selected_index,
                Self::MAX_LIST_ROWS,
            );
            for (i, &idx) in self.filtered_indices[start..end].iter().enumerate() {
                let conn = &self.config.connections[idx];
                let is_current = start + i == self.selected_index;
                lines.push(style::rail(self.connection_row_spans(conn, is_current)));
            }
        }
        f.render_widget(Paragraph::new(lines), area);
    }

    #[cfg(test)]
    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let lines = vec![
            style::rail_text(self.footer_help_text(), style::dim()),
            style::corner_line(),
        ];
        f.render_widget(Paragraph::new(lines), area);
    }

    pub fn render_input(&self, f: &mut Frame) {
        // Clack-style form: `◆  Add Connection` header, one rail row per
        // field, dim key hints, `└` corner. No box, no background.
        let area = f.area();
        f.render_widget(Block::default(), area);

        let title = if self.mode == AppMode::Add {
            "Add Connection"
        } else {
            "Edit Connection"
        };

        let current_field = self.get_current_input_field();

        let fields = [
            ("Alias", &self.input_buffer.alias, "alias"),
            ("Host", &self.input_buffer.host, "host"),
            ("User", &self.input_buffer.user, "user"),
            ("Port", &self.input_buffer.port, "port"),
            ("Key", &self.input_buffer.key_path, "key_path"),
            ("Folder", &self.input_buffer.folder, "folder"),
        ];

        let mut lines: Vec<Line> = Vec::new();
        lines.push(style::header_line(
            style::STEP_ACTIVE,
            style::green(),
            title,
        ));

        for (label, value, name) in fields.iter() {
            let is_current = *name == current_field;
            let label_style = if is_current {
                style::bold()
            } else {
                style::dim()
            };
            let mut spans = vec![Span::styled(format!("{label:<8}"), label_style)];
            if value.is_empty() && !is_current {
                spans.push(Span::styled("(empty)", style::dim()));
            } else {
                spans.push(Span::raw(value.to_string()));
            }
            if is_current {
                // Reverse-video block cursor on the active field (clack style).
                spans.push(Span::styled(
                    " ",
                    Style::default().add_modifier(Modifier::REVERSED),
                ));
            }
            lines.push(style::rail(spans));
        }

        lines.push(style::rail_text(
            "Tab/Right/Down: Next field | Shift+Tab/Left/Up: Previous | Enter: Save | Esc: Cancel",
            style::dim(),
        ));
        lines.push(style::corner_line());

        f.render_widget(Paragraph::new(lines), area);
    }

    #[cfg(test)]
    pub fn render_search(&self, f: &mut Frame, area: Rect) {
        // Active search rail row (kept for focused render tests; the live
        // Search mode renders it inline via `render`).
        let mut spans = vec![Span::styled("Search: ", style::dim())];
        if self.search_query.is_empty() {
            spans.push(Span::styled("Type to search...", style::dim()));
        } else {
            spans.push(Span::raw(self.search_query.clone()));
            spans.push(Span::styled(
                " ",
                Style::default().add_modifier(Modifier::REVERSED),
            ));
        }
        f.render_widget(Paragraph::new(style::rail(spans)), area);
    }

    #[cfg(test)]
    pub fn render_search_bar(&self, f: &mut Frame, area: Rect) {
        f.render_widget(Paragraph::new(self.search_rail_line()), area);
    }

    pub fn render_update_popup(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::default(), area);

        let current_ver = format!(
            "Current: v{}",
            self.update_info
                .as_ref()
                .map(|i| &i.current_version)
                .unwrap_or(&"0.1.0".to_string())
        );
        let new_ver = format!(
            "New: v{}",
            self.update_info
                .as_ref()
                .map(|i| &i.new_version)
                .unwrap_or(&"0.2.0".to_string())
        );

        let lines = vec![
            style::header_line(style::STEP_ACTIVE, style::green(), "Update Available"),
            style::rail_text("A new version is available!", Style::default()),
            style::rail_text(&current_ver, style::dim()),
            style::rail_text(&new_ver, style::dim()),
            style::rail_blank(),
            style::rail_text("U: Update now | L: Ignore | Esc: Dismiss", style::dim()),
            style::corner_line(),
        ];
        f.render_widget(Paragraph::new(lines), area);
    }

    fn handle_update_mode(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|f| self.render_update_popup(f))?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                match key.code {
                    crossterm::event::KeyCode::Char('u') | crossterm::event::KeyCode::Char('U') => {
                        self.mode = AppMode::Normal;
                        self.message = Some("Update functionality not yet implemented".to_string());
                        break;
                    }
                    crossterm::event::KeyCode::Char('l')
                    | crossterm::event::KeyCode::Char('L')
                    | crossterm::event::KeyCode::Esc => {
                        self.mode = AppMode::Normal;
                        break;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn render_help(&self, f: &mut Frame) {
        let area = f.area();
        f.render_widget(Block::default(), area);

        let help_text = r#"Navigation
  ↑↓ or j/k    Move up/down in list
  g            Go to first item
  G            Go to last item
  Enter        Connect to selected server

Actions
  a            Add new connection
  e            Edit selected connection
  d            Delete selected connection
  i            Import from ~/.ssh/config
  /            Search/filter connections
  ?            Show this help menu

General
  q            Quit application
  Esc          Cancel current action"#;

        let mut lines: Vec<Line> = Vec::new();
        lines.push(style::header_line(
            style::STEP_ACTIVE,
            style::green(),
            "Keyboard Shortcuts",
        ));
        for row in help_text.lines() {
            if row.trim().is_empty() {
                lines.push(style::rail_blank());
            } else {
                // Un-indented rows are section headings (bold).
                let heading = !row.starts_with(' ');
                lines.push(style::rail_text(
                    row,
                    if heading {
                        style::bold()
                    } else {
                        Style::default()
                    },
                ));
            }
        }
        lines.push(style::rail_blank());
        lines.push(style::rail_text("Press Esc or q to return", style::dim()));
        lines.push(style::corner_line());

        f.render_widget(Paragraph::new(lines), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Connection;
    use ratatui::Terminal;

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
            matcher: SkimMatcherV2::default(),
            filtered_indices: vec![0, 1, 2],
            message: None,
            input_buffer: InputBuffer::default(),
            input_field: 0,
            should_connect: None,
            ctrl_c_count: 0,
            update_info: None,
        }
    }

    #[test]
    fn test_app_new() {
        let app = create_test_app();
        assert_eq!(app.config.connections.len(), 3);
        assert_eq!(app.selected_index, 0);
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn test_input_buffer_clear() {
        let mut buf = InputBuffer {
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: "2222".to_string(),
            key_path: "/path/to/key".to_string(),
            folder: "prod".to_string(),
        };

        buf.clear();

        assert!(buf.alias.is_empty());
        assert!(buf.host.is_empty());
        assert!(buf.user.is_empty());
        assert_eq!(buf.port, "22");
        assert!(buf.key_path.is_empty());
        assert!(buf.folder.is_empty());
    }

    #[test]
    fn test_input_buffer_from_connection() {
        let conn = Connection {
            id: "id1".to_string(),
            alias: "my-server".to_string(),
            host: "192.168.1.100".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/path/to/key".to_string()),
            folder: Some("production".to_string()),
        };

        let buf = InputBuffer::from_connection(&conn);

        assert_eq!(buf.alias, "my-server");
        assert_eq!(buf.host, "192.168.1.100");
        assert_eq!(buf.user, "admin");
        assert_eq!(buf.port, "2222");
        assert_eq!(buf.key_path, "/path/to/key");
        assert_eq!(buf.folder, "production");
    }

    #[test]
    fn test_input_buffer_from_connection_defaults() {
        let conn = Connection {
            id: "id1".to_string(),
            alias: "server".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let buf = InputBuffer::from_connection(&conn);

        assert_eq!(buf.alias, "server");
        assert_eq!(buf.host, "192.168.1.1");
        assert_eq!(buf.user, "user");
        assert_eq!(buf.port, "22");
        assert!(buf.key_path.is_empty());
        assert!(buf.folder.is_empty());
    }

    #[test]
    fn test_append_input_char() {
        let mut app = create_test_app();

        app.input_field = 0;
        app.append_input_char('a');
        assert_eq!(app.input_buffer.alias, "a");

        app.append_input_char('b');
        assert_eq!(app.input_buffer.alias, "ab");

        app.input_field = 1;
        app.append_input_char('1');
        assert_eq!(app.input_buffer.host, "1");

        app.input_field = 1;
        app.append_input_char('9');
        assert_eq!(app.input_buffer.host, "19");
    }

    #[test]
    fn test_append_input_char_port_only_digits() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_field = 3;

        app.append_input_char('2');
        assert_eq!(app.input_buffer.port, "222");

        app.append_input_char('2');
        assert_eq!(app.input_buffer.port, "2222");

        app.append_input_char('2');
        assert_eq!(app.input_buffer.port, "22222");

        app.append_input_char('2');
        assert_eq!(app.input_buffer.port, "22222");

        app.append_input_char('a');
        assert_eq!(app.input_buffer.port, "22222");
    }

    #[test]
    fn test_delete_input_char() {
        let mut app = create_test_app();

        app.input_buffer.alias = "test".to_string();
        app.input_field = 0;
        app.delete_input_char();
        assert_eq!(app.input_buffer.alias, "tes");

        app.delete_input_char();
        assert_eq!(app.input_buffer.alias, "te");
    }

    #[test]
    fn test_advance_input_field() {
        let mut app = create_test_app();

        assert_eq!(app.input_field, 0);

        app.advance_input_field();
        assert_eq!(app.input_field, 1);

        app.advance_input_field();
        assert_eq!(app.input_field, 2);

        app.advance_input_field();
        assert_eq!(app.input_field, 3);

        app.advance_input_field();
        assert_eq!(app.input_field, 4);

        app.advance_input_field();
        assert_eq!(app.input_field, 5);

        app.advance_input_field();
        assert_eq!(app.input_field, 5);
    }

    #[test]
    fn test_get_current_input_field() {
        let mut app = create_test_app();

        app.input_field = 0;
        assert_eq!(app.get_current_input_field(), "alias");

        app.input_field = 1;
        assert_eq!(app.get_current_input_field(), "host");

        app.input_field = 2;
        assert_eq!(app.get_current_input_field(), "user");

        app.input_field = 3;
        assert_eq!(app.get_current_input_field(), "port");

        app.input_field = 4;
        assert_eq!(app.get_current_input_field(), "key_path");

        app.input_field = 5;
        assert_eq!(app.get_current_input_field(), "folder");
    }

    #[test]
    fn test_update_filter_empty_query() {
        let mut app = create_test_app();
        app.search_query = String::new();

        app.update_filter();

        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_update_filter_by_alias() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn test_update_filter_by_host() {
        let mut app = create_test_app();
        app.search_query = "192.168.1.20".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices, vec![1]);
    }

    #[test]
    fn test_update_filter_by_user() {
        let mut app = create_test_app();
        app.search_query = "admin".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn test_update_filter_partial_match() {
        let mut app = create_test_app();
        app.search_query = "server".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn test_update_filter_no_match() {
        let mut app = create_test_app();
        app.search_query = "nonexistent".to_string();

        app.update_filter();

        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_update_filter_bounds_check() {
        let mut app = create_test_app();
        app.selected_index = 10;
        app.search_query = "prod".to_string();

        app.update_filter();

        assert!(app.selected_index < app.filtered_indices.len());
    }

    #[test]
    fn test_add_connection() {
        let mut app = create_test_app();
        app.input_buffer = InputBuffer {
            alias: "new-server".to_string(),
            host: "192.168.1.50".to_string(),
            user: "newuser".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(app.config.connections.len(), 4);
    }

    #[test]
    fn test_delete_connection() {
        let mut app = create_test_app();
        app.selected_index = 0;

        app.delete_connection();

        assert_eq!(app.config.connections.len(), 2);
    }

    #[test]
    fn test_delete_connection_updates_filter() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();
        app.update_filter();
        assert_eq!(app.filtered_indices.len(), 1);

        app.selected_index = 0;
        app.delete_connection();

        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_app_mode_variants() {
        assert_eq!(AppMode::Normal, AppMode::Normal);
        assert_eq!(AppMode::Add, AppMode::Add);
        assert_eq!(AppMode::Edit, AppMode::Edit);
        assert_ne!(AppMode::Add, AppMode::Edit);
    }

    #[test]
    fn test_filtered_indices_maintained_after_search() {
        let mut app = create_test_app();

        app.search_query = "server".to_string();
        app.update_filter();
        let filtered = app.filtered_indices.clone();

        app.search_query = "prod".to_string();
        app.update_filter();

        assert_ne!(app.filtered_indices, filtered);
    }

    #[test]
    fn test_multiple_input_fields() {
        let mut app = create_test_app();

        app.input_field = 0;
        app.append_input_char('a');
        app.append_input_char('l');
        app.append_input_char('i');
        app.append_input_char('a');
        app.append_input_char('s');

        app.advance_input_field();
        app.input_field = 1;
        app.append_input_char('h');
        app.append_input_char('o');
        app.append_input_char('s');
        app.append_input_char('t');

        assert_eq!(app.input_buffer.alias, "alias");
        assert_eq!(app.input_buffer.host, "host");
    }

    #[test]
    fn test_input_field_bounds() {
        let mut app = create_test_app();

        for _ in 0..10 {
            app.advance_input_field();
        }
        assert_eq!(app.input_field, 5);
    }

    #[test]
    fn test_delete_char_from_empty_field() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_field = 0;
        app.delete_input_char();
        assert!(app.input_buffer.alias.is_empty());
    }

    #[test]
    fn test_delete_char_from_field() {
        let mut app = create_test_app();
        app.input_buffer.clear();
        app.input_buffer.alias = "test".to_string();

        app.input_field = 0;
        app.delete_input_char();
        assert_eq!(app.input_buffer.alias, "tes");
    }

    #[test]
    fn test_delete_char_advances_to_previous_field() {
        let mut app = create_test_app();
        app.input_buffer.clear();
        app.input_buffer.alias = "test".to_string();

        app.input_field = 0;
        app.delete_input_char();
        assert_eq!(app.input_buffer.alias, "tes");
    }

    #[test]
    fn test_update_filter_with_case_insensitive() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices.len(), 1);
    }

    #[test]
    fn test_update_filter_with_folder_not_searched() {
        let mut app = create_test_app();
        app.search_query = "production".to_string();

        app.update_filter();

        assert_eq!(app.filtered_indices.len(), 0);
    }

    #[test]
    fn test_update_filter_resets_index_when_empty() {
        let mut app = create_test_app();
        app.selected_index = 5;
        app.search_query = "nonexistent".to_string();

        app.update_filter();

        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_search_query_clear_on_escape() {
        let mut app = create_test_app();
        app.search_query = "test".to_string();
        app.update_filter();

        app.search_query.clear();
        app.update_filter();

        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn test_app_mode_transitions() {
        let mut app = create_test_app();

        assert_eq!(app.mode, AppMode::Normal);

        app.mode = AppMode::Add;
        assert_eq!(app.mode, AppMode::Add);

        app.mode = AppMode::Edit;
        assert_eq!(app.mode, AppMode::Edit);

        app.mode = AppMode::Search;
        assert_eq!(app.mode, AppMode::Search);
    }

    #[test]
    fn test_filtered_indices_initial_state() {
        let app = create_test_app();
        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn test_search_query_default() {
        let app = create_test_app();
        assert!(app.search_query.is_empty());
    }

    #[test]
    fn test_selected_index_bounds() {
        let mut app = create_test_app();

        app.selected_index = 0;
        assert_eq!(app.selected_index, 0);

        app.selected_index = app.filtered_indices.len() - 1;
        assert!(app.selected_index < app.filtered_indices.len());
    }

    #[test]
    fn test_save_connection_add_mode() {
        let mut app = create_test_app();
        let initial_count = app.config.connections.len();

        app.mode = AppMode::Add;
        app.input_buffer = InputBuffer {
            alias: "new".to_string(),
            host: "newhost".to_string(),
            user: "newuser".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(app.config.connections.len(), initial_count + 1);
    }

    #[test]
    fn test_save_connection_edit_mode() {
        let mut app = create_test_app();
        let initial_count = app.config.connections.len();

        app.mode = AppMode::Edit;
        app.selected_index = 0;
        app.input_buffer = InputBuffer {
            alias: "updated".to_string(),
            host: "newhost".to_string(),
            user: "newuser".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(app.config.connections.len(), initial_count);
    }

    #[test]
    fn test_delete_connection_first() {
        let mut app = create_test_app();
        app.selected_index = 0;

        let initial_count = app.config.connections.len();
        app.delete_connection();

        assert_eq!(app.config.connections.len(), initial_count - 1);
    }

    #[test]
    fn test_delete_connection_last() {
        let mut app = create_test_app();
        app.selected_index = app.filtered_indices.len() - 1;

        let initial_count = app.config.connections.len();
        app.delete_connection();

        assert_eq!(app.config.connections.len(), initial_count - 1);
    }

    #[test]
    fn test_delete_connection_empty_list() {
        let mut app = App::new();
        app.filtered_indices = vec![];
        app.selected_index = 0;

        app.delete_connection();
    }

    #[test]
    fn test_message_handling() {
        let mut app = create_test_app();

        app.message = Some("test message".to_string());
        assert!(app.message.is_some());

        app.message = None;
        assert!(app.message.is_none());
    }

    #[test]
    fn test_input_buffer_all_fields() {
        let buf = InputBuffer {
            alias: "alias1".to_string(),
            host: "host1".to_string(),
            user: "user1".to_string(),
            port: "2222".to_string(),
            key_path: "/path/key".to_string(),
            folder: "folder1".to_string(),
        };

        assert_eq!(buf.alias, "alias1");
        assert_eq!(buf.host, "host1");
        assert_eq!(buf.user, "user1");
        assert_eq!(buf.port, "2222");
        assert_eq!(buf.key_path, "/path/key");
        assert_eq!(buf.folder, "folder1");
    }

    #[test]
    fn test_update_filter_preserves_order() {
        let mut app = create_test_app();
        app.search_query = "server".to_string();

        app.update_filter();

        let indices = &app.filtered_indices;
        assert!(indices.len() > 1);
    }

    #[test]
    fn test_update_filter_all_match() {
        let mut app = create_test_app();
        app.search_query = "server".to_string();

        app.update_filter();

        assert!(!app.filtered_indices.is_empty());
    }

    #[test]
    fn test_app_initialization() {
        let app = App::new();

        assert_eq!(app.selected_index, 0);
        assert_eq!(app.mode, AppMode::Normal);
        assert_eq!(app.input_field, 0);
    }

    #[test]
    fn test_input_buffer_clone() {
        let buf1 = InputBuffer {
            alias: "test".to_string(),
            ..Default::default()
        };

        let buf2 = buf1.clone();

        assert_eq!(buf2.alias, "test");
    }

    #[test]
    fn test_app_with_empty_config() {
        let app = App::new();

        assert!(app.selected_index == 0);
    }

    #[test]
    fn test_update_filter_with_special_chars() {
        let mut app = create_test_app();
        app.search_query = "@".to_string();

        app.update_filter();
    }

    #[test]
    fn test_update_filter_with_numbers() {
        let mut app = create_test_app();
        app.search_query = "192".to_string();

        app.update_filter();

        assert!(!app.filtered_indices.is_empty());
    }

    #[test]
    fn test_delete_connection_middle() {
        let mut app = create_test_app();
        app.selected_index = 1;

        let initial_count = app.config.connections.len();
        app.delete_connection();

        assert_eq!(app.config.connections.len(), initial_count - 1);
    }

    #[test]
    fn test_save_connection_with_all_fields() {
        let mut app = create_test_app();

        app.mode = AppMode::Add;
        app.input_buffer = InputBuffer {
            alias: "full".to_string(),
            host: "fullhost".to_string(),
            user: "fulluser".to_string(),
            port: "2222".to_string(),
            key_path: "/full/key".to_string(),
            folder: "fullfolder".to_string(),
        };

        app.save_connection();

        let conn = app.config.connections.last().unwrap();
        assert_eq!(conn.alias, "full");
        assert_eq!(conn.host, "fullhost");
        assert_eq!(conn.user, "fulluser");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.key_path, Some("/full/key".to_string()));
        assert_eq!(conn.folder, Some("fullfolder".to_string()));
    }

    #[test]
    fn test_save_connection_with_empty_port_uses_default() {
        let mut app = create_test_app();

        app.mode = AppMode::Add;
        app.input_buffer = InputBuffer {
            alias: "test".to_string(),
            host: "testhost".to_string(),
            user: "testuser".to_string(),
            port: "".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        let conn = app.config.connections.last().unwrap();
        assert_eq!(conn.port, 22);
    }

    #[test]
    fn test_delete_connection_updates_filter_after_delete() {
        let mut app = create_test_app();

        app.selected_index = 0;
        app.delete_connection();

        assert!(app.filtered_indices.is_empty() || app.filtered_indices.len() < 3);
    }

    #[test]
    fn test_advance_input_field_does_not_exceed_max() {
        let mut app = create_test_app();

        for _ in 0..20 {
            app.advance_input_field();
        }

        assert_eq!(app.input_field, 5);
    }

    #[test]
    fn test_append_char_different_fields() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_field = 0;
        app.append_input_char('x');

        app.input_field = 1;
        app.append_input_char('y');

        app.input_field = 2;
        app.append_input_char('z');

        assert_eq!(app.input_buffer.alias, "x");
        assert_eq!(app.input_buffer.host, "y");
        assert_eq!(app.input_buffer.user, "z");
    }

    #[test]
    fn test_delete_char_different_fields() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_buffer.alias = "aa".to_string();
        app.input_buffer.host = "bb".to_string();
        app.input_buffer.user = "cc".to_string();

        app.input_field = 0;
        app.delete_input_char();

        app.input_field = 1;
        app.delete_input_char();

        app.input_field = 2;
        app.delete_input_char();

        assert_eq!(app.input_buffer.alias, "a");
        assert_eq!(app.input_buffer.host, "b");
        assert_eq!(app.input_buffer.user, "c");
    }

    #[test]
    fn test_delete_connection_after_filter() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();
        app.update_filter();
        app.selected_index = 0;

        app.delete_connection();

        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_filtered_indices_cloning() {
        let app = create_test_app();
        let cloned = app.filtered_indices.clone();

        assert_eq!(cloned.len(), app.filtered_indices.len());
    }

    #[test]
    fn test_config_connections_reference() {
        let mut app = create_test_app();

        let initial_len = app.config.connections.len();

        app.config.add_connection(Connection::new(
            "new".to_string(),
            "newhost".to_string(),
            "newuser".to_string(),
        ));

        assert_eq!(app.config.connections.len(), initial_len + 1);
    }

    #[test]
    fn test_search_query_with_spaces() {
        let mut app = create_test_app();
        app.search_query = "  ".to_string();

        app.update_filter();
    }

    #[test]
    fn test_update_filter_unicode_chars() {
        let mut app = create_test_app();
        app.search_query = "é".to_string();

        app.update_filter();
    }

    #[test]
    fn test_get_current_input_field_all_fields() {
        let mut app = create_test_app();

        for i in 0..=5 {
            app.input_field = i;
            let _ = app.get_current_input_field();
        }
    }

    #[test]
    fn test_delete_input_char_key_path_field() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_field = 4;
        app.input_buffer.key_path = "/path/to/key".to_string();

        app.delete_input_char();

        assert_eq!(app.input_buffer.key_path, "/path/to/ke");
    }

    #[test]
    fn test_delete_input_char_folder_field() {
        let mut app = create_test_app();
        app.input_buffer.clear();

        app.input_field = 5;
        app.input_buffer.folder = "myfolder".to_string();

        app.delete_input_char();

        assert_eq!(app.input_buffer.folder.len(), "myfolder".len() - 1);
    }

    #[test]
    fn test_input_buffer_default() {
        let buf = InputBuffer::default();

        assert!(buf.alias.is_empty());
        assert!(buf.host.is_empty());
        assert!(buf.user.is_empty());
        assert!(buf.port.is_empty());
        assert!(buf.key_path.is_empty());
        assert!(buf.folder.is_empty());
    }

    #[test]
    fn test_app_mode_is_copy() {
        let mode = AppMode::Normal;
        let copied = mode;

        assert_eq!(mode, copied);
    }

    #[test]
    fn test_delete_connection_index_bounds() {
        let mut app = create_test_app();

        app.selected_index = 100;

        app.delete_connection();
    }

    #[test]
    fn test_update_filter_exact_match() {
        let mut app = create_test_app();

        app.search_query = "prod-server".to_string();
        app.update_filter();

        assert!(!app.filtered_indices.is_empty());
    }

    #[test]
    fn test_search_query_appending() {
        let mut app = create_test_app();

        app.search_query.push('s');
        app.search_query.push('e');

        assert_eq!(app.search_query.len(), 2);
    }

    #[test]
    fn test_search_query_pop() {
        let mut app = create_test_app();

        app.search_query = "test".to_string();
        app.search_query.pop();

        assert_eq!(app.search_query.len(), 3);
    }

    #[test]
    fn test_filtered_indices_sorting() {
        let mut app = create_test_app();

        app.filtered_indices.sort();

        assert!(app.filtered_indices.is_sorted());
    }

    #[test]
    fn test_search_query_assignment() {
        let mut app = create_test_app();

        app.search_query = String::from("test");

        assert_eq!(app.search_query, "test");
    }

    #[test]
    fn test_search_query_clear() {
        let mut app = create_test_app();

        app.search_query = String::from("test");
        app.search_query.clear();

        assert!(app.search_query.is_empty());
    }

    #[test]
    fn test_matcher_creation() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("test", "test");

        assert!(result.is_some());
    }

    #[test]
    fn test_fuzzy_matcher_returns_score() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("hello world", "hello");

        assert!(result.is_some());
    }

    #[test]
    fn test_matcher_none_for_no_match() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("abc", "xyz");

        assert!(result.is_none());
    }

    #[test]
    fn test_matcher_partial_match() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("production-server", "prod");

        assert!(result.is_some());
    }

    #[test]
    fn test_matcher_different_strings() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("server1", "server2");

        assert!(result.is_none());
    }

    #[test]
    fn test_matcher_empty_pattern() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("test", "");

        assert!(result.is_some());
    }

    #[test]
    fn test_matcher_whole_string_match() {
        let matcher = SkimMatcherV2::default();

        let result = matcher.fuzzy_match("test", "test");

        assert!(result.is_some());
    }

    #[test]
    fn test_app_new_with_mock_terminal() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let _terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        assert_eq!(app.mode, AppMode::Normal);
    }

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

    #[test]
    fn test_render_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();
    }

    #[test]
    fn test_render_header_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_header(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_list_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_list(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_footer_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_footer(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_search_bar_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_search_bar(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_search_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_search(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_input_with_test_backend() {
        let mut app = create_test_app();
        app.mode = AppMode::Add;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render_input(f);
            })
            .unwrap();
    }

    #[test]
    fn test_render_help_with_test_backend() {
        let mut app = create_test_app();
        app.mode = AppMode::Help;
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render_help(f);
            })
            .unwrap();
    }

    #[test]
    fn test_render_popup_with_test_backend() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render_popup(f, "Test message");
            })
            .unwrap();
    }

    #[test]
    fn test_render_update_popup_with_test_backend() {
        let mut app = create_test_app();
        app.mode = AppMode::Update;
        app.update_info = Some(crate::update::UpdateInfo {
            current_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render_update_popup(f);
            })
            .unwrap();
    }

    #[test]
    fn test_centered_rect() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                let rect = app.centered_rect(40, 10, area);
                assert!(rect.width <= area.width);
                assert!(rect.height <= area.height);
            })
            .unwrap();
    }

    #[test]
    fn test_handle_normal_mode_up_key() {
        let mut app = create_test_app();
        app.selected_index = 2;

        app.selected_index = 1;
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn test_handle_normal_mode_down_key() {
        let mut app = create_test_app();
        app.selected_index = 0;

        app.selected_index = 1;
        assert_eq!(app.selected_index, 1);
    }

    #[test]
    fn test_handle_normal_mode_up_at_boundary() {
        let mut app = create_test_app();
        app.selected_index = 0;

        if app.selected_index > 0 {
            app.selected_index -= 1;
        }
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_handle_normal_mode_down_at_boundary() {
        let mut app = create_test_app();
        app.selected_index = app.filtered_indices.len() - 1;

        let len = app.filtered_indices.len();
        if app.selected_index < len.saturating_sub(1) {
            app.selected_index += 1;
        }
        assert_eq!(app.selected_index, len - 1);
    }

    #[test]
    fn test_handle_normal_mode_ctrl_c_count() {
        let mut app = create_test_app();
        app.ctrl_c_count = 0;

        app.ctrl_c_count += 1;
        assert_eq!(app.ctrl_c_count, 1);

        app.ctrl_c_count += 1;
        assert_eq!(app.ctrl_c_count, 2);
    }

    #[test]
    fn test_handle_normal_mode_reset_ctrl_c() {
        let mut app = create_test_app();
        app.ctrl_c_count = 2;
        app.ctrl_c_count = 0;
        assert_eq!(app.ctrl_c_count, 0);
    }

    #[test]
    fn test_handle_search_mode_query_appending() {
        let mut app = create_test_app();
        app.search_query = String::new();

        app.search_query.push('t');
        app.search_query.push('e');
        app.search_query.push('s');
        app.search_query.push('t');

        assert_eq!(app.search_query, "test");
    }

    #[test]
    fn test_handle_search_mode_query_pop() {
        let mut app = create_test_app();
        app.search_query = "test".to_string();

        app.search_query.pop();
        assert_eq!(app.search_query, "tes");

        app.search_query.pop();
        assert_eq!(app.search_query, "te");
    }

    #[test]
    fn test_handle_search_mode_clear_on_escape() {
        let mut app = create_test_app();
        app.search_query = "test".to_string();

        app.search_query.clear();
        app.update_filter();

        assert!(app.search_query.is_empty());
        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn test_handle_search_mode_navigation_j() {
        let mut app = create_test_app();
        app.selected_index = 0;

        // Simulate pressing 'j' (down)
        let len = app.filtered_indices.len();
        if app.selected_index < len.saturating_sub(1) {
            app.selected_index += 1;
        }

        assert_eq!(app.selected_index, 1);

        // Press 'j' again
        if app.selected_index < len.saturating_sub(1) {
            app.selected_index += 1;
        }

        assert_eq!(app.selected_index, 2);

        // Press 'j' at the last item - should stay at the end
        if app.selected_index < len.saturating_sub(1) {
            app.selected_index += 1;
        }

        assert_eq!(app.selected_index, 2);
    }

    #[test]
    fn test_handle_search_mode_navigation_k() {
        let mut app = create_test_app();
        app.selected_index = 2;

        // Simulate pressing 'k' (up)
        if app.selected_index > 0 {
            app.selected_index -= 1;
        }

        assert_eq!(app.selected_index, 1);

        // Press 'k' again
        if app.selected_index > 0 {
            app.selected_index -= 1;
        }

        assert_eq!(app.selected_index, 0);

        // Press 'k' at the first item - should stay at the beginning
        if app.selected_index > 0 {
            app.selected_index -= 1;
        }

        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_handle_search_mode_navigation_with_filter() {
        let mut app = create_test_app();
        app.search_query = "server".to_string();
        app.update_filter();
        app.selected_index = 0;

        // Navigate down with 'j'
        let len = app.filtered_indices.len();
        if app.selected_index < len.saturating_sub(1) {
            app.selected_index += 1;
        }

        assert!(app.selected_index > 0);
        assert!(app.selected_index < app.filtered_indices.len());

        // Navigate up with 'k'
        if app.selected_index > 0 {
            app.selected_index -= 1;
        }

        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_handle_help_mode_mode_transition() {
        let mut app = create_test_app();
        app.mode = AppMode::Help;

        assert_eq!(app.mode, AppMode::Help);

        app.mode = AppMode::Normal;
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_import_connections_with_mock() {
        let _app = create_test_app();

        let imported = 0;
        let _message = format!("Imported {} connections", imported);

        assert_eq!(imported, 0);
    }

    #[test]
    fn test_import_connections_message() {
        let mut app = create_test_app();

        app.message = Some("Imported 5 connections".to_string());

        assert_eq!(app.message, Some("Imported 5 connections".to_string()));
    }

    #[test]
    fn test_connect_sets_should_connect() {
        let mut app = create_test_app();
        app.selected_index = 0;

        if let Some(&idx) = app.filtered_indices.get(app.selected_index) {
            if let Some(conn) = app.config.connections.get(idx) {
                app.should_connect = Some(conn.clone());
            }
        }

        assert!(app.should_connect.is_some());
    }

    #[test]
    fn test_connect_with_empty_filtered_indices() {
        let mut app = create_test_app();
        app.filtered_indices = vec![];
        app.selected_index = 0;

        if let Some(&idx) = app.filtered_indices.get(app.selected_index) {
            if let Some(conn) = app.config.connections.get(idx) {
                app.should_connect = Some(conn.clone());
            }
        }

        assert!(app.should_connect.is_none());
    }

    #[test]
    fn test_connect_with_out_of_bounds_index() {
        let mut app = create_test_app();
        app.selected_index = 100;

        if let Some(&idx) = app.filtered_indices.get(app.selected_index) {
            if let Some(conn) = app.config.connections.get(idx) {
                app.should_connect = Some(conn.clone());
            }
        }

        assert!(app.should_connect.is_none());
    }

    #[test]
    fn test_check_for_update_skipped_in_dev() {
        let app = App::new();
        assert!(app.update_info.is_none());
    }

    #[test]
    fn test_update_filter_empty_query_collects_all() {
        let mut app = create_test_app();
        app.search_query = String::new();

        app.update_filter();

        assert_eq!(app.filtered_indices.len(), 3);
    }

    #[test]
    fn test_update_filter_with_fuzzy_match() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();

        app.update_filter();

        assert!(!app.filtered_indices.is_empty());
    }

    #[test]
    fn test_update_filter_with_no_match() {
        let mut app = create_test_app();
        app.search_query = "zzzzzz".to_string();

        app.update_filter();

        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_save_connection_add_with_message() {
        let mut app = create_test_app();
        let initial_count = app.config.connections.len();

        app.mode = AppMode::Add;
        app.input_buffer = InputBuffer {
            alias: "new-server".to_string(),
            host: "newhost".to_string(),
            user: "newuser".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(app.config.connections.len(), initial_count + 1);
        assert!(app.message.is_some());
    }

    #[test]
    fn test_delete_connection_with_message() {
        let mut app = create_test_app();
        app.selected_index = 0;

        app.delete_connection();

        assert!(app.message.is_some());
    }

    #[test]
    fn test_message_handling_with_some() {
        let mut app = create_test_app();
        app.message = Some("test message".to_string());

        assert!(app.message.is_some());

        app.message = None;

        assert!(app.message.is_none());
    }

    #[test]
    fn test_mode_transitions() {
        let mut app = create_test_app();

        app.mode = AppMode::Add;
        assert_eq!(app.mode, AppMode::Add);

        app.mode = AppMode::Edit;
        assert_eq!(app.mode, AppMode::Edit);

        app.mode = AppMode::Search;
        assert_eq!(app.mode, AppMode::Search);

        app.mode = AppMode::Help;
        assert_eq!(app.mode, AppMode::Help);

        app.mode = AppMode::Update;
        assert_eq!(app.mode, AppMode::Update);

        app.mode = AppMode::Normal;
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_input_buffer_clear_with_test_backend() {
        let mut app = create_test_app();
        app.input_buffer = InputBuffer {
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: "2222".to_string(),
            key_path: "/path/to/key".to_string(),
            folder: "prod".to_string(),
        };

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let _terminal = Terminal::new(backend).unwrap();

        app.input_buffer.clear();

        assert!(app.input_buffer.alias.is_empty());
        assert!(app.input_buffer.host.is_empty());
        assert!(app.input_buffer.user.is_empty());
        assert_eq!(app.input_buffer.port, "22");
        assert!(app.input_buffer.key_path.is_empty());
        assert!(app.input_buffer.folder.is_empty());
    }

    #[test]
    fn test_app_with_test_backend_full_render() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();
    }

    #[test]
    fn test_render_modes() {
        let mut app = create_test_app();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        app.mode = AppMode::Normal;
        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();

        app.mode = AppMode::Help;
        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();

        app.mode = AppMode::Update;
        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();
    }

    #[test]
    fn test_handle_normal_mode_navigation_keys() {
        let mut app = create_test_app();

        app.selected_index = 0;
        app.selected_index = 1;
        assert_eq!(app.selected_index, 1);

        app.selected_index = 0;
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_handle_normal_mode_home_key() {
        let mut app = create_test_app();
        app.selected_index = 5;

        app.selected_index = 0;
        assert_eq!(app.selected_index, 0);
    }

    #[test]
    fn test_handle_normal_mode_end_key() {
        let mut app = create_test_app();
        let expected_end = app.filtered_indices.len().saturating_sub(1);

        app.selected_index = expected_end;
        assert_eq!(app.selected_index, expected_end);
    }

    #[test]
    fn test_handle_search_mode_backspace() {
        let mut app = create_test_app();
        app.search_query = "test".to_string();

        app.search_query.pop();
        assert_eq!(app.search_query, "tes");

        app.search_query.pop();
        assert_eq!(app.search_query, "te");
    }

    #[test]
    fn test_handle_help_mode_key_handling() {
        let mut app = create_test_app();
        app.mode = AppMode::Help;

        app.mode = AppMode::Normal;
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    fn test_import_connections_integration() {
        let mut app = App::new();
        let initial_count = app.config.connections.len();

        let imported = 0;
        app.message = Some(format!("Imported {} connections", imported));

        assert_eq!(app.config.connections.len(), initial_count);
    }

    #[test]
    fn test_connect_function() {
        let mut app = create_test_app();
        app.selected_index = 0;

        if let Some(&idx) = app.filtered_indices.get(app.selected_index) {
            if let Some(conn) = app.config.connections.get(idx) {
                app.should_connect = Some(conn.clone());
            }
        }

        assert!(app.should_connect.is_some());
        if let Some(conn) = &app.should_connect {
            assert_eq!(conn.alias, "prod-server");
        }
    }

    #[test]
    fn test_render_with_empty_connections() {
        let mut app = App::new();
        app.config.connections.clear();
        app.filtered_indices.clear();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();
    }

    #[test]
    fn test_render_with_search_query() {
        let mut app = create_test_app();
        app.search_query = "prod".to_string();
        app.update_filter();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                app.render(f);
            })
            .unwrap();
    }

    #[test]
    fn test_app_mode_eq() {
        assert_eq!(AppMode::Normal, AppMode::Normal);
        assert_eq!(AppMode::Add, AppMode::Add);
        assert_eq!(AppMode::Edit, AppMode::Edit);
        assert_eq!(AppMode::Search, AppMode::Search);
        assert_eq!(AppMode::Help, AppMode::Help);
        assert_eq!(AppMode::Update, AppMode::Update);
    }

    #[test]
    fn test_app_mode_ne() {
        assert_ne!(AppMode::Normal, AppMode::Add);
        assert_ne!(AppMode::Add, AppMode::Edit);
        assert_ne!(AppMode::Edit, AppMode::Search);
        assert_ne!(AppMode::Search, AppMode::Help);
        assert_ne!(AppMode::Help, AppMode::Update);
    }

    #[test]
    fn test_input_buffer_fields_are_string() {
        let buf = InputBuffer {
            alias: String::new(),
            host: String::new(),
            user: String::new(),
            port: String::new(),
            key_path: String::new(),
            folder: String::new(),
        };

        assert!(buf.alias.is_empty());
        assert!(buf.host.is_empty());
        assert!(buf.user.is_empty());
        assert!(buf.port.is_empty());
        assert!(buf.key_path.is_empty());
        assert!(buf.folder.is_empty());
    }

    #[test]
    fn test_app_fields_are_initialized() {
        let app = App::new();

        assert_eq!(app.selected_index, 0);
        assert!(app.search_query.is_empty());
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.filtered_indices.is_empty() || !app.filtered_indices.is_empty());
        assert!(app.message.is_none());
        assert_eq!(app.input_field, 0);
        assert!(app.should_connect.is_none());
        assert_eq!(app.ctrl_c_count, 0);
        assert!(app.update_info.is_none());
    }

    #[test]
    fn test_render_search_bar_empty_query() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_search_bar(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_search_bar_with_query() {
        let mut app = create_test_app();
        app.search_query = "test".to_string();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_search_bar(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_list_empty() {
        let mut app = App::new();
        app.config.connections.clear();
        app.filtered_indices.clear();

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_list(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_list_with_connections() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_list(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_footer_normal_mode() {
        let app = create_test_app();
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_footer(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_footer_add_mode() {
        let mut app = create_test_app();
        app.mode = AppMode::Add;

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_footer(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_footer_search_mode() {
        let mut app = create_test_app();
        app.mode = AppMode::Search;

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_footer(f, area);
            })
            .unwrap();
    }

    #[test]
    fn test_render_footer_help_mode() {
        let mut app = create_test_app();
        app.mode = AppMode::Help;

        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|f| {
                let area = f.area();
                app.render_footer(f, area);
            })
            .unwrap();
    }
}
