# Colour tokens

Single source of truth: `sshm/src/theme.rs`. This is the map of the roles the UI
draws with and what each resolves to.

There is **no *guaranteed* contrast table here — but there is a reference one.**
The palette is Clack-only as of the #35 cut-over: every token is a named ANSI
colour, `Reset`, or a neutral grey off the 256-colour ramp, and the frame owns
no background — it borrows the user's terminal. A WCAG ratio needs two RGB
triples and the frame only ever owns one of them, so no number written here can
promise anything about the reader's own theme.

What #35 concluded from that — that a contrast table *cannot exist* — is the
reasoning that let issue #42 ship. A guarantee is impossible; a **reference
check** is not. `every_informational_role_clears_the_reference_contrast_floor`
pins two real palettes (the reporter's One Dark, and Windows Terminal's stock
Campbell), resolves each role the way a terminal would, and fails with the
computed ratio. It cannot promise the reader's palette is fine. What it
promises is that nobody re-introduces a 2.67:1 body tier without a red test in
front of them.

## Roles

`Theme` has **seven roles and no background role at all** — not "a transparent
one": there is no `bg` field to set. `bg`, `fg_bright`, `selection_bg` and
`selection_fg` were deleted in #35 because no live render path read them, and
a frame that cannot measure the surface it borrows has nothing to spend a
background token on.

| Role | Clack | Drawn as |
|---|---|---|
| `fg` | `Gray` (ANSI 7) | Body text. **Wired, not inherited.** This role used to be a fiction: it was declared, documented and asserted-on while no render code read it — body spans were `Style::default().add_modifier(BOLD)` with no `.fg()` at all, emitting `39` and inheriting the terminal's ink. Every bold body span now carries `.fg(t.fg)` (`the_fg_role_is_read_by_render_code`). On the One Dark palette behind #42 that is `#ABB2BF` at **7.57:1**, where the inherited `#5C6370` was **2.67:1**. |
| `fg_muted` | `Indexed(245)` | Folder prefix, `user@host:port` meta, the empty / no-match state copy, the hint rail. `#8A8A8A` — **4.67:1** on One Dark, a real second tier instead of a copy of the body. **Carries no `DIM`**; see the note below the table. |
| `accent` | `Cyan` | The `◆` step icon and the `❯` cursor |
| `border` | `DarkGray` | The `│` rail and `└` corner — chrome, no state |
| `highlight` | `Green` | Fuzzy-matched characters |
| `success` | `Green` | Declared, not drawn: no live surface paints a positive signal yet |
| `warning` | `Yellow` | The `!` line a refused add step shows under the header (`! alias is required`). Reserved in #33 so a warning never invents a hue; first drawn by the #37 add sequence. |

What *is* enforceable about this table: no token is a fixed RGB and no token is
an indexed **hue** (`the_clack_palette_is_named_ansi_or_neutral_grey`,
`clack_tokens_are_named_ansi_or_neutral_grey`); the two hues are cyan for the
active step and green for a match
(`the_clack_palette_hues_are_cyan_and_green`); and every informational role
clears 4.5:1 against both reference palettes
(`every_informational_role_clears_the_reference_contrast_floor`). What is
*not* enforceable is how readable any of it is on the reader's own theme —
which is exactly why every state is carried by a glyph or a modifier rather
than by a hue.

### `DIM` is banned on informational text

`DIM` (SGR 2) used to be stacked on `fg_muted` throughout the frame, on the
theory that a colour alone "does not recede at all". Two facts killed it
(issue #42):

- **Windows Terminal ignores SGR 2 entirely**
  ([microsoft/terminal#6703](https://github.com/microsoft/terminal/issues/6703)).
  The attribute never rendered where it mattered.
- On the reporter's palette, `fg_muted` as `DarkGray` was **the same colour as
  the body text**, so neither spelling receded.

Recession now comes from the token. Emitting `DIM` claims an effect the target
terminal cannot deliver, so the tests assert its **absence**:
`the_row_meta_recedes_by_token_not_by_attribute`,
`the_hint_rail_is_muted_not_dim`, and the no-`2`-anywhere check in
`a_frame_serializes_to_ansi_carrying_its_palette`.

### Shared values

One pair still shares deliberately; one pair no longer does.

- **`fg_muted` and `border` used to both be `DarkGray`**, separated only by
  the `DIM` modifier. That invariant is **broken as of #42, on purpose** — see
  the `DIM` note above. `fg_muted` moved to `Indexed(245)`; `border` stayed
  `DarkGray`, because the rail is chrome that carries no information and a
  faint gutter under brighter text is the intended look.
- **`highlight` and `success` are both `Green`.** Clack's vocabulary is cyan
  for the active step and green for a good outcome; the frame draws
  `highlight` and never draws `success`. If a surface ever draws both at once
  and needs them told apart, give `success` its own value then — do not invent
  one now to satisfy the table.

## Adding a role

1. Add the field to `Theme` and set it in `Theme::clack()`. Never a fresh RGB
   literal — `no_color_literals_outside_the_theme_module` greps every render
   module for one.
2. Give it a named ANSI colour, `Reset`, or a neutral grey off the 24-step ramp
   (232..=255). An `Rgb(..)` token pins a colour the frame has no business
   choosing, and an indexed **hue** skips the terminal's own mapping for a
   colour it does not own; both are rejected by
   `the_clack_palette_is_named_ansi_or_neutral_grey`. The indexed exception
   exists because the 16-colour set has no neutral tone between `brightBlack`
   and `white` — without it there is nowhere to put a second tier (#42).
3. If the role carries a state, that state must *also* be carried by a glyph
   or a modifier: the frame has to say the same thing with colour turned off
   (`the_monochrome_frame_keeps_every_glyph_and_modifier_that_carries_state`).
   `BOLD` only — `DIM` is banned (see above).
4. If the role carries text the user must read, add it to
   `every_informational_role_clears_the_reference_contrast_floor`. It resolves
   the token against two pinned palettes and fails with the computed ratio.
   That is a *reference* check, not a guarantee: the frame still owns no
   background, so nothing here can promise the reader's own theme is fine.
   `border` is excluded on purpose — chrome that carries no information is
   allowed to be faint.

The rule is not negotiable; the colour is. When a token reads badly on some
terminal, move the token or give the state a glyph — do not add a hue that
only works on one theme.

## Reduced colour

`Theme::resolve(ColorSupport)` collapses the whole palette to `Reset` under
`NO_COLOR` or `TERM=dumb`. `Ansi256` and `Truecolor` pass through untouched.
`Ansi16` **snaps `fg_muted` to `DarkGray`** and leaves the body tier alone:
`Indexed(245)` lives in the 256-colour cube, a 16-colour terminal has no such
cell, and the emitted `38;5;245` would be dropped or mapped unpredictably
rather than degrading gracefully (`ansi16_snaps_the_muted_grey_it_cannot_render`,
`named_colours_survive_the_reduced_modes_unmapped`,
`truecolor_resolution_is_a_no_op`, `monochrome_suppresses_every_token`).

So the muted tier is a 256-colour luxury and the body tier is not. A purely
named-colour palette needed no snapping at all; buying back a real second tier
bought back a piece of the snapping job the deleted Nord `resolve` used to do.

**`NO_COLOR` is not a readability fallback.** Every role becomes `Reset`, which
is the terminal's own ink — and on the palette behind issue #42 that ink is
`#5C6370` at 2.67:1. `NO_COLOR` means "give me no colour", and that is what
it delivers, consequences included.

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
| `·` | `fg_muted` | Separates hint segments | Dropped with its segment, never stranded |
| `■` | `fg_muted`, with the alias and the answer in `BOLD` | **A question that has been answered** — the settled confirm step | `■ Delete [prod] web-01? Yes` / `? No`. `◆` asks, `■` has been answered: the glyph is the entire difference between a live confirm and a settled one, which is what keeps that difference readable with colour off. Both the yes and the no answer wear it — a declined delete leaves a trace too (story 22). |
| `◇` | `fg_muted`, with the value in `BOLD` | **What the last action did** — the dim note above the rows; and each settled step of the add sequence | `◇ deleted [prod] web-01`, `◇ added [prod] web-01`, `◇ edited [prod] web-01x`. In the add sequence every answered step settles to a `◇ <label>  <value>` line (`◇ Alias  web-01`); an optional field left empty settles to `◇ <label>  —` rather than a blank, because a blank after `◇ Key` reads as a step that lost its answer, not one that deliberately has none. Backing out of the first step leaves `◇ add abandoned — nothing saved`, and backing out of an edit leaves `◇ edit abandoned — nothing saved`. Also the note that contradicts the settled `■ Yes` when the store removed nothing (`◇ delete failed — no such Connection: …`), and the same refusal shape for an edit whose target vanished before the write (`◇ edit failed — no such Connection: …`). |
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

`■` and `◇` are `fg_muted` rather than `accent` because they are
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
