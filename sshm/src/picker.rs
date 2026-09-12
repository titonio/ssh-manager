use crate::config::Connection;
use crate::ssh::build_ssh_args;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use std::collections::HashSet;
use std::io;

/// Outcome of running the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOutcome {
    /// User accepted a Connection.
    Selected(Connection),
    /// User cancelled (Esc or Ctrl-C).
    Cancel,
}

/// A fuzzy match result: (original connection index, score, highlight indices
/// within the best-matching field).
pub type MatchResult = (usize, i64, Vec<usize>);

/// How many lines the inline picker occupies by default.
const DEFAULT_HEIGHT: u16 = 15;

/// Emphasis applied to matched characters and the active row.
const HIGHLIGHT_MOD: Modifier = Modifier::BOLD;

// ─────────────────────────────────────────────────────────────────────────────
// Pure helpers (unit-tested, no terminal)
// ─────────────────────────────────────────────────────────────────────────────

/// Pure fuzzy filter over `&[Connection]` + `SkimMatcherV2` + query.
///
/// Searches alias, host, user, and **folder** fields. For each connection, the
/// field yielding the highest `fuzzy_match` score is used for the highlight
/// indices. Results are sorted by score descending (best match first), with
/// original index as tiebreaker.
///
/// An empty query returns all connections with score `0` and no highlights.
pub fn compute_matches(
    connections: &[Connection],
    matcher: &SkimMatcherV2,
    query: &str,
) -> Vec<MatchResult> {
    if query.is_empty() {
        return connections
            .iter()
            .enumerate()
            .map(|(i, _)| (i, 0, vec![]))
            .collect();
    }

    let mut results: Vec<MatchResult> = Vec::new();

    for (i, conn) in connections.iter().enumerate() {
        let fields: Vec<(&str, &str)> = vec![
            ("alias", &conn.alias),
            ("host", &conn.host),
            ("user", &conn.user),
            ("folder", conn.folder.as_deref().unwrap_or("")),
        ];

        let offsets = compute_field_offsets(conn);

        let mut best_score: i64 = i64::MIN;
        let mut best_display_indices: Option<Vec<usize>> = None;

        for (field_name, field) in &fields {
            if let Some((score, indices)) = matcher.fuzzy_indices(field, query) {
                if score > best_score {
                    best_score = score;
                    // Map field-relative indices to absolute display positions.
                    let field_start = offsets
                        .iter()
                        .find(|(n, _, _)| n == field_name)
                        .map(|(_, s, _)| *s)
                        .unwrap_or(0);
                    best_display_indices = Some(indices.iter().map(|&i| field_start + i).collect());
                }
            }
        }

        if let Some(display_indices) = best_display_indices {
            results.push((i, best_score, display_indices));
        }
    }

    results.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    results
}

/// Build the display text for a picker row (without selection prefix).
///
/// Format:
///   `[folder] alias (user@host:port)` when folder is present,
///   `alias (user@host:port)` when it is not.
pub fn build_row_text(conn: &Connection) -> String {
    let folder = conn.folder.as_deref().unwrap_or("");
    if folder.is_empty() {
        format!("{} ({}@{}:{})", conn.alias, conn.user, conn.host, conn.port)
    } else {
        format!(
            "[{}] {} ({}@{}:{})",
            folder, conn.alias, conn.user, conn.host, conn.port
        )
    }
}

/// Compute the start/end character-offsets of each searchable field within the
/// row display text produced by `build_row_text`.
pub fn compute_field_offsets(conn: &Connection) -> Vec<(&str, usize, usize)> {
    let folder = conn.folder.as_deref().unwrap_or("");
    let mut offsets: Vec<(&str, usize, usize)> = Vec::new();

    if !folder.is_empty() {
        offsets.push(("folder", 1, 1 + folder.len()));
    }

    let alias_start = if folder.is_empty() {
        0
    } else {
        folder.len() + 3
    };
    offsets.push(("alias", alias_start, alias_start + conn.alias.len()));

    let user_start = alias_start + conn.alias.len() + 2; // " (" is 2 chars
    offsets.push(("user", user_start, user_start + conn.user.len()));

    let host_start = user_start + conn.user.len() + 1;
    offsets.push(("host", host_start, host_start + conn.host.len()));

    let port_start = host_start + conn.host.len() + 1;
    offsets.push(("port", port_start, port_start + conn.port.to_string().len()));

    offsets
}

/// Build a `Vec<Span>` for a single picker row with matched-char highlighting.
///
/// `highlight_indices` are **absolute display positions** (as produced by
/// `compute_matches`). No offset translation is needed here.
pub fn build_picker_row_spans<'a>(
    conn: &'a Connection,
    highlight_indices: &'a [usize],
    is_selected: bool,
) -> Vec<Span<'a>> {
    let t = crate::theme::active();
    let text = build_row_text(conn);
    let highlight_set: HashSet<usize> = highlight_indices.iter().copied().collect();

    let highlight_style = Style::default().fg(t.highlight).add_modifier(HIGHLIGHT_MOD);

    // Selected row. Uses the dedicated selection roles rather than accent +
    // fg_bright, which measured 2.34:1 and failed WCAG AA.
    let selected_style = Style::default()
        .bg(t.selection_bg)
        .fg(t.selection_fg)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span> = Vec::new();

    // Explicit selection marker. `List::highlight_symbol` only paints when the
    // list is rendered statefully (`render_stateful_widget` + `ListState`); this
    // picker renders stateless, so the symbol never appeared and selection was
    // carried by background color alone — at 2.34:1, effectively invisible.
    // A glyph marker means the selection survives color-blindness, NO_COLOR and
    // a 16-color terminal alike.
    if is_selected {
        spans.push(Span::styled(
            "> ",
            Style::default()
                .fg(t.selection_fg)
                .bg(t.selection_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }

    for (idx, ch) in text.char_indices() {
        if highlight_set.contains(&idx) {
            spans.push(if is_selected {
                Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(t.selection_fg)
                        .bg(t.highlight)
                        .add_modifier(HIGHLIGHT_MOD),
                )
            } else {
                Span::styled(ch.to_string(), highlight_style)
            });
        } else if is_selected {
            // Apply selected row style to non-highlighted characters
            spans.push(Span::styled(ch.to_string(), selected_style));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
    }

    spans
}

/// Build the full list of `ListItem`s for the picker.
pub fn build_picker_rows<'a>(
    all_connections: &'a [Connection],
    matches: &'a [MatchResult],
    selected_index: usize,
) -> Vec<ListItem<'a>> {
    if matches.is_empty() {
        return vec![];
    }

    let sel = if selected_index < matches.len() {
        selected_index
    } else {
        matches.len().saturating_sub(1)
    };

    matches
        .iter()
        .enumerate()
        .map(|(i, (conn_idx, _score, highlights))| {
            let conn = &all_connections[*conn_idx];
            let spans = build_picker_row_spans(conn, highlights, i == sel);
            ListItem::new(Line::from(spans))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Render (uses terminal)
// ─────────────────────────────────────────────────────────────────────────────

/// Render a single inline picker frame onto `frame`.
pub fn render_picker_frame(
    frame: &mut ratatui::Frame,
    connections: &[Connection],
    matches: &[MatchResult],
    selected_index: usize,
    query: &str,
) {
    let t = crate::theme::active();
    let area = frame.area();
    // The picker draws inside someone else's live terminal, so it may not inherit
    // the background: `fg` on a white terminal is 1.35:1, effectively invisible.
    // Paint an opaque `t.bg` over the whole frame — the same thing every
    // fullscreen panel does — so the dark-background contrast ratios actually
    // apply instead of depending on whatever theme the user happens to run.
    let block = Block::default()
        .title(" Pick Connection ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .style(Style::default().bg(t.bg));

    frame.render_widget(block, area);

    let inner = Rect::new(
        area.x + 1,
        area.y + 1,
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );

    if connections.is_empty() {
        let empty = Paragraph::new("No Connections")
            .alignment(Alignment::Center)
            .style(Style::default().fg(t.fg_muted).bg(t.bg));
        frame.render_widget(empty, inner);
        return;
    }

    if matches.is_empty() && !query.is_empty() {
        let no_matches = Paragraph::new(format!("No matches for \"{}\"", query))
            .alignment(Alignment::Center)
            .style(Style::default().fg(t.fg_muted).bg(t.bg));
        frame.render_widget(no_matches, inner);
        return;
    }

    let items = build_picker_rows(connections, matches, selected_index);

    let query_text = if query.is_empty() {
        "Type to search...".to_string()
    } else {
        query.to_string()
    };
    let query_style = if query.is_empty() {
        Style::default().fg(t.fg_muted).bg(t.bg)
    } else {
        Style::default().fg(t.fg).bg(t.bg)
    };
    let query_para = Paragraph::new(query_text).style(query_style);
    let query_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(query_para, query_area);

    let list_height = inner.height.saturating_sub(1);
    let list_area = Rect::new(inner.x, inner.y + 1, inner.width, list_height);
    // NOTE: `highlight_style`/`highlight_symbol` are inert under a stateless
    // `render_widget` — they only apply via `render_stateful_widget` + `ListState`.
    // The visible selection marker is the ">" span built in
    // `build_picker_row_spans`. These are kept on the selection roles so that a
    // future stateful migration inherits the correct palette instead of a
    // low-contrast accident.
    let list = List::new(items)
        // The base style is what an unstyled row cell actually paints with.
        // Plain rows are built from `Span::raw`, so without this they inherit
        // the terminal's default fg *and* bg — and once the frame goes opaque a
        // light terminal would put its dark default text on our dark panel.
        // Naming both roles here keeps every row deterministic, not just the
        // background.
        .style(Style::default().fg(t.fg).bg(t.bg))
        .highlight_style(
            Style::default()
                .bg(t.selection_bg)
                .fg(t.selection_fg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("  ");
    frame.render_widget(list, list_area);

    let footer_text = "↑↓/j k: Navigate | Enter: Select | Esc/Ctrl-C: Cancel";
    let footer_area = Rect::new(
        inner.x,
        inner.y + inner.height.saturating_sub(1),
        inner.width,
        1,
    );
    let footer = Paragraph::new(footer_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(t.success).bg(t.bg));
    frame.render_widget(footer, footer_area);
}

// ─────────────────────────────────────────────────────────────────────────────
// Runner (interactive)
// ─────────────────────────────────────────────────────────────────────────────

/// Which stream the inline-picker TUI should be drawn to.
///
/// This is the pure routing decision, separated from the side-effect of
/// opening `/dev/tty` so it can be unit-tested in isolation. Test-only: the
/// actual fd-1 redirection is performed by [`StdoutRedirect`] in `run_pick`.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiOutputKind {
    /// Draw the TUI to stdout (the normal `sshm pick` invocation).
    Stdout,
    /// Draw the TUI to `/dev/tty` because stdout is captured and reserved for
    /// returning the selected `ssh` command.
    DevTty,
}

/// Pure atuin-style routing decision (test-only): when stdout is a real TTY,
/// draw the inline picker there as usual; when stdout is captured (e.g. shell
/// command substitution `result=$(sshm pick --query ...)`), route the TUI to
/// `/dev/tty` so the escape sequences reach the real terminal. Without this,
/// ratatui's inline viewport cannot read the cursor position and the picker
/// fails to render when invoked from a ZLE widget. The real side-effect of
/// redirecting fd 1 lives in [`StdoutRedirect`] / `run_pick`.
#[cfg(test)]
fn tui_output_kind(stdout_is_tty: bool) -> TuiOutputKind {
    if stdout_is_tty {
        TuiOutputKind::Stdout
    } else {
        TuiOutputKind::DevTty
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// fd-1 redirection (Unix only) — the atuin-style fix for command substitution
// ─────────────────────────────────────────────────────────────────────────────
//
// Under `result=$(sshm pick --query ...)` the shell connects sshm pick's
// stdout (fd 1) to a pipe and reads the accepted `ssh` command from it.
// But crossterm::cursor::position() (used by ratatui's inline viewport) writes
// `ESC [ 6n` directly to `io::stdout()` — i.e. fd 1 — to ask the terminal for
// the cursor position, and reads the reply via crossterm's event source. When
// fd 1 is a pipe, the request never reaches the terminal, so the read times
// out and the picker fails to render.
//
// The fix: when stdout is captured, temporarily point fd 1 at `/dev/tty` for
// the duration of the picker (so all UI escapes — including crossterm's CPR —
// reach the real terminal) while saving the original pipe fd. The caller then
// prints the accepted `ssh` command to the *restored* stdout (the pipe), so
// `$(...)` still captures exactly the clean `ssh ...` string. This mirrors
// how atuin runs its search behind command substitution.

#[cfg(unix)]
mod fd {
    use std::os::unix::io::RawFd;

    extern "C" {
        fn dup(fd: RawFd) -> RawFd;
        fn dup2(old: RawFd, new: RawFd) -> RawFd;
        fn close(fd: RawFd) -> RawFd;
    }

    /// Duplicate fd 1 (stdout) onto `/dev/tty`, returning the saved original
    /// fd (which [`restore`] will later dup back over fd 1). Returns `None` if
    /// stdout is already a TTY (no redirection needed).
    pub fn redirect_stdout_to_tty() -> std::io::Result<Option<RawFd>> {
        use std::io::IsTerminal;
        use std::os::unix::io::AsRawFd;
        if std::io::stdout().is_terminal() {
            return Ok(None);
        }
        let tty = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")?;
        let saved = unsafe { dup(1) };
        if saved < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { dup2(tty.as_raw_fd(), 1) } < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { close(saved) };
            return Err(e);
        }
        Ok(Some(saved))
    }

    /// Restore fd 1 to the saved original.
    pub fn restore_stdout(saved: RawFd) {
        unsafe {
            dup2(saved, 1);
            close(saved);
        }
    }
}

/// RAII guard that redirects fd 1 to `/dev/tty` on construction (when stdout is
/// captured) and restores the original fd 1 on drop. While live, every write
/// to `io::stdout()` — including crossterm's cursor-position-request — lands on
/// the real terminal, and the saved pipe is reinstated before the caller
/// prints the result.
struct StdoutRedirect {
    #[cfg(unix)]
    saved: Option<std::os::unix::io::RawFd>,
}

#[cfg(unix)]
impl StdoutRedirect {
    fn new() -> std::io::Result<Self> {
        Ok(Self {
            saved: fd::redirect_stdout_to_tty()?,
        })
    }
}

#[cfg(unix)]
impl Drop for StdoutRedirect {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            fd::restore_stdout(saved);
        }
    }
}

#[cfg(not(unix))]
impl StdoutRedirect {
    fn new() -> std::io::Result<Self> {
        Ok(Self {})
    }
}

#[cfg(not(unix))]
impl Drop for StdoutRedirect {
    fn drop(&mut self) {}
}

/// Run the inline picker with live query editing.
///
/// `initial_query` seeds the picker (used by `sshm pick --query <text>`).
pub fn run_pick(connections: Vec<Connection>, initial_query: String) -> io::Result<PickerOutcome> {
    // Atuin-style: when stdout is captured by a shell command substitution
    // (`result=$(sshm pick --query ...)`), redirect fd 1 to `/dev/tty` for the
    // duration of the picker so that *every* terminal escape — including
    // crossterm's cursor-position-request, which writes directly to
    // `io::stdout()` regardless of ratatui's backend writer — reaches the real
    // terminal. The original stdout (the pipe) is saved and reinstated on
    // drop, so the caller's `println!` of the accepted `ssh` command is still
    // captured by `$(...)`. Construct this guard *before* raw mode so the
    // RawModeGuard drop (which shows the cursor via io::stdout()) also lands
    // on /dev/tty; the StdoutRedirect drops last, restoring fd 1.
    let _stdout_redirect = StdoutRedirect::new()?;

    crossterm::terminal::enable_raw_mode()?;

    struct RawModeGuard;
    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            // fd 1 is still routed to /dev/tty while this guard drops (the
            // StdoutRedirect guard outlives it), so cursor::Show reaches the
            // same terminal the picker drew to.
            let _ = crossterm::execute!(io::stdout(), crossterm::cursor::Show);
            ratatui::restore();
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
    let _guard = RawModeGuard;

    // Disable mouse support to ensure all input goes to key handlers
    crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
    // Disable focus events to prevent them from interfering with key handling
    crossterm::execute!(io::stdout(), crossterm::event::DisableFocusChange)?;
    // Disable bracketed paste to ensure paste events don't interfere
    crossterm::execute!(io::stdout(), crossterm::event::DisableBracketedPaste)?;
    crossterm::execute!(io::stdout(), crossterm::cursor::Hide)?;

    // With fd 1 optionally redirected to /dev/tty, the ratatui backend can
    // simply use `io::stdout()` — all draws go to the real terminal.
    let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
    let mut terminal = ratatui::Terminal::with_options(
        backend,
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(DEFAULT_HEIGHT),
        },
    )?;

    let matcher = SkimMatcherV2::default();
    let mut query = initial_query;
    let mut matches = compute_matches(&connections, &matcher, &query);
    let mut selected_index = 0;

    // Ensure selected_index is valid even with empty matches
    if !matches.is_empty() && selected_index >= matches.len() {
        selected_index = matches.len().saturating_sub(1);
    }

    if connections.is_empty() {
        terminal
            .draw(|f| render_picker_frame(f, &connections, &matches, selected_index, &query))?;
        loop {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    break;
                }
            }
        }
        return Ok(PickerOutcome::Cancel);
    }

    // Initial draw to ensure the UI is rendered before we start reading input
    terminal.draw(|f| render_picker_frame(f, &connections, &matches, selected_index, &query))?;

    // Ensure the terminal is ready for input before entering the main loop
    // This helps with terminals that need a moment to initialize after enabling raw mode
    let _ = event::poll(std::time::Duration::from_millis(100));

    // Recompute matches from the current query and clamp selected_index.
    // Shared between Backspace and Char arms to avoid duplicated code.
    let recompute = |query: &str, matches: &mut Vec<MatchResult>, selected_index: &mut usize| {
        *matches = compute_matches(&connections, &matcher, query);
        // Clamp selected_index to valid range, handling empty matches case
        if matches.is_empty() {
            *selected_index = 0;
        } else if *selected_index >= matches.len() {
            *selected_index = matches.len().saturating_sub(1);
        }
    };

    let outcome = loop {
        terminal
            .draw(|f| render_picker_frame(f, &connections, &matches, selected_index, &query))?;

        if let Event::Key(key) = event::read()? {
            // Process all key events (Press, Repeat, Release)
            // This ensures navigation works even if terminal sends different event kinds
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected_index = selected_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected_index < matches.len().saturating_sub(1) {
                        selected_index += 1;
                    }
                }
                KeyCode::Enter => {
                    if !matches.is_empty() {
                        let (conn_idx, _, _) = matches[selected_index];
                        break PickerOutcome::Selected(connections[conn_idx].clone());
                    }
                }
                KeyCode::Esc => {
                    break PickerOutcome::Cancel;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break PickerOutcome::Cancel;
                }
                KeyCode::Backspace => {
                    if !query.is_empty() {
                        query.pop();
                        recompute(&query, &mut matches, &mut selected_index);
                    }
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    recompute(&query, &mut matches, &mut selected_index);
                }
                _ => {}
            }
        }
    };

    Ok(outcome)
}

/// Build the literal `ssh` command string for a Connection.
pub fn build_ssh_command(conn: &Connection) -> String {
    let args = build_ssh_args(conn);
    let mut parts = vec!["ssh".to_string()];
    for arg in args {
        parts.push(arg.to_string_lossy().to_string());
    }
    parts.join(" ")
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_conn(
        alias: &str,
        host: &str,
        user: &str,
        port: u16,
        folder: Option<&str>,
    ) -> Connection {
        Connection {
            id: String::new(),
            alias: alias.to_string(),
            host: host.to_string(),
            user: user.to_string(),
            port,
            key_path: None,
            folder: folder.map(|s| s.to_string()),
        }
    }

    // ── tui_output_kind (atuin-style /dev/tty routing) ──────────────────

    #[test]
    fn test_tui_output_kind_routes_to_stdout_when_stdout_is_tty() {
        // When stdout is a real TTY (normal `sshm pick`), draw there as usual.
        assert_eq!(tui_output_kind(true), TuiOutputKind::Stdout);
    }

    #[test]
    fn test_tui_output_kind_routes_to_dev_tty_when_stdout_captured() {
        // When stdout is captured (shell command substitution `$(sshm pick ...)`),
        // the TUI must go to /dev/tty so only the ssh command is returned on stdout.
        assert_eq!(tui_output_kind(false), TuiOutputKind::DevTty);
    }

    // ── compute_matches ──────────────────────────────────────────────────

    #[test]
    fn test_compute_matches_empty_query_returns_all() {
        let conns = vec![
            make_conn("alpha", "h1", "u1", 22, None),
            make_conn("beta", "h2", "u2", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1, 0);
        assert!(results[0].2.is_empty());
    }

    #[test]
    fn test_compute_matches_folder_searched() {
        let conns = vec![
            make_conn("web", "web.com", "www", 22, Some("production")),
            make_conn("db", "db.com", "dba", 5432, Some("staging")),
            make_conn("app", "app.com", "dev", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "prod");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0); // original index of "web"
    }

    #[test]
    fn test_compute_matches_folder_staging() {
        let conns = vec![
            make_conn("web", "web.com", "www", 22, Some("production")),
            make_conn("db", "db.com", "dba", 5432, Some("staging")),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "stag");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    #[test]
    fn test_compute_matches_score_ordering() {
        let conns = vec![
            make_conn("xyz-server", "host-a", "u", 22, None),
            make_conn("abc-server", "host-b", "u", 22, None),
            make_conn("server-abc", "host-c", "u", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "abc");
        assert!(results.len() >= 2);
        assert!(results[0].1 >= results[1].1);
    }

    #[test]
    fn test_compute_matches_no_match() {
        let conns = vec![
            make_conn("alpha", "h1", "u1", 22, None),
            make_conn("beta", "h2", "u2", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "zzzznonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_compute_matches_highlights_returned() {
        let conns = vec![make_conn("server", "host.com", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "ser");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].2, vec![0, 1, 2]);
    }

    #[test]
    fn test_compute_matches_searches_user_field() {
        let conns = vec![
            make_conn("web", "web.com", "admin", 22, None),
            make_conn("db", "db.com", "www", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "adm");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0);
    }

    #[test]
    fn test_compute_matches_searches_host_field() {
        let conns = vec![
            make_conn("web", "web.com", "u", 22, None),
            make_conn("db", "database.internal", "u", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "database");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 1);
    }

    // ── build_row_text ───────────────────────────────────────────────────

    #[test]
    fn test_build_row_text_without_folder() {
        let conn = make_conn("web", "example.com", "www", 22, None);
        assert_eq!(build_row_text(&conn), "web (www@example.com:22)");
    }

    #[test]
    fn test_build_row_text_with_folder() {
        let conn = make_conn(
            "prod-web",
            "prod.example.com",
            "admin",
            22,
            Some("production"),
        );
        assert_eq!(
            build_row_text(&conn),
            "[production] prod-web (admin@prod.example.com:22)"
        );
    }

    #[test]
    fn test_build_row_text_with_non_default_port() {
        let conn = make_conn("dev", "localhost", "dev", 2222, None);
        assert_eq!(build_row_text(&conn), "dev (dev@localhost:2222)");
    }

    // ── compute_field_offsets ────────────────────────────────────────────

    #[test]
    fn test_compute_field_offsets_without_folder() {
        let conn = make_conn("web", "example.com", "www", 22, None);
        let offsets = compute_field_offsets(&conn);
        // alias starts at 0, ends at 3
        assert_eq!(offsets[0], ("alias", 0, 3));
        // user starts at 5 (alias(3) + " (" (2)), ends at 8
        assert_eq!(offsets[1], ("user", 5, 8));
        // host starts at 9 (user(8) + "@" (1))
        assert_eq!(offsets[2].0, "host");
    }

    #[test]
    fn test_compute_field_offsets_with_folder() {
        let conn = make_conn("web", "example.com", "www", 22, Some("prod"));
        let offsets = compute_field_offsets(&conn);
        // folder: positions 1..5 (after `[`, before `]`)
        assert_eq!(offsets[0], ("folder", 1, 5));
        // alias starts at 7 (folder.len(4) + 3 = `[` `]` ` ` + 1)
        assert_eq!(offsets[1], ("alias", 7, 10));
    }

    // ── build_picker_row_spans ───────────────────────────────────────────

    #[test]
    fn test_build_picker_row_spans_no_highlights() {
        let conn = make_conn("web", "example.com", "www", 22, None);
        let spans = build_picker_row_spans(&conn, &[], false);
        // All spans should be raw (no bold modifier).
        for span in &spans {
            assert!(!span.style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn test_build_picker_row_spans_with_highlights() {
        let conn = make_conn("server", "host.com", "u", 22, None);
        // "ser" matches positions 0,1,2 in "server"
        let spans = build_picker_row_spans(&conn, &[0, 1, 2], false);
        // First three spans should be styled (highlighted) with bold.
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(spans[1].style.add_modifier.contains(Modifier::BOLD));
        assert!(spans[2].style.add_modifier.contains(Modifier::BOLD));
        // Span after the matched region should not be bold.
        assert!(!spans[3].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_build_picker_row_spans_selected_inverted() {
        let conn = make_conn("server", "host.com", "u", 22, None);
        let spans = build_picker_row_spans(&conn, &[0, 1, 2], true);
        // Selected + highlighted → should have bg set (inverted style).
        assert!(spans[0].style.bg.is_some());
    }

    #[test]
    fn test_build_picker_row_spans_folder_highlight() {
        // `highlight_indices` are **absolute display positions** (as produced
        // by `compute_matches`). Positions 1..4 = 'p','r','o','d' in
        // "[production] web (www@web.com:22)".
        let conn = make_conn("web", "web.com", "www", 22, Some("production"));
        let spans = build_picker_row_spans(&conn, &[1, 2, 3, 4], false);
        // Position 0 = '[' should NOT be highlighted.
        assert!(!spans[0].style.add_modifier.contains(Modifier::BOLD));
        // Positions 1..4 = 'p','r','o','d' SHOULD be highlighted.
        for (i, span) in spans.iter().enumerate().skip(1).take(4) {
            assert!(
                span.style.add_modifier.contains(Modifier::BOLD),
                "position {} should be highlighted",
                i
            );
        }
    }

    // ── build_picker_rows ────────────────────────────────────────────────

    #[test]
    fn test_build_picker_rows_empty_matches() {
        let conns = vec![make_conn("web", "h", "u", 22, None)];
        let items = build_picker_rows(&conns, &[], 0);
        assert!(items.is_empty());
    }

    #[test]
    fn test_build_picker_rows_single_item() {
        let conns = vec![make_conn("web", "h", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "web");
        let items = build_picker_rows(&conns, &matches, 0);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn test_build_picker_rows_bounds_adjustment() {
        let conns = vec![make_conn("web", "h", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "web");
        // selected_index beyond matches.len() should be clamped.
        let items = build_picker_rows(&conns, &matches, 100);
        assert_eq!(items.len(), 1);
    }

    // ── render_picker_frame ──────────────────────────────────────────────

    #[test]
    fn test_render_empty_connections_shows_message() {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &[], &[], 0, ""))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("No Connections"));
    }

    #[test]
    fn test_render_no_matches_shows_message() {
        let conns = vec![make_conn("web", "h1", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "zzzznonexistent");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, "zzzznonexistent"))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("No matches"));
    }

    #[test]
    fn test_render_pre_narrowed_list() {
        let conns = vec![
            make_conn(
                "prod-server",
                "192.168.1.10",
                "admin",
                22,
                Some("production"),
            ),
            make_conn(
                "dev-server",
                "192.168.1.20",
                "developer",
                2222,
                Some("development"),
            ),
            make_conn("web-server", "example.com", "www", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "prod");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, "prod"))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("prod-server"));
        // "dev-server" should NOT appear since query narrows to "prod".
        assert!(!content.contains("dev-server"));
    }

    #[test]
    fn test_render_folder_prefix_rows() {
        let conns = vec![
            make_conn("web", "h.com", "u", 22, Some("staging")),
            make_conn("db", "h2.com", "u", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, ""))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        // First row has folder prefix.
        assert!(content.contains("[staging]"));
        // Second row has no folder prefix.
        assert!(content.contains("db (u@h2.com:22)"));
    }

    #[test]
    fn test_render_match_highlight_present() {
        let t = crate::theme::Theme::nord();
        let conns = vec![make_conn("server", "host.com", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "ser");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, "ser"))
            .unwrap();
        let buf = terminal.backend().buffer();
        // Check that some cells have the highlight style applied.
        let has_highlight = buf.content.iter().any(|c| {
            c.style().fg == Some(t.highlight) || c.style().add_modifier.contains(Modifier::BOLD)
        });
        assert!(has_highlight, "Expected highlighted characters in output");
    }

    #[test]
    fn test_render_footer_present() {
        let conns = vec![make_conn("web", "h", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, ""))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains("Navigate"));
        assert!(content.contains("Select"));
        assert!(content.contains("Cancel"));
    }

    // ── build_ssh_command (existing tests, preserved) ────────────────────

    #[test]
    fn test_build_ssh_command_basic() {
        let conn = make_conn("test", "example.com", "admin", 22, None);
        assert_eq!(build_ssh_command(&conn), "ssh admin@example.com");
    }

    #[test]
    fn test_build_ssh_command_with_key() {
        let conn = Connection {
            id: String::new(),
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
        let conn = make_conn("test", "example.com", "admin", 2222, None);
        assert_eq!(build_ssh_command(&conn), "ssh -p 2222 admin@example.com");
    }

    #[test]
    fn test_build_ssh_command_empty_user() {
        let conn = make_conn("test", "example.com", "", 22, None);
        assert_eq!(build_ssh_command(&conn), "ssh example.com");
    }

    #[test]
    fn test_build_ssh_command_full() {
        let conn = Connection {
            id: String::new(),
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

        let conn = make_conn("u", "h", "u", 22, None);
        let selected = PickerOutcome::Selected(conn.clone());
        assert_eq!(selected, PickerOutcome::Selected(conn));
        assert_ne!(selected, PickerOutcome::Cancel);
    }

    // ── compute_field_offsets ↔ build_row_text sync ──────────────────────

    #[test]
    fn test_field_offsets_sync_with_row_text() {
        // compute_field_offsets and build_row_text must agree on row layout.
        // If build_row_text changes, this test catches drift.
        let cases = vec![
            make_conn("web", "example.com", "www", 22, None),
            make_conn(
                "prod-web",
                "prod.example.com",
                "admin",
                22,
                Some("production"),
            ),
            make_conn("dev", "localhost", "dev", 2222, Some("staging")),
        ];
        for conn in cases {
            let text = build_row_text(&conn);
            let offsets = compute_field_offsets(&conn);

            // Field ranges must not overlap.
            for (i, (_, si, ei)) in offsets.iter().enumerate() {
                for (_, sj, ej) in offsets.iter().skip(i + 1) {
                    assert!(
                        ei <= sj || ej <= si,
                        "overlap: ({},{}) vs ({},{})",
                        si,
                        ei,
                        sj,
                        ej
                    );
                }
            }

            // Each field's offset must be within text bounds.
            for (_, s, e) in &offsets {
                assert!(*s < text.len() && *e <= text.len());
            }

            // Start positions must be monotonically increasing.
            for windows in offsets.windows(2) {
                assert!(
                    windows[0].1 < windows[1].1,
                    "field order drift: {:?} should come before {:?}",
                    windows[0].0,
                    windows[1].0
                );
            }

            // Each field's substring in the display text must match the actual value.
            for (name, s, e) in &offsets {
                let display_field = &text[*s..*e];
                let actual: &str = match *name {
                    "folder" => conn.folder.as_deref().unwrap_or(""),
                    "alias" => &conn.alias,
                    "user" => &conn.user,
                    "host" => &conn.host,
                    "port" => &conn.port.to_string(),
                    _ => panic!("unknown field: {}", name),
                };
                assert_eq!(
                    display_field, actual,
                    "field {:?} text drift: display={:?}, actual={:?}",
                    name, display_field, actual
                );
            }
        }
    }

    // ── compute_matches maps indices to display positions ────────────────

    #[test]
    fn test_compute_matches_display_indices_mapped() {
        let conns = vec![make_conn("web", "web.com", "www", 22, Some("production"))];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "prod");
        assert_eq!(results.len(), 1);
        // "prod" matches positions 0..3 in "production".
        // Display: "[production] web (www@web.com:22)" → p,r,o,d at 1,2,3,4.
        assert_eq!(results[0].2, vec![1, 2, 3, 4]);
    }

    #[test]
    fn test_compute_matches_alias_no_folder_zero_offset() {
        // When there's no folder, alias starts at 0, so display = field positions.
        let conns = vec![make_conn("server", "host.com", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let results = compute_matches(&conns, &matcher, "ser");
        assert_eq!(results[0].2, vec![0, 1, 2]);
    }

    // ── insta snapshots (AC11) ───────────────────────────────────────────

    #[test]
    fn test_snapshot_pre_narrowed_list() {
        let conns = vec![
            make_conn(
                "prod-server",
                "192.168.1.10",
                "admin",
                22,
                Some("production"),
            ),
            make_conn(
                "dev-server",
                "192.168.1.20",
                "developer",
                2222,
                Some("development"),
            ),
            make_conn("web-server", "example.com", "www", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, ""))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        insta::assert_snapshot!(content);
    }

    #[test]
    fn test_snapshot_matched_character_highlight() {
        let conns = vec![make_conn(
            "prod-server",
            "192.168.1.10",
            "admin",
            22,
            Some("production"),
        )];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "prod");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, "prod"))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        insta::assert_snapshot!(content);
    }

    #[test]
    fn test_snapshot_folder_vs_no_folder_rows() {
        let conns = vec![
            make_conn("web", "h.com", "u", 22, Some("staging")),
            make_conn("db", "h2.com", "u", 22, None),
        ];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, ""))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        insta::assert_snapshot!(content);
    }

    #[test]
    fn test_snapshot_no_matches_state() {
        let conns = vec![make_conn("web", "h1", "u", 22, None)];
        let matcher = SkimMatcherV2::default();
        let matches = compute_matches(&conns, &matcher, "zzzznonexistent");
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &conns, &matches, 0, "zzzznonexistent"))
            .unwrap();
        let content: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        insta::assert_snapshot!(content);
    }
}
