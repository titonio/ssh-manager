# Colour tokens

Single source of truth: `sshm/src/theme.rs`. This is the map of the roles the UI
draws with and what each resolves to.

There is **no contrast table here, and there cannot be one.** The palette is
Clack-only as of the #35 cut-over: every token is a named ANSI colour or
`Reset`, and the frame owns no background — it borrows the user's terminal,
which it cannot measure. A WCAG ratio needs two RGB triples; this palette has
none, and the terminal maps every named colour to whatever the user's theme
says. The table of "measured WCAG pairs" that used to open this file described
the Nord palette that was deleted with the fullscreen TUI, and the `contrast`
helper that computed those numbers is gone with it. What replaced the ratio
check is the structural rules below, each gated by a test.

## Roles

`Theme` has **seven roles and no background role at all** — not "a transparent
one": there is no `bg` field to set. `bg`, `fg_bright`, `selection_bg` and
`selection_fg` were deleted in #35 because no live render path read them, and
a frame that cannot measure the surface it borrows has nothing to spend a
background token on.

| Role | Clack | Drawn as |
|---|---|---|
| `fg` | `Reset` | The terminal's own foreground. Body text is built with `Span::raw`, which emits the same `39`; the role exists so body text is still named by role, never by literal. |
| `fg_muted` | `DarkGray` | Folder prefix, `user@host:port` meta, the empty / no-match state copy, the hint rail. The meta and the hint rail add `DIM`; the state copy does not. |
| `accent` | `Cyan` | The `◆` step icon and the `❯` cursor |
| `border` | `DarkGray` | The `│` rail and `└` corner — chrome, no state |
| `highlight` | `Green` | Fuzzy-matched characters |
| `success` | `Green` | Declared, not drawn: no live surface paints a positive signal yet |
| `warning` | `Yellow` | Declared, not drawn: reserved so a warning never invents a hue |

What *is* enforceable about this table: no token is a fixed RGB
(`the_clack_palette_is_named_ansi_only`, `clack_tokens_are_named_ansi_or_reset`),
and the two hues are cyan for the active step and green for a match
(`the_clack_palette_hues_are_cyan_and_green`). What is *not* enforceable is
how readable any of it is — that depends on the terminal theme the frame is
borrowing. Which is exactly why every state is carried by a glyph or a
modifier rather than by a hue.

Two role pairs share a value on purpose, and both are separated by *modifier*
rather than by hue:

- **`fg_muted` and `border` are both `DarkGray`.** In Clack the rail and the
  meta are meant to recede together. What tells them apart is that meta
  carries `DIM` and the rail does not. There is no second mid-tone in the
  16-colour palette that survives both a black and a white terminal — `Gray`
  (#C0C0C0) is 1.2:1 on white — so a hue split would break one of the two
  backgrounds the frame has to live on.
- **`highlight` and `success` are both `Green`.** Clack's vocabulary is cyan
  for the active step and green for a good outcome; the frame draws
  `highlight` and never draws `success`. If a surface ever draws both at once
  and needs them told apart, give `success` its own value then — do not invent
  one now to satisfy the table.

## Adding a role

1. Add the field to `Theme` and set it in `Theme::clack()`. Never a fresh RGB
   literal — `no_color_literals_outside_the_theme_module` greps every render
   module for one.
2. Give it a named ANSI colour or `Reset`. A `Rgb(..)` or `Indexed(..)` token
   pins a colour the frame has no business choosing and skips the terminal's
   own mapping (`the_clack_palette_is_named_ansi_only`).
3. If the role carries a state, that state must *also* be carried by a glyph
   or a modifier: the frame has to say the same thing with colour turned off
   (`the_monochrome_frame_keeps_every_glyph_and_modifier_that_carries_state`).
4. There is no contrast test to add a pair to. `every_text_pair_meets_wcag_aa`
   and `every_color_mode_stays_readable` measured the Nord palette against a
   painted background; both were deleted in #35 rather than left asserting
   against a palette that no longer exists. A new role that wants a contrast
   guarantee has to name the background it is measured against — and the
   inline frame does not own one.

The rule is not negotiable; the colour is. When a token reads badly on some
terminal, move the token or give the state a glyph — do not add a hue that
only works on one theme.

## Reduced colour

`Theme::resolve(ColorSupport)` collapses the whole palette to `Reset` under
`NO_COLOR` or `TERM=dumb`. In every other mode the palette passes through
untouched, because named ANSI colours are valid in 256- and 16-colour
terminals alike and the terminal maps them to the user's own values
(`named_colours_survive_the_reduced_modes_unmapped`,
`truecolor_resolution_is_a_no_op`, `monochrome_suppresses_every_token`).
A fixed-RGB palette would need snapping here; that is what the deleted Nord
`resolve` did, and why a named-colour palette is what a transparent frame
gets to be.

The transparent frame routes through the same call: `build_frame` takes a
`Canvas { width, height, support }` and draws with
`Theme::clack().resolve(canvas.support)`. Under `NO_COLOR` or `TERM=dumb`
every role becomes `Reset` before a span is built, so the frame emits no hue
rather than emitting cyan and hoping the terminal ignores it. Colour support
is an *input* to that seam for exactly this reason — a caller must be able to
change it without reshaping `build_frame`.

What gates transparency now, in place of the deleted ratio table:

- `no_span_in_any_frame_sets_a_background` — across every frame state, mode
  and query, no span carries `style.bg`.
- `the_serialized_frame_asks_for_no_background_colour` — read off the bytes:
  no SGR parameter in the serialized frame is a background code (`40..47` or
  `48;…`). An unset colour emits `49`, the terminal's default, so the frame
  stays transparent all the way down to what it writes.

## Clack grammar glyphs

The inline frame's glyphs, their role, and what state each carries. These are
load-bearing: a glyph that stops rendering is a lost state, not a cosmetic
regression.

| Glyph | Role | Carries | Notes |
|---|---|---|---|
| `◆` | `accent` | The step of the flow | Opens every frame: `◆ <question>`; also opens both settle traces (`◆ picked …`, `◆ cancelled`) |
| `│` | `border` | The rail every body line hangs off | Chrome; never carries state |
| `❯` | `accent` + `BOLD` | **Selection** | Blank (same width) on unselected rows |
| `└` | `border` | Closes the rail | Chrome; the frame's last line |
| `·` | `fg_muted` + `DIM` | Separates hint segments | Dropped with its segment, never stranded |

`◆` is `accent` **wherever it appears**, including the cancel trace. That is
not decoration: the cancel icon *is* state ("this step was abandoned"), and
`border` is defined as structural chrome that carries none. A cancel that
wore `border` would be a state-carrying glyph drawn in a stateless role —
the exact drift the role table exists to catch. What distinguishes the two
traces is the word after the icon, not a third colour; the Clack palette
has no "abandoned" hue and inventing one is not on the table.

## Selection is one marker

Selection on this surface is the `❯` glyph plus a bold alias — never a filled
row, because a transparent frame has no background to fill and a hue alone
vanishes under `NO_COLOR`.

`❯` is the **only** selection marker in the binary. The old opaque picker's
`> ` marker went with that picker in the #35 cut-over, and so did the
separate gate that held it in place. This file used to say the two were
"different markers for the same state" and that `> ` "stays" — it does not,
and there is no longer a rule about it beyond the one that matters: do not
add a second marker. A surface that shows selection shows it with a glyph in
this grammar, and that glyph is `❯`.
