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
use sshm::frame::{
    build_frame, build_frame_with_flow, build_frame_with_note, Canvas, FrameFlow, FrameMode,
};
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
        ("fg", t.fg),
        ("fg_muted", t.fg_muted),
        ("accent", t.accent),
        ("border", t.border),
        ("highlight", t.highlight),
        ("success", t.success),
        ("warning", t.warning),
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

    assert_eq!(
        t.fg,
        Color::Reset,
        "body text is the terminal's own foreground, so it is readable on the \
         terminal's own background by construction"
    );
    // There is no background role left to assert on: `bg`, `selection_bg`
    // and `selection_fg` were deleted with the fullscreen TUI (#35)
    // because no live render path read them. A transparent frame cannot
    // own a background even by accident now — and what it actually emits
    // is gated where it is produced, by
    // `no_span_in_any_frame_sets_a_background` and
    // `the_serialized_frame_asks_for_no_background_colour` in `frame_test.rs`.
}

// ─────────────────────────────────────────────────────────────────────────────
// Rule H — a settle trace may only name a state the spec has
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
    for color in [
        t.fg,
        t.fg_muted,
        t.accent,
        t.border,
        t.highlight,
        t.success,
        t.warning,
    ] {
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
                },
            ),
        ),
        // The #37 in-place editor: the header names the target, the field
        // line carries the seeded value and the caret, and the refusal is
        // the warning glyph. Dumped so the grammar gets the same human
        // read the delete grammar gets.
        (
            "frame-manage-edit",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: None,
                    edit: Some(sshm::frame::EditFlow {
                        target: c[0].clone(),
                        label: "Alias",
                        input: "web-01".into(),
                        error: None,
                    }),
                },
            ),
        ),
        (
            "frame-manage-edit-refused",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: None,
                    edit: Some(sshm::frame::EditFlow {
                        target: c[0].clone(),
                        label: "Port",
                        input: "99999".into(),
                        error: Some("port must be a number from 1 to 65535".into()),
                    }),
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
                },
            ),
        ),
        // The #37 add sequence: the live caret, the `—` an optional field
        // settles to, and the `!` a refusal wears. These three glyphs are
        // new grammar and had never been produced for a human read.
        (
            "frame-manage-add",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: Some(sshm::frame::AddFlow {
                        label: "Alias",
                        input: "web-03".into(),
                        error: None,
                        settled: vec![],
                        last: false,
                    }),
                    edit: None,
                },
            ),
        ),
        (
            "frame-manage-add-settled",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: Some(sshm::frame::AddFlow {
                        label: "Folder",
                        input: "stag".into(),
                        error: None,
                        settled: vec![
                            sshm::frame::SettledField {
                                label: "Alias",
                                value: Some("web-03".into()),
                            },
                            sshm::frame::SettledField {
                                label: "Host",
                                value: Some("10.0.0.9".into()),
                            },
                            sshm::frame::SettledField {
                                label: "Port",
                                value: Some("22".into()),
                            },
                            // The optional field left empty: `—`, not a blank.
                            sshm::frame::SettledField {
                                label: "Key",
                                value: None,
                            },
                        ],
                        last: true,
                    }),
                    edit: None,
                },
            ),
        ),
        (
            "frame-manage-add-refused",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: Some(sshm::frame::AddFlow {
                        label: "Alias",
                        input: String::new(),
                        error: Some("alias is required".into()),
                        settled: vec![],
                        last: false,
                    }),
                    edit: None,
                },
            ),
        ),
        // The new grammar at 60 columns and in monochrome: the rail drop
        // and the NO_COLOR downgrade are the two things every other gate
        // here cannot see, and the edit/add lines are new enough that
        // nobody has read them under either.
        (
            "frame-manage-edit-60",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(60, 24, truecolor),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: None,
                    edit: Some(sshm::frame::EditFlow {
                        target: c[0].clone(),
                        label: "Port",
                        input: "22".into(),
                        error: None,
                    }),
                },
            ),
        ),
        (
            "frame-manage-edit-refused-80-mono",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, mono),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: None,
                    edit: Some(sshm::frame::EditFlow {
                        target: c[0].clone(),
                        label: "Port",
                        input: "99999".into(),
                        error: Some("port must be a number from 1 to 65535".into()),
                    }),
                },
            ),
        ),
        (
            "frame-manage-add-settled-80-mono",
            build_frame_with_flow(
                &c,
                "",
                0,
                FrameMode::Manage,
                Canvas::new(80, 24, mono),
                &FrameFlow {
                    import_offer: None,
                    confirming: None,
                    trace: None,
                    add: Some(sshm::frame::AddFlow {
                        label: "Folder",
                        input: "stag".into(),
                        error: None,
                        settled: vec![
                            sshm::frame::SettledField {
                                label: "Alias",
                                value: Some("web-03".into()),
                            },
                            sshm::frame::SettledField {
                                label: "Key",
                                value: None,
                            },
                        ],
                        last: true,
                    }),
                    edit: None,
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
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
                    add: None,
                    edit: None,
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
        let path = format!("{dir}/{name}.ansi");
        std::fs::write(&path, frame.to_ansi()).unwrap_or_else(|e| panic!("write {path}: {e}"));
        eprintln!("wrote {path}");
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
