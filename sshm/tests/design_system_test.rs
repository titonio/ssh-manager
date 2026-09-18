//! Deterministic design-system checks for sshm.
//!
//! This is the gate that the glyph-only snapshot suite could never be. It needs no
//! LLM, no network and no judgement — every rule here is arithmetic or a grep, so
//! it runs in CI on every push and cannot drift.
//!
//! The rules exist because of what was found when the palette was first made
//! inspectable: the inline picker's selected row — the single most-looked-at
//! element in the app — sat at **2.34:1** against WCAG's 4.5:1 floor, and all
//! nineteen existing snapshots were blind to it, because `TestBackend`'s
//! `Display` writes glyphs and drops every SGR attribute.
//!
//! ## What each rule protects
//!
//! | Rule | Guards against |
//! |---|---|
//! | palette purity | a new ad-hoc `Color::Rgb` sneaking in beside the tokens |
//! | Clack palette shape | a fixed hue pinned onto the transparent frame |
//! | the reference contrast floor | a body tier that is unreadable on a real scheme |
//! | hint fitting | key hints clipped off an 80-column terminal |
//! | degradation | the UI collapsing in `NO_COLOR` terminals |
//! | glyph-carried state | a map whose five field states are told apart by hue |
//! | the `▶` row's three states | a button that cannot say whether it will fire |
//! | the 8-row map budget | a refusal that changes the frame's height mid-typing |
//! | the placeholder tier | a placeholder wearing its own label's colour |
//!
//! ## What this file drives
//!
//! The form map — the one surface `Ctrl+A` and `Ctrl+E` now share — is driven
//! through `manage::step` with real `KeyEvent`s rather than by hand-assembling a
//! `FrameFlow`. That is deliberate: a hand-built flow proves only that the frame
//! *can be drawn* holding those values, while the keymap proves the frame is the
//! one the user actually reaches. The flow projection itself (`FrameFlow::from`)
//! is exercised on the way, so a break in either half shows up here.
//!
//! The fullscreen TUI's gates (WCAG pairs against a painted background, the
//! opaque-panel rule, the minimum-size gate) were deleted with the surface in
//! #35: a transparent frame that borrows the user's terminal has no painted
//! background to contrast-check, and the Clack structural claims above are
//! what replaced the ratio table by design (see the spec, #31). The inline
//! frame's own selection-marker, emphasis and degradation properties are
//! gated in `frame_test.rs`, at the seam that produces them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use sshm::config::Connection;
use sshm::frame::{
    build_frame, build_frame_with_flow, build_frame_with_note, Canvas, Frame, FrameFlow,
    FrameMode, FRAME_LINES, VISIBLE_ROWS,
};
use sshm::manage::{self, FormCursor, ManageState};
use sshm::theme::{ColorSupport, Theme};
use std::collections::HashSet;
use unicode_width::UnicodeWidthStr;

// ─────────────────────────────────────────────────────────────────────────────
// Fixtures
// ─────────────────────────────────────────────────────────────────────────────

fn prod_server() -> Connection {
    Connection {
        id: "1".into(),
        alias: "prod-server".into(),
        host: "10.0.0.1".into(),
        user: "root".into(),
        port: 22,
        key_path: None,
        folder: Some("production".into()),
    }
}

fn dev_box() -> Connection {
    Connection {
        id: "2".into(),
        alias: "dev-box".into(),
        host: "10.0.0.2".into(),
        user: "dev".into(),
        port: 2222,
        key_path: None,
        folder: None,
    }
}

fn conns() -> Vec<Connection> {
    vec![prod_server(), dev_box()]
}

/// One unmodified keystroke.
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// A Ctrl chord. Named rather than spelled out at each call site because the
/// alternative — a `KeyEvent` literal per chord — is how a test ends up pressing
/// something other than the key it means.
fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

/// Typing a word, one keystroke per character. The map's draft is live, so a
/// word is genuinely N keystrokes and there is no "set the field" shortcut to
/// fake — which is the point of driving the keymap at all.
fn typed(word: &str) -> Vec<KeyEvent> {
    word.chars().map(|c| key(KeyCode::Char(c))).collect()
}

/// The same key `n` times.
fn times(n: usize, k: KeyEvent) -> Vec<KeyEvent> {
    (0..n).map(|_| k).collect()
}

/// Push a run of keys through the real keymap and keep the resulting state.
///
/// Effects are dropped: none of these rules care what the store was asked to do,
/// only what the frame ends up showing.
fn drive(mut state: ManageState, keys: &[KeyEvent], selected: Option<&Connection>) -> ManageState {
    for k in keys {
        state = manage::step(&state, *k, selected).state;
    }
    state
}

/// `Ctrl+A` on a fresh list: the blank map, cursor on `Alias`.
fn add_blank() -> ManageState {
    drive(ManageState::new(), &[ctrl('a')], None)
}

/// Alias and Host filled, cursor advanced to `User` — the map half-written, with
/// the two optional tiers below it still empty.
fn add_mid() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.extend(typed("web-03"));
    keys.push(key(KeyCode::Enter));
    keys.extend(typed("10.0.0.9"));
    keys.push(key(KeyCode::Enter));
    drive(ManageState::new(), &keys, None)
}

/// Every required field filled, cursor walked down to the `▶` row: the lit
/// button.
fn add_ready_on_submit() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.extend(typed("web-03"));
    keys.push(key(KeyCode::Enter));
    keys.extend(typed("10.0.0.9"));
    // Host -> User -> Port -> Key -> Folder -> Submit.
    keys.extend(times(5, key(KeyCode::Down)));
    drive(ManageState::new(), &keys, None)
}

/// The same filled map with the cursor still on a field, for the pair
/// "ready but not standing on the button".
fn add_ready_on_field() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.extend(typed("web-03"));
    keys.push(key(KeyCode::Enter));
    keys.extend(typed("10.0.0.9"));
    keys.extend(times(4, key(KeyCode::Down)));
    drive(ManageState::new(), &keys, None)
}

/// The blank map walked to the `▶` row and Enter pressed with nothing filled:
/// a refusal, so the error line has taken over the rule row.
fn add_refused_empty() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.extend(times(FormCursor::ROWS - 1, key(KeyCode::Down)));
    keys.push(key(KeyCode::Enter));
    drive(ManageState::new(), &keys, None)
}

/// The blank map with the cursor still on a field: not ready, and not
/// standing on the `▶` row either. The fourth corner of the matrix the
/// submit row has to survive.
fn add_not_ready_on_field() -> ManageState {
    drive(ManageState::new(), &[ctrl('a')], None)
}

/// A refused map whose problems are exactly "a missing host and a bad port" —
/// the case the error line exists to state in one breath.
fn add_host_missing_port_bad() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.extend(typed("web-03"));
    keys.push(key(KeyCode::Enter)); // -> Host
    keys.push(key(KeyCode::Enter)); // -> User, Host left empty
    keys.push(key(KeyCode::Enter)); // -> Port
    keys.extend(typed("99999"));
    keys.extend(times(3, key(KeyCode::Down))); // Key, Folder, Submit
    keys.push(key(KeyCode::Enter));
    drive(ManageState::new(), &keys, None)
}

/// The worst a refusal can say: both required fields empty *and* the port
/// invalid, so all three problems land on the one row.
fn add_worst_refusal() -> ManageState {
    let mut keys = vec![ctrl('a')];
    keys.push(key(KeyCode::Enter)); // Alias -> Host, nothing typed
    keys.push(key(KeyCode::Enter)); // -> User
    keys.push(key(KeyCode::Enter)); // -> Port
    keys.extend(typed("0"));
    keys.extend(times(3, key(KeyCode::Down)));
    keys.push(key(KeyCode::Enter));
    drive(ManageState::new(), &keys, None)
}

/// `Ctrl+E` on `prod-server`: the same map, seeded, nothing changed.
fn edit_seeded() -> ManageState {
    let target = prod_server();
    drive(ManageState::new(), &[ctrl('e')], Some(&target))
}

/// The seeded map with two fields changed and the cursor on the `▶` row — the
/// reviewable save: two `●` rows above a lit button.
fn edit_changed() -> ManageState {
    let target = prod_server();
    let mut keys = vec![ctrl('e')];
    keys.extend(typed("x")); // Alias changed
    keys.push(key(KeyCode::Enter)); // -> Host
    keys.extend(typed("-2")); // Host changed
    keys.extend(times(5, key(KeyCode::Down))); // -> Submit
    drive(ManageState::new(), &keys, Some(&target))
}

/// The seeded map with **all five row glyphs present at once**, cursor parked on
/// `Folder` so the focus diamond lands on a row whose own glyph (`·`) is also
/// visible elsewhere.
///
/// This is the fixture the glyph test needs: one map that says `● ○ ✓ ! · ◆`
/// without any two states sharing a row. It is built by editing `dev-box` —
/// change the alias, clear the required Host, leave User alone, type a port that
/// will not validate, and leave the optional Key empty.
fn edit_all_glyphs() -> ManageState {
    let target = dev_box();
    let mut keys = vec![ctrl('e')];
    keys.extend(typed("x")); // Alias: filled, differs -> ●
    keys.push(key(KeyCode::Enter)); // -> Host
    keys.extend(times(8, key(KeyCode::Backspace))); // Host cleared, required -> ○
    keys.push(key(KeyCode::Enter)); // -> User: untouched -> ✓
    keys.push(key(KeyCode::Enter)); // -> Port
    keys.extend(times(4, key(KeyCode::Backspace))); // "2222" cleared
    keys.extend(typed("99999")); // -> !
    keys.push(key(KeyCode::Enter)); // -> Key: empty, optional -> ·
    keys.push(key(KeyCode::Enter)); // -> Folder: focused -> ◆
    drive(ManageState::new(), &keys, Some(&target))
}

// ─────────────────────────────────────────────────────────────────────────────
// Render-reading helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The map frame for a state, at `width` columns in a 24-row terminal.
///
/// Every geometry assertion in this file goes through here so "the frame the
/// tests read" is one construction, not six that could drift apart.
fn map_frame(state: &ManageState, width: usize, support: ColorSupport) -> Frame {
    build_frame_with_flow(
        &conns(),
        "",
        0,
        FrameMode::Manage,
        Canvas::new(width, 24, support),
        &FrameFlow::from(state),
    )
}

/// A line's drawn text with every style stripped — what the reader sees if the
/// terminal had no colour at all.
fn plain(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

/// The frame's body rows: everything between the `◆` header and the hint rail.
///
/// This is the region whose height the inline driver counts, and the region the
/// form map is required to fill with exactly eight rows.
fn body(frame: &Frame) -> &[Line<'static>] {
    &frame.lines()[1..frame.lines().len() - 2]
}

/// The leading glyph of a body row — the character after the `│` rail and the
/// gutter pad.
fn row_glyph(line: &Line<'static>) -> char {
    plain(line)
        .trim_start_matches(|c| c == '│' || c == ' ')
        .chars()
        .next()
        .expect("a body row leads with its glyph")
}

/// The `(glyph, label)` style pair of the `▶` row, read off a rendered frame.
///
/// Read from the render rather than from `FormFlow`'s booleans on purpose: the
/// rule under test is what the row *looks* like, and the mapping from
/// `submit_ready`/`submit_focused` to that look is exactly the thing that could
/// regress.
fn submit_styles(state: &ManageState, support: ColorSupport) -> (Option<Color>, bool, Option<Color>, bool) {
    let frame = map_frame(state, 80, support);
    let line = &body(&frame)[FormCursor::ROWS];
    let text = plain(line);
    assert!(
        text.contains('▶'),
        "the row after the rule row must be the ▶ row, got |{text}|"
    );
    let glyph = &line.spans[2];
    let label = &line.spans[3];
    (
        glyph.style.fg,
        glyph.style.add_modifier.contains(Modifier::BOLD),
        label.style.fg,
        label.style.add_modifier.contains(Modifier::BOLD),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule A — palette purity
// ─────────────────────────────────────────────────────────────────────────────

/// No render code may name a color directly.
///
/// Every literal color must live in `theme.rs`. A new `Color::Rgb(..)` in a
/// render function is a palette fork: it can't be rethemed, can't be audited,
/// and silently duplicates a token that already exists.
#[test]
fn no_color_literals_outside_the_theme_module() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let sources = [
        "src/connections.rs",
        "src/emit.rs",
        "src/frame.rs",
        "src/inline.rs",
        "src/main.rs",
        // `manage.rs` feeds the rendered flow through `FrameFlow`, so it is
        // render-adjacent code on the live path. It carries no colour today,
        // but this whitelist *is* the enforcement mechanism: a module left
        // off it is a module the gate does not read, and the day someone
        // gives a trace a hue the test would say nothing.
        "src/manage.rs",
    ];
    let forbidden = [
        "Color::Rgb(",
        "Color::Indexed(",
        "Color::Black",
        "Color::Red",
        "Color::Green",
        "Color::Yellow",
        "Color::Blue",
        "Color::Magenta",
        "Color::Cyan",
        "Color::Gray",
        "Color::DarkGray",
        "Color::LightRed",
        "Color::LightGreen",
        "Color::LightYellow",
        "Color::LightBlue",
        "Color::LightMagenta",
        "Color::LightCyan",
        "Color::White",
    ];

    let mut offenders: Vec<String> = Vec::new();
    for rel in sources {
        let path = format!("{crate_dir}/{rel}");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
        for (i, line) in text.lines().enumerate() {
            // `Color::Reset` is always allowed: it means "no color".
            if line.contains("Color::Reset") {
                continue;
            }
            for pat in forbidden {
                if line.contains(pat) {
                    offenders.push(format!("{rel}:{}: contains {pat}", i + 1));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "colors must be named by role via theme::, not by literal.\n{}",
        offenders.join("\n")
    );
}

/// The Clack palette carries no fixed RGB, and its only indexed colour is a
/// **neutral grey**.
///
/// The transparent inline frame (#33) borrows the user's terminal background, so
/// it cannot contrast-check a hue of its own against a surface it cannot see.
/// Every role is therefore a **named ANSI colour** — which the terminal maps to
/// *its* palette, at the user's chosen values — or `Reset`.
///
/// The exception is the muted tier, and issue #42 is why it exists. The
/// 16-colour set contains no neutral tone between `brightBlack` and `white`, so
/// a two-tier neutral hierarchy is not expressible in named ANSI *at all*: on
/// the One Dark scheme that prompted the issue, `Reset` and `DarkGray` are the
/// same colour (`#5C6370`, 2.67:1) and the muted tier never rendered.
/// `Indexed(245)` is the one step that fills the gap.
///
/// The exception is deliberately narrow — the 24-step grey ramp (232..=255) and
/// nothing else. An indexed *hue* would be a palette fork wearing a grey's
/// clothes: it skips the terminal's own mapping for a colour the frame has no
/// business choosing.
#[test]
fn the_clack_palette_is_named_ansi_or_neutral_grey() {
    let t = Theme::clack();
    let neutral_grey = |c: Color| matches!(c, Color::Indexed(n) if (232..=255).contains(&n));
    let fork = |c: Color| {
        matches!(c, Color::Rgb(..)) || (matches!(c, Color::Indexed(_)) && !neutral_grey(c))
    };

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
            !fork(color),
            "clack role `{role}` is {color:?} — the palette must be a named ANSI \
             colour, Reset, or a neutral grey from the 232..=255 ramp"
        );
    }
}

/// The hues the Clack palette is allowed to have, pinned to the spec's words:
/// cyan for the active step, green for a match. Everything else is the terminal's.
#[test]
fn the_clack_palette_hues_are_cyan_and_green() {
    let t = Theme::clack();

    assert_eq!(t.accent, Color::Cyan, "the ◆ step icon is Clack cyan");
    assert_eq!(t.highlight, Color::Green, "a fuzzy hit is Clack green");
    assert_eq!(t.border, Color::DarkGray, "the │ rail is bright black");
    assert_eq!(
        t.fg_muted,
        Color::Indexed(245),
        "the muted meta is the grey the 16-colour set cannot supply (#42)"
    );
    assert_eq!(
        t.fg_placeholder,
        Color::Indexed(240),
        "the placeholder tier is the ramp step below muted, and nothing else"
    );
}

/// A transparent frame has no background of its own, so the WCAG table that
/// once governed the fullscreen Nord palette cannot govern it. What replaces
/// that check is the structural claim: the frame owns no background, body
/// text is a *named* tone rather than an inherited one, and selection is a
/// glyph rather than a fill.
///
/// Body text used to be `Reset`, justified as "the terminal's own foreground,
/// readable by construction". Issue #42 disproved that: the common One Dark
/// scheme sets `foreground` to `#5C6370` — Atom's *comment* colour — which is
/// 2.67:1 against that scheme's own background. Inheriting the terminal's ink
/// is not the same as being readable, so body text names ANSI 7 instead.
#[test]
fn the_clack_palette_leaves_the_surface_to_the_terminal() {
    let t = Theme::clack();

    assert_eq!(
        t.fg,
        Color::Gray,
        "body text names the palette's own text tone; `Reset` inherits an ink the \
         frame cannot vouch for — see issue #42"
    );
    // There is no background role left to assert on: `bg`, `selection_bg`
    // and `selection_fg` were deleted with the fullscreen TUI (#35)
    // because no live render path read them. A transparent frame cannot
    // own a background even by accident now — and what it actually emits
    // is gated where it is produced, by
    // `no_span_in_any_frame_sets_a_background` and
    // `the_serialized_frame_asks_for_no_background_colour` in `frame_test.rs`.
}

/// A role that nothing reads governs nothing.
///
/// `fg` was declared, documented and asserted-on for the entire life of the
/// token layer while **no render code read it**. Body text was drawn as
/// `Style::default().add_modifier(BOLD)` with no `.fg()` at all, inheriting the
/// terminal's ink — so when issue #42 arrived, retuning the `fg` token would
/// have changed nothing on screen, and nothing in this suite would have said why.
///
/// This is the guard against that class of silent no-op: the token must be
/// *consumed*, not just defined.
#[test]
fn the_fg_role_is_read_by_render_code() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let mut reads = 0usize;

    for rel in ["src/frame.rs", "src/inline.rs"] {
        let text = std::fs::read_to_string(format!("{crate_dir}/{rel}"))
            .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
        for line in text.lines() {
            if line.contains("fg_muted") {
                continue;
            }
            if line.contains("t.fg") || line.contains("theme.fg") {
                reads += 1;
            }
        }
    }

    assert!(
        reads >= 10,
        "`fg` is read in only {reads} place(s) — body text is inheriting the \
         terminal's ink again, which is the issue #42 failure mode"
    );
}

/// The placeholder tier has to be *consumed* too, for the same reason `fg` was
/// found to be a fiction: a token that no render path reads cannot be the thing
/// that makes a placeholder read fainter.
///
/// The form map is the tier's only consumer, and it has six rows' worth of
/// places to be drawn, so the bar here is the same order as `fg`'s.
#[test]
fn the_placeholder_role_is_read_by_render_code() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let mut reads = 0usize;

    for rel in ["src/frame.rs", "src/inline.rs"] {
        let text = std::fs::read_to_string(format!("{crate_dir}/{rel}"))
            .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
        reads += text
            .lines()
            .filter(|line| line.contains("fg_placeholder"))
            .count();
    }

    assert!(
        reads >= 1,
        "`fg_placeholder` is read nowhere in the render modules — the tier exists \
         but nothing draws it, which is the `fg` failure mode all over again"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule B — the reference contrast floor
// ─────────────────────────────────────────────────────────────────────────────

/// WCAG 2.x relative luminance.
fn luminance(rgb: (u8, u8, u8)) -> f64 {
    let chan = |v: u8| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * chan(rgb.0) + 0.7152 * chan(rgb.1) + 0.0722 * chan(rgb.2)
}

fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

/// The xterm 24-step grey ramp, indices 232..=255 → 8, 18, 28, … 238.
fn grey_ramp(n: u8) -> (u8, u8, u8) {
    let v = 8 + (n - 232) * 10;
    (v, v, v)
}

/// Resolve a theme colour to RGB against one *concrete* terminal palette.
///
/// A named ANSI colour has no pixels of its own — the terminal maps it — so a
/// contrast number is only meaningful against a stated palette. That is the
/// point of this rule: it is a reference check, not a guarantee.
fn resolve_against(color: Color, palette: &[(u8, u8, u8); 16]) -> (u8, u8, u8) {
    match color {
        Color::Indexed(n) if (232..=255).contains(&n) => grey_ramp(n),
        Color::Indexed(n) => palette[(n as usize) % 16],
        Color::Rgb(r, g, b) => (r, g, b),
        named => {
            let idx = match named {
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
                _ => 7,
            };
            palette[idx]
        }
    }
}

/// The reporter's One Dark, verbatim from the `settings.json` in issue #42.
const ONE_DARK: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // black
    (0xE0, 0x6C, 0x75), // red
    (0x98, 0xC3, 0x79), // green
    (0xD1, 0x9A, 0x66), // yellow
    (0x61, 0xAF, 0xEF), // blue
    (0xC6, 0x78, 0xDD), // purple
    (0x56, 0xB6, 0xC2), // cyan
    (0xAB, 0xB2, 0xBF), // white  ← One Dark's actual text colour
    (0x5C, 0x63, 0x70), // brightBlack == the scheme's `foreground`
    (0xE0, 0x6C, 0x75),
    (0x98, 0xC3, 0x79),
    (0xD1, 0x9A, 0x66),
    (0x61, 0xAF, 0xEF),
    (0xC6, 0x78, 0xDD),
    (0x56, 0xB6, 0xC2),
    (0xFF, 0xFF, 0xFF),
];
const ONE_DARK_BG: (u8, u8, u8) = (0x1E, 0x21, 0x27);

/// Windows Terminal's stock Campbell, as a second, unrelated reference.
const CAMPBELL: [(u8, u8, u8); 16] = [
    (0x0C, 0x0C, 0x0C),
    (0xC5, 0x0F, 0x1F),
    (0x13, 0xA1, 0x0E),
    (0xC1, 0x9C, 0x00),
    (0x00, 0x37, 0xDA),
    (0x88, 0x17, 0x98),
    (0x3A, 0x96, 0xDD),
    (0xCC, 0xCC, 0xCC),
    (0x76, 0x76, 0x76),
    (0xE7, 0x48, 0x56),
    (0x16, 0xC6, 0x0C),
    (0xF9, 0xF1, 0xA5),
    (0x3B, 0x78, 0xFF),
    (0xB4, 0x00, 0x9E),
    (0x61, 0xD6, 0xD6),
    (0xF2, 0xF2, 0xF2),
];
const CAMPBELL_BG: (u8, u8, u8) = (0x0C, 0x0C, 0x0C);

/// Every informational role clears 4.5:1 against both reference palettes.
///
/// The fullscreen TUI's WCAG gates were deleted in #35 on the argument that a
/// transparent frame owns no background and so no ratio can be checked. That
/// argument is sound as far as it goes, but it was stated as *"a contrast table
/// cannot exist"* — and issue #42 shipped under exactly that reasoning.
///
/// A guaranteed table is impossible. A **reference** table is not: pin two real
/// palettes, resolve each role the way a terminal would, and fail with the
/// computed number. It cannot promise anything about the reader's own palette.
/// What it does promise is that nobody re-introduces a 2.67:1 body tier without
/// a failing test in front of them.
///
/// `border` is deliberately excluded: the `│` rail is chrome that carries no
/// information, and a faint gutter is the intended look. `fg_placeholder` is
/// excluded for the opposite reason, pinned by
/// [`the_placeholder_tier_is_fainter_than_muted_and_carries_no_state`]: it is
/// *meant* to sit under the floor.
#[test]
fn every_informational_role_clears_the_reference_contrast_floor() {
    let t = Theme::clack();
    let roles = [
        ("fg", t.fg),
        ("fg_muted", t.fg_muted),
        ("accent", t.accent),
        ("highlight", t.highlight),
        ("warning", t.warning),
    ];

    for (name, palette, bg, label) in [
        (
            "One Dark",
            &ONE_DARK,
            ONE_DARK_BG,
            "the issue #42 reporter's palette",
        ),
        (
            "Campbell",
            &CAMPBELL,
            CAMPBELL_BG,
            "Windows Terminal's stock palette",
        ),
    ] {
        for (role, color) in roles {
            let ratio = contrast(resolve_against(color, palette), bg);
            assert!(
                ratio >= 4.5,
                "`{role}` on {name} ({label}) is {ratio:.2}:1, under the 4.5:1 \
                 floor — resolved to {:?} against {bg:?}",
                resolve_against(color, palette)
            );
        }
    }
}

/// `fg_placeholder` is deliberately **below** the informational floor, and
/// deliberately excluded from the test above. This pins that as a decision.
///
/// The placeholder tier exists to be *fainter than `fg_muted`* — a
/// placeholder wearing the field label's colour is indistinguishable from a
/// label, which is the defect the tier was cut to fix. Fainter than
/// `fg_muted` (≈4.6:1) necessarily means under 4.5:1, so the two
/// requirements cannot both be met and the tier sits at `Indexed(240)`,
/// ≈2.2:1.
///
/// It may sit there because **it carries no state**. Whether a field needs
/// filling is said by that field's own glyph — `○` required, `·` optional —
/// which is colour-independent. If the placeholder is unreadable the user
/// loses a hint about format, not a fact about the form. That is the same
/// reasoning that keeps `border` out of the informational set.
#[test]
fn the_placeholder_tier_is_fainter_than_muted_and_carries_no_state() {
    let t = Theme::clack();
    let grey = |c: Color| match c {
        Color::Indexed(n) => n,
        other => panic!("placeholder tier must be a ramp grey, got {other:?}"),
    };
    assert!(
        grey(t.fg_placeholder) < grey(t.fg_muted),
        "placeholder {} is not fainter than muted {}",
        grey(t.fg_placeholder),
        grey(t.fg_muted)
    );
    // And it is not in the informational set the floor test walks.
    let informational = ["fg", "fg_muted", "accent", "highlight", "warning"];
    assert!(
        !informational.contains(&"fg_placeholder"),
        "if the placeholder becomes informational it must clear 4.5:1, and then \
         it can no longer be a placeholder"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule C — a settle trace may only name a state the spec has
// ─────────────────────────────────────────────────────────────────────────────

/// The `Settled` state set from the parent spec (#31):
/// `picked | cancelled | added | edited | deleted | error`.
///
/// A settle trace is the user's last sight of the frame, so the verb it
/// carries is a claim about what the command *did*. `editing` is not one of
/// the six — and the #35 cut-over changes nothing on disk: Enter in
/// `sshm manage` routes the selection to the edit path and leaves
/// `connections.json` byte-identical. A trace that said `editing` would
/// describe the interaction #36/#37 have not built yet.
///
/// The gate reads the verbs out of the source that emits them, the same way
/// the colour-literal gate above does, so a new verb cannot slip in
/// unreviewed.
#[test]
fn every_settle_verb_is_a_state_from_the_spec_state_set() {
    const SPEC_STATES: [&str; 6] = ["picked", "cancelled", "added", "edited", "deleted", "error"];
    const EMITTER: &str = "connection_trace(\"";

    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{crate_dir}/src/inline.rs");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

    let verbs: Vec<&str> = text
        .match_indices(EMITTER)
        .map(|(i, _)| {
            let rest = &text[i + EMITTER.len()..];
            let end = rest.find('"').expect("unterminated settle-verb literal");
            &rest[..end]
        })
        .collect();

    assert!(
        !verbs.is_empty(),
        "no settle verb found through `{EMITTER}` — the emitter moved and this \
         gate is now blind; update it rather than trust a vacuous pass"
    );
    for verb in &verbs {
        assert!(
            SPEC_STATES.contains(verb),
            "the settle trace emits `{verb}`, which is not a state in #31's \
             Settled set {SPEC_STATES:?}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule D — key hints must fit the terminal
// ─────────────────────────────────────────────────────────────────────────────

/// `fit_hints` must drop from the end, never truncate a label mid-word.
#[test]
fn fit_hints_drops_whole_labels_only() {
    let hints = ["Navigate", "Connect", "Search", "Help", "Quit"];

    // Wide enough for everything.
    assert_eq!(
        sshm::frame::fit_hints(&hints, 80),
        "Navigate | Connect | Search | Help | Quit"
    );
    // Tight: only the first two fit.
    assert_eq!(sshm::frame::fit_hints(&hints, 18), "Navigate | Connect");
    // Impossible: never emit a partial label or a dangling separator.
    assert_eq!(sshm::frame::fit_hints(&hints, 3), "");
    // Exactly one label.
    assert_eq!(sshm::frame::fit_hints(&hints, 8), "Navigate");
}

/// The form map's rail fits an 80-column terminal and keeps both escape hatches.
///
/// The rail is the only place the map says how to leave, and the map is the
/// first screen a new user of `sshm manage` ever sees. The defect this is cut
/// to catch is the #36 rail: 87 columns of hints in an 80-column terminal,
/// clipped mid-word into `Ctrl+X dele`. The drop-from-the-end rule is what
/// makes that impossible rather than merely unlikely, and the ordering is what
/// makes the survivors the right survivors.
///
/// Checked at 80 and at 60 because the map's rail is short enough for both,
/// and a rail that only fits the width nobody tests at is not a rail.
#[test]
fn the_form_rail_fits_80_columns_and_keeps_the_escape_hatch() {
    for (label, state) in [
        ("cursor on a field", add_mid()),
        ("cursor on the ▶ row", add_ready_on_submit()),
        ("a refused map", add_refused_empty()),
    ] {
        for width in [80usize, 60] {
            let frame = map_frame(&state, width, ColorSupport::Truecolor);
            let rail = plain(&frame.lines()[frame.lines().len() - 2]);
            assert!(
                rail.width() <= 80,
                "the form rail is {} columns with {label} at width {width}: |{rail}|",
                rail.width()
            );
            assert!(
                rail.contains("Esc"),
                "the rail with {label} lost `Esc` — the way out must never be the \
                 first thing a narrow terminal drops: |{rail}|"
            );
            assert!(
                rail.contains("Ctrl+C"),
                "the rail with {label} lost `Ctrl+C` — the second escape hatch \
                 must survive an 80-column terminal: |{rail}|"
            );
        }
    }
}

/// The rail names what Enter means on the row the cursor is actually on.
///
/// `Enter` advances on a field and commits on the `▶` row, so one label for
/// both would be lying about one of them — and the row the user is standing on
/// is exactly the thing the rail exists to disambiguate.
#[test]
fn the_form_rail_names_what_enter_means_on_the_row_under_the_cursor() {
    let field = plain(
        &map_frame(&add_ready_on_field(), 80, ColorSupport::Truecolor)
            .lines()[FRAME_LINES - 2],
    );
    let submit = plain(
        &map_frame(&add_ready_on_submit(), 80, ColorSupport::Truecolor)
            .lines()[FRAME_LINES - 2],
    );

    assert!(
        field.contains("Enter next"),
        "with the cursor on a field the rail must say Enter advances: |{field}|"
    );
    assert!(
        !field.contains("Enter save"),
        "with the cursor on a field the rail must not promise a save: |{field}|"
    );
    assert!(
        submit.contains("Enter save"),
        "with the cursor on the ▶ row the rail must say Enter saves: |{submit}|"
    );
    assert!(
        !submit.contains("Enter next"),
        "with the cursor on the ▶ row the rail must not say Enter advances: \
         |{submit}|"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule E — graceful degradation
// ─────────────────────────────────────────────────────────────────────────────

/// `NO_COLOR` must suppress color without suppressing structure.
///
/// The Clack palette resolved to monochrome is all `Reset`: the frame's
/// meaning then lives entirely in the glyphs and the modifiers, which is
/// what `frame_test.rs` proves survives the downgrade.
#[test]
fn no_color_suppresses_color_not_structure() {
    let t = Theme::clack().resolve(ColorSupport::Monochrome);
    for color in [
        t.fg,
        t.fg_muted,
        t.accent,
        t.border,
        t.highlight,
        t.success,
        t.warning,
        t.fg_placeholder,
    ] {
        assert_eq!(color, Color::Reset, "monochrome theme still emits color");
    }
}

/// Every state the map can be in is carried by a **glyph**, not by a colour.
///
/// The map has five field states — valid, changed, needed, invalid, empty — plus
/// the focus marker, and it draws them with `✓ ● ○ ! ·` and `◆`. That is not
/// decoration: the frame owns no background and borrows the user's terminal,
/// so a hue alone is a state that vanishes under `NO_COLOR`, on a light
/// scheme, and for a colour-blind reader. The rule this test holds is the one
/// the whole palette argument rests on — take the colour away and the map must
/// still say the same thing.
///
/// The fixture is one edit map carrying all five states at once, so the
/// assertion is about the *set* of glyphs and not about any single row
/// happening to look right.
#[test]
fn the_map_carries_every_state_in_a_glyph_not_a_colour() {
    let state = edit_all_glyphs();
    let flow = FrameFlow::from(&state);
    let rows = &flow.form.as_ref().expect("Ctrl+E opens the map").rows;

    let colour = map_frame(&state, 80, ColorSupport::Truecolor);
    let mono = map_frame(&state, 80, ColorSupport::Monochrome);

    let drawn_colour: Vec<char> = body(&colour)[..FormCursor::ROWS - 1]
        .iter()
        .map(|l| row_glyph(l))
        .collect();
    let drawn_mono: Vec<char> = body(&mono)[..FormCursor::ROWS - 1]
        .iter()
        .map(|l| row_glyph(l))
        .collect();

    // The glyph is the state: the same rows draw the same characters with the
    // colour turned off.
    assert_eq!(
        drawn_colour, drawn_mono,
        "the map's glyphs changed between truecolour and monochrome"
    );

    // All five state glyphs and the focus diamond are on screen at once.
    for glyph in ['✓', '●', '○', '!', '·', '◆'] {
        assert!(
            drawn_mono.contains(&glyph),
            "`{glyph}` never appears on the monochrome map — a state has lost \
             its marker. Drawn: {drawn_mono:?}"
        );
    }

    // Six rows, six distinct leading glyphs: no two states collapse onto one
    // marker, which is how `○` (required, empty) would start looking like `·`
    // (optional, empty).
    let distinct: HashSet<char> = drawn_mono.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        drawn_mono.len(),
        "two map rows share a glyph, so their states cannot be told apart: \
         {drawn_mono:?}"
    );

    // And the drawn glyph is the model's glyph, with the frame's one swap —
    // the focused row wears `◆` — applied and nothing else.
    for (row, drawn) in rows.iter().zip(body(&mono)) {
        let expected = if row.focused { '◆' } else { row.glyph.char() };
        assert_eq!(
            row_glyph(drawn),
            expected,
            "row `{}` draws {:?} while the model says {:?}",
            row.label,
            row_glyph(drawn),
            row.glyph
        );
    }

    // Finally: the monochrome map really does emit no colour, so the
    // distinguishability above is not riding on a hue that survived the
    // downgrade by accident.
    for line in mono.lines() {
        for span in &line.spans {
            assert!(
                span.style.fg.is_none() || span.style.fg == Some(Color::Reset),
                "a monochrome map span still carries {:?} — the state would be \
                 riding on colour after all",
                span.style.fg
            );
        }
    }
}

/// The `▶` row says which of its three states it is in **without a hue**.
///
/// The button is the only gate in the whole form, so the user has to be able
/// to answer "will this fire?" and "am I standing on it?" from the row alone.
/// The design carries both in weight and tier and never in the glyph appearing
/// or disappearing — a `▶` that vanished when the form was incomplete would
/// read as a rendering glitch, and one that only appeared when the cursor
/// arrived would hide the shape of the form until it was ready.
///
/// "Without colour" here means *without hue*: every colour is reduced to its
/// greyscale lightness against both reference palettes, the way a
/// greyscale terminal or a monochrome print would render it. The three states
/// stay three apart on both, and the not-ready recession is a **lightness**
/// difference, not a hue difference — which is the whole point of the check.
///
/// **Still-open gap, reported rather than hidden.** Under full `NO_COLOR`
/// every role collapses to `Reset`, and with it the not-ready/ready
/// distinction: those two rows render byte-identically and only **focus**
/// survives, on `BOLD`. Readiness is carried by tier alone, so a truly
/// colourless terminal cannot tell "this will fire" from "this will not".
/// Closing that needs a weight or glyph difference on the not-ready row —
/// not a colour — and would collide with focus's claim on weight, so it is
/// left open deliberately. What this test pins is the property that does
/// survive: focus is always readable, and readiness is a lightness
/// difference wherever colour is available.
#[test]
fn the_submit_row_is_distinguishable_in_all_three_states_without_colour() {
    let not_ready = submit_styles(&add_refused_empty(), ColorSupport::Truecolor);
    let not_ready_unfocused = submit_styles(&add_not_ready_on_field(), ColorSupport::Truecolor);
    let ready_unfocused = submit_styles(&add_ready_on_field(), ColorSupport::Truecolor);
    let ready_focused = submit_styles(&add_ready_on_submit(), ColorSupport::Truecolor);

    // The glyph is always there, in every state, in both colour modes. The
    // row is identifiable as the submit row before it is anything else.
    for (label, state) in [
        ("not ready", add_refused_empty()),
        ("ready, cursor elsewhere", add_ready_on_field()),
        ("ready, cursor here", add_ready_on_submit()),
    ] {
        for support in [ColorSupport::Truecolor, ColorSupport::Monochrome] {
            let frame = map_frame(&state, 80, support);
            let row = plain(&body(&frame)[FormCursor::ROWS]);
            assert!(
                row.contains('▶'),
                "the ▶ vanished with {label} under {support:?}: |{row}|"
            );
        }
    }

    // Reduce each state to a hue-free signature: the greyscale lightness of
    // the glyph and the label, plus whether each is bold.
    let signature = |s: &(Option<Color>, bool, Option<Color>, bool),
                    palette: &[(u8, u8, u8); 16]|
     -> (f64, bool, f64, bool) {
        (
            luminance(resolve_against(s.0.expect("glyph has a role"), palette)),
            s.1,
            luminance(resolve_against(s.2.expect("label has a role"), palette)),
            s.3,
        )
    };

    for (name, palette) in [("One Dark", &ONE_DARK), ("Campbell", &CAMPBELL)] {
        let a = signature(&not_ready, palette);
        let b = signature(&ready_unfocused, palette);
        let c = signature(&ready_focused, palette);

        assert_ne!(a, b, "not-ready and ready are the same row with no hue on {name}");
        assert_ne!(
            b, c,
            "ready and ready+focused are the same row with no hue on {name}"
        );
        assert_ne!(
            a, c,
            "not-ready and ready+focused are the same row with no hue on {name}"
        );

        // The recession is lightness, not hue: the not-ready row is dimmer on
        // both the glyph and the label.
        assert!(
            a.0 < b.0,
            "the not-ready glyph is not dimmer than the ready glyph on {name} \
             ({:.4} vs {:.4}) — the button would not read as disabled",
            a.0,
            b.0
        );
        assert!(
            a.2 < b.2,
            "the not-ready label is not dimmer than the ready label on {name} \
             ({:.4} vs {:.4})",
            a.2,
            b.2
        );
    }

    // Readiness is carried by tier; **focus is carried by BOLD in every
    // readiness state.** Those are two different questions this row has to
    // answer, and under full `NO_COLOR` the tier collapses to `Reset` while
    // the bold survives — so bold is the only thing left carrying "am I
    // standing on the commit row?", and it may not be traded away to keep a
    // not-ready row quiet.
    //
    // The earlier version of this check asserted the not-ready row must not
    // be bold, using a fixture whose cursor *was* on the ▶ row. That
    // conflated the two questions and left the cursor invisible on a
    // not-ready form at exactly the moment the user is arrowing around
    // looking for the row that will fire.
    assert!(
        not_ready.1 && not_ready.3,
        "the not-ready ▶ row the cursor is standing on is not bold — with colour \
         off there would be nothing telling it apart from the row above it"
    );
    assert!(
        !not_ready_unfocused.1 && !not_ready_unfocused.3,
        "the not-ready ▶ row the cursor is NOT on is bold — only the row the \
         cursor is on may carry the weight"
    );
    assert!(
        !ready_unfocused.1 && !ready_unfocused.3,
        "the ready-but-not-standing-on-it ▶ row is bold — only the commit row \
         may carry the weight"
    );
    assert!(
        ready_focused.1 && ready_focused.3,
        "the focused ▶ row is not bold — the commit row must be unmistakable \
         with no colour to help"
    );

    // And the two not-ready variants must be tellable apart with no hue.
    // This is the pair that was byte-identical before focus carried weight.
    for (name, palette) in [("One Dark", &ONE_DARK), ("Campbell", &CAMPBELL)] {
        assert_ne!(
            signature(&not_ready, palette),
            signature(&not_ready_unfocused, palette),
            "standing on the not-ready ▶ row looks identical to not standing on \
             it with no hue on {name}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule F — the map's row budget
// ─────────────────────────────────────────────────────────────────────────────

/// The map is exactly eight body rows whether or not anything is wrong.
///
/// Six field rows, one rule row, one `▶` row. When a refusal arrives the
/// **rule row is replaced**, not appended to, so the frame's height never
/// moves. This is not cosmetic: the inline driver erases and rewrites the rows
/// a redraw changed and counts one frame line as one physical row, so a frame
/// that grew a line the moment a keystroke changed the validation state would
/// jump the user's terminal mid-type — and the settle-collapse that keeps the
/// shell line clean is counted off the same budget.
///
/// The old stepped add could not break this because it only ever drew one field
/// at a time. The map draws all six, so the budget became a live invariant and
/// needed a gate.
#[test]
fn the_map_is_eight_body_rows_with_or_without_an_error() {
    for width in [80usize, 60] {
        let calm = map_frame(&add_mid(), width, ColorSupport::Truecolor);
        let loud = map_frame(&add_host_missing_port_bad(), width, ColorSupport::Truecolor);

        assert_eq!(
            body(&calm).len(),
            VISIBLE_ROWS,
            "the map without an error is not {VISIBLE_ROWS} body rows at width {width}"
        );
        assert_eq!(
            body(&loud).len(),
            VISIBLE_ROWS,
            "the map with an error is not {VISIBLE_ROWS} body rows at width {width} \
             — the error must replace the rule row, not add a line"
        );
        assert_eq!(
            calm.lines().len(),
            loud.lines().len(),
            "the frame changed height between a clean map and a refused one at \
             width {width}"
        );
        assert_eq!(calm.lines().len(), FRAME_LINES);
        assert_eq!(loud.lines().len(), FRAME_LINES);

        // The row that changes is the rule row, one-for-one, and it sits in
        // the same slot both times.
        let calm_rule = plain(&body(&calm)[FormCursor::ROWS - 1]);
        let loud_rule = plain(&body(&loud)[FormCursor::ROWS - 1]);
        assert!(
            calm_rule.contains('─') && !calm_rule.contains('!'),
            "the clean map's rule row is not a rule: |{calm_rule}|"
        );
        assert!(
            loud_rule.contains("!"),
            "the refused map's rule row did not become the error line: |{loud_rule}|"
        );

        // And the `▶` row is the last body row in both — the button never
        // moves down when a refusal arrives.
        assert!(plain(&body(&calm)[FormCursor::ROWS]).contains('▶'));
        assert!(plain(&body(&loud)[FormCursor::ROWS]).contains('▶'));
    }
}

/// The refusal line names every offending field, and the worst case fits 80
/// columns.
///
/// The map shows all six fields at once, so a refusal that names one problem
/// is a refusal that sends the user hunting: they fix the field it named, press
/// Enter again, and find the next one. The line carries **all** of them, in
/// map order, joined with `·`, because the cheapest thing on this surface is a
/// sentence and the expensive thing is a third round of Enter.
///
/// The two-field case below is the one the ticket describes — a missing host
/// and a port that will not parse — and it has to survive an 80-column
/// terminal whole. A truncation here would cut the *example* out of the port
/// message, which is the half the user was missing.
#[test]
fn the_error_line_names_every_offending_field_and_fits_80_columns() {
    let frame = map_frame(&add_host_missing_port_bad(), 80, ColorSupport::Truecolor);
    let line = plain(&body(&frame)[FormCursor::ROWS - 1]);

    assert!(
        line.contains("host is required"),
        "the refusal does not name the missing host: |{line}|"
    );
    assert!(
        line.contains("port must be 1–65535 (e.g. 22)"),
        "the refusal does not carry the port rule with its example: |{line}|"
    );
    assert!(
        line.width() <= 80,
        "the refusal line is {} columns and does not fit an 80-column terminal: \
         |{line}|",
        line.width()
    );
    // It is drawn as a refusal, not as the rule it replaced.
    assert!(
        line.contains('!'),
        "the error row lost its `!` marker: |{line}|"
    );

    // The worst case — both required fields empty *and* a bad port — still
    // fits 80 columns, which is what the contract claims.
    let worst = plain(&body(&map_frame(
        &add_worst_refusal(),
        80,
        ColorSupport::Truecolor,
    ))[FormCursor::ROWS - 1]);
    assert!(
        worst.contains("alias is required")
            && worst.contains("host is required")
            && worst.contains("port must be 1–65535 (e.g. 22)"),
        "the worst-case refusal does not name all three problems: |{worst}|"
    );
    assert!(
        worst.width() <= 80,
        "the worst-case refusal is {} columns, over 80: |{worst}|",
        worst.width()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule G — the placeholder tier in the rendered map
// ─────────────────────────────────────────────────────────────────────────────

/// A placeholder is drawn in `fg_placeholder` and **never** in `fg_muted`.
///
/// This is the entire reason the tier exists. The field label is `fg_muted`; a
/// placeholder that also wore `fg_muted` would be the same colour as the word
/// two columns to its left, and `<optional>` would read as part of the label
/// rather than as an invitation to write something. The two tiers have to be
/// two tiers *in the same row*, or the new token is decoration on paper.
///
/// Asserted against the render, not the token: `fg_placeholder` being defined
/// fainter proves nothing about what the map draws, which is the class of bug
/// issue #42 was (`fg` existed, nothing read it).
#[test]
fn the_placeholder_is_drawn_in_the_placeholder_tier_never_the_muted_one() {
    let t = Theme::clack();

    for (mode, state, placeholder) in [
        ("add", add_blank(), "<optional>"),
        ("edit", edit_seeded(), "<not set>"),
    ] {
        let frame = map_frame(&state, 80, ColorSupport::Truecolor);

        let mut drawn = 0usize;
        for line in frame.lines() {
            for span in &line.spans {
                if span.content == placeholder {
                    drawn += 1;
                    assert_eq!(
                        span.style.fg,
                        Some(t.fg_placeholder),
                        "the {mode} map draws `{placeholder}` in {:?}, not in \
                         `fg_placeholder`",
                        span.style.fg
                    );
                    assert_ne!(
                        span.style.fg,
                        Some(t.fg_muted),
                        "the {mode} map draws `{placeholder}` in `fg_muted` — \
                         the same tier as its own label, which is the defect the \
                         placeholder tier was cut to fix"
                    );
                }
            }
        }
        assert!(
            drawn > 0,
            "the {mode} map never drew `{placeholder}` — the placeholder is not \
             reaching the frame at all"
        );
    }

    // The label in a placeholder row really is `fg_muted`, so the two tiers
    // are actually adjacent and actually different.
    let frame = map_frame(&add_blank(), 80, ColorSupport::Truecolor);
    let host_row = &body(&frame)[1];
    let label = host_row
        .spans
        .iter()
        .find(|s| s.content.trim() == "Host")
        .expect("the second map row is Host");
    assert_eq!(
        label.style.fg,
        Some(t.fg_muted),
        "the field label is not `fg_muted`, so the placeholder/label tier \
         difference is not the one being tested"
    );
    assert_ne!(
        label.style.fg,
        Some(t.fg_placeholder),
        "the label and the placeholder share a tier"
    );

    // A field that holds a value draws the value, not the placeholder.
    let seeded = map_frame(&edit_seeded(), 80, ColorSupport::Truecolor);
    let alias_row = plain(&body(&seeded)[0]);
    assert!(
        !alias_row.contains("<not set>"),
        "a filled field still shows its placeholder: |{alias_row}|"
    );
    assert!(
        alias_row.contains("prod-server"),
        "the seeded edit did not bring the stored alias onto its row: |{alias_row}|"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule H — the frame as it really renders, for human review
// ─────────────────────────────────────────────────────────────────────────────

/// Dump real ANSI frames for human review.
///
/// Gated on `SSHM_DUMP_FRAMES=1` so an ordinary test run writes nothing. This is
/// what makes "looked at" a step we can actually perform without a PTY, a browser
/// or a screen recorder: `cat target/design-frames/*.ansi` renders with true
/// colour in any modern terminal, and the frames are diffable between runs.
///
/// Dumped at two widths and two colour modes on purpose. The rail fitting and
/// the `NO_COLOR` downgrade are invisible to every other gate in this file:
/// one is about what happens at a width nobody is looking at, the other is
/// about bytes no contrast table can measure.
///
/// The six form-map frames are built by pushing real `KeyEvent`s through
/// `manage::step`, not by assembling a `FormFlow` by hand. The dump is a
/// review artefact, so it has to show the frame the keymap actually produces:
/// a hand-built flow would let a broken binding pass review with a beautiful
/// picture of a screen nobody can reach.
#[test]
fn dump_frames_for_review() {
    if std::env::var("SSHM_DUMP_FRAMES").is_err() {
        eprintln!("SSHM_DUMP_FRAMES unset — skipping frame dump");
        return;
    }

    let dir = format!("{}/target/design-frames", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).expect("create frame dir");

    let c = conns();
    let truecolor = ColorSupport::Truecolor;
    let mono = ColorSupport::Monochrome;

    let write = |name: &str, frame: &Frame| {
        let path = format!("{dir}/{name}.ansi");
        std::fs::write(&path, frame.to_ansi()).unwrap_or_else(|e| panic!("write {path}: {e}"));
        eprintln!("wrote {path}");
    };

    // The form map, at both widths and both colour modes, driven from the
    // keys. These six are the whole grammar the map has: the blank form, a
    // half-written one, the lit button, a refusal that ate the rule row, the
    // seeded edit, and a save with two `●` rows on the review.
    let maps: [(&str, ManageState); 6] = [
        ("add-empty", add_blank()),
        ("add-mid", add_mid()),
        ("add-ready", add_ready_on_submit()),
        ("add-error", add_refused_empty()),
        ("edit-seeded", edit_seeded()),
        ("edit-changed", edit_changed()),
    ];
    for (name, state) in &maps {
        let flow = FrameFlow::from(state);
        for (width, support, tag) in [
            (80usize, truecolor, ""),
            (60, truecolor, ""),
            (80, mono, "-mono"),
            (60, mono, "-mono"),
        ] {
            let frame = build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(width, 24, support),
                &flow,
            );
            write(
                &format!("frame-{name}-{width}{tag}"),
                &frame,
            );
        }
    }

    // The worst refusal the map can say, at both widths: this is the frame to
    // look at if the three-problem line ever stops fitting.
    for (width, tag) in [(80usize, ""), (60, "")] {
        let frame = map_frame(
            &add_worst_refusal(),
            width,
            ColorSupport::Truecolor,
        );
        write(
            &format!("frame-add-worst-error-{width}{tag}"),
            &frame,
        );
    }

    for (name, frame) in [
        (
            "frame-pick-80",
            build_frame(
                &c,
                "prod",
                0,
                FrameMode::Pick,
                Canvas::new(80, 24, truecolor),
            ),
        ),
        (
            "frame-manage-80",
            build_frame(&c, "", 1, FrameMode::Manage, Canvas::new(80, 24, truecolor)),
        ),
        (
            "frame-pick-60",
            build_frame(
                &c,
                "prod",
                0,
                FrameMode::Pick,
                Canvas::new(60, 24, truecolor),
            ),
        ),
        (
            "frame-manage-60",
            build_frame(&c, "", 1, FrameMode::Manage, Canvas::new(60, 24, truecolor)),
        ),
        (
            "frame-pick-80-mono",
            build_frame(&c, "prod", 0, FrameMode::Pick, Canvas::new(80, 24, mono)),
        ),
        (
            "frame-manage-80-mono",
            build_frame(&c, "", 1, FrameMode::Manage, Canvas::new(80, 24, mono)),
        ),
        (
            "frame-empty",
            build_frame(&[], "", 0, FrameMode::Pick, Canvas::new(80, 24, truecolor)),
        ),
        (
            "frame-no-match",
            build_frame(
                &c,
                "zzz",
                0,
                FrameMode::Pick,
                Canvas::new(80, 24, truecolor),
            ),
        ),
        // The #36 flow glyphs: `◆` asks, `■` has been answered, `◇` says
        // what happened. They are load-bearing state, so they get dumped for
        // the same human read the rest of the grammar gets — including the
        // one note whose whole job is *not* to claim a deletion.
        (
            "frame-manage-confirm",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: Some(c[0].clone()),
                    trace: None,
                    form: None,
                },
            ),
        ),
        (
            "frame-manage-deleted",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::Deleted {
                        connection: c[0].clone(),
                    }),
                    form: None,
                },
            ),
        ),
        (
            "frame-manage-delete-failed",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::DeleteFailed {
                        connection: c[0].clone(),
                    }),
                    form: None,
                },
            ),
        ),
        (
            "frame-manage-edited",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::Edited {
                        connection: c[0].clone(),
                    }),
                    form: None,
                },
            ),
        ),
        (
            "frame-manage-edit-failed",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::EditFailed {
                        connection: c[0].clone(),
                    }),
                    form: None,
                },
            ),
        ),
        // The notes an abandoned map leaves. Both exist to answer the one
        // question the user has after backing out of a half-filled form —
        // *did any of it get saved?* — so they get the same human read the
        // rest of the honesty grammar gets.
        (
            "frame-manage-add-abandoned",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::AddAbandoned),
                    form: None,
                },
            ),
        ),
        (
            "frame-manage-edit-abandoned",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::EditAbandoned),
                    form: None,
                },
            ),
        ),
        // The #38 import grammar: the offer asks on the header with the
        // count and the file it scanned, the `◇` notes report what the
        // answer actually did (earned from the store, not carried from
        // the ask), and the decline is the note that makes the empty
        // CTA underneath read as *heard and refused* rather than as a
        // screen that never asked anything.
        (
            "frame-import-offer-80",
            build_frame_with_flow(
                &[],
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: Some(sshm::frame::ImportOfferFlow {
                        count: 12,
                        path: "/home/dev/.ssh/config".into(),
                    }),
                    confirming: None,
                    trace: None,
                    form: None,
                },
            ),
        ),
        (
            "frame-import-offer-80-mono",
            build_frame_with_flow(
                &[],
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, mono),
                &FrameFlow {
                    import_offer: Some(sshm::frame::ImportOfferFlow {
                        count: 12,
                        path: "/home/dev/.ssh/config".into(),
                    }),
                    confirming: None,
                    trace: None,
                    form: None,
                },
            ),
        ),
        (
            "frame-imported-80",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::Imported {
                        imported: 12,
                        failed: 0,
                    }),
                    form: None,
                },
            ),
        ),
        (
            "frame-imported-partial-80",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::Imported {
                        imported: 9,
                        failed: 3,
                    }),
                    form: None,
                },
            ),
        ),
        (
            "frame-import-declined-80",
            build_frame_with_flow(
                &[],
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: Some(sshm::manage::Trace::ImportDeclined),
                    form: None,
                },
            ),
        ),
        // The #39 cached update note: a dim `◆` line above the header, read
        // from a previous run's cache. Dumped at truecolour and monochrome
        // so the human read confirms the note recedes behind the frame it
        // sits above, and that the `◆` glyph carries it when colour is off.
        (
            "frame-pick-update-note-80",
            build_frame_with_note(
                &c,
                "",
                0,
                FrameMode::Pick,
                Canvas::new(80, 24, truecolor),
                &FrameFlow::default(),
                Some("0.1.11"),
            ),
        ),
        (
            "frame-pick-update-note-80-mono",
            build_frame_with_note(
                &c,
                "",
                0,
                FrameMode::Pick,
                Canvas::new(80, 24, mono),
                &FrameFlow::default(),
                Some("0.1.11"),
            ),
        ),
        // The narrow-terminal degradation: the version is dropped so the
        // action survives. Dumped so the human read confirms the note
        // keeps what matters when the width runs out.
        (
            "frame-pick-update-note-narrow",
            build_frame_with_note(
                &c,
                "",
                0,
                FrameMode::Pick,
                Canvas::new(38, 24, truecolor),
                &FrameFlow::default(),
                Some("0.1.11"),
            ),
        ),
    ] {
        // The frame's own name already carries the support it was built with;
        // appending the *detected* mode here would mislabel a truecolour frame
        // as "Monochrome" on a CI box that has no TERM.
        write(name, &frame);
    }

    // The `◆ error` settle is a single line, not a frame, so it is dumped
    // beside them rather than as one.
    let error_line = sshm::inline::settle_trace(
        &sshm::inline::Settle::Error {
            connection: c[0].clone(),
            message: "Permission denied (os error 13)".into(),
        },
        Canvas::new(80, 24, truecolor),
    );
    let path = format!("{dir}/frame-manage-error-settle.ansi");
    std::fs::write(
        &path,
        error_line
            .iter()
            .map(|l| sshm::theme::ansi::line_to_ansi(l))
            .collect::<String>(),
    )
    .unwrap_or_else(|e| panic!("write {path}: {e}"));
    eprintln!("wrote {path}");

    // The `◆ import failed: <reason>` settle (#38) is the same
    // single-line shape, so it is dumped the same way: the trace lines'
    // ANSI joined by newlines, through the same `line_to_ansi` the
    // frames' `to_ansi` writes with.
    let import_failed_line = sshm::inline::settle_trace(
        &sshm::inline::Settle::ImportFailed {
            message: "disk on fire: read-only filesystem".into(),
        },
        Canvas::new(80, 24, truecolor),
    );
    let path = format!("{dir}/frame-import-failed-settle.ansi");
    std::fs::write(
        &path,
        import_failed_line
            .iter()
            .map(|l| sshm::theme::ansi::line_to_ansi(l))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap_or_else(|e| panic!("write {path}: {e}"));
    eprintln!("wrote {path}");
}
