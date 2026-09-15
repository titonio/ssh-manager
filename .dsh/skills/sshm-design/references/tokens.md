# Colour tokens

Single source of truth: `sshm/src/theme.rs`. This is the map, with the measured
contrast of every pair the UI draws. Numbers are WCAG 2.1 ratios computed over the
actual RGB triples.

## Roles

| Role | Nord | Hex | Drawn as |
|---|---|---|---|
| `bg` | NORD0 | `#2E3440` | Every panel's background |
| `fg` | NORD4 | `#D8DEE9` | Body text: rows, form values, popup copy |
| `fg_bright` | NORD6 | `#ECEFF4` | Highest emphasis: block titles |
| `fg_muted` | NORD8 | `#88C0D0` | Placeholders, empty-state and no-match text |
| `accent` | NORD9 | `#81A1C1` | Interactive frames: popups, the picker's border |
| `border` | NORD3 | `#4C566A` | Structural chrome only — carries no state |
| `highlight` | NORD13 | `#EBCB8B` | Fuzzy-match characters, the active row |
| `success` | NORD14 | `#A3BE8C` | Footer key hints, update-available border |
| `warning` | NORD12 | `#D08770` | The dismissable message popup border |
| `selection_bg` | NORD9 | `#81A1C1` | Selected row background |
| `selection_fg` | NORD0 | `#2E3440` | Selected row foreground |

## Measured pairs

| Pair | Ratio | Grade | Note |
|---|---|---|---|
| `fg_bright` on `bg` | 10.84:1 | AAA | Titles |
| `fg` on `bg` | 9.25:1 | AAA | Body text |
| `bg` on `highlight` | 8.00:1 | AAA | Matched char inside a selected row |
| `highlight` on `bg` | 8.00:1 | AAA | Matched char, unselected |
| `fg_muted` on `bg` | 6.24:1 | AA | Muted text still clears AA — that is the floor for "de-emphasised" |
| `success` on `bg` | 6.13:1 | AA | Footer hints |
| `selection_fg` on `selection_bg` | 4.64:1 | AA | Was 2.34:1 before the fix |
| `accent` on `bg` | 4.64:1 | AA | Popup borders |
| `warning` on `bg` | 4.39:1 | 3:1 only | **Border only.** Fails 4.5:1 as text — do not draw `warning` as body copy |
| `border` on `bg` | 1.69:1 | decorative | Intentionally subtle; exempt from 1.4.11 because it carries no state |

## Adding a role

1. Add the field to `Theme` and set it in `Theme::nord()` from a `nord::` constant.
   Never a fresh RGB literal.
2. Add its drawn pairs to `every_text_pair_meets_wcag_aa` in
   `tests/design_system_test.rs` and confirm they clear 4.5:1 (3:1 for a
   non-text boundary).
3. If the role is for text, check it against `bg` *and* any background it can
   land on — the inline picker inherits the user's terminal background, so a role
   used there needs its own opaque background.

The threshold is not negotiable; the token is. When a pair fails, move the token,
not the bar.

## Reduced colour

`Theme::resolve(ColorSupport)` downgrades the whole palette for 256-colour,
16-colour and `NO_COLOR` terminals. `every_color_mode_stays_readable` asserts that
in every mode the background stays dark, body text stays readable, and the
selection stays distinct from the background. A new role must hold that line too —
a role that collapses into `bg` after downgrade makes selection invisible.

The transparent inline frame routes through the same call: `build_frame` takes a
`Canvas { width, support }` and draws with `Theme::clack().resolve(support)`.
Under `NO_COLOR` or `TERM=dumb` every role becomes `Reset` before a span is
built, so the frame emits no hue rather than emitting cyan and hoping the
terminal ignores it. Colour support is an *input* to that seam for exactly this
reason — a caller must be able to change it without reshaping `build_frame`.

## The Clack palette (transparent inline frame)

`Theme::clack()` is built differently from Nord and deliberately has no RGB at
all. A transparent frame borrows a background it cannot measure, so it owns only
two hues and lets the terminal supply the rest:

| Role | Clack | Drawn as |
|---|---|---|
| `bg`, `fg`, `fg_bright`, `selection_bg`, `selection_fg` | `Reset` | The terminal's own colours — the frame never paints a background |
| `fg_muted` | `DarkGray` + `DIM` | Folder prefix, `user@host:port` meta, hint rail |
| `border` | `DarkGray` | The `│` rail and `└` corner — chrome, no state |
| `accent` | `Cyan` | The `◆` step icon and the `❯` cursor |
| `highlight` | `Green` | Fuzzy-matched characters |
| `warning` | `Yellow` | Reserved; not drawn by the frame |

Two role pairs share a value on purpose, and both are separated by *modifier*
rather than by hue:

- **`fg_muted` and `border` are both `DarkGray`.** In Clack the rail and the meta
  are meant to recede together. What tells them apart is that meta carries `DIM`
  and the rail does not. There is no second mid-tone in the 16-colour palette
  that survives both a black and a white terminal — `Gray` (#C0C0C0) is 1.2:1 on
  white — so a hue split would break one of the two backgrounds the frame has to
  live on.
- **`highlight` and `success` are both `Green`.** Clack's vocabulary is cyan for
  the active step and green for a good outcome; the frame draws `highlight` and
  never draws `success`. If a surface ever draws both at once and needs them
  told apart, give `success` its own value then — do not invent one now to satisfy
  the table.

The rule that keeps this honest is the rendered one, not the table: under
`NO_COLOR` the frame must still show every state. `the_monochrome_frame_keeps_every_glyph_and_modifier_that_carries_state`
and `a_monochrome_canvas_emits_no_colour_at_all` enforce it.

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

Selection on this surface is the `❯` glyph plus a bold alias — never a filled
row, because a transparent frame has no background to fill and a hue alone
vanishes under `NO_COLOR`.

**The inline picker's `> ` marker is a different marker for the same state, and
stays.** It is prepended in `build_picker_row_spans` on the current opaque
picker. The two surfaces are not yet the same surface; #35's cut-over is what
reconciles them. Do not "unify" the markers before then — changing `> ` breaks
the picker's selection signalling, which is a separately-gated rule.
