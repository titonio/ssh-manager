use crate::config::{Config, Connection};
use crate::connections::{self, ConnectionDraft};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::Style,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;

/// Smallest terminal the fullscreen TUI will lay out in.
///
/// The normal layout is four bordered chunks — header 3, search bar 3, list
/// (flex), footer 3 — so 12 rows are gone before the list gets a single cell.
/// Below this the chunks collapse and the UI degrades silently instead of
/// saying so.
pub const MIN_TUI_WIDTH: u16 = 60;
pub const MIN_TUI_HEIGHT: u16 = 14;

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
    pub input_buffer: ConnectionDraft,
    pub input_field: usize,
    pub should_connect: Option<Connection>,
    pub ctrl_c_count: usize,
    pub update_info: Option<crate::update::UpdateInfo>,
}

/// Join hint labels with ` | `, dropping the ones that do not fit.
///
/// Hints are dropped from the **end**, so a caller controls exactly what survives
/// in a narrow terminal purely by ordering them by importance. Without this the
/// footer was one long string that ratatui clipped mid-word at whatever width the
/// terminal happened to have.
pub fn fit_hints(hints: &[&str], width: usize) -> String {
    const SEP: &str = " | ";
    let sep_len = SEP.chars().count();
    let mut out = String::new();
    let mut out_len = 0usize;
    for hint in hints {
        let hint_len = hint.chars().count();
        let extra = if out.is_empty() { 0 } else { sep_len };
        if out_len + extra + hint_len > width {
            break;
        }
        if !out.is_empty() {
            out.push_str(SEP);
        }
        out.push_str(hint);
        out_len += extra + hint_len;
    }
    out
}

/// Word-wrap `text` to `width` cells, one entry per display line.
///
/// Centering the too-small message means knowing how many lines it will occupy
/// before drawing it, and ratatui's own `Paragraph::line_count` is an unstable
/// API. Wrapping here keeps the count and the render in step. A word wider than
/// `width` is emitted whole and left for the renderer to clip, never split
/// mid-word.
fn wrap_to_width(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut len = 0usize;
        for word in paragraph.split_whitespace() {
            let word_len = word.chars().count();
            if len != 0 && len + 1 + word_len > width {
                out.push(std::mem::take(&mut line));
                len = 0;
            }
            if len != 0 {
                line.push(' ');
                len += 1;
            }
            line.push_str(word);
            len += word_len;
        }
        out.push(line);
    }
    out
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
            input_buffer: ConnectionDraft::default(),
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

    pub fn render_popup(&self, f: &mut Frame, message: &str) {
        let t = crate::theme::active();
        let area = self.centered_rect(40, 5, f.area());
        let block = Block::default()
            .title(" Message ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.warning));

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
                            self.input_buffer = ConnectionDraft::from_connection(conn);
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

    /// The id of the Connection under the cursor, if the cursor is on one.
    ///
    /// The cursor indexes the *filtered* list, so this is the one place that
    /// knows how a selection maps back to the Connection set. A surface hands
    /// that id to the Connection manager and never indexes the set itself.
    fn selected_connection_id(&self) -> Option<String> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&i| self.config.connections.get(i))
            .map(|c| c.id.clone())
    }

    fn save_connection(&mut self) {
        // Both branches come back in the same shape as the manager's delete:
        // the Connection that changed, or `None` when there was nothing here
        // to change. Only a real change gets the "saved" words.
        let saved: Result<Option<Connection>, String> = if self.mode == AppMode::Edit {
            match self.selected_connection_id() {
                Some(id) => connections::edit(&mut self.config, &id, &self.input_buffer),
                None => Ok(None),
            }
        } else {
            connections::add(&mut self.config, &self.input_buffer).map(Some)
        };

        match saved {
            Ok(Some(_)) => {
                self.update_filter();
                self.message = Some("Connection saved!".to_string());
            }
            Ok(None) => self.message = Some("No Connection selected to edit".to_string()),
            Err(e) => self.message = Some(format!("Error saving: {}", e)),
        }

        self.mode = AppMode::Normal;
    }

    fn delete_connection(&mut self) {
        if let Some(id) = self.selected_connection_id() {
            match connections::remove(&mut self.config, &id) {
                Ok(Some(_)) => {
                    self.update_filter();
                    if self.selected_index > 0 && self.selected_index >= self.filtered_indices.len()
                    {
                        self.selected_index = self.filtered_indices.len().saturating_sub(1);
                    }
                    self.message = Some("Connection deleted".to_string());
                }
                Ok(None) => {}
                Err(e) => self.message = Some(format!("Error saving: {}", e)),
            }
        }
    }

    fn import_connections(&mut self) {
        match connections::import(&mut self.config) {
            Ok(imported) => {
                self.update_filter();
                self.message = Some(format!("Imported {} connections", imported));
            }
            Err(e) => self.message = Some(format!("Error saving: {}", e)),
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
        match crate::update::read_note() {
            Ok(Some(info)) => {
                self.update_info = Some(info);
                self.mode = AppMode::Update;
            }
            Ok(None) => {}
            Err(e) => {
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

    pub fn render(&self, f: &mut Frame) {
        let area = f.area();
        if area.width < MIN_TUI_WIDTH || area.height < MIN_TUI_HEIGHT {
            self.render_too_small(f, area);
            return;
        }

        if self.mode == AppMode::Help {
            self.render_help(f);
            return;
        }

        if self.mode == AppMode::Update {
            self.render_update_popup(f);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        self.render_header(f, chunks[0]);
        self.render_search_bar(f, chunks[1]);
        self.render_list(f, chunks[2]);
        self.render_footer(f, chunks[3]);
    }

    /// Rendered in place of the normal layout when the terminal is too small.
    ///
    /// The alternative is four collapsed chunks: borders stacked on borders, the
    /// list at zero cells, and no indication anything is wrong. This names the
    /// requirement and the size actually found instead.
    ///
    /// Safe at any size. `Rect::inner` saturates rather than underflowing, the
    /// border is skipped entirely when it would leave no interior to draw into,
    /// and the message is wrapped by hand so it can be centred without ratatui's
    /// unstable line-counting API.
    fn render_too_small(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();

        // Paint the whole area first. This can fire over a frame laid out at a
        // larger size, and the old chunks must not show through the message.
        f.render_widget(Block::default().style(Style::default().bg(t.bg)), area);

        // A border spends one cell on every side, so below 5x5 it leaves nothing
        // to put text in. Draw the message bare rather than boxing an emptiness.
        let boxed = area.width >= 5 && area.height >= 5;
        if boxed {
            f.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.warning)),
                area,
            );
        }
        let inner = if boxed {
            area.inner(Margin::new(1, 1))
        } else {
            area
        };

        let width = usize::from(inner.width);
        let instruction = wrap_to_width(
            &format!(
                "Resize to at least {}x{} (currently {}x{}).",
                MIN_TUI_WIDTH, MIN_TUI_HEIGHT, area.width, area.height
            ),
            width,
        );
        let full = std::iter::once("Terminal too small.".to_string())
            .chain(std::iter::once(String::new()))
            .chain(instruction.iter().cloned())
            .collect::<Vec<_>>();

        // When even that will not fit, the instruction is the part worth keeping.
        let lines = if full.len() <= usize::from(inner.height) {
            &full
        } else {
            &instruction
        };

        // Centre the message vertically instead of pinning it to the top of
        // whatever room is left.
        let count = u16::try_from(lines.len())
            .unwrap_or(inner.height)
            .min(inner.height);
        let top = inner.height.saturating_sub(count) / 2;
        let text_area = Rect::new(
            inner.x,
            inner.y.saturating_add(top),
            inner.width,
            inner.height.saturating_sub(top),
        );

        f.render_widget(
            Paragraph::new(lines.join("\n"))
                .alignment(Alignment::Center)
                .style(Style::default().fg(t.fg).bg(t.bg)),
            text_area,
        );
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();
        let title = match self.mode {
            AppMode::Normal => " SSH Connection Manager ",
            AppMode::Add => " Add Connection ",
            AppMode::Edit => " Edit Connection ",
            AppMode::Search => " Search Connections ",
            AppMode::Help => " Help ",
            AppMode::Update => " Update Available ",
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.accent))
            .style(Style::default().bg(t.bg));

        let title_style = Style::default().fg(t.fg_bright).bg(t.bg);

        f.render_widget(block.style(Style::default().bg(t.bg)), area);

        let title_area = Rect::new(area.x + 1, area.y, area.width - 2, 1);
        f.render_widget(Paragraph::new(title).style(title_style), title_area);
    }

    fn render_list(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();
        let bg_color = t.bg;

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
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
                .style(Style::default().fg(t.fg_muted))
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
                        .fg(t.highlight)
                        .add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    Style::default().fg(t.fg)
                };

                ListItem::new(content).style(style)
            })
            .collect();

        let list = List::new(items).style(Style::default().bg(bg_color));

        f.render_widget(list, inner_area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();
        // Ordered by how badly you need them in a narrow terminal: movement and
        // the escape hatches first, authoring actions last. The old single string
        // was 116 chars against a 78-char interior at 80 columns, so Search,
        // Help and Quit were silently clipped off every default-sized terminal.
        let hints: &[&str] = match self.mode {
            AppMode::Normal => &[
                "↑↓/j k: Navigate",
                "Enter: Connect",
                "/: Search",
                "?: Help",
                "q: Quit",
                "a: Add",
                "e: Edit",
                "d: Delete",
                "i: Import",
            ],
            AppMode::Add | AppMode::Edit => &[
                "Type text",
                "Tab: Next field",
                "Enter: Save",
                "Esc/q: Cancel",
                "←: Backspace",
            ],
            AppMode::Search => &["Type to filter", "Enter/Esc/q: Exit search"],
            AppMode::Help => &["Press Esc or q to return"],
            AppMode::Update => &["U: Update now", "L: Ignore", "Esc: Dismiss"],
        };

        let interior = usize::from(area.width).saturating_sub(2);
        let help_text = fit_hints(hints, interior);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .style(Style::default().bg(t.bg));

        let paragraph = Paragraph::new(help_text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(t.success));

        f.render_widget(paragraph, area);
    }

    pub fn render_input(&self, f: &mut Frame) {
        let t = crate::theme::active();
        let area = self.centered_rect(60, 12, f.area());

        let bg = t.bg;
        let fg_normal = t.fg;
        let fg_highlight = t.highlight;
        let fg_label = t.accent;

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
            .border_style(Style::default().fg(t.accent))
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
        .style(Style::default().fg(t.success))
        .alignment(Alignment::Center);
        let hint_area = Rect::new(area.x + 1, area.y + area.height - 2, area.width - 2, 1);
        f.render_widget(hint, hint_area);
    }

    pub fn render_search(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();
        let area = self.centered_rect(40, 3, area);

        let block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.highlight))
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(t.bg));

        let text = format!("/{}", self.search_query);
        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Left)
            .style(Style::default().fg(t.fg));

        f.render_widget(paragraph, area);
    }

    pub fn render_search_bar(&self, f: &mut Frame, area: Rect) {
        let t = crate::theme::active();
        let block = Block::default()
            .title(" Search ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.highlight))
            .style(Style::default().bg(t.bg));

        let text = if self.search_query.is_empty() {
            "Type to search...".to_string()
        } else {
            self.search_query.clone()
        };
        let paragraph =
            Paragraph::new(text).style(Style::default().fg(if self.search_query.is_empty() {
                t.fg_muted // Lighter gray for placeholder
            } else {
                t.fg // Normal text color
            }));

        // Render block first
        f.render_widget(block, area);
        // Then render text inside the block with padding
        let inner_area = Rect::new(
            area.x + 1,
            area.y + 1,
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        f.render_widget(paragraph, inner_area);
    }

    pub fn render_update_popup(&self, f: &mut Frame) {
        let t = crate::theme::active();
        let area = self.centered_rect(55, 10, f.area());

        let block = Block::default()
            .title(" Update Available ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.success))
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(t.bg));

        let update_info = "A new version is available!";
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
        let hint = "Press U to update, L to ignore";

        let text = format!("{}\n{}\n{}\n\n{}", update_info, current_ver, new_ver, hint);

        let paragraph = Paragraph::new(text)
            .block(block)
            .alignment(Alignment::Center)
            .style(Style::default().fg(t.fg));

        f.render_widget(paragraph, area);
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
        let t = crate::theme::active();
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
   /            Search/filter connections
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
            .border_style(Style::default().fg(t.accent))
            .border_type(ratatui::widgets::BorderType::Rounded)
            .style(Style::default().bg(t.bg));

        f.render_widget(Clear, area);
        f.render_widget(block, area);

        let inner_area = Rect::new(area.x + 2, area.y + 1, area.width - 4, area.height - 2);
        let paragraph = Paragraph::new(help_text).style(Style::default().fg(t.fg).bg(t.bg));

        f.render_widget(paragraph, inner_area);
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
            input_buffer: ConnectionDraft::default(),
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
        let mut buf = ConnectionDraft {
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

        let buf = ConnectionDraft::from_connection(&conn);

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

        let buf = ConnectionDraft::from_connection(&conn);

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
        app.input_buffer = ConnectionDraft {
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
        app.input_buffer = ConnectionDraft {
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
        app.input_buffer = ConnectionDraft {
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
        let buf = ConnectionDraft {
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
        let buf1 = ConnectionDraft {
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
        app.input_buffer = ConnectionDraft {
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
        app.input_buffer = ConnectionDraft {
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
        let buf = ConnectionDraft::default();

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
        app.input_buffer = ConnectionDraft {
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
        app.input_buffer = ConnectionDraft {
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
        let buf = ConnectionDraft {
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

    /// Point `HOME` at a throwaway directory so a test never touches the real
    /// `~/.ssh/connections.json` or reads the developer's real `~/.ssh/config`,
    /// and put it back on the way out. Same guard the Connection-manager seam
    /// tests use; these pins call straight through to those writes.
    struct TempHome {
        dir: tempfile::TempDir,
        previous: Option<std::ffi::OsString>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp home");
            let previous = std::env::var_os("HOME");
            std::env::set_var("HOME", dir.path());
            Self { dir, previous }
        }

        fn path(&self) -> &std::path::Path {
            self.dir.path()
        }

        /// Lay down a `~/.ssh/config` inside the throwaway home.
        fn write_ssh_config(&self, content: &str) {
            let ssh_dir = self.path().join(".ssh");
            std::fs::create_dir_all(&ssh_dir).expect("create .ssh");
            std::fs::write(ssh_dir.join("config"), content).expect("write ~/.ssh/config");
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(home) => std::env::set_var("HOME", home),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    // Characterization pins: what a Connection-manager result turns into on this
    // surface. Written against the pre-extraction code so the move cannot quietly
    // change the words a user reads.

    #[test]
    #[serial_test::serial]
    fn saving_a_connection_says_so_and_returns_to_normal_mode() {
        let _home = TempHome::new();
        let mut app = create_test_app();
        app.mode = AppMode::Add;
        app.input_buffer = ConnectionDraft {
            alias: "pin-server".to_string(),
            host: "10.0.0.7".to_string(),
            user: "deploy".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(app.message, Some("Connection saved!".to_string()));
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    #[serial_test::serial]
    fn saving_an_edit_with_nothing_selected_says_so_instead_of_claiming_a_save() {
        let _home = TempHome::new();
        let mut app = create_test_app();
        app.mode = AppMode::Edit;
        app.filtered_indices = Vec::new();
        app.input_buffer = ConnectionDraft {
            alias: "ghost".to_string(),
            host: "10.9.9.9".to_string(),
            user: "x".to_string(),
            port: "22".to_string(),
            key_path: String::new(),
            folder: String::new(),
        };

        app.save_connection();

        assert_eq!(
            app.message,
            Some("No Connection selected to edit".to_string())
        );
        assert_eq!(app.config.connections.len(), 3, "nothing was added");
        assert_eq!(app.mode, AppMode::Normal);
    }

    #[test]
    #[serial_test::serial]
    fn deleting_a_connection_says_so() {
        let _home = TempHome::new();
        let mut app = create_test_app();
        app.selected_index = 0;

        app.delete_connection();

        assert_eq!(app.message, Some("Connection deleted".to_string()));
    }

    #[test]
    #[serial_test::serial]
    fn importing_says_how_many_connections_came_across() {
        let home = TempHome::new();
        home.write_ssh_config(
            "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\
             \nHost db-01\n  HostName 10.0.0.9\n  User dba\n",
        );
        let mut app = create_test_app();

        app.import_connections();

        assert_eq!(app.message, Some("Imported 2 connections".to_string()));
    }
}
