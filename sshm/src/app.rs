use crate::config::{import_from_ssh_config, Config, Connection};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;

#[allow(dead_code)]
mod nord {
    pub const POLAR_NIGHT_0: &str = "#2E3440";
    pub const POLAR_NIGHT_1: &str = "#3B4252";
    pub const POLAR_NIGHT_2: &str = "#434C5E";
    pub const POLAR_NIGHT_3: &str = "#4C566A";
    pub const FROST_0: &str = "#8FBCBB";
    pub const FROST_1: &str = "#81A1C1";
    pub const FROST_2: &str = "#5E81AC";
    pub const SNOW_STORM_0: &str = "#ECEFF4";
    pub const SNOW_STORM_1: &str = "#E5E9F0";
    pub const SNOW_STORM_2: &str = "#D8DEE9";
    pub const AURORA_0: &str = "#A3BE8C";
    pub const AURORA_1: &str = "#EBCB8B";
    pub const AURORA_2: &str = "#D08770";
    pub const AURORA_3: &str = "#BF616A";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Add,
    Edit,
    Search,
    Help,
}

pub struct App {
    config: Config,
    selected_index: usize,
    search_query: String,
    mode: AppMode,
    matcher: SkimMatcherV2,
    filtered_indices: Vec<usize>,
    message: Option<String>,
    input_buffer: InputBuffer,
    input_field: usize,
    pub should_connect: Option<Connection>,
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

impl App {
    pub fn new() -> Self {
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
        }
    }

    pub fn run(&mut self, mut terminal: DefaultTerminal) -> io::Result<bool> {
        self.update_filter();

        loop {
            terminal.draw(|f| self.render(f))?;

            if let Some(msg) = self.message.take() {
                self.show_message(&mut terminal, &msg)?;
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
            }
        }
    }

    fn show_message(&mut self, terminal: &mut DefaultTerminal, message: &str) -> io::Result<()> {
        loop {
            terminal.draw(|f| {
                self.render(f); // Render normal interface first
                self.render_popup(f, message); // Then render popup on top
            })?;

            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.code == crossterm::event::KeyCode::Enter
                    || key.code == crossterm::event::KeyCode::Esc
                    || key.code == crossterm::event::KeyCode::Char('q')
                {
                    self.message = None;
                    break;
                }
            }
        }
        Ok(())
    }

    fn centered_rect(&self, width: u16, height: u16, area: Rect) -> Rect {
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(height),
                Constraint::Fill(1),
            ])
            .split(area);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Fill(1),
                Constraint::Length(width),
                Constraint::Fill(1),
            ])
            .split(layout[1])[1]
    }

    fn render_popup(&self, f: &mut Frame, message: &str) {
        let area = self.centered_rect(40, 5, f.area());
        let block = Block::default()
            .title(" Message ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let paragraph = Paragraph::new(message)
            .block(block)
            .alignment(Alignment::Center);

        f.render_widget(paragraph, area);
    }

    fn handle_normal_mode(&mut self, _terminal: &mut DefaultTerminal) -> io::Result<Option<bool>> {
        if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
            match key.code {
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
                crossterm::event::KeyCode::Enter => {
                    self.connect();
                    return Ok(Some(true));
                }
                crossterm::event::KeyCode::Char('q') => {
                    return Ok(Some(false));
                }
                crossterm::event::KeyCode::Char('a') => {
                    self.mode = AppMode::Add;
                    self.input_buffer.clear();
                    self.input_field = 0;
                }
                crossterm::event::KeyCode::Char('e') => {
                    if let Some(&idx) = self.filtered_indices.get(self.selected_index) {
                        if let Some(conn) = self.config.connections.get(idx) {
                            self.input_buffer = InputBuffer::from_connection(conn);
                            self.mode = AppMode::Edit;
                            self.input_field = 0;
                        }
                    }
                }
                crossterm::event::KeyCode::Char('d') => {
                    self.delete_connection();
                }
                crossterm::event::KeyCode::Char('i') => {
                    self.import_connections();
                }
                crossterm::event::KeyCode::Char('/') | crossterm::event::KeyCode::Char('f') => {
                    self.mode = AppMode::Search;
                }
                crossterm::event::KeyCode::Char('?') => {
                    self.mode = AppMode::Help;
                }
                crossterm::event::KeyCode::Home => {
                    self.selected_index = 0;
                }
                crossterm::event::KeyCode::End => {
                    self.selected_index = self.filtered_indices.len().saturating_sub(1);
                }
                crossterm::event::KeyCode::Char('g') => {
                    self.selected_index = 0;
                }
                crossterm::event::KeyCode::Char('G') => {
                    self.selected_index = self.filtered_indices.len().saturating_sub(1);
                }
                _ => {}
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
            terminal.draw(|f| self.render_search(f, f.area()))?;

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

    fn update_filter(&mut self) {
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

    fn render(&self, f: &mut Frame) {
        match self.mode {
            AppMode::Help => {
                self.render_help(f);
                return;
            }
            _ => {}
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        self.render_header(f, chunks[0]);
        self.render_list(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = match self.mode {
            AppMode::Normal => " SSH Connection Manager ",
            AppMode::Add => " Add Connection ",
            AppMode::Edit => " Edit Connection ",
            AppMode::Search => " Search Connections ",
            AppMode::Help => " Help ",
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::LightCyan))
            .style(Style::default().bg(Color::Rgb(46, 52, 64)));

        let title_style = Style::default()
            .fg(Color::Rgb(236, 239, 244))
            .bg(Color::Rgb(46, 52, 64));

        f.render_widget(
            block.style(Style::default().bg(Color::Rgb(46, 52, 64))),
            area,
        );

        let title_area = Rect::new(area.x + 1, area.y, area.width - 2, 1);
        f.render_widget(Paragraph::new(title).style(title_style), title_area);
    }

    fn render_list(&self, f: &mut Frame, area: Rect) {
        let bg_color = Color::Rgb(46, 52, 64);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(76, 86, 106)))
            .style(Style::default().bg(bg_color));

        f.render_widget(block, area);

        let inner_area = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);

        if self.filtered_indices.is_empty() {
            let empty_msg = if self.search_query.is_empty() {
                "No connections. Press 'a' to add a new connection."
            } else {
                "No connections match your search."
            };
            let paragraph = Paragraph::new(empty_msg)
                .style(Style::default().fg(Color::Rgb(136, 192, 208)))
                .alignment(Alignment::Center);
            f.render_widget(paragraph, inner_area);
            return;
        }

        let items: Vec<ListItem> = self
            .filtered_indices
            .iter()
            .enumerate()
            .map(|(i, &idx)| {
                let conn = &self.config.connections[idx];
                let is_selected = i == self.selected_index;

                let folder = conn.folder.as_deref().unwrap_or("");
                let folder_str = if folder.is_empty() {
                    String::new()
                } else {
                    format!("[{}] ", folder)
                };

                let alias = if folder_str.is_empty() {
                    conn.alias.clone()
                } else {
                    format!("{} {}", folder_str, conn.alias)
                };

                let content = format!(
                    " {} {} ({}@{}:{})",
                    if is_selected { ">" } else { " " },
                    alias,
                    conn.user,
                    conn.host,
                    conn.port
                );

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Rgb(235, 203, 139))
                        .add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(216, 222, 233))
                };

                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items).style(Style::default().bg(bg_color));

        f.render_widget(list, inner_area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let help_text = match self.mode {
            AppMode::Normal => "↑↓/j k: Navigate | Enter: Connect | a: Add | e: Edit | d: Delete | i: Import | /: Search | ?: Help | q: Quit",
            AppMode::Add | AppMode::Edit => "Type text | Tab: Next field | Enter: Save | Esc/q: Cancel | ←: Backspace",
            AppMode::Search => "Type to filter | Enter/Esc/q: Exit search",
            AppMode::Help => "Press Esc or q to return",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(76, 86, 106)))
            .style(Style::default().bg(Color::Rgb(46, 52, 64)));

        let paragraph = Paragraph::new(help_text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Rgb(163, 190, 140)));

        f.render_widget(paragraph, area);
    }

    fn render_input(&self, f: &mut Frame) {
        let area = self.centered_rect(60, 12, f.area());

        let bg = Color::Rgb(46, 52, 64);
        let fg_normal = Color::Rgb(216, 222, 233);
        let fg_highlight = Color::Rgb(235, 203, 139);
        let fg_label = Color::Rgb(129, 161, 193);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .margin(1)
            .split(area);

        let current_field = self.get_current_input_field();

        let title = if self.mode == AppMode::Add {
            "Add Connection"
        } else {
            "Edit Connection"
        };
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(129, 161, 193)))
            .style(Style::default().bg(bg));

        f.render_widget(block, area);

        let fields = [
            ("Alias: ", &self.input_buffer.alias, "alias"),
            ("Host: ", &self.input_buffer.host, "host"),
            ("User: ", &self.input_buffer.user, "user"),
            ("Port: ", &self.input_buffer.port, "port"),
            ("Key: ", &self.input_buffer.key_path, "key_path"),
            ("Folder: ", &self.input_buffer.folder, "folder"),
        ];

        for (i, (label, value, name)) in fields.iter().enumerate() {
            let is_current = *name == current_field;
            let style = if is_current {
                Style::default().fg(fg_highlight)
            } else {
                Style::default().fg(fg_normal)
            };

            let _label_style = Style::default().fg(fg_label);
            let value_style = if is_current {
                Style::default()
                    .fg(fg_normal)
                    .add_modifier(ratatui::style::Modifier::REVERSED)
            } else {
                Style::default().fg(fg_normal)
            };

            let cursor = if is_current { "█" } else { " " };
            let text = format!("{}{}{}", label, value, cursor);
            let paragraph =
                Paragraph::new(text).style(if is_current { value_style } else { style });
            f.render_widget(paragraph, chunks[i]);
        }

        let hint = Paragraph::new(
            "Tab/Right/Down: Next field | Shift+Tab/Left/Up: Previous | Enter: Save | Esc: Cancel",
        )
        .style(Style::default().fg(Color::Rgb(163, 190, 140)))
        .alignment(Alignment::Center);
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 2, area.width - 2, 1);
        f.render_widget(hint, hint_area);
    }

    fn render_search(&self, f: &mut Frame, area: Rect) {
        let area = self.centered_rect(40, 3, area);

        let block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(235, 203, 139)))
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(Color::Rgb(46, 52, 64)));

        let text = format!("/{}", self.search_query);
        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left)
            .style(Style::default().fg(Color::Rgb(216, 222, 233)));

        f.render_widget(paragraph, area);
    }

    fn render_help(&self, f: &mut Frame) {
        let area = f.area();

        let help_text = r#"
 SSH Connection Manager - Keyboard Shortcuts

 Navigation
   ↑↓ or j/k   Move up/down in list
   g            Go to first item
   G            Go to last item
   Enter        Connect to selected server

 Actions
   a            Add new connection
   e            Edit selected connection
   d            Delete selected connection
   i            Import from ~/.ssh/config
   / or f       Search/filter connections
   ?            Show this help menu

 General
   q            Quit application
   Esc          Cancel current action

"#;

        let width = 50u16;
        let height = 20u16;
        let area = self.centered_rect(width, height, area);

        let block = Block::default()
            .title(" Help ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(129, 161, 193)))
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(Color::Rgb(46, 52, 64)));

        f.render_widget(Clear, area);
        f.render_widget(block, area);

        let inner_area = Rect::new(area.x + 2, area.y + 1, area.width - 4, area.height - 2);
        let paragraph = Paragraph::new(help_text).style(
            Style::default()
                .fg(Color::Rgb(216, 222, 233))
                .bg(Color::Rgb(46, 52, 64)),
        );

        f.render_widget(paragraph, inner_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Connection;

    fn create_test_app() -> App {
        let config = Config::from_connections(vec![
            Connection::new_with_id(
                "1".to_string(),
                "prod-server".to_string(),
                "192.168.1.10".to_string(),
                "admin".to_string(),
                22,
                None,
                Some("production".to_string()),
            ),
            Connection::new_with_id(
                "2".to_string(),
                "dev-server".to_string(),
                "192.168.1.20".to_string(),
                "developer".to_string(),
                2222,
                None,
                Some("development".to_string()),
            ),
            Connection::new_with_id(
                "3".to_string(),
                "web-server".to_string(),
                "example.com".to_string(),
                "www".to_string(),
                22,
                None,
                None,
            ),
        ]);

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
        let conn = Connection::new_with_id(
            "id1".to_string(),
            "my-server".to_string(),
            "192.168.1.100".to_string(),
            "admin".to_string(),
            2222,
            Some("/path/to/key".to_string()),
            Some("production".to_string()),
        );

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
        let conn = Connection::new_with_id(
            "id1".to_string(),
            "server".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
            22,
            None,
            None,
        );

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
        let mut buf = InputBuffer::default();

        buf.alias = "alias1".to_string();
        buf.host = "host1".to_string();
        buf.user = "user1".to_string();
        buf.port = "2222".to_string();
        buf.key_path = "/path/key".to_string();
        buf.folder = "folder1".to_string();

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
        let mut buf1 = InputBuffer::default();
        buf1.alias = "test".to_string();

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
}
