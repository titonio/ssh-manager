# inquire capability: filter-as-you-type list with custom styled rows

Research for #30. **Verified 2026-09-13** against inquire **0.9.4** source
(crates.io `.crate`, read directly — not README prose). Line references are to
that source. Comparison baseline: `docs/research/cliclack-capability.md` (#27).

## One-paragraph answer to the critical question

**`inquire` does not hit the wall that killed cliclack — but it hits a different,
narrower one.** The cliclack failure was four things at once: the candidate set
captured behind a private `Rc<RefCell<…>>` at `interact()`, a hardcoded
Jaro-Winkler matcher with no injection seam, no match positions, and ANSI-styled
labels poisoning its own filter. `inquire` breaks all four. Its `Prompt<Backend>`
trait is **public and unsealed** (`prompts/prompt.rs:44–167`), its
`handle(&mut self, action)` hook hands the implementor **mutable access to their
own state on every keystroke** (`prompt.rs:105`), and its `prompt()` loop is a
*provided* method explicitly documented as overridable when warranted
(`prompt.rs:114–118`) — so a custom prompt holding a `Vec<Connection>` can
re-supply, reload or re-sort that universe between keystrokes, which sidesteps
the cliclack limitation entirely rather than working around it. The matcher is a
first-class injectable `Scorer<'a, T> = &dyn Fn(&str, &T, &str, usize) ->
Option<i64>` (`type_aliases.rs:74`) that receives the **typed option**, so it can
match on struct fields and ignore the display string — and the frame renderer
parses output with its own ANSI state machine (`ansi.rs:20–98`), treating escape
sequences as **zero-width** (`frame_renderer.rs:67–79`), so ANSI-styled rows no
longer corrupt layout. Matched-character highlighting is **not built in** but is
reachable without forking (own matcher → `Row{matches}` → `Display` emits SGR →
`render_options<D: Display>`), and the settle-trace collapse is **built in and
better than cliclack's** — `finish_current_frame` diffs frames row-by-row and
`clear_line()`s rows that vanish (`frame_renderer.rs:304–306`), so the live list
automatically collapses to the `prompt + answer` summary line. **The narrower
wall is styling expressiveness, not state:** there is no `StyledStr`, `Styled<T>`
carries exactly one `StyleSheet` per content unit (`style.rs:132–141`),
`Attributes` is BOLD|ITALIC with **no DIM** (`style.rs:25–31`), and `Backend`
exposes no free-form draw primitive (only `pub fn new`, `backend.rs:94`) — so
per-segment colour must be smuggled as inline ANSI through a `Display` impl, and
the Clack rail geometry has to be squeezed into `RenderConfig`'s prefix slots.
**Strategy B is genuinely back on the table** — as "write a custom `Prompt` impl
on inquire's frame plumbing", not as "use `Select` out of the box".

## 0. Canonical repo resolution — VERIFIED

Resolved from the crates.io `repository` field, not from any prose source:

```
GET https://crates.io/api/v1/crates/inquire
  repository: "https://github.com/mikaelmello/inquire"
  homepage:   "https://github.com/mikaelmello/inquire"
```

**Canonical repo: <https://github.com/mikaelmello/inquire>** (author Mikael
Mello, `authors = ["Mikael Mello <git@mikaelmello.com>"]` in `Cargo.toml.orig`).
The GitHub URLs floated in the #30 ticket body (`mikaylag/…`, `mikaym/inquire`)
**do not exist** and are disregarded. Corroborated by the shipped
`.cargo_vcs_info.json` (`sha1 3d5b6542…`, `path_in_vcs: "inquire"`), i.e. the
crate is published from a monorepo subdirectory of that repo.

## 1. Version, license, maintenance — VERIFIED

- **Latest version 0.9.4**, published **2026-02-24** (crates.io API).
  Checksum of the audited `.crate`:
  `sha256 6654738b8024300cf062d04a1c13c10c8e2cea598ec1c47dc9b6641159429756`.
- **License MIT** (crates.io `license: "MIT"`, `Cargo.toml.orig`
  `license = "MIT"`, GitHub license field `MIT License` — all three agree).
- **MSRV 1.80.0** (`Cargo.toml.orig` `rust-version = "1.80.0"`).
- **Downloads:** 20,354,436 total / **5,194,510 recent** (crates.io,
  2026-09-13). The #29 "5.2M recent" figure checks out.
- **GitHub:** 2,625 stars, 113 forks, 90 open issues, not archived,
  `pushed_at` **2026-03-02** (<https://github.com/mikaelmello/inquire>).
- **Release cadence** (GitHub releases): 0.9.2 2026-01-17 → 0.9.3 2026-02-06 →
  0.9.4 2026-02-24 (three releases in five weeks), before that 0.8.0/0.9.0/0.9.1
  all 2025-09-14..16, and a **17-month gap** from 0.7.5 (2024-04-23) to 0.8.0.
  Pattern: revived and active 2024–early 2026, but with a history of long
  dormancy. As of the 2026-09-13 verification date the last push was ~6 months
  prior — quieter than the download count suggests.
- **Bus factor: effectively 1.** Commit distribution: `mikaelmello` **687**,
  next contributor `hampuslidin` 22, `dnaka91` 15 (~91% single-author).
  7 watchers. This is a single-maintainer project carrying 5.2M recent downloads.
- **Issue response: present but shallow.** Recent issues are being seen and
  answered (all sampled have 1–3 comments): #216 (2026-09-09, 2 comments),
  #348 (2026-09-01, 2), #252 (2026-08-29, 3), #290 (updated 2026-07-27).
  Triage happens; fixes lag.
- **Directly relevant open issue — #290 "Allow formatting options in `Select`"**
  (<https://github.com/mikaelmello/inquire/issues/290>, open since 2025-02-26,
  updated 2026-07-27, +1s). It independently confirms the §4 finding:
  *"the existing formatter api only format the option after select, this is not
  necessary in my opinion… format the options listed, is very important."*
  The `OptionFormatter` attached via `with_formatter` (`select/mod.rs:267`) is
  wired only into `format_answer` (`select/prompt.rs:178–180`), **not** into the
  rendered list. Per-row display formatting in the built-in `Select` is an
  acknowledged missing feature.
- Also open and relevant: **#344 "Tab characters are measured as zero-width,
  corrupting redraws"** (2026-07-20, 0 comments) — a live defect in the
  width-accounting path that §4 relies on. Do not put literal tabs in rows.

## 2. Extensibility surface: the `Prompt` trait — VERIFIED

`inquire` is built on a **public, unsealed `Prompt<Backend>` trait**
(`src/prompts/prompt.rs:44–167`). It is not sealed — there is no private
supertrait, no `pub(crate)` marker, and the crate's own prompts (`Select`,
`MultiSelect`, `Text`, …) implement it from outside the trait's module exactly as
a downstream crate would (`SelectPrompt` impls `Prompt<Backend>` at
`prompts/select/prompt.rs:161`). **A caller can implement a fully custom prompt
without forking.**

Required surface (`prompt.rs:44–112`):

```rust
pub trait Prompt<Backend>
where Backend: CommonBackend, Self: Sized {
    type Config;
    type InnerAction: InnerAction<Config = Self::Config>;
    type Output;

    fn message(&self) -> &str;
    fn config(&self) -> &Self::Config;
    fn format_answer(&self, answer: &Self::Output) -> String;
    fn setup(&mut self) -> InquireResult<()> { Ok(()) }          // :74
    fn pre_cancel(&mut self) -> InquireResult<bool> { Ok(true) } // :81
    fn submit(&mut self) -> InquireResult<Option<Self::Output>>;
    fn handle(&mut self, action: Self::InnerAction) -> InquireResult<ActionResult>;
    fn render(&self, backend: &mut Backend) -> InquireResult<()>;

    fn prompt(mut self, backend: &mut Backend) -> InquireResult<Self::Output> { … } // :118
}
```

Control handed over:

- **Key handling is ours.** `type InnerAction` is our own enum; we implement
  `InnerAction::from_key(key, config) -> Option<Self>`
  (`prompts/action.rs:53–66`), so we decide which keys mean what. `Action::from_key`
  (`action.rs:34–46`) is **public** and reserves only Enter/`^J` (Submit),
  Esc/`^G`/`^D` (Cancel), `^C` (Interrupt); everything else falls through to our
  `InnerAction::from_key`. We can also bypass `Action::from_key` entirely.
- **Business logic is ours.** `handle(&mut self, action)` gets `&mut self` on
  every keystroke — this is where a candidate set gets mutated (§3).
- **The render loop is ours, and so is the whole loop if needed.** `prompt()` is a
  **provided default method** documented as "should not be reimplemented … unless
  the situation really warrants it" (`prompt.rs:114–117`) — i.e. overriding the
  top-level loop is explicitly contemplated, not prohibited.
- **`render(&self, backend)` is ours**, but see the caveat below — the *backend's*
  public drawing vocabulary is narrow.

**Caveat — the render vocabulary is trait-methods-only.** `Backend`'s inherent
impl exposes exactly one public method, `pub fn new` (`ui/backend.rs:94`); every
other inherent method is private (`fn print_option_value`, `fn print_input`, …)
and `frame_renderer` is a private field (`backend.rs:83`). So a custom `Prompt`
can only draw through the backend **trait** methods:

| Trait | Methods | Ref |
|---|---|---|
| `CommonBackend` | `frame_setup`, `frame_finish(is_last_frame)`, `render_canceled_prompt`, `render_prompt_with_answer`, `render_error_message`, `render_help_message` | `backend.rs:17–26` |
| `SelectBackend` | `render_select_prompt(prompt, Option<&Input>)`, `render_options<D: Display>(page)` | `backend.rs:43–46` |
| `TextBackend` | `render_prompt`, `render_suggestions<D: Display>` | `backend.rs:28–36` |

`render_options<D: Display>` is the load-bearing one: the row type is generic over
`Display`, so **row content is whatever our `Display` impl emits** — including
ANSI (see §4). But there is no public "draw this arbitrary styled cell at this
position" primitive. A custom prompt composes the existing frame pieces rather
than painting a free-form frame.

**Autocomplete traits.** There is no `Autocomplete` trait. The equivalents are
function aliases (`src/type_aliases.rs`): `Scorer<'a, T> = &dyn Fn(&str, &T, &str, usize) -> Option<i64>`
(:74), `Sorter<'a> = &dyn Fn(&mut [(usize, i64)])` (:81),
`Suggester<'a>` (:86) and `Completer<'a>` (:91). The `Scorer` is **not
prefix-only** — it is a fully user-supplied ranking function, and the default is
`fuzzy_matcher`'s `SkimMatcherV2` (`type_aliases.rs:54–58`, feature `fuzzy`).
This is the single biggest structural difference from cliclack, whose matcher was
hardcoded.

## 3. The candidate-set question (the one that killed cliclack) — VERIFIED

Two different answers depending on which path you take, and the distinction is the
whole ballgame.

**Built-in `Select`: universe fixed at start, visible set recomputed per keystroke.**
`Select::new(message, options: Vec<T>)` (`prompts/select/mod.rs:212`) takes the
whole universe up front. `SelectPrompt` holds `options: Vec<T>`,
`string_options: Vec<String>` and `scored_options: Vec<usize>`
(`select/prompt.rs:17–29`). On **every** content-changing keystroke,
`handle()` calls `run_scorer()` (`select/prompt.rs:205–216`), which re-scores
the universe through our `Scorer`, sorts via our `Sorter`, and swaps
`self.scored_options` (`prompt.rs:127–158`). So the *displayed* list genuinely
re-filters live. But `options` itself is never re-supplied — `Select`'s builder
consumes the `Vec` by value and `prompt_with_backend` takes `self` by value
(`mod.rs:375`), so **the universe cannot be changed mid-interaction through the
`Select` API**. In that narrow sense the cliclack wall still stands.

**Custom `Prompt` impl: the wall is gone.** `handle(&mut self, action)` receives
`&mut self` on every keystroke (`prompt.rs:105`), and the impl's own struct is
ours. A `SshPicker` holding `connections: Vec<Connection>` can reassign, reload,
or lazily extend that vector between keystrokes — nothing in the trait or the
default loop assumes the candidate universe is immutable. **This does sidestep the
cliclack limitation entirely**: cliclack's equivalent hook was unreachable because
its `RadioButton` set was captured into a private `Rc<RefCell<…>>` at
`interact()` (`cliclack-capability.md` §3), whereas inquire's loop hands mutable
access to *our* state and never touches how we store candidates.

**The matcher is injectable, which is the deeper fix.** cliclack's `Select`
hardcoded `Suggest for Vec<…>` (`select.rs:123` there) and offered no seam. Here
`with_scorer(Scorer<'a, T>)` (`select/mod.rs:255`) and `with_sorter` (:261) are
first-class builder methods, and the scorer signature
`Fn(&str /*input*/, &T /*typed option*/, &str /*string_value*/, usize /*idx*/) -> Option<i64>`
hands the closure the **typed option, not just its string**. A scorer can match on
`conn.alias`/`conn.host` fields directly and ignore `string_value` altogether —
which also neutralizes the "ANSI in the matched string" failure mode (§4).
`with_starting_filter_input` (:282) and `without_filtering` (:301) round it out.

**What is NOT available:** no async / no callback-driven candidate source, no
streaming. `Scorer` is a synchronous `&dyn Fn` re-run over the whole universe on
each keystroke — O(n) per keypress with our own scoring cost. For a few thousand
SSH connections that is fine (**inference**, no benchmarks in repo); it is not a
lazy "fetch more when the user types" design.

## 4. Per-row styling: `StyledStr` and the text projection — VERIFIED

**There is no `StyledStr`.** The styling types are `StyleSheet { fg, bg, att }`
and `Styled<T> { content: T, style: StyleSheet }` (`ui/api/style.rs:52–59,
132–141`). One `StyleSheet` per `Styled<T>` unit — so **the built-in styling
model is one style per content unit, not per segment**. Worse for our spec,
`Attributes` is a two-bit flag set: `BOLD` and `ITALIC` only
(`style.rs:25–31`). **There is no `DIM`/faint attribute in the public style
API.** A "folder dim" segment cannot be expressed through `StyleSheet` at all.

**But the row type is generic over `Display`, and the renderer is ANSI-aware.**
`render_options<D: Display>(&mut self, page: Page<'_, ListOption<D>>)`
(`ui/backend.rs:45`) renders the row by its `Display` output, and the frame
renderer parses that output with an **ANSI state machine** before doing any width
or cursor math:

```rust
for piece in value.content.ansi_aware_chars() {
    match piece {
        AnsiAwareChar::AnsiEscapeSequence(seq) => {
            // we don't care for escape sequences when calculating
            // cursor position and box size
            self.current_styled.content.push_str(seq);
            continue;
        }
        AnsiAwareChar::Char(c) => { /* width counted */ }
    }
}
```
(`ui/frame_renderer.rs:67–79`)

The parser is inquire's own (`src/ansi.rs:20–98`), a simplified DEC ANSI parser
covering ESC/CSI/OSC/DCS — so arbitrary SGR sequences (`\x1b[2m` faint,
`\x1b[1m` bold, `\x1b[38;5;…`) are recognized as **zero-width** and preserved
verbatim in the output. Base style and inline ANSI compose: `FrameState::write`
sets the unit's `StyleSheet` first, then inline escapes layer over it
(`frame_renderer.rs:66`).

**Verdict on the `[folder] alias (user@host:port)` row:** achievable, but as
**ANSI-in-`Display`, not as a first-class per-segment style API**. There is no
`StyledStr`-equivalent to build. The saving grace is that the renderer treats
those escapes as zero-width, so layout stays correct.

**The text projection is separate — and this is where cliclack died.**
`SelectPrompt` precomputes `string_options: Vec<String>` from `T::to_string()`
once at construction (`select/prompt.rs:50`) and `run_scorer` passes
`self.string_options.get(i)` as the scorer's `string_value`
(`prompt.rs:138`). If `T`'s `Display` emits ANSI, that projection contains the
escape bytes too — so the **default** fuzzy scorer would match against them, the
identical cliclack failure mode. The escape hatch cliclack never had: the scorer
signature is `Fn(&str, &T, &str, usize)` and receives the **typed option `&T`**
(`type_aliases.rs:74`), so a custom scorer can match on struct fields and ignore
`string_value` entirely. `src/ansi.rs:136+` also ships a public
`AnsiStrippedChars` iterator for stripping a projection if you want one.
**The separation exists; it is your job to use it.**

## 5. Matched-character highlighting — VERIFIED: not built in, achievable without forking

**Built in: no.** `Scorer` returns `Option<i64>` — a score, no match offsets
(`type_aliases.rs:74`). `render_options` applies a single stylesheet per row
(`backend.rs:122–139`, `print_option_value`) and receives no position data.
Nothing in the built-in `Select` path can highlight the characters that matched.

**Achievable via the custom-prompt path: yes, cleanly.** The pieces compose:

1. Our `Prompt::handle(&mut self, action)` owns mutable state, so we run **our
   own matcher** per keystroke and store match ranges alongside the visible rows.
2. We build the visible row list each frame as our own type
   `Row { conn: &Connection, matches: Vec<usize> }` and implement
   `Display for Row` that emits SGR around the matched characters.
3. `backend.render_options::<Row>(page)` accepts it — `D: Display` is opaque to
   the backend (`backend.rs:45`).
4. `FrameState::write` treats those escapes as zero-width
   (`frame_renderer.rs:71–78`), so column alignment survives highlighting.

Cost: one `String` allocation per visible row per keystroke (**inference** from
the code path; no benchmarks in the repo). At a page size of 10–15 rows that is
noise.

**Caveat:** the row's base style still comes from `render_config.option` /
`render_config.selected_option` (`backend.rs:122–139`), and the cursor glyph is
`render_config.highlighted_option_prefix` / `unhighlighted_option_prefix`
(`backend.rs:104–119`). We control all of those through `RenderConfig`
(`ui/api/render_config.rs`, 534 lines of public style slots) — but the
*per-character* styling inside the row is our `Display` impl's responsibility,
not the crate's.

## 6. Terminal ownership — VERIFIED

**Inline. No alternate screen.** Grep for `AlternateScreen` /
`EnterAlternateScreen` across `src/`: **zero hits**. The renderer moves the cursor
relative to a frame anchor and rewrites in place
(`frame_renderer.rs:286–322`). Consistent with the inline-picker requirement.

**Raw mode for the whole prompt, on the crossterm backend:**

```rust
pub fn new() -> InquireResult<Self> {
    terminal::enable_raw_mode()?;
    crossterm::execute!(stderr(), event::EnableBracketedPaste)?;
    Ok(Self { io: IO::Std(stderr()) })
}
```
(`terminal/crossterm.rs:89–99`) — raw mode is enabled at construction and
disabled on drop (`crossterm.rs:236–238`, which also disables bracketed paste).
So inquire owns the TTY's mode for the prompt's duration, globally.

**Default output stream is stderr**, hardcoded in `CrosstermTerminal::new()`
(`crossterm.rs:97`). The `IO` enum is private (`crossterm.rs:20`), so you cannot
point `CrosstermTerminal` at stdout. **But `Terminal` is a public trait**
(`terminal/mod.rs:55–74`: `get_size`, `write`, `write_styled`, `clear_line`,
`clear_until_new_line`, `cursor_hide/show/up/down/left/right`, `flush`) and
`Backend::new(input_reader, terminal, render_config)` is public
(`ui/backend.rs:94`). A caller can supply a `Terminal` over any `impl Write`.

**Under `$(sshm pick)` with stdout captured:** this is the *good* case. The UI
goes to stderr and never touches stdout, so the captured value stays clean — the
app prints the chosen alias itself. No `StdoutRedirect` workaround is needed
because inquire never writes to stdout by default. The real hazard is the
inverse: if stdin is not a TTY, crossterm's event reader has nothing to read
(**inference** — no TTY guard found in `CrosstermKeyReader::new()`; the failure
would surface as an `io::Error` from `enable_raw_mode`/event read, not a clean
"non-interactive" path).

**Coexistence with a ratatui render loop: sequential only, not concurrent.**
Two independent reasons:

1. inquire keeps its **own cursor-position model** inside `FrameRenderer`
   (`frame_renderer.rs:186+`, `self.cursor_position`) and issues relative
   cursor moves against it every frame. A ratatui inline viewport tracking the
   same region maintains a second, conflicting model of where the cursor is.
   Same class of conflict recorded for cliclack (`cliclack-capability.md` §4).
2. Raw mode is process-global crossterm state; nesting two owners means whoever
   drops last disables it under the other.

Sequential interleaving is fine — neither enters the alternate screen, so
ratatui can draw, tear down, then an inquire prompt can run (or the reverse).
For the sshm inline picker, inquire would own the terminal for the prompt's
duration. Ratatui inline-viewport mechanics are already verified in this repo at
`docs/research/rust-inline-picker.md` §1 and `docs/research/clack-style.md` §6 —
not re-derived here.

## 7. Transparent background + named-ANSI-only — VERIFIED

**Named ANSI only: yes, by convention.** `Color` is 16 named variants
(`Black … DarkGrey`, `ui/api/color.rs:9–135`) plus opt-in `Rgb { r, g, b }`
(:137) and `AnsiValue(u8)` (:154). Nothing forces truecolor; a palette that
uses only the 16 named variants is fully expressible.

**Zero background painting: yes for the list path.** In
`RenderConfig::default_colored()` (`render_config.rs:200–232`) every style slot
is set with `.with_fg(...)` only — `placeholder`, `help_message`, `answer`,
`highlighted_option_prefix`, `unhighlighted_option_prefix`, `selected_option`,
`editor_prompt`. Grep for `with_bg` in `render_config.rs` returns **exactly one
hit: line 520, inside the `DateSelect` calendar's `selected_date`** — a prompt
type the picker does not use. The select/list rendering paints no background.

**Themeable without forking: yes.** `RenderConfig<'a>` is a public struct
(`render_config.rs:23`) with a `with_*` builder for every slot
(`:234–344`), attached per-prompt via `Select::with_render_config`
(`select/mod.rs:314`) or passed straight to `Backend::new`
(`ui/backend.rs:94`). There is no global theme registry (unlike cliclack's
`set_theme` RwLock) — configuration is per-prompt value-passing, which is
arguably cleaner for a `theme.rs`-driven app. Bonus: `RenderConfig::default()`
honours the `NO_COLOR` env var by falling back to `Self::empty()`
(`render_config.rs:350–356`) — accessibility handled for free.

**Gap that matters for the sshm spec:** the `Attributes` bitflags are
`BOLD | ITALIC` only (`style.rs:25–31`). **No `DIM`.** A dimmed `[folder]`
segment cannot be expressed through `StyleSheet` and must come from inline
`\x1b[2m` in the row's `Display` impl (§4). Also note
`set_attributes` in the crossterm backend only maps Bold and Italic
(`crossterm.rs:112–121`) — confirming DIM is absent from the abstraction, not
merely undocumented.

## 8. Settle-trace collapse — VERIFIED: built in

This is the one place where inquire's render model is *ahead* of cliclack's.
The default `prompt()` loop ends with:

```rust
let formatted = self.format_answer(&final_answer);
backend.frame_setup()?;
backend.render_prompt_with_answer(self.message(), &formatted)?;
backend.frame_finish(true)?;
```
(`prompts/prompt.rs:159–163`)

`finish_current_frame` is a **diffing renderer**, not a clear-and-rewrite
(`frame_renderer.rs:265–330`):

- It iterates `max(last_frame.height, current_frame.height)` rows with the
  cursor rewound to the frame anchor (`:288–290`).
- Row present in both frames → rewritten only if the row hash differs
  (`:296–301`), then `clear_until_new_line()`.
- Row present in the last frame but **absent from the current frame →
  `clear_line()`** (`:304–306`).

So when the final frame is one line (`prompt + answer`) and the live frame was
N lines of list, rows `1..N` hit the `(Some, None)` arm and are erased. **The
live frame collapses to a compact summary line automatically** — the same
behaviour Clack produces, without the caller managing line counts.
`frame_finish(true)` also appends one empty line (`:325–328`) to seat the
prompt cleanly in scrollback.

Cancel collapses too, via the `Action::Cancel` branch:
`render_canceled_prompt(message)` + `frame_finish(true)`
(`prompt.rs:144–147`), rendering the `<canceled>` indicator
(`render_config.canceled_prompt_indicator`, `render_config.rs:214`).

**Caveat:** the collapsed line is `answered_prompt_prefix + message + answer`,
where `answer` comes from `format_answer` (`prompt.rs:159`) — a `String`. It
goes through `Styled::new(line).with_style_sheet(render_config.answer)`
(`backend.rs:272–289`), and because the frame renderer is ANSI-aware (§4), a
multi-styled answer string renders correctly. So a settle line like
`◆ picked  web-01  (deploy@10.0.0.4:22)` with per-segment colour is achievable
via inline ANSI in `format_answer`'s return value.

## 9. dialoguer — one paragraph

**Correction to the #29 triage: dialoguer *does* have a filter-as-you-type
list.** `FuzzyListSelect` (`src/prompts/fuzzy_select.rs`, feature
`fuzzy-select`) re-filters on every keystroke — `filtered_list` is rebuilt from
`self.items` inside the key-handling loop, scored with
`fuzzy_matcher::skim::SkimMatcherV2` and sorted by score
(`fuzzy_select.rs:242–250`) — and it even ships **matched-character
highlighting** via `.highlight_matches(true)` (`fuzzy_select.rs:121`), which
calls `matcher.fuzzy_indices(text, search_term)` and bolds the matched runes
(`theme/mod.rs:232–242`). So "no filter list" is wrong. It still does not
qualify for sshm, for three reasons that end the enquiry: (1) the item universe is
fixed at construction (`items()`/`item()`, `fuzzy_select.rs:78–94`) with **no
custom-prompt escape hatch** — dialoguer exposes a `Theme` trait for *formatting*
but no `Prompt`-equivalent that hands over the render loop or key handling, so
the cliclack wall is not sidestepped the way inquire sidesteps it; (2) the
highlight is **bold-only and whole-item** — `style(c).for_stderr().bold()`
(`theme/mod.rs:235`), no per-segment colour, no dim, and the matcher scores the
item's raw `ToString` output, so ANSI-smuggled multi-style rows corrupt matching
(the exact cliclack failure mode, with no typed-option escape hatch like
inquire's `Scorer`); (3) rendering goes through `console` line-oriented writes
with no diffing frame model, so the settle-collapse and rail geometry we need are
not expressible. **Ruled out.** (Version 0.12.0, 16.4M recent downloads,
<https://github.com/console-rs/dialoguer>, updated 2025-08-23; checksum
`sha256 25f104b5…c0ad96`.)

## Summary table

| Capability | Verdict | vs cliclack (#27) |
|---|---|---|
| (a) Re-suppliable candidate set per keystroke | **Yes — via custom `Prompt`.** `handle(&mut self, …)` owns mutable state (`prompt.rs:105`); the universe can be reassigned between keystrokes. Built-in `Select` fixes the universe at `Select::new` (`select/mod.rs:212`) but re-filters live via `run_scorer` per keystroke (`select/prompt.rs:205–216`) | **Strictly better** — cliclack's set was unreachable behind a private `Rc<RefCell<…>>` captured at `interact()` |
| (b) Custom multi-style per-row layout | **Half.** No `StyledStr`; `Styled<T>` = one `StyleSheet` per unit (`style.rs:132–141`); `Attributes` is BOLD\|ITALIC only, **no DIM** (`style.rs:25–31`). Achievable as **ANSI-in-`Display`**, which the renderer parses as zero-width (`frame_renderer.rs:67–79`). Not a first-class per-segment API | **Better** — cliclack's ANSI-in-label broke its filter; inquire's ANSI-in-`Display` renders correctly and the scorer can bypass the string entirely |
| (c) Matched-character highlighting | **Not built in; achievable without forking.** `Scorer` returns `Option<i64>`, no offsets (`type_aliases.rs:74`). Custom path: own matcher in `handle` → `Row{matches}` → `Display` emits SGR → `render_options<D: Display>` (`backend.rs:45`) | **Better** — cliclack produced no match positions *and* offered no seam to compute our own |
| (d) Transparent bg, named-ANSI-only | **Yes.** 16 named `Color` variants (`color.rs:9–135`); `default_colored()` sets fg only — the sole `with_bg` in `render_config.rs` is line 520, in the unused `DateSelect` calendar. Per-prompt `RenderConfig` builder (`:234–344`), no fork | **Equal** — both satisfy this by default |
| (e) Settle-trace collapse | **Yes, built in and better.** `frame_finish` diffs frames row-by-row; rows absent from the new frame get `clear_line()` (`frame_renderer.rs:304–306`), so the N-line live frame collapses to the `prompt + answer` line emitted at `prompt.rs:159–163` | **Better** — cliclack clear-and-rewrote; inquire diffs by row hash |
| (f) Coexistence with ratatui | **Sequential only, not concurrent.** Global crossterm raw mode (`crossterm.rs:90`) plus inquire's own cursor-position model (`frame_renderer.rs:186+`) conflict with a ratatui inline viewport over the same region | **Equal** — same class of conflict |

## Verified facts vs inference

**Verified by reading the 0.9.4 source** (checksum
`6654738b…59429756`): the `Prompt` trait shape and that it is unsealed
(`prompt.rs:44–167`); `prompt()` being an overridable provided method
(`prompt.rs:114–118`); `Action::from_key` being public with Enter/Esc/`^C`/`^G`/`^D`
reserved (`action.rs:34–46`); `Scorer`/`Sorter` signatures and the SkimMatcherV2
default (`type_aliases.rs:74/81/54–58`); `Select::new` taking `Vec<T>` by value
(`select/mod.rs:212`); `run_scorer` being invoked on every content-changing
keystroke (`select/prompt.rs:205–216`); `string_options` as a separate text
projection (`prompt.rs:50,138`); `with_formatter` wired only to `format_answer`
(`prompt.rs:178–180`); `Attributes` = BOLD|ITALIC only (`style.rs:25–31`); the
ANSI-aware zero-width escape handling (`frame_renderer.rs:67–79`) and inquire's own
DEC parser (`ansi.rs:20–98`); `Backend`'s only public inherent method being `new`
(`backend.rs:94`) and `frame_renderer` being a private field (`backend.rs:83`);
the row-diffing `finish_current_frame` with `clear_line()` on vanished rows
(`frame_renderer.rs:265–330`); raw mode at construction / off at drop and the
hardcoded `stderr()` sink (`crossterm.rs:89–99, 236–238`); no alternate-screen
anywhere (grep, zero hits); `RenderConfig` fg-only defaults with the single
calendar `with_bg` (`render_config.rs:200–232, 520`); the `Color` variant list
including `Rgb`/`AnsiValue` (`color.rs:137/154`); dialoguer's `FuzzyListSelect`
filtering + `fuzzy_indices` bold-highlighting (`fuzzy_select.rs:242–250`,
`theme/mod.rs:232–242`).

**Verified against external APIs** (2026-09-13): crates.io
`repository = github.com/mikaelmello/inquire`, version, license, download
counts; GitHub stars/forks/open-issues/`pushed_at`; release dates; contributor
distribution; issue #290 text and comments.

**Inference (stated as such, not verified by execution):** per-keystroke cost of
our own matcher plus per-row `String` allocation is acceptable at sshm scale (no
benchmarks exist in the repo); the exact failure mode of `CrosstermKeyReader`
under a non-TTY stdin (no guard found in source; behaviour inferred from
crossterm semantics); that the ~91% single-author commit share is a practical
bus-factor risk rather than merely a statistic; that the ~6-month gap since the
last push signals cooling maintenance rather than a pause.

**Not tested:** nothing in this audit was compiled or run against a real terminal.
Every claim is static source reading plus API metadata. Per `AGENTS.md`, no
rendering claim should be treated as final until a frame has been looked at in a
real terminal — that belongs to #28's decision, not this ticket.
