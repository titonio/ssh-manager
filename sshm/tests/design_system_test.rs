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
//! | text contrast | unreadable foreground/background combinations |
//! | selection perceivability | the primary interactive state washing out |
//! | non-color signaling | meaning carried by color alone (colour-blind users) |
//! | footer width | key hints clipped off an 80-column terminal |
//! | degradation | the UI collapsing in 16-colour / `NO_COLOR` terminals |
//! | minimum size gate | a four-chunk layout collapsing below 60x14 |

use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use sshm::app::{App, AppMode, MIN_TUI_HEIGHT, MIN_TUI_WIDTH};
use sshm::config::Config;
use sshm::connections::ConnectionDraft;
use sshm::frame::{build_frame, Canvas, FrameMode};
use sshm::picker::{compute_matches, render_picker_frame};
use sshm::theme::{contrast, nord, ColorSupport, Theme};

/// A foreground/background pair the UI actually draws together, with the WCAG
/// floor that pair has to clear.
struct Pair {
    label: &'static str,
    fg: Color,
    bg: Color,
    min: f64,
}

const TEXT: f64 = 4.5; // WCAG AA, normal text
const LARGE: f64 = 3.0; // WCAG AA, large/bold text and 1.4.11 UI contrast

fn conns() -> Vec<sshm::config::Connection> {
    vec![
        sshm::config::Connection {
            id: "1".into(),
            alias: "prod-server".into(),
            host: "10.0.0.1".into(),
            user: "root".into(),
            port: 22,
            key_path: None,
            folder: Some("production".into()),
        },
        sshm::config::Connection {
            id: "2".into(),
            alias: "dev-box".into(),
            host: "10.0.0.2".into(),
            user: "dev".into(),
            port: 2222,
            key_path: None,
            folder: None,
        },
    ]
}

fn test_app(mode: AppMode) -> App {
    App {
        config: Config::new(),
        selected_index: 0,
        search_query: String::new(),
        mode,
        matcher: SkimMatcherV2::default(),
        filtered_indices: vec![],
        message: None,
        input_buffer: ConnectionDraft::default(),
        input_field: 0,
        should_connect: None,
        ctrl_c_count: 0,
        update_info: None,
    }
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
        "src/app.rs",
        "src/connections.rs",
        "src/frame.rs",
        "src/inline.rs",
        "src/picker.rs",
        "src/main.rs",
        "src/runtime.rs",
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

/// The Clack palette carries no fixed RGB.
///
/// The transparent inline frame (#33) borrows the user's terminal background, so
/// it cannot contrast-check a hue of its own against a surface it cannot see.
/// Every role is therefore a **named ANSI colour** — which the terminal maps to
/// *its* palette, at the user's chosen values — or `Reset`, meaning "whatever
/// the terminal is already using". A `Rgb(..)` or `Indexed(..)` here would pin a
/// colour the frame has no business choosing, and would break the degrade to 16
/// colours by skipping the terminal's own mapping.
#[test]
fn the_clack_palette_is_named_ansi_only() {
    let t = Theme::clack();
    let fixed = |c: Color| matches!(c, Color::Rgb(..) | Color::Indexed(..));

    for (role, color) in [
        ("bg", t.bg),
        ("fg", t.fg),
        ("fg_bright", t.fg_bright),
        ("fg_muted", t.fg_muted),
        ("accent", t.accent),
        ("border", t.border),
        ("highlight", t.highlight),
        ("success", t.success),
        ("warning", t.warning),
        ("selection_bg", t.selection_bg),
        ("selection_fg", t.selection_fg),
    ] {
        assert!(
            !fixed(color),
            "clack role `{role}` is a fixed colour ({color:?}) — the transparent \
             frame's palette must be named ANSI colours or Reset only"
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
    assert_eq!(t.fg_muted, Color::DarkGray, "the dim meta is bright black");
}

/// A transparent frame has no background of its own, so the WCAG table that
/// governs Nord cannot govern it. What replaces that check is the structural
/// claim: the frame owns no background, body text is the terminal's own
/// foreground, and selection is a glyph rather than a fill.
#[test]
fn the_clack_palette_leaves_the_surface_to_the_terminal() {
    let t = Theme::clack();

    assert_eq!(t.bg, Color::Reset, "the frame must not own a background");
    assert_eq!(
        t.fg,
        Color::Reset,
        "body text is the terminal's own foreground, so it is readable on the \
         terminal's own background by construction"
    );
    assert_eq!(
        t.selection_bg,
        Color::Reset,
        "selection is carried by the ❯ glyph plus a bold alias, never by a filled row"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule B — text contrast on pairs the UI actually draws
// ─────────────────────────────────────────────────────────────────────────────

/// Every text-bearing pair in the palette must clear WCAG AA.
///
/// The pairs are enumerated from the render code, so this tracks the real UI.
/// Adding a role that gets drawn as text means adding it here — which is the
/// point: the check is cheap enough to run on every push.
#[test]
fn every_text_pair_meets_wcag_aa() {
    let t = Theme::nord();
    let pairs = [
        Pair {
            label: "header title on bg",
            fg: t.fg_bright,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "normal row on bg",
            fg: t.fg,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "selected row (highlight) on bg",
            fg: t.highlight,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "empty/no-match message on bg",
            fg: t.fg_muted,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "footer key hints on bg",
            fg: t.success,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "popup body text on bg",
            fg: t.fg,
            bg: t.bg,
            min: TEXT,
        },
        Pair {
            label: "matched char on selection highlight",
            fg: t.bg,
            bg: t.highlight,
            min: TEXT,
        },
        // Interactive state: a user must be able to read what they have selected.
        Pair {
            label: "selection_fg on selection_bg",
            fg: t.selection_fg,
            bg: t.selection_bg,
            min: TEXT,
        },
        // Non-text UI boundaries (WCAG 1.4.11).
        Pair {
            label: "accent frame on bg",
            fg: t.accent,
            bg: t.bg,
            min: LARGE,
        },
        Pair {
            label: "search border on bg",
            fg: t.highlight,
            bg: t.bg,
            min: LARGE,
        },
    ];

    let mut failures: Vec<String> = Vec::new();
    for p in &pairs {
        let ratio = contrast::ratio(p.fg, p.bg);
        if ratio < p.min {
            failures.push(format!(
                "  {:<42} {:>5.2}:1  (needs {:.1}:1)",
                p.label, ratio, p.min
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "accessible-contrast violations:\n{}\n\nFix by choosing a different token in \
         Theme::nord() — do not lower the threshold.",
        failures.join("\n")
    );
}

/// Decorative borders are reported, not asserted.
///
/// WCAG 1.4.11 covers boundaries "required to identify user interface
/// components and their states". A panel border that carries no state is
/// generally exempt, and Nord's `NORD3` is deliberately subtle. This test prints
/// the real numbers so the tradeoff stays visible instead of being silently
/// accepted.
#[test]
fn decorative_border_contrast_is_reported() {
    let t = Theme::nord();
    let ratio = contrast::ratio(t.border, t.bg);
    println!(
        "decorative panel border (NORD3 on NORD0): {ratio:.2}:1 \
         [below 3:1 — accepted as intentional subtlety, carries no state]"
    );
    assert!(
        ratio > 1.0,
        "border should at least be distinguishable from bg"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule C — scan what actually renders
// ─────────────────────────────────────────────────────────────────────────────

/// Walk rendered cells and reject any low-contrast text the palette table missed.
///
/// The table in Rule B is hand-maintained and can drift from the render code.
/// This reads the real buffer, so a pair drawn somewhere nobody listed it still
/// gets caught. Cells with no explicit background are skipped: the picker is
/// inline and inherits the user's terminal background, which we cannot measure.
#[test]
fn rendered_frames_contain_no_low_contrast_cells() {
    let mut offenders: Vec<String> = Vec::new();

    // Inline picker, with matches and with a selection.
    let c = conns();
    let matcher = SkimMatcherV2::default();
    for query in ["", "prod", "dev"] {
        let matches = compute_matches(&c, &matcher, query);
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &c, &matches, 0, query))
            .unwrap();
        scan_buffer(
            terminal.backend().buffer(),
            &format!("picker query={query:?}"),
            &mut offenders,
        );
    }

    // Fullscreen TUI, normal mode.
    let app = test_app(AppMode::Normal);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    scan_buffer(terminal.backend().buffer(), "tui normal", &mut offenders);

    assert!(
        offenders.is_empty(),
        "low-contrast cells found in rendered frames:\n{}",
        offenders.join("\n")
    );
}

fn scan_buffer(buffer: &ratatui::buffer::Buffer, where_: &str, offenders: &mut Vec<String>) {
    for (i, cell) in buffer.content.iter().enumerate() {
        // No explicit color on both sides → depends on the user's terminal, skip.
        if cell.fg == Color::Reset || cell.bg == Color::Reset {
            continue;
        }
        // Box-drawing and block elements are chrome, not text. This rule's 4.5:1
        // floor is the WCAG *text* threshold; applying it to borders contradicts
        // the exemption Rule B documents. `border` on `bg` is 1.69:1 by design
        // and is surfaced by decorative_border_contrast_is_reported instead.
        if is_chrome(cell.symbol()) {
            continue;
        }
        let ratio = contrast::ratio(cell.fg, cell.bg);
        if ratio < TEXT {
            offenders.push(format!(
                "  {where_} cell #{i} {:?} fg={:?} bg={:?} -> {:.2}:1",
                cell.symbol(),
                cell.fg,
                cell.bg,
                ratio
            ));
        }
    }
}

/// True for glyphs that draw structure rather than carry readable content.
fn is_chrome(symbol: &str) -> bool {
    let first = match symbol.chars().next() {
        Some(c) => c,
        None => return true,
    };
    matches!(first as u32, 0x2500..=0x259F) || matches!(first, '+' | '-' | '|')
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule D — meaning must not live in color alone
// ─────────────────────────────────────────────────────────────────────────────

/// Selection must be identifiable without perceiving color.
///
/// ~8% of men have some colour-vision deficiency. If the only thing that changes
/// when you move down the list is a background hue, the UI is unreadable to
/// them. The `> ` highlight symbol is the non-color marker; this asserts it
/// actually moves with the selection.
#[test]
fn selection_carries_a_non_color_marker() {
    let c = conns();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(&c, &matcher, "");

    let marker_row = |selected: usize| -> Option<usize> {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render_picker_frame(f, &c, &matches, selected, ""))
            .unwrap();
        let buf = terminal.backend().buffer();
        let width = usize::from(buf.area().width);
        (0..usize::from(buf.area().height)).find(|&y| {
            // Read the interior only; column 0 is the box-drawing border.
            let row: String = buf.content[y * width + 1..(y + 1) * width - 1]
                .iter()
                .map(|c| c.symbol())
                .collect();
            row.trim_start().starts_with('>')
        })
    };

    let row0 = marker_row(0).expect("no '>' marker with selection 0");
    let row1 = marker_row(1).expect("no '>' marker with selection 1");
    assert_ne!(
        row0, row1,
        "the '>' selection marker did not move — selection is color-only"
    );
}

/// Bold is applied independently of the palette, so emphasis survives `NO_COLOR`.
#[test]
fn emphasis_survives_a_colorless_terminal() {
    let c = conns();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(&c, &matcher, "prod");
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_picker_frame(f, &c, &matches, 0, "prod"))
        .unwrap();
    let has_bold = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .any(|c| c.modifier.contains(Modifier::BOLD));
    assert!(
        has_bold,
        "matched characters lost their bold emphasis — under NO_COLOR they would be indistinguishable"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule E — key hints must fit the terminal
// ─────────────────────────────────────────────────────────────────────────────

/// The narrowest terminal we promise to support.
///
/// 80x24 is the conservative baseline: it is what a default xterm, a plain SSH
/// session and tmux's default pane all give you.
const MIN_WIDTH: u16 = 80;

/// The hints a user must be able to see at the minimum supported width.
///
/// Not "the whole hint set" — that was the original bug, a 116-char string in a
/// 78-char interior. This asserts the contract instead: movement, the primary
/// action, search, help and the escape hatch all survive at 80x24.
const CORE_HINTS: [&str; 5] = ["Navigate", "Connect", "Search", "Help", "Quit"];

#[test]
fn footer_hints_fit_the_minimum_supported_width() {
    let app = test_app(AppMode::Normal);
    let backend = TestBackend::new(MIN_WIDTH, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    let width = usize::from(buf.area().width);

    // The footer is the bottom-most bordered row; read its interior.
    let footer_row = usize::from(buf.area().height) - 2;
    let rendered: String = buf.content[footer_row * width + 1..(footer_row + 1) * width - 1]
        .iter()
        .map(|c| c.symbol())
        .collect();
    let rendered = rendered.trim();

    let missing: Vec<&str> = CORE_HINTS
        .iter()
        .copied()
        .filter(|h| !rendered.contains(h))
        .collect();

    assert!(
        missing.is_empty(),
        "core key hints missing at {} columns: {:?}\nrendered footer: {:?}",
        MIN_WIDTH,
        missing,
        rendered
    );
}

/// `fit_hints` must drop from the end, never truncate a label mid-word.
#[test]
fn fit_hints_drops_whole_labels_only() {
    let hints = ["Navigate", "Connect", "Search", "Help", "Quit"];

    // Wide enough for everything.
    assert_eq!(
        sshm::app::fit_hints(&hints, 80),
        "Navigate | Connect | Search | Help | Quit"
    );
    // Tight: only the first two fit.
    assert_eq!(sshm::app::fit_hints(&hints, 18), "Navigate | Connect");
    // Impossible: never emit a partial label or a dangling separator.
    assert_eq!(sshm::app::fit_hints(&hints, 3), "");
    // Exactly one label.
    assert_eq!(sshm::app::fit_hints(&hints, 8), "Navigate");
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule F — graceful degradation
// ─────────────────────────────────────────────────────────────────────────────

/// Reduced-color modes must stay readable, not just stay quiet.
#[test]
fn every_color_mode_stays_readable() {
    for support in [
        ColorSupport::Truecolor,
        ColorSupport::Ansi256,
        ColorSupport::Ansi16,
    ] {
        let t = Theme::nord().resolve(support);
        assert!(
            contrast::meets_aa_text(t.fg, t.bg),
            "{support:?}: body text unreadable at {:.2}:1",
            contrast::ratio(t.fg, t.bg)
        );
        assert!(
            contrast::luminance(contrast::to_rgb(t.bg).unwrap()) < 0.2,
            "{support:?}: background drifted light"
        );
        // The selection must still stand out from ordinary text in every mode.
        assert!(
            t.selection_bg != t.bg,
            "{support:?}: selection collapsed into the background"
        );
    }
}

/// `NO_COLOR` must suppress color without suppressing structure.
#[test]
fn no_color_suppresses_color_not_structure() {
    let t = Theme::nord().resolve(ColorSupport::Monochrome);
    for color in [t.bg, t.fg, t.accent, t.highlight, t.selection_bg] {
        assert_eq!(color, Color::Reset, "monochrome theme still emits color");
    }
    // Contrast math treats Reset as unmeasurable rather than silently passing.
    assert_eq!(contrast::to_rgb(Color::Reset), None);
}

/// The palette must never hand back a color that is not Nord.
#[test]
fn resolved_palette_stays_within_nord_for_truecolor() {
    for color in [
        Theme::nord().bg,
        Theme::nord().fg,
        Theme::nord().fg_bright,
        Theme::nord().fg_muted,
        Theme::nord().accent,
        Theme::nord().border,
        Theme::nord().highlight,
        Theme::nord().success,
        Theme::nord().warning,
    ] {
        assert!(
            nord::ALL.contains(&color),
            "non-Nord color leaked into the palette: {color:?}"
        );
    }
}

/// Sanity: the active theme resolves without panicking in a bare environment.
#[test]
fn active_theme_resolves_lazily() {
    let t: &Theme = sshm::theme::active();
    // Whatever the ambient terminal is, the roles must be populated with
    // something we can reason about. Asserted by property rather than by Color
    // variant, because a 16-colour downgrade hands back named colors and a
    // 256-colour one hands back Indexed — all of them legitimate.
    assert_ne!(t.fg, Color::Rgb(0, 0, 0), "fg resolved to unset black");
    match contrast::to_rgb(t.bg) {
        Some(rgb) => assert!(
            contrast::luminance(rgb) < 0.2,
            "background drifted light in this colour mode: {:?}",
            t.bg
        ),
        None => assert_eq!(
            t.bg,
            Color::Reset,
            "an unresolvable background should only ever be Reset"
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule G — the frame as it really rendered, in this process's colour mode
// ─────────────────────────────────────────────────────────────────────────────

/// The selection must still be visibly distinct in whatever mode this process
/// actually resolved.
///
/// `every_color_mode_stays_readable` checks the palette arithmetic for all four
/// modes up front. This checks the frame that really rendered, so a downgrade
/// that collapsed two roles onto the same terminal colour is caught at the cell
/// level rather than in the table. Driven by the environment, which is exactly
/// what `ColorSupport::detect` reads — that is what makes the CI matrix work
/// without any injection.
#[test]
fn rendered_selection_is_distinct_in_this_mode() {
    let t = sshm::theme::active();
    let mode = format!("{:?}", ColorSupport::detect());

    if t.selection_bg == Color::Reset {
        // NO_COLOR / dumb terminal: no colour to be distinct with. The '>' marker
        // asserted in Rule D is what carries selection here.
        return;
    }

    assert_ne!(
        t.selection_bg, t.bg,
        "{mode}: selection background collapsed onto the page background"
    );

    let c = conns();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(&c, &matcher, "");
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_picker_frame(f, &c, &matches, 0, ""))
        .unwrap();

    let painted = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .filter(|cell| cell.bg == t.selection_bg)
        .count();

    assert!(
        painted > 0,
        "{mode}: no cell was painted with the selection background — the selection \
         does not render at all in this colour mode"
    );
}

/// Dump real ANSI frames for human review.
///
/// Gated on `SSHM_DUMP_FRAMES=1` so an ordinary test run writes nothing. This is
/// what makes "looked at" a step we can actually perform without a PTY, a browser
/// or a screen recorder: `cat target/design-frames/*.ansi` renders with true
/// colour in any modern terminal, and the frames are diffable between runs.
#[test]
fn dump_frames_for_review() {
    if std::env::var("SSHM_DUMP_FRAMES").is_err() {
        eprintln!("SSHM_DUMP_FRAMES unset — skipping frame dump");
        return;
    }

    let dir = format!("{}/target/design-frames", env!("CARGO_MANIFEST_DIR"));
    std::fs::create_dir_all(&dir).expect("create frame dir");
    let mode = format!("{:?}", ColorSupport::detect());

    let c = conns();
    let matcher = SkimMatcherV2::default();

    let matches = compute_matches(&c, &matcher, "prod");
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_picker_frame(f, &c, &matches, 0, "prod"))
        .unwrap();
    write_frame(&dir, &mode, "picker", terminal.backend().buffer());

    let app = test_app(AppMode::Normal);
    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    write_frame(&dir, &mode, "tui", terminal.backend().buffer());

    // The inline frame seam (#33). These need no terminal at all: a Frame is a
    // value, and `to_ansi` turns it into bytes a human can look at. This is the
    // step that makes "verified" mean something for a transparent frame — the
    // WCAG table cannot govern it, so the eyeball has to.
    //
    // Dumped at two widths and two colour modes on purpose. The rail fitting and
    // the `NO_COLOR` downgrade are invisible to every other gate in this file:
    // one is about what happens at a width nobody is looking at, the other is
    // about bytes no contrast table can measure.
    let truecolor = ColorSupport::Truecolor;
    let mono = ColorSupport::Monochrome;
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
    ] {
        // The frame's own name already carries the support it was built with;
        // appending the *detected* mode here would mislabel a truecolour frame
        // as "Monochrome" on a CI box that has no TERM.
        let path = format!("{dir}/{name}.ansi");
        std::fs::write(&path, frame.to_ansi()).unwrap_or_else(|e| panic!("write {path}: {e}"));
        eprintln!("wrote {path}");
    }
}

fn write_frame(dir: &str, mode: &str, surface: &str, buffer: &ratatui::buffer::Buffer) {
    let path = format!("{dir}/{surface}-{mode}.ansi");
    std::fs::write(&path, sshm::theme::ansi::buffer_to_ansi(buffer))
        .unwrap_or_else(|e| panic!("write {path}: {e}"));
    eprintln!("wrote {path}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule H — a surface must paint its own background
// ─────────────────────────────────────────────────────────────────────────────

/// The inline picker paints an opaque frame rather than overlaying the shell.
///
/// The picker draws inside someone else's live terminal, so it may not inherit a
/// background it has not measured — `fg` on a white terminal is 1.35:1. The
/// block's `.bg(t.bg)` is the one call that makes every dark-background ratio in
/// Rule B actually apply on this surface. When that call was added the snapshot
/// diff was *empty*: `TestBackend` writes glyphs and drops SGR, so the suite was
/// blind to the fix and would be equally blind to its removal.
///
/// The border ring is deliberately included rather than excluded. `border_style`
/// names a foreground only, so the ring is the one region whose background the
/// block is solely responsible for — it is where a dropped `.bg(t.bg)` shows
/// first, while every interior row is also painted by the list and paragraph
/// base styles. Excluding it would leave this test unable to fail.
#[test]
fn picker_paints_an_opaque_background() {
    let t = sshm::theme::active();
    if t.bg == Color::Reset {
        return; // NO_COLOR / dumb terminal: there is no background to paint.
    }
    let c = conns();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(&c, &matcher, "");
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_picker_frame(f, &c, &matches, 0, ""))
        .unwrap();
    let bare: Vec<usize> = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.bg == Color::Reset)
        .map(|(i, _)| i)
        .collect();
    assert!(
        bare.is_empty(),
        "picker frame left {} cell(s) unpainted (first: #{}) — the frame must carry \
         an explicit t.bg everywhere, including its border ring",
        bare.len(),
        bare[0]
    );
}

/// The selected row paints `selection_bg` onto the buffer.
///
/// Rule B proves the selection pair is readable and Rule D proves the `>` marker
/// moves; neither proves the background reaches the screen. `List::highlight_style`
/// is inert under a stateless `render_widget`, so the only thing that paints a
/// selection is the explicit `.bg(t.selection_bg)` in `build_picker_row_spans`.
/// Drop it and the selection degrades to a lone `>` on the page background, and
/// no glyph snapshot would notice. Index 1 is used so this covers a row other
/// than the one Rule G checks.
#[test]
fn selected_row_paints_selection_bg() {
    let t = sshm::theme::active();
    if t.selection_bg == Color::Reset {
        // NO_COLOR: nothing to paint with. The '>' marker asserted in Rule D is
        // what carries the selection there.
        return;
    }
    let c = conns();
    let matcher = SkimMatcherV2::default();
    let matches = compute_matches(&c, &matcher, "");
    let backend = TestBackend::new(80, 15);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_picker_frame(f, &c, &matches, 1, ""))
        .unwrap();
    let painted = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .filter(|cell| cell.bg == t.selection_bg)
        .count();
    assert!(
        painted > 0,
        "no cell was painted with selection_bg ({:?}) — the selected row renders \
         without its background",
        t.selection_bg
    );
}

/// The fullscreen TUI paints a background inside every panel it draws.
///
/// Same defect class as the picker, opposite constraint: here the TUI owns the
/// whole terminal, so an unpainted cell lets the user's own background show
/// through the layout and a panel visibly floats. Every block in `App::render`
/// sets `.bg(t.bg)`; this asserts a real 80x24 frame comes back fully painted
/// rather than trusting four separate call sites.
#[test]
fn tui_paints_its_panel_background() {
    let t = sshm::theme::active();
    if t.bg == Color::Reset {
        return; // NO_COLOR: nothing to paint with.
    }
    let app = test_app(AppMode::Normal);
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let buf = terminal.backend().buffer();
    let w = usize::from(buf.area().width);
    let bare: Vec<usize> = (1..usize::from(buf.area().height) - 1)
        .flat_map(|y| (1..w - 1).map(move |x| y * w + x))
        .filter(|&i| buf.content[i].bg == Color::Reset)
        .collect();
    assert!(
        bare.is_empty(),
        "TUI interior left {} cell(s) with no explicit background (first: #{}) — \
         a panel that does not paint t.bg lets the user's terminal show through",
        bare.len(),
        bare[0]
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule I — the minimum size gate
// ─────────────────────────────────────────────────────────────────────────────

/// Flatten a rendered buffer to text so a test can ask whether a string was drawn.
///
/// Every other assertion here works cell-by-cell because it is about colour. This
/// rule is about *content* — what words reached the screen — so it needs the
/// whole frame as one string.
fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let w = usize::from(buffer.area().width);
    (0..usize::from(buffer.area().height))
        .map(|y| {
            buffer.content[y * w..(y + 1) * w]
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Below the declared minimum the gate *replaces* the layout rather than adding
/// a message on top of a broken one.
///
/// The normal layout is four bordered chunks — 3 + 3 + flex + 3 — so twelve rows
/// are spent before the list gets a single cell. At 40x8 that renders borders
/// stacked on borders and passes for a UI. The gate has to suppress it, which is
/// why this asserts the header title is *absent* and not merely that the message
/// is present.
#[test]
fn below_the_minimum_the_gate_replaces_the_layout() {
    let app = test_app(AppMode::Normal);
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    let text = buffer_text(terminal.backend().buffer());

    assert!(
        text.contains("Resize to at least") && text.contains("60x14"),
        "the too-small message did not render at 40x8:\n{text}"
    );
    assert!(
        text.contains("40x8"),
        "the message did not name the size it was actually given:\n{text}"
    );
    assert!(
        !text.contains("SSH Connection Manager"),
        "the normal header rendered below the minimum — the gate replaced nothing:\n{text}"
    );
}

/// Nothing in the gate may panic, whatever size it is handed.
///
/// The guard fires on geometries the layout was never designed for, down to a
/// single cell with no room for a border, a line, or a whole word. Every
/// computation in `render_too_small` saturates for exactly this reason. A panic
/// here is worse than a bad frame: inside a real fullscreen session it leaves the
/// user's terminal with no echo and no cursor.
#[test]
fn the_gate_does_not_panic_at_any_size() {
    for (w, h) in [(1u16, 1u16), (20u16, 5u16), (40u16, 8u16)] {
        let app = test_app(AppMode::Normal);
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            !text.contains("SSH Connection Manager"),
            "{w}x{h}: the normal layout rendered below the minimum"
        );
    }
}

/// At exactly the minimum, and above it, the normal layout comes back.
///
/// The boundary is inclusive: 60x14 is supported, not merely not-rejected. This
/// is the half of the gate a too-generous minimum would break, and the reason
/// every existing 80x24 / 80x20 / 100x30 frame is unaffected.
#[test]
fn at_or_above_the_minimum_the_normal_layout_renders() {
    for (w, h) in [
        (MIN_TUI_WIDTH, MIN_TUI_HEIGHT),
        (80u16, 24u16),
        (100u16, 30u16),
    ] {
        let app = test_app(AppMode::Normal);
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("SSH Connection Manager"),
            "{w}x{h}: the header did not render at or above the minimum"
        );
        assert!(
            !text.contains("Resize to at least"),
            "{w}x{h}: the too-small message fired at or above the minimum"
        );
    }
}

/// The guard sits above the Help and Update checks, so no mode can bypass it.
///
/// Help and the update popup lay out their own centered boxes and would each need
/// a separate gate if this check came after them. One check at the top of
/// `render` covers every surface that passes through it — including the message
/// popup, which draws the normal frame underneath.
#[test]
fn the_gate_covers_every_mode_including_the_popups() {
    for mode in [
        AppMode::Normal,
        AppMode::Add,
        AppMode::Search,
        AppMode::Help,
        AppMode::Update,
    ] {
        let app = test_app(mode);
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let text = buffer_text(terminal.backend().buffer());
        assert!(
            text.contains("Resize to at least"),
            "{mode:?}: the gate did not fire below the minimum"
        );
    }
}
