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
