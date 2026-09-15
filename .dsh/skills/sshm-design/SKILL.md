---
name: sshm-design
description: sshm-specific UI design rules — semantic colour tokens in theme.rs, the one transparent inline frame and the three commands that open it, and the verification loop that proves a change is readable. Use when changing or reviewing any rendering, layout, keybinding, hint text, or colour in the sshm crate.
---

# sshm design rules

sshm is a ratatui SSH connection manager that draws **inline**, inside the
user's own shell — no alternate screen, since the #35 cut-over deleted the
fullscreen TUI. The general craft behind these rules lives in the
`tui-design` skill; this skill is what is specific to *this* codebase, and it
exists because three real defects hid in it until the palette was made
inspectable — a 2.34:1 selection colour, a selection marker that never
rendered, and a footer whose escape-hatch hints were clipped off every
80-column terminal.

## Three rules the tests enforce

These fail CI, not style review. Know them before writing, not after.

**1. Name a role, never a colour.** Every colour comes from a role on `Theme`
in `sshm/src/theme.rs` (`t.fg_muted`, `t.border`, `t.accent`). A literal
`Color::Rgb(..)` in render code is a palette fork: unthemeable, unauditable,
and it duplicates a token that already exists.
`no_color_literals_outside_the_theme_module` greps for it. Need a colour with
no role yet? Add the role first — and make it a named ANSI colour or `Reset`,
never a fixed RGB (`the_clack_palette_is_named_ansi_only`).

**2. Selection must be readable with no colour at all.** A glyph marker, not a
hue. The frame's `❯` is built in `cursor_span` — bold, in the accent role,
with a blank of the same width on unselected rows — and the selected alias is
bold. There is no filled row anywhere, because the frame owns **no background
role at all**: it borrows the user's terminal and cannot measure it, so a fill
is not available and a hue alone would vanish under `NO_COLOR`. Any new
selectable surface needs its own visible glyph, not a contrast pair against a
background it does not own.

**3. No visual sign-off from code alone.** Reading a diff tells you what was
written, not what renders. A change is verified when the detector is green,
the dumped frame has been read, and a frame has been looked at in a real
terminal. See `references/verification.md`.

## The surfaces

There is one drawing surface and three commands that open it, plus the shell
glue. Designing one as if it were another is the most common mistake here.

| Surface | Its hard constraint |
|---|---|
| The inline frame (`frame.rs` + `inline.rs`) | Owns no background. Borrows the user's terminal, must leave the line clean on cancel, and every state must survive `NO_COLOR`. |
| The three commands (`sshm` / `sshm pick` / `sshm manage`) | One frame, one difference: what Enter means (the `--emit` axis). Same keys, same glyphs, three different outcomes. |
| The shell widget (`sshm init zsh\|bash`) | Not a UI at all — it is glue. Its failure mode is a broken shell, so it must fall through to normal completion when its trigger is absent. |

Full constraints per surface: `references/surfaces.md`.

## Making a UI change

1. Read the relevant surface in `references/surfaces.md` — the frame and the
   shell widget disagree about everything except that neither may damage the
   user's command line.
2. Pick roles in `theme.rs`. A role is a named ANSI colour or `Reset`; there
   is no contrast table to check a pair against, because the frame has no
   background of its own. If the role carries a state, that state must also
   be carried by a glyph or a modifier. See `references/tokens.md`.
3. Render it at 80×24 and at a large size, in truecolour **and** under
   `NO_COLOR`.
4. Run the loop in `references/verification.md`. Every step, every time.
5. Hint text goes through `fit_hints_with`, ordered most-needed-first, so a
   narrow terminal keeps movement and the escape hatch rather than losing the
   tail. Never hint a key the frame does not read.

## Vocabulary

Use the domain words from `CONTEXT.md`: a **Connection** is the unit (not
"entry", "host" or "server" — host is a *field* of one). The **Inline Picker**
is not a dropdown or popup. The **Shell Widget** is not a hook. Naming things
the way the glossary does keeps issues, test names and commits searchable.
