---
name: sshm-design
description: sshm-specific UI design rules — semantic colour tokens in theme.rs, the three distinct surfaces (fullscreen TUI, inline shell picker, shell init widget), and the verification loop that proves a change is readable. Use when changing or reviewing any rendering, layout, keybinding, hint text, or colour in the sshm crate.
---

# sshm design rules

sshm is a ratatui SSH connection manager. The general craft behind these rules
lives in the `tui-design` skill; this skill is what is specific to *this* codebase,
and it exists because three real defects hid in it until the palette was made
inspectable — a 2.34:1 selection colour, a selection marker that never rendered,
and a footer whose escape-hatch hints were clipped off every 80-column terminal.

## Three rules the tests enforce

These fail CI, not style review. Know them before writing, not after.

**1. Name a role, never a colour.** Every colour comes from a role on
`Theme` in `sshm/src/theme.rs` (`t.bg`, `t.fg_muted`, `t.selection_bg`). A
literal `Color::Rgb(..)` in render code is a palette fork: unthemeable, unauditable,
and it duplicates a token that already exists. `no_color_literals_outside_the_theme_module`
greps for it. Need a colour with no role yet? Add the role first.

**2. Selection must be readable with no colour at all.** A glyph marker, not a hue.
The picker's `>` is a `Span` built in `build_picker_row_spans` — `List::highlight_symbol`
does *not* work there, because the picker renders statelessly. Any new selectable
surface needs its own visible marker plus a passing contrast pair.

**3. No visual sign-off from code alone.** Reading a diff tells you what was written,
not what renders. A change is verified when the detector is green, the snapshot diff
has been read, and a frame has been looked at. See `references/verification.md`.

## The three surfaces

They are three different products sharing a binary, and each has constraints the
others do not. Designing one as if it were another is the most common mistake here.

| Surface | Its hard constraint |
|---|---|
| Fullscreen TUI (`sshm`) | Owns the whole terminal. Must declare a minimum size and survive resize. |
| Inline Picker (`sshm pick`) | Draws *inside the user's live shell*. Never repaints the prompt, must leave the line clean on cancel, inherits the user's background. |
| Shell widget (`sshm init zsh\|bash`) | Not a UI at all — it is glue. Its failure mode is a broken shell, so it must fall through to normal completion when its trigger is absent. |

Full constraints per surface: `references/surfaces.md`.

## Making a UI change

1. Read the relevant surface in `references/surfaces.md` — the picker and the TUI
   disagree about backgrounds, focus and what "cancel" means.
2. Pick roles in `theme.rs`. If a new state needs emphasis, add the role and give it
   a contrast-checked pair before wiring it up.
3. Render it at the minimum supported size, 80×24, and at a large size.
4. Run the loop in `references/verification.md`. Every step, every time.
5. Hint text goes through `fit_hints`, ordered most-needed-first, so a narrow
   terminal keeps movement and the escape hatch rather than losing the tail.

## Vocabulary

Use the domain words from `CONTEXT.md`: a **Connection** is the unit (not "entry",
"host" or "server" — host is a *field* of one). The **Inline Picker** is not a
dropdown or popup. The **Shell Widget** is not a hook. Naming things the way the
glossary does keeps issues, test names and commits searchable.
