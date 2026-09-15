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
//! | hint fitting | key hints clipped off an 80-column terminal |
//! | degradation | the UI collapsing in `NO_COLOR` terminals |
//!
//! The fullscreen TUI's gates (WCAG pairs against a painted background, the
//! opaque-panel rule, the minimum-size gate) were deleted with the surface in
//! #35: a transparent frame that borrows the user's terminal has no painted
//! background to contrast-check, and the Clack structural claims above are
//! what replaced the ratio table by design (see the spec, #31). The inline
//! frame's own selection-marker, emphasis and degradation properties are
//! gated in `frame_test.rs`, at the seam that produces them.

use ratatui::style::Color;
use sshm::frame::{build_frame, Canvas, FrameMode};
use sshm::theme::{ColorSupport, Theme};

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
/// once governed the fullscreen Nord palette cannot govern it. What replaces
/// that check is the structural claim: the frame owns no background, body
/// text is the terminal's own foreground, and selection is a glyph rather
/// than a fill.
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
// Rule E — key hints must fit the terminal
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

// ─────────────────────────────────────────────────────────────────────────────
// Rule F — graceful degradation
// ─────────────────────────────────────────────────────────────────────────────

/// `NO_COLOR` must suppress color without suppressing structure.
///
/// The Clack palette resolved to monochrome is all `Reset`: the frame's
/// meaning then lives entirely in the glyphs and the modifiers, which is
/// what `frame_test.rs` proves survives the downgrade.
#[test]
fn no_color_suppresses_color_not_structure() {
    let t = Theme::clack().resolve(ColorSupport::Monochrome);
    for color in [t.bg, t.fg, t.accent, t.highlight, t.selection_bg] {
        assert_eq!(color, Color::Reset, "monochrome theme still emits color");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule G — the frame as it really renders, for human review
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
