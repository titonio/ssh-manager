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
| `warning` | `Yellow` | The `!` line a refused add step shows under the header (`! alias is required`). Reserved in #33 so a warning never invents a hue; first drawn by the #37 add sequence. |

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
| `◆` | `accent` | The step of the flow | Opens every frame: `◆ <question>`; also opens the settle traces (`◆ picked …`, `◆ cancelled`, `◆ error …`). The two chords that change a Connection wear the same header shape — `◆ Delete [prod] web-01? (y/N)` and `◆ Edit [prod] web-01` — so they read as the same kind of thing before either writes. |
| `│` | `border` | The rail every body line hangs off | Chrome; never carries state |
| `❯` | `accent` + `BOLD` | **Selection** | Blank (same width) on unselected rows |
| `└` | `border` | Closes the rail | Chrome; the frame's last line |
| `·` | `fg_muted` + `DIM` | Separates hint segments | Dropped with its segment, never stranded |
| `■` | `fg_muted` + `DIM`, with the alias and the answer in `BOLD` | **A question that has been answered** — the settled confirm step | `■ Delete [prod] web-01? Yes` / `? No`. `◆` asks, `■` has been answered: the glyph is the entire difference between a live confirm and a settled one, which is what keeps that difference readable with colour off. Both the yes and the no answer wear it — a declined delete leaves a trace too (story 22). |
| `◇` | `fg_muted` + `DIM`, with the value in `BOLD` | **What the last action did** — the dim note above the rows; and each settled step of the add sequence | `◇ deleted [prod] web-01`, `◇ added [prod] web-01`, `◇ edited [prod] web-01x`. In the add sequence every answered step settles to a `◇ <label>  <value>` line (`◇ Alias  web-01`); an optional field left empty settles to `◇ <label>  —` rather than a blank, because a blank after `◇ Key` reads as a step that lost its answer, not one that deliberately has none. Backing out of the first step leaves `◇ add abandoned — nothing saved`, and backing out of an edit leaves `◇ edit abandoned — nothing saved`. Also the note that contradicts the settled `■ Yes` when the store removed nothing (`◇ delete failed — no such Connection: …`), and the same refusal shape for an edit whose target vanished before the write (`◇ edit failed — no such Connection: …`). |
| `!` | `warning` (the glyph `BOLD`) | **A refused answer** — the step stayed put and says why | `! alias is required`, `! port must be a number from 1 to 65535`. The state is carried by the glyph and the sentence, not the yellow, so it reads under `NO_COLOR`; the yellow is the `warning` role doing its job, never a literal. |
| `_` | `fg_muted` | **The live text field** — where the keystrokes are going | Drawn by the frame because the hardware cursor is hidden for the frame's whole life; a field with no cursor and no echo of its own is a field the user cannot see themselves filling. ASCII on purpose: every font that renders the box-drawing renders `_`, and a caret that turns into tofu is worse than no caret. In the add sequence the live field rides the header (`◆ Alias  web-01_`); in the edit step it rides its own line under the target header (`◆ Alias   web-01_`), with the field's `◆` dim like the settled `◇` lines — the label recedes, the value is bold, the caret marks the live end. |

### The ask / answered / noted grammar

Three glyphs carry the manage frame's whole sense of time:

- `◆` — **asking now**. The header is the question.
- `■` — **answered**. The question is no longer live; the answer is on the record.
- `◇` — **what happened as a result**. The note is a claim about the world, not
  about the keystroke.

The two are deliberately different glyphs because they are deliberately
different *claims*, and the frame must be able to make one without the other.
`■ Delete [prod] web-01? Yes` followed by
`◇ delete failed — no such Connection: [prod] web-01` is a frame that
records an answer the user gave and refuses to record an outcome the store
never produced. Collapsing both onto one glyph is what let a delete that did
not happen read as one that did.

**The honesty rule for `◇`: the words `deleted` and `added` appear on a
frame only when the store really changed the file.** `Store::remove`
returning `Ok(None)` means nothing was deleted and nothing was written, and
the note for that is `delete failed — no such Connection`, worded so it
cannot be scanned as `deleted`. The gate for this is
`a_delete_that_removed_nothing_never_renders_as_deleted` in
`manage_frame_test.rs`; the seam that enforces it is `manage::settle_delete`,
which is the only place a `Trace::Deleted` may be created.

The add half is held to the same discipline by the same shape: the sequence
finishing is what the user *did*, and `◇ added` is a claim about
`connections.json`. `manage::settle_add` is the only place a
`Trace::Added` may be created, and it creates one only from a
`Store::add` that returned the Connection it actually wrote. A refused add
collapses the frame to `◆ error …` instead — the same contract a refused
delete has. An abandoned sequence goes the other way: it leaves
`◇ add abandoned — nothing saved`, worded to answer the one question the
user has after backing out of a half-filled form. The gates are
`an_add_the_store_wrote_earns_the_added_note`,
`an_add_the_store_refused_does_not_claim_a_connection_was_added` and
`nothing_claims_an_add_before_the_store_says_so`.

The edit half holds the line the same way: `◇ edited` is a claim about
`connections.json`, and `manage::settle_edit` creates a `Trace::Edited`
only from a `Store::update` that returned the Connection it actually wrote.
An update that found no such Connection settles to
`◇ edit failed — no such Connection: …` — the word `edited` never appears
on a frame where the write did not happen — and an Esc from the editor
settles to `◇ edit abandoned — nothing saved`, because the field was
half-typed and none of it was written. The target is captured by value at
the `Ctrl+E` chord, so arrowing through the five fields afterwards cannot
move the write onto a neighbour: the header and the write agree because
they read the same captured Connection, not because the cursor says so.
The gates are `settle_edit_grants_the_note_only_on_a_real_write`,
`the_edit_outcome_classifies_the_store_answer` and
`the_edit_target_is_captured_at_the_chord_and_never_moves` in
`manage_test.rs`, with the frame-side mirror in `manage_frame_test.rs`
(`an_edit_that_landed_on_nothing_says_so`,
`an_abandoned_edit_says_nothing_was_saved`).

`■` and `◇` are `fg_muted` + `DIM` rather than `accent` because they are
history, not the live step: the frame's accent marks *where you are*, and a
settled line that kept shouting in cyan would compete with the header for the
one thing the accent role means. The state they carry is in the glyph and the
words, so it survives `NO_COLOR` intact
(`the_flow_glyphs_survive_monochrome`).

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
