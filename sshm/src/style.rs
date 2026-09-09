//! Clack-style design tokens and line builders.
//!
//! This module is the single source of truth for the app's visual language,
//! which mimics the inline prompt style of
//! [`@clack/prompts`](https://github.com/bombshell-dev/clack) as used by
//! [`vercel-labs/skills`](https://github.com/vercel-labs/skills) — see
//! [`docs/research/clack-style.md`](../docs/research/clack-style.md).
//!
//! Core rules (ported from `vercel-labs/skills/src/prompts/search-multiselect.ts`):
//!
//! - No boxes around the whole frame: a dim `│` left rail + `└` closing corner.
//! - Header line: step icon (`◆` active, `◇` submit, `■` cancel) + two spaces + bold message.
//! - Current item: cyan `❯` cursor column; `●`/`○` state dots; label underlined on the cursor row.
//! - Secondary text is dim (ANSI faint), never a background colour — frames render on the
//!   terminal's default background so they blend into the shell like Clack prompts do.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

// ─────────────────────────────────────────────────────────────────────────────
// Glyphs (mirrors the symbols used by @clack/prompts and the skills CLI)
// ─────────────────────────────────────────────────────────────────────────────

/// Active prompt step icon.
pub const STEP_ACTIVE: &str = "◆";
/// Submitted (settled) prompt step icon.
pub const STEP_SUBMIT: &str = "◇";
/// Cancelled prompt step icon.
pub const STEP_CANCEL: &str = "■";
/// Cursor column marker.
pub const CURSOR: &str = "❯";
/// Selected / current radio dot.
pub const RADIO_ON: &str = "●";
/// Unselected radio dot.
pub const RADIO_OFF: &str = "○";
/// Left rail drawn on every content row.
pub const BAR: &str = "│";
/// Closing corner drawn on the last row of a frame.
pub const CORNER: &str = "└";

// ─────────────────────────────────────────────────────────────────────────────
// Palette
// ─────────────────────────────────────────────────────────────────────────────
//
// Clack uses picocolors' raw ANSI names (`green`, `cyan`, `dim`, …) on the
// terminal's default colours. We mirror that with ratatui's named colours and
// modifiers, and never set a background.

/// Accent green (`picocolors.green`) — step icons, selected dots, highlights.
pub fn green() -> Style {
    Style::default().fg(ratatui::style::Color::Green)
}

/// Cursor cyan (`picocolors.cyan`).
pub fn cyan() -> Style {
    Style::default().fg(ratatui::style::Color::Cyan)
}

/// Faint secondary text (`picocolors.dim` is the ANSI faint attribute).
pub fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

/// Bold message text (`picocolors.bold`).
pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

/// Label emphasis for the row under the cursor (`picocolors.underline`).
pub fn underline() -> Style {
    Style::default().add_modifier(Modifier::UNDERLINED)
}

/// Fuzzy-match character highlight: bold green.
pub fn highlight() -> Style {
    green().add_modifier(Modifier::BOLD)
}

// ─────────────────────────────────────────────────────────────────────────────
// Frame line builders
// ─────────────────────────────────────────────────────────────────────────────

/// Header row: `<icon>  <bold message>`.
///
/// ```
/// use ratatui::style::Style;
/// use sshm::style::{header_line, STEP_ACTIVE};
/// let line = header_line(STEP_ACTIVE, Style::default(), "Pick Connection");
/// assert_eq!(line.spans.len(), 3);
/// ```
pub fn header_line<'a>(icon: &str, icon_style: Style, message: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(icon.to_string(), icon_style),
        Span::raw("  "),
        Span::styled(message, bold()),
    ])
}

/// A rail row: dim `│` + two spaces + the given content spans.
pub fn rail<'a>(spans: Vec<Span<'a>>) -> Line<'a> {
    let mut out = vec![Span::styled(BAR.to_string(), dim()), Span::raw("  ")];
    out.extend(spans);
    Line::from(out)
}

/// A rail row carrying a single styled text span (text is owned).
pub fn rail_text(text: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(BAR.to_string(), dim()),
        Span::raw("  "),
        Span::styled(text.to_string(), style),
    ])
}

/// An empty rail row (just the dim `│`).
pub fn rail_blank() -> Line<'static> {
    Line::from(vec![Span::styled(BAR.to_string(), dim())])
}

/// The closing `└` corner row.
pub fn corner_line() -> Line<'static> {
    Line::from(vec![Span::styled(CORNER.to_string(), dim())])
}

/// Build the spans of one selectable list row inside a rail.
///
/// Layout: `│ ` + cursor column (`❯ ` cyan on the current row, two spaces
/// otherwise) + radio dot (`● ` green current / `○ ` dim otherwise) + label
/// (underlined on the current row) + optional dim ` (hint)`.
///
/// `label` is plain text; use [`rail_row_spans_styled`] when you need
/// per-character spans (e.g. fuzzy-match highlighting).
#[cfg(test)]
pub fn rail_row_spans(is_current: bool, label: &str, hint: Option<&str>) -> Vec<Span<'static>> {
    rail_row_spans_styled(
        is_current,
        vec![Span::styled(
            label.to_string(),
            if is_current {
                underline()
            } else {
                Style::default()
            },
        )],
        hint,
    )
}

/// Like `rail_row_spans` but takes pre-built label spans. The cursor column
/// and radio dot are derived from `is_current`; callers style the label itself
/// (bold/underline/highlights as appropriate).
pub fn rail_row_spans_styled<'a>(
    is_current: bool,
    label: Vec<Span<'a>>,
    hint: Option<&str>,
) -> Vec<Span<'a>> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    if is_current {
        spans.push(Span::styled(format!("{CURSOR} "), cyan()));
        spans.push(Span::styled(format!("{RADIO_ON} "), green()));
    } else {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("{RADIO_OFF} "), dim()));
    }
    spans.extend(label);
    if let Some(hint) = hint {
        spans.push(Span::styled(format!(" ({hint})"), dim()));
    }
    spans
}

/// A sliding visible window over `len` rows keeping `selected` in view,
/// mirroring the `maxVisible` window in skills' `searchMultiselect`.
///
/// Returns `(start, end)` with `start <= selected < end` when `len > 0`, and
/// `(0, 0)` when `len == 0`.
///
/// ```
/// use sshm::style::visible_window;
/// assert_eq!(visible_window(0, 0, 8), (0, 0));
/// assert_eq!(visible_window(3, 2, 8), (0, 3));
/// // With a tiny window, the selection sits one row from the top edge,
/// // mirroring skills' `entryCursor - floor(limit / 2)` centring.
/// assert_eq!(visible_window(20, 10, 4), (8, 12));
/// // Near the end the window snaps back to include the last row.
/// assert_eq!(visible_window(20, 19, 4), (16, 20));
/// ```
pub fn visible_window(len: usize, selected: usize, max_visible: usize) -> (usize, usize) {
    if len == 0 {
        return (0, 0);
    }
    let max_visible = max_visible.max(1);
    if len <= max_visible {
        return (0, len);
    }
    let mut start = selected.saturating_sub(max_visible / 2);
    if start + max_visible > len {
        start = len - max_visible;
    }
    (start, start + max_visible)
}

/// Plain-text lines for a settled (submit/cancel) trace that stays in the
/// shell scrollback after a transient prompt exits — the signature Clack
/// behaviour of collapsing the frame to a compact summary.
///
/// Submit:
/// ```text
/// ◇  Pick Connection
/// │  [staging] web (u@h.com:22)
/// ```
///
/// Cancel:
/// ```text
/// ■  Pick Connection
/// │  Cancelled
/// ```
///
/// Styling (green/red icon, dim summary) is applied by the writer; this pure
/// builder keeps the content unit-testable without escape sequences.
pub fn settle_plain(message: &str, summary: &str, cancelled: bool) -> Vec<String> {
    let icon = if cancelled { STEP_CANCEL } else { STEP_SUBMIT };
    let body = if cancelled { "Cancelled" } else { summary };
    vec![format!("{icon}  {message}"), format!("{BAR}  {body}")]
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn header_line_has_icon_two_spaces_and_message() {
        let line = header_line(STEP_ACTIVE, green(), "Pick Connection");
        assert_eq!(line_text(&line), "◆  Pick Connection");
        // message span is bold
        assert!(line.spans[2].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn rail_rows_carry_bar_prefix() {
        let line = rail_text("hello", Style::default());
        assert_eq!(line_text(&line), "│  hello");
        // bar is dim
        assert!(line.spans[0].style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn rail_blank_and_corner() {
        assert_eq!(line_text(&rail_blank()), "│");
        assert_eq!(line_text(&corner_line()), "└");
    }

    #[test]
    fn row_spans_current_row_shows_cursor_and_radio_on() {
        let spans = rail_row_spans(true, "web (u@h:22)", None);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "❯ ● web (u@h:22)");
        assert!(spans[0].style.fg == Some(ratatui::style::Color::Cyan));
        assert!(spans[1].style.fg == Some(ratatui::style::Color::Green));
        // label underlined on current row
        assert!(spans[2].style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn row_spans_other_row_shows_dim_radio_off_and_hint() {
        let spans = rail_row_spans(false, "db (u@h:22)", Some("dev"));
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "  ○ db (u@h:22) (dev)");
    }

    #[test]
    fn visible_window_clamps_and_tracks_selection() {
        assert_eq!(visible_window(0, 0, 8), (0, 0));
        assert_eq!(visible_window(3, 1, 8), (0, 3));
        // selection in the middle: window centred on it
        assert_eq!(visible_window(20, 10, 4), (8, 12));
        // selection near the start clamps to 0
        assert_eq!(visible_window(20, 0, 4), (0, 4));
        // selection at the end snaps the window to include the last row
        assert_eq!(visible_window(20, 19, 4), (16, 20));
        // never exceeds max_visible, never excludes the selection
        for (s, e) in (0..20).map(|sel| visible_window(20, sel, 4)) {
            assert!(e - s <= 4);
        }
    }

    #[test]
    fn settle_plain_submit_and_cancel_variants() {
        let submit = settle_plain("Pick Connection", "[staging] web (u@h:22)", false);
        assert_eq!(submit[0], "◇  Pick Connection");
        assert_eq!(submit[1], "│  [staging] web (u@h:22)");

        let cancel = settle_plain("Pick Connection", "", true);
        assert_eq!(cancel[0], "■  Pick Connection");
        assert_eq!(cancel[1], "│  Cancelled");
    }
}
