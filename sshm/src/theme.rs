//! Design tokens — the single source of truth for sshm's visual language.
//!
//! Before this module existed, the Nord palette was smeared across `app.rs` and
//! `picker.rs` as ~41 bare `Color::Rgb(..)` literals, with three stray named
//! ANSI colors mixed in. That made the palette impossible to audit, impossible to
//! retheme, and — because the insta snapshots capture glyphs but not SGR
//! attributes — invisible to the test suite.
//!
//! The rule this module encodes: **no render code may name a color directly.**
//! Render code names a *role* (`fg_muted`, `selection_bg`), and this module decides
//! what pixels that role owns. `tests/design_system_test.rs` enforces it.
//!
//! ```text
//! render code  ──▶  Theme role  ──▶  nord:: constant  ──▶  Color::Rgb
//!                  (semantic)        (single source)       (truecolor | 256 | 16 | none)
//! ```

use ratatui::style::Color;
use std::sync::OnceLock;

/// The Nord palette, defined exactly once.
///
/// These are the canonical Nord hex values; the RGB triples are their literal
/// expansion. Nothing outside this module should cite a Nord hex.
pub mod nord {
    use ratatui::style::Color;

    // Polar Night
    pub const NORD0: Color = Color::Rgb(46, 52, 64); // #2E3440
    pub const NORD1: Color = Color::Rgb(59, 66, 82); // #3B4252
    pub const NORD2: Color = Color::Rgb(67, 76, 94); // #434C5E
    pub const NORD3: Color = Color::Rgb(76, 86, 106); // #4C566A
                                                      // Snow Storm
    pub const NORD4: Color = Color::Rgb(216, 222, 233); // #D8DEE9
    pub const NORD5: Color = Color::Rgb(229, 233, 240); // #E5E9F0
    pub const NORD6: Color = Color::Rgb(236, 239, 244); // #ECEFF4
                                                        // Frost
    pub const NORD7: Color = Color::Rgb(143, 188, 187); // #8FBCBB
    pub const NORD8: Color = Color::Rgb(136, 192, 208); // #88C0D0
    pub const NORD9: Color = Color::Rgb(129, 161, 193); // #81A1C1
    pub const NORD10: Color = Color::Rgb(94, 129, 172); // #5E81AC
                                                        // Aurora
    pub const NORD11: Color = Color::Rgb(191, 97, 106); // #BF616A
    pub const NORD12: Color = Color::Rgb(208, 135, 112); // #D08770
    pub const NORD13: Color = Color::Rgb(235, 203, 139); // #EBCB8B
    pub const NORD14: Color = Color::Rgb(163, 190, 140); // #A3BE8C
    pub const NORD15: Color = Color::Rgb(180, 142, 173); // #B48EAD

    /// Every Nord color, for palette-membership checks.
    pub const ALL: [Color; 16] = [
        NORD0, NORD1, NORD2, NORD3, NORD4, NORD5, NORD6, NORD7, NORD8, NORD9, NORD10, NORD11,
        NORD12, NORD13, NORD14, NORD15,
    ];
}

/// Accessibility math over terminal colors.
///
/// WCAG 2.1 relative luminance and contrast ratio. Terminal cells are just RGB
/// pairs, so the same formula the web uses applies unchanged — and because sshm's
/// palette is small and enumerable, every pair the app can ever draw is cheap to
/// check exhaustively.
pub mod contrast {
    use ratatui::style::Color;

    /// Approximate sRGB triples for the 16 named ANSI colors (xterm defaults).
    ///
    /// Needed so contrast can be evaluated when the app is running in a
    /// reduced-color terminal, where a `Color::Named` is all we have.
    const ANSI16_RGB: [(u8, u8, u8); 16] = [
        (0, 0, 0),       // 0  Black
        (135, 0, 0),     // 1  DarkRed
        (0, 135, 0),     // 2  DarkGreen
        (175, 175, 0),   // 3  DarkYellow
        (0, 0, 135),     // 4  DarkBlue
        (135, 0, 135),   // 5  DarkMagenta
        (0, 175, 175),   // 6  DarkCyan
        (192, 192, 192), // 7  Gray
        (128, 128, 128), // 8  DarkGray
        (255, 0, 0),     // 9  Red
        (0, 255, 0),     // 10 Green
        (255, 255, 0),   // 11 Yellow
        (0, 0, 255),     // 12 Blue
        (255, 0, 255),   // 13 Magenta
        (0, 255, 255),   // 14 Cyan
        (255, 255, 255), // 15 White
    ];

    /// Resolve a `Color` to an sRGB triple.
    ///
    /// `Reset` has no fixed RGB and returns `None`.
    pub fn to_rgb(color: Color) -> Option<(u8, u8, u8)> {
        match color {
            Color::Rgb(r, g, b) => Some((r, g, b)),
            Color::Indexed(n) => Some(indexed_to_rgb(n)),
            Color::Black => Some(ANSI16_RGB[0]),
            Color::Red => Some(ANSI16_RGB[1]),
            Color::Green => Some(ANSI16_RGB[2]),
            Color::Yellow => Some(ANSI16_RGB[3]),
            Color::Blue => Some(ANSI16_RGB[4]),
            Color::Magenta => Some(ANSI16_RGB[5]),
            Color::Cyan => Some(ANSI16_RGB[6]),
            Color::Gray => Some(ANSI16_RGB[7]),
            Color::DarkGray => Some(ANSI16_RGB[8]),
            Color::LightRed => Some(ANSI16_RGB[9]),
            Color::LightGreen => Some(ANSI16_RGB[10]),
            Color::LightYellow => Some(ANSI16_RGB[11]),
            Color::LightBlue => Some(ANSI16_RGB[12]),
            Color::LightMagenta => Some(ANSI16_RGB[13]),
            Color::LightCyan => Some(ANSI16_RGB[14]),
            Color::White => Some(ANSI16_RGB[15]),
            _ => None,
        }
    }

    /// Inverse of the xterm 256-color encoding.
    ///
    /// 0..=15 are the base ANSI colors, 16..=231 a 6×6×6 RGB cube, and
    /// 232..=255 a 24-step grayscale ramp. Needed so contrast can be measured
    /// after a color has been downgraded for a reduced-color terminal.
    fn indexed_to_rgb(n: u8) -> (u8, u8, u8) {
        const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
        if n < 16 {
            ANSI16_RGB[n as usize]
        } else if n <= 231 {
            let i = (n - 16) as usize;
            (CUBE[i / 36], CUBE[(i / 6) % 6], CUBE[i % 6])
        } else {
            let g = 8 + (n - 232) * 10;
            (g, g, g)
        }
    }

    /// WCAG 2.1 relative luminance.
    pub fn luminance(rgb: (u8, u8, u8)) -> f64 {
        fn channel(v: u8) -> f64 {
            let v = f64::from(v) / 255.0;
            if v <= 0.039_28 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(rgb.0) + 0.7152 * channel(rgb.1) + 0.0722 * channel(rgb.2)
    }

    /// WCAG 2.1 contrast ratio between two colors, in `1.0..=21.0`.
    ///
    /// Returns `21.0` for a pair we cannot resolve (treated as safe rather than
    /// silently passing a bogus number).
    pub fn ratio(a: Color, b: Color) -> f64 {
        let (Some(a), Some(b)) = (to_rgb(a), to_rgb(b)) else {
            return 21.0;
        };
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// WCAG AA for normal text: 4.5:1.
    pub fn meets_aa_text(fg: Color, bg: Color) -> bool {
        ratio(fg, bg) >= 4.5
    }

    /// WCAG AA for large/bold text, and 1.4.11 non-text UI contrast: 3:1.
    pub fn meets_aa_large(fg: Color, bg: Color) -> bool {
        ratio(fg, bg) >= 3.0
    }
}

/// How much color the current terminal can actually render.
///
/// The TUI analogue of a responsive breakpoint: the design has to survive the
/// drop from 16.7M colors down to none.
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
    ///
    /// Honours the <https://no-color.org> convention and treats `TERM=dumb` as
    /// "no color, and don't try".
    pub fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return Self::Monochrome;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term == "dumb" || term.is_empty() {
            return Self::Monochrome;
        }
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
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
    /// Window background — every panel sits on this.
    pub bg: Color,
    /// Primary body text: connection rows, form values, popup copy.
    pub fg: Color,
    /// Highest-emphasis text: block titles, selected-row labels.
    pub fg_bright: Color,
    /// De-emphasised text: placeholders, empty-state and no-match messages.
    pub fg_muted: Color,
    /// Interactive accent: popup borders, the picker's frame.
    pub accent: Color,
    /// Structural chrome: the borders that hold the layout together.
    pub border: Color,
    /// Fuzzy-match highlight and the active-row foreground.
    pub highlight: Color,
    /// Positive signal: footer key hints, update-available border.
    pub success: Color,
    /// Caution signal: the dismissable message popup border.
    pub warning: Color,
    /// Background of the highlighted/selected row.
    pub selection_bg: Color,
    /// Foreground of the highlighted/selected row.
    pub selection_fg: Color,
}

impl Theme {
    /// The Nord theme, exactly as sshm has always rendered it.
    pub const fn nord() -> Self {
        Self {
            bg: nord::NORD0,
            fg: nord::NORD4,
            fg_bright: nord::NORD6,
            fg_muted: nord::NORD8,
            accent: nord::NORD9,
            border: nord::NORD3,
            highlight: nord::NORD13,
            success: nord::NORD14,
            warning: nord::NORD12,
            selection_bg: nord::NORD9,
            // Dark-on-blue rather than white-on-blue: the previous NORD6 pairing
            // measured 2.34:1, well under WCAG AA. NORD0 on NORD9 is 4.64:1 and
            // matches how the picker already styles a matched character inside a
            // selected row, so the two states agree instead of fighting.
            selection_fg: nord::NORD0,
        }
    }

    /// The Clack palette: named ANSI colours and modifiers, **no fixed RGB**.
    ///
    /// This is the palette the transparent inline frame (#33) draws with, and it
    /// is built differently from Nord on purpose. A transparent frame borrows the
    /// user's terminal background, which it cannot measure — so a body-text hue
    /// of its own is a liability: `White` on a white terminal is 1.35:1, the
    /// exact defect Rule H was written for. Instead:
    ///
    /// * **Body text is `Reset`** — the terminal's own foreground, which is by
    ///   construction readable on the terminal's own background.
    /// * **Emphasis is a modifier, not a hue** — `BOLD`/`DIM` survive a light
    ///   terminal, `NO_COLOR`, and colour-blindness alike.
    /// * **Only state gets a hue**, and only from the two named colours Clack
    ///   uses: `Cyan` for the active step, `Green` for a match.
    ///
    /// `bg`, `selection_bg` and `selection_fg` are `Reset` because the frame
    /// never paints a background. Selection is carried by the `❯` glyph plus a
    /// bold alias, not by a filled row.
    pub const fn clack() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Reset,
            fg_bright: Color::Reset,
            fg_muted: Color::DarkGray,
            accent: Color::Cyan,
            border: Color::DarkGray,
            highlight: Color::Green,
            success: Color::Green,
            warning: Color::Yellow,
            selection_bg: Color::Reset,
            selection_fg: Color::Reset,
        }
    }

    /// A theme that emits no color at all.
    ///
    /// Structure has to survive on glyphs and layout alone here — which is the
    /// same argument as designing for a monochrome terminal, so nothing may rely
    /// on color to carry meaning.
    pub const fn monochrome() -> Self {
        Self {
            bg: Color::Reset,
            fg: Color::Reset,
            fg_bright: Color::Reset,
            fg_muted: Color::Reset,
            accent: Color::Reset,
            border: Color::Reset,
            highlight: Color::Reset,
            success: Color::Reset,
            warning: Color::Reset,
            selection_bg: Color::Reset,
            selection_fg: Color::Reset,
        }
    }

    /// Resolve the theme for a given terminal capability.
    ///
    /// Truecolor keeps the exact Nord RGB. Reduced-color modes snap each token to
    /// the nearest color the terminal can express, so the hierarchy degrades
    /// instead of collapsing into mangled escape sequences.
    pub fn resolve(&self, support: ColorSupport) -> Self {
        match support {
            ColorSupport::Truecolor => *self,
            ColorSupport::Monochrome => Self::monochrome(),
            ColorSupport::Ansi256 => Self {
                bg: nearest_xterm256(self.bg),
                fg: nearest_xterm256(self.fg),
                fg_bright: nearest_xterm256(self.fg_bright),
                fg_muted: nearest_xterm256(self.fg_muted),
                accent: nearest_xterm256(self.accent),
                border: nearest_xterm256(self.border),
                highlight: nearest_xterm256(self.highlight),
                success: nearest_xterm256(self.success),
                warning: nearest_xterm256(self.warning),
                selection_bg: nearest_xterm256(self.selection_bg),
                selection_fg: nearest_xterm256(self.selection_fg),
            },
            ColorSupport::Ansi16 => Self {
                bg: nearest_ansi16(self.bg),
                fg: nearest_ansi16(self.fg),
                fg_bright: nearest_ansi16(self.fg_bright),
                fg_muted: nearest_ansi16(self.fg_muted),
                accent: nearest_ansi16(self.accent),
                border: nearest_ansi16(self.border),
                highlight: nearest_ansi16(self.highlight),
                success: nearest_ansi16(self.success),
                warning: nearest_ansi16(self.warning),
                selection_bg: nearest_ansi16(self.selection_bg),
                selection_fg: nearest_ansi16(self.selection_fg),
            },
        }
    }

    /// The theme for whatever terminal we are actually in.
    pub fn from_env() -> Self {
        Self::nord().resolve(ColorSupport::detect())
    }
}

/// Snap a color to the nearest of the 256-color xterm palette.
///
/// Colors 16..=231 are a 6×6×6 RGB cube; 232..=255 is a 24-step grayscale ramp.
/// We score both and keep the closer one.
pub fn nearest_xterm256(color: Color) -> Color {
    let Some((r, g, b)) = contrast::to_rgb(color) else {
        return color;
    };
    let cube = [0u16, 95, 135, 175, 215, 255];

    let nearest_level = |v: u8| -> usize {
        cube.iter()
            .enumerate()
            .min_by_key(|(_, &c)| (i32::from(c) - i32::from(v)).abs())
            .map(|(i, _)| i)
            .unwrap_or(0)
    };

    let (ri, gi, bi) = (nearest_level(r), nearest_level(g), nearest_level(b));
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;
    let cube_rgb = (cube[ri], cube[gi], cube[bi]);
    let cube_d = dist((r, g, b), cube_rgb);

    // Grayscale ramp: 8, 18, 28, … , 238.
    let gray_idx = ((i32::from(r) + i32::from(g) + i32::from(b)) / 3 - 8).div_euclid(10);
    let gray_idx = gray_idx.clamp(0, 23) as usize;
    let gray_val = 8 + 10 * gray_idx as u16;
    let gray_d = dist((r, g, b), (gray_val, gray_val, gray_val));

    if gray_d < cube_d {
        Color::Indexed(232 + gray_idx as u8)
    } else {
        Color::Indexed(cube_idx as u8)
    }
}

/// Snap a color to the nearest of the 16 base ANSI colors.
pub fn nearest_ansi16(color: Color) -> Color {
    let Some((r, g, b)) = contrast::to_rgb(color) else {
        return color;
    };
    let named = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    named
        .iter()
        .map(|&c| (c, contrast::to_rgb(c).unwrap_or((0, 0, 0))))
        .min_by_key(|&(_, rgb)| dist((r, g, b), (rgb.0 as u16, rgb.1 as u16, rgb.2 as u16)))
        .map(|(c, _)| c)
        .unwrap_or(Color::Reset)
}

fn dist(a: (u8, u8, u8), b: (u16, u16, u16)) -> i64 {
    let dr = i64::from(a.0) - i64::from(b.0);
    let dg = i64::from(a.1) - i64::from(b.1);
    let db = i64::from(a.2) - i64::from(b.2);
    dr * dr + dg * dg + db * db
}

static ACTIVE: OnceLock<Theme> = OnceLock::new();

/// The palette the running app should draw with.
///
/// Resolved once from the environment. Render code binds this at the top of a
/// function (`let t = theme::active();`) and then speaks only in roles.
pub fn active() -> &'static Theme {
    ACTIVE.get_or_init(Theme::from_env)
}

/// Serialize a rendered buffer back to an ANSI string.
///
/// The stock `TestBackend` snapshot writes **glyphs only** — every color, bold and
/// reverse-video attribute is dropped on the floor. That is how nineteen snapshot
/// files sat on top of an unreadable 2.34:1 selection color and never flinched.
/// This walks the buffer cell by cell and re-emits the SGR runs, so a snapshot —
/// or a reviewer with a real terminal — sees what the user sees.
pub mod ansi {
    use ratatui::buffer::Buffer;
    use ratatui::style::{Color, Modifier, Style};

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
    /// Shared by the buffer serializer below and by the inline frame's own
    /// serializer, so there is exactly one place that turns a `Style` into
    /// bytes. A second hand-rolled emitter would be a palette fork in escape
    /// code form.
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

    /// Render a buffer as an ANSI string, one line per terminal row.
    ///
    /// Style is emitted only when it changes, so the output stays readable and
    /// diffs cleanly between snapshots.
    pub fn buffer_to_ansi(buffer: &Buffer) -> String {
        let width = usize::from(buffer.area().width);
        let height = usize::from(buffer.area().height);
        let content = buffer.content();

        let mut out = String::new();
        let mut current: Option<(Color, Color, Modifier)> = None;

        for y in 0..height {
            for x in 0..width {
                let Some(cell) = content.get(y * width + x) else {
                    continue;
                };
                let style = (cell.fg, cell.bg, cell.modifier);
                if current != Some(style) {
                    // Reset first so removed attributes actually disappear.
                    out.push_str("\x1b[0m");
                    out.push_str(&sgr_codes(style.2, Some(style.0), Some(style.1)));
                    current = Some(style);
                }
                out.push_str(cell.symbol());
            }
            out.push_str("\x1b[0m\n");
            current = None;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Modifier, Style};

    #[test]
    fn nord_tokens_are_all_actual_nord_colors() {
        for (role, color) in [
            ("bg", Theme::nord().bg),
            ("fg", Theme::nord().fg),
            ("fg_bright", Theme::nord().fg_bright),
            ("fg_muted", Theme::nord().fg_muted),
            ("accent", Theme::nord().accent),
            ("border", Theme::nord().border),
            ("highlight", Theme::nord().highlight),
            ("success", Theme::nord().success),
            ("warning", Theme::nord().warning),
            ("selection_bg", Theme::nord().selection_bg),
            ("selection_fg", Theme::nord().selection_fg),
        ] {
            assert!(
                nord::ALL.contains(&color),
                "role `{role}` is not a Nord palette color"
            );
        }
    }

    #[test]
    fn nord_rgb_values_match_published_hex() {
        assert_eq!(nord::NORD0, Color::Rgb(0x2E, 0x34, 0x40));
        assert_eq!(nord::NORD3, Color::Rgb(0x4C, 0x56, 0x6A));
        assert_eq!(nord::NORD4, Color::Rgb(0xD8, 0xDE, 0xE9));
        assert_eq!(nord::NORD6, Color::Rgb(0xEC, 0xEF, 0xF4));
        assert_eq!(nord::NORD8, Color::Rgb(0x88, 0xC0, 0xD0));
        assert_eq!(nord::NORD9, Color::Rgb(0x81, 0xA1, 0xC1));
        assert_eq!(nord::NORD13, Color::Rgb(0xEB, 0xCB, 0x8B));
        assert_eq!(nord::NORD14, Color::Rgb(0xA3, 0xBE, 0x8C));
    }

    #[test]
    fn truecolor_resolution_is_a_no_op() {
        assert_eq!(
            Theme::nord().resolve(ColorSupport::Truecolor),
            Theme::nord()
        );
    }

    #[test]
    fn monochrome_suppresses_every_token() {
        let t = Theme::nord().resolve(ColorSupport::Monochrome);
        assert_eq!(t, Theme::monochrome());
    }

    #[test]
    fn xterm256_snaps_to_known_indices() {
        // Nord0 #2E3440 → xterm 236 (#303030), the official Nord mapping.
        assert_eq!(nearest_xterm256(nord::NORD0), Color::Indexed(236));
        // Nord6 #ECEFF4 lands on the grayscale ramp (xterm 255 = #EEEEEE-ish),
        // which is closer than the 6x6x6 cube's pure white — the ramp is doing
        // its job for near-white neutrals.
        assert_eq!(nearest_xterm256(nord::NORD6), Color::Indexed(255));
        // Pure white should land on an indexed color, never a Reset.
        assert!(matches!(
            nearest_xterm256(Color::Rgb(255, 255, 255)),
            Color::Indexed(_)
        ));
    }

    #[test]
    fn ansi16_fallback_keeps_background_dark_and_text_light() {
        let t = Theme::nord().resolve(ColorSupport::Ansi16);
        // Background must stay dark, text must stay light, or the UI is unreadable.
        assert!(
            contrast::luminance(contrast::to_rgb(t.bg).unwrap()) < 0.2,
            "16-color background became too light"
        );
        assert!(
            contrast::luminance(contrast::to_rgb(t.fg).unwrap()) > 0.5,
            "16-color foreground became too dark"
        );
        assert!(contrast::meets_aa_text(t.fg, t.bg));
    }

    #[test]
    fn ansi_serializer_preserves_what_glyph_snapshots_drop() {
        use ratatui::buffer::Buffer;

        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 3, 1));
        buf[(0u16, 0u16)]
            .set_symbol("A")
            .set_style(Style::default().fg(nord::NORD13).bg(nord::NORD0))
            .modifier = Modifier::BOLD;
        buf[(1u16, 0u16)].set_symbol("B");

        let s = ansi::buffer_to_ansi(&buf);

        assert!(
            s.contains("38;2;235;203;139"),
            "highlight foreground missing from ANSI: {s:?}"
        );
        assert!(
            s.contains("48;2;46;52;64"),
            "background missing from ANSI: {s:?}"
        );
        assert!(s.contains("\x1b[1;"), "bold modifier missing: {s:?}");
        assert!(s.contains('A') && s.contains('B'), "glyphs missing: {s:?}");
    }

    #[test]
    fn ansi_serializer_resets_between_style_runs() {
        use ratatui::buffer::Buffer;

        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 2, 1));
        buf[(0u16, 0u16)].set_style(Style::default().fg(nord::NORD13));
        buf[(1u16, 0u16)].set_style(Style::default().fg(nord::NORD9));

        let s = ansi::buffer_to_ansi(&buf);
        // Every style switch must be preceded by a reset, or a dropped attribute
        // would silently bleed into the next run.
        assert_eq!(s.matches("\x1b[0m").count(), 3, "unexpected resets: {s:?}");
    }

    #[test]
    fn contrast_ratio_matches_known_reference_values() {
        // Black on white is the canonical 21:1.
        assert!((contrast::ratio(Color::Black, Color::White) - 21.0).abs() < 0.1);
        // Same color is 1:1.
        assert!((contrast::ratio(nord::NORD0, nord::NORD0) - 1.0).abs() < 0.001);
    }

    #[test]
    fn reset_color_is_not_measurable() {
        assert_eq!(contrast::ratio(Color::Reset, nord::NORD0), 21.0);
        assert!(contrast::to_rgb(Color::Reset).is_none());
    }
}
