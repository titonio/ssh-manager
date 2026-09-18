//! Design tokens — the single source of truth for sshm's visual language.
//!
//! Before this module existed, the Nord palette was smeared across `app.rs` and
//! `picker.rs` as ~41 bare `Color::Rgb(..)` literals, with three stray named
//! ANSI colors mixed in. That made the palette impossible to audit, impossible to
//! retheme, and — because the insta snapshots capture glyphs but not SGR
//! attributes — invisible to the test suite.
//!
//! The rule this module encodes: **no render code may name a color directly.**
//! Render code names a *role* (`fg_muted`, `border`), and this module decides
//! what pixels that role owns. `tests/design_system_test.rs` enforces it.
//!
//! ```text
//! render code  ──▶  Theme role  ──▶  Clack token  ──▶  Color (named ANSI | neutral grey | Reset)
//!                  (semantic)       (single source)     (mapped by the terminal)
//! ```
//!
//! This palette is **Clack-only** as of the #35 cut-over. The Nord token set
//! survived only because the fullscreen TUI needed a painted background to
//! contrast its text against; with that render path deleted, no live surface
//! owns a background, so there is nothing left for a fixed-RGB palette to
//! govern. The transparent inline frame borrows the user's terminal, and its
//! palette is named ANSI colours plus one indexed neutral grey — see
//! [`Theme::clack`].
//!
//! Two things this module learned the hard way, both from issue #42: a role that
//! no render code reads governs nothing (that was `fg` until it was wired in),
//! and inheriting the terminal's own ink is not the same as being readable —
//! some schemes set that ink to their comment colour.

use ratatui::style::Color;

/// How much color the current terminal can actually render.
///
/// The terminal analogue of a responsive breakpoint: the design has to survive
/// the drop from 16.7M colors down to none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    /// 24-bit truecolor (`COLORTERM=truecolor|24bit`).
    Truecolor,
    /// 256-color palette (`TERM` ends in `256color`).
    Ansi256,
    /// 16-color palette (baseline `xterm`, `vt100`, …).
    Ansi16,
    /// Color suppressed (`NO_COLOR` set, or `TERM=dumb`).
    Monochrome,
}

impl ColorSupport {
    /// Detect capability from the environment.
    pub fn detect() -> Self {
        Self::from_env_vars(
            std::env::var_os("NO_COLOR").as_deref(),
            &std::env::var("TERM").unwrap_or_default(),
            &std::env::var("COLORTERM").unwrap_or_default(),
        )
    }

    /// The capability implied by a given set of environment values.
    ///
    /// Honours the <https://no-color.org> convention — `NO_COLOR` set to
    /// anything non-empty — and treats `TERM=dumb` as "no color, and don't
    /// try". Split out from [`ColorSupport::detect`] because the whole point of
    /// those two rules is that they must hold; testing them against the real
    /// process environment would mean mutating a global that every other test
    /// in the binary is reading concurrently. Taking the values as arguments
    /// makes the rule assertable without that hazard.
    pub fn from_env_vars(no_color: Option<&std::ffi::OsStr>, term: &str, colorterm: &str) -> Self {
        if no_color.is_some_and(|v| !v.is_empty()) {
            return Self::Monochrome;
        }
        if term == "dumb" || term.is_empty() {
            return Self::Monochrome;
        }
        if colorterm == "truecolor" || colorterm == "24bit" {
            return Self::Truecolor;
        }
        if term.contains("256color") {
            return Self::Ansi256;
        }
        Self::Ansi16
    }
}

/// The semantic color roles sshm draws with.
///
/// Render code picks a role; it never picks a color. Adding a new visual state
/// means adding a role here first, which is what keeps the palette coherent as the
/// UI grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Primary body text: row content that carries no other state.
    pub fg: Color,
    /// De-emphasised text: the dim meta, placeholders, empty-state copy.
    pub fg_muted: Color,
    /// Interactive accent: the `◆` step icon, the active step.
    pub accent: Color,
    /// Structural chrome: the `│` rail and the `└` corner.
    pub border: Color,
    /// Fuzzy-match highlight.
    pub highlight: Color,
    /// Positive signal: the hint rail's action words.
    pub success: Color,
    /// Caution signal: reserved for warning states; no live surface paints it
    /// yet, but the role exists so a warning never invents a hue.
    pub warning: Color,
    /// The faintest tier: a field's placeholder, `<optional>` / `<not set>`.
    ///
    /// This role exists to be **fainter than `fg_muted`**, because a
    /// placeholder that wears the field label's colour cannot be told apart
    /// from a label — which is the defect this tier was cut to fix. That
    /// makes it the one role deliberately below the informational contrast
    /// floor, and it is exempt from
    /// `every_informational_role_clears_the_reference_contrast_floor` for
    /// the same reason `border` is: it carries no state.
    ///
    /// A placeholder is an invitation shown where a value is absent. If it
    /// were unreadable the user loses a hint, not information — the field's
    /// own glyph (`○` required, `·` optional) already says whether it needs
    /// filling, so the tier may recede without any state riding on it.
    pub fg_placeholder: Color,
}

impl Theme {
    /// The Clack palette: named ANSI colours, one indexed neutral grey, and
    /// **no fixed RGB**.
    ///
    /// This is the palette the transparent inline frame (#33) draws with. A
    /// transparent frame borrows the user's terminal background, so a hue of
    /// its own is a liability: `White` on a white terminal is 1.35:1.
    ///
    /// **`Reset` is NOT "readable by construction".** That claim used to live
    /// here and issue #42 disproves it. The common One Dark scheme for Windows
    /// Terminal sets `foreground` to `#5C6370` — Atom's *comment* colour, not
    /// its text colour — which is **2.67:1** against that scheme's own
    /// `#1E2127` background. A frame that inherits that ink is unreadable no
    /// matter how principled the inheritance is. Body text therefore names
    /// ANSI 7 (`Gray`), which is One Dark's actual text colour at 7.57:1.
    ///
    /// * **Body text is `Gray`** (ANSI 7) — the palette's own text tone.
    /// * **The muted tier is `Indexed(245)`** (`#8A8A8A`, 4.67:1) — the one
    ///   neutral step the 16-colour set cannot supply. The 16-colour collapse
    ///   is handled in [`Theme::resolve`].
    /// * **Emphasis is a modifier, not a hue** — `BOLD` survives a light
    ///   terminal, `NO_COLOR`, and colour-blindness alike. `DIM` is **not**
    ///   used on informational text: Windows Terminal ignores SGR 2 entirely
    ///   (microsoft/terminal#6703), so emitting it is a claim the target
    ///   terminal cannot honour.
    /// * **Only state gets a hue**, and only from the two named colours Clack
    ///   uses: `Cyan` for the active step, `Green` for a match.
    ///
    /// The palette owns **no background role**. `bg`, `selection_bg`,
    /// `selection_fg` and `fg_bright` were deleted with the fullscreen TUI
    /// (#35): no live render path read them, and a transparent frame that
    /// cannot measure the surface it borrows has nothing to spend a
    /// background token on. Selection is carried by the `❯` glyph plus a
    /// bold alias, not by a filled row.
    pub const fn clack() -> Self {
        Self {
            fg: Color::Gray,
            fg_muted: Color::Indexed(245),
            accent: Color::Cyan,
            border: Color::DarkGray,
            highlight: Color::Green,
            success: Color::Green,
            warning: Color::Yellow,
            fg_placeholder: Color::Indexed(240),
        }
    }

    /// A theme that emits no color at all.
    ///
    /// Structure has to survive on glyphs and layout alone here — which is the
    /// same argument as designing for a monochrome terminal, so nothing may rely
    /// on color to carry meaning.
    ///
    /// **This is not a readability fallback.** Every role is `Reset`, which is
    /// the terminal's own ink — and on the scheme behind issue #42 that ink is
    /// `#5C6370` at 2.67:1. `NO_COLOR` means "give me no colour", and that is
    /// what this delivers, including its consequences.
    pub const fn monochrome() -> Self {
        Self {
            fg: Color::Reset,
            fg_muted: Color::Reset,
            accent: Color::Reset,
            border: Color::Reset,
            highlight: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            fg_placeholder: Color::Reset,
        }
    }

    /// Resolve the theme for a given terminal capability.
    ///
    /// Named ANSI colours are valid in every colour mode — the terminal maps
    /// them to its own palette at the user's chosen values — so `Ansi256` and
    /// `Truecolor` pass through untouched, and `Monochrome` collapses every
    /// role to `Reset`.
    ///
    /// `Ansi16` is the one mode that needs snapping, and it exists purely
    /// because of `fg_muted`. `Indexed(245)` lives in the 256-colour cube; a
    /// 16-colour terminal has no such cell, so the emitted `38;5;245` is
    /// dropped or mapped unpredictably rather than degrading gracefully. It
    /// snaps to `DarkGray`, the nearest thing a 16-colour set offers. The
    /// muted tier is a 256-colour luxury; the body tier (`Gray`, index 7) is
    /// valid everywhere and does not move.
    pub fn resolve(&self, support: ColorSupport) -> Self {
        match support {
            ColorSupport::Monochrome => Self::monochrome(),
            ColorSupport::Ansi16 => Self {
                fg_muted: Color::DarkGray,
                // A 16-colour terminal has no grey ramp to sit below
                // `fg_muted` in, so the placeholder collapses onto the same
                // token. It loses its extra recession and gains nothing that
                // carries state, which is the trade the role is built for.
                fg_placeholder: Color::DarkGray,
                ..*self
            },
            _ => *self,
        }
    }
}

/// Serialize rendered text back to an ANSI string.
///
/// The stock `TestBackend` snapshot writes **glyphs only** — every color, bold
/// and reverse-video attribute is dropped on the floor. That is how nineteen
/// snapshot files sat on top of an unreadable 2.34:1 selection color and never
/// flinched. This re-emits the SGR runs, so a snapshot — or a reviewer with a
/// real terminal — sees what the user sees.
pub mod ansi {
    use ratatui::style::{Color, Modifier, Style};

    /// The SGR full reset.
    ///
    /// Every emitter in this module ends a style run with it, so an attribute
    /// dropped between runs actually disappears instead of bleeding into the
    /// next one. Named once because two hand-written copies of `"\x1b[0m"` is
    /// how the two serializers would drift apart.
    pub const RESET: &str = "\x1b[0m";

    /// Writes styled runs to a `String`, emitting SGR only when the style changes.
    ///
    /// Every serializer here needs the same thing: one escape-code emitter, a
    /// reset before every change of style, and silence while the style holds.
    /// Sharing the writer is what keeps every surface byte-identical for the
    /// same `Style`, which is the promise a theme is worth making.
    pub struct RunWriter<'a> {
        out: &'a mut String,
        current: Option<Style>,
    }

    impl<'a> RunWriter<'a> {
        /// Write into `out`.
        pub fn new(out: &'a mut String) -> Self {
            Self { out, current: None }
        }

        /// Append `text` drawn with `style`.
        pub fn push(&mut self, style: Style, text: &str) {
            if self.current != Some(style) {
                self.out.push_str(RESET);
                self.out.push_str(&style_to_ansi(style));
                self.current = Some(style);
            }
            self.out.push_str(text);
        }

        /// Close the line: reset so nothing bleeds into the next one, then a newline.
        pub fn end_line(&mut self) {
            self.out.push_str(RESET);
            self.out.push('\n');
            self.current = None;
        }
    }

    /// Serialize one rendered line to its ANSI text, without a trailing newline.
    ///
    /// The inline driver (#34) draws row by row — it erases and rewrites the
    /// rows a redraw actually changed — so it needs a line-level serializer.
    /// Same [`RunWriter`], same bytes: a `Style` cannot mean two different
    /// things depending on which surface it is drawn on.
    pub fn line_to_ansi(line: &ratatui::text::Line<'_>) -> String {
        let mut out = String::new();
        let mut runs = RunWriter::new(&mut out);

        for span in &line.spans {
            runs.push(span.style, &span.content);
        }
        out.push_str(RESET);

        out
    }

    /// Named ANSI colors as their 0..=15 index, for SGR emission.
    fn ansi_index(color: Color) -> Option<u8> {
        Some(match color {
            Color::Black => 0,
            Color::Red => 1,
            Color::Green => 2,
            Color::Yellow => 3,
            Color::Blue => 4,
            Color::Magenta => 5,
            Color::Cyan => 6,
            Color::Gray => 7,
            Color::DarkGray => 8,
            Color::LightRed => 9,
            Color::LightGreen => 10,
            Color::LightYellow => 11,
            Color::LightBlue => 12,
            Color::LightMagenta => 13,
            Color::LightCyan => 14,
            Color::White => 15,
            _ => return None,
        })
    }

    fn fg_code(color: Color) -> String {
        match color {
            Color::Reset => "39".to_string(),
            Color::Indexed(n) => format!("38;5;{n}"),
            Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
            named => match ansi_index(named) {
                Some(i) if i < 8 => format!("{}", 30 + i),
                Some(i) => format!("{}", 90 + (i - 8)),
                None => "39".to_string(),
            },
        }
    }

    fn bg_code(color: Color) -> String {
        match color {
            Color::Reset => "49".to_string(),
            Color::Indexed(n) => format!("48;5;{n}"),
            Color::Rgb(r, g, b) => format!("48;2;{r};{g};{b}"),
            named => match ansi_index(named) {
                Some(i) if i < 8 => format!("{}", 40 + i),
                Some(i) => format!("{}", 100 + (i - 8)),
                None => "49".to_string(),
            },
        }
    }

    /// Build the SGR parameter list for a set of modifiers.
    fn modifier_codes(m: Modifier) -> Vec<String> {
        let mut codes = Vec::new();
        if m.contains(Modifier::BOLD) {
            codes.push("1".to_string());
        }
        if m.contains(Modifier::DIM) {
            codes.push("2".to_string());
        }
        if m.contains(Modifier::ITALIC) {
            codes.push("3".to_string());
        }
        if m.contains(Modifier::UNDERLINED) {
            codes.push("4".to_string());
        }
        if m.contains(Modifier::REVERSED) {
            codes.push("7".to_string());
        }
        if m.contains(Modifier::HIDDEN) {
            codes.push("8".to_string());
        }
        if m.contains(Modifier::CROSSED_OUT) {
            codes.push("9".to_string());
        }
        codes
    }

    /// The full SGR escape sequence that puts the terminal into `style`.
    ///
    /// There is exactly one place that turns a `Style` into bytes. A second
    /// hand-rolled emitter would be a palette fork in escape code form.
    pub fn style_to_ansi(style: Style) -> String {
        sgr_codes(style.add_modifier, style.fg, style.bg)
    }

    /// Assemble one SGR sequence from its parts.
    ///
    /// An unset colour becomes the terminal's default (`39`/`49`) rather than a
    /// colour of our choosing — which is what keeps a transparent frame
    /// transparent all the way down to the bytes it emits.
    fn sgr_codes(modifier: Modifier, fg: Option<Color>, bg: Option<Color>) -> String {
        let mut codes = modifier_codes(modifier);
        codes.push(fg_code(fg.unwrap_or(Color::Reset)));
        codes.push(bg_code(bg.unwrap_or(Color::Reset)));
        format!("\x1b[{}m", codes.join(";"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style};
    use ratatui::text::{Line, Span};

    #[test]
    fn clack_tokens_are_named_ansi_or_neutral_grey() {
        // Mirrors `the_clack_palette_is_named_ansi_or_neutral_grey`: a named
        // colour, Reset, or a grey off the 24-step ramp. An indexed *hue* is a
        // palette fork and must not pass.
        let neutral_grey = |c: Color| matches!(c, Color::Indexed(n) if (232..=255).contains(&n));
        let allowed =
            |c: Color| !matches!(c, Color::Rgb(..) | Color::Indexed(_)) || neutral_grey(c);
        let t = Theme::clack();
        for (role, color) in [
            ("fg", t.fg),
            ("fg_muted", t.fg_muted),
            ("accent", t.accent),
            ("border", t.border),
            ("highlight", t.highlight),
            ("success", t.success),
            ("warning", t.warning),
            ("fg_placeholder", t.fg_placeholder),
        ] {
            assert!(
                allowed(color),
                "role `{role}` is {color:?} — named ANSI, Reset, or a neutral \
                 grey from the 232..=255 ramp only"
            );
        }
        // The exception is exercised, not just permitted: the muted tier is the
        // grey the 16-colour set cannot supply (issue #42).
        assert_eq!(t.fg_muted, Color::Indexed(245));
        assert!(neutral_grey(t.fg_muted));
    }

    #[test]
    fn truecolor_resolution_is_a_no_op() {
        assert_eq!(
            Theme::clack().resolve(ColorSupport::Truecolor),
            Theme::clack()
        );
    }

    #[test]
    fn monochrome_suppresses_every_token() {
        let t = Theme::clack().resolve(ColorSupport::Monochrome);
        assert_eq!(t, Theme::monochrome());
    }

    #[test]
    fn named_colours_survive_the_reduced_modes_unmapped() {
        // Named ANSI colours are valid in 256- and 16-colour terminals alike:
        // the terminal maps them, so resolve must not rewrite them into
        // Indexed() and skip the user's own palette.
        for support in [ColorSupport::Ansi256, ColorSupport::Ansi16] {
            let t = Theme::clack().resolve(support);
            assert_eq!(t.accent, Color::Cyan, "{support:?} rewrote the accent");
            assert_eq!(t.highlight, Color::Green, "{support:?} rewrote the hit");
            assert_eq!(t.border, Color::DarkGray, "{support:?} rewrote the rail");
        }
    }

    #[test]
    fn ansi16_snaps_the_muted_grey_it_cannot_render() {
        // Indexed(245) lives in the 256-colour cube. A 16-colour terminal has no
        // such cell, so the emitted `38;5;245` is dropped or mapped
        // unpredictably rather than degrading gracefully — it must be snapped.
        let t = Theme::clack().resolve(ColorSupport::Ansi16);
        assert_eq!(
            t.fg_muted,
            Color::DarkGray,
            "Ansi16 must not ship an index the palette does not have"
        );
        // The body tier is index 7, valid everywhere, and must not move.
        assert_eq!(t.fg, Color::Gray, "Ansi16 must not downgrade the body tier");

        // The cube exists at 256 and above, so the muted grey survives there.
        for support in [ColorSupport::Ansi256, ColorSupport::Truecolor] {
            assert_eq!(
                Theme::clack().resolve(support).fg_muted,
                Color::Indexed(245),
                "{support:?} has a grey ramp and should keep the muted tier"
            );
        }
    }

    #[test]
    fn ansi_serializer_preserves_what_glyph_snapshots_drop() {
        let line = Line::from(vec![
            Span::styled(
                "A",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("B"),
        ]);

        let s = ansi::line_to_ansi(&line);

        assert!(s.contains("\x1b[1;33;49m"), "styled run missing: {s:?}");
        assert!(s.contains('A') && s.contains('B'), "glyphs missing: {s:?}");
    }

    #[test]
    fn ansi_serializer_resets_between_style_runs() {
        let line = Line::from(vec![
            Span::styled("A", Style::default().fg(Color::Cyan)),
            Span::styled("B", Style::default().fg(Color::Green)),
        ]);

        let s = ansi::line_to_ansi(&line);
        // Every style switch must be preceded by a reset, or a dropped attribute
        // would silently bleed into the next run.
        assert_eq!(s.matches("\x1b[0m").count(), 3, "unexpected resets: {s:?}");
    }
}
