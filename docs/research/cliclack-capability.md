# cliclack capability: filter-as-you-type list with custom styled rows

Research for #27. **Verified 2026-09-12** against cliclack **0.5.6** source
(crates.io `.crate`, checksum `134778b6…5cf30b`, extracted and read directly —
not README prose). Line references are to that source.

## One-paragraph answer to the critical question

**cliclack's `Select` is a static-list prompt with a built-in, non-customizable
fuzzy filter — it cannot drive a persistent filter-as-you-type list the way atuin
does.** The candidate set is fixed at `interact()` time (`select.rs:114`,
`self.filter.set(self.items.to_vec())`); items are supplied only through the
builder (`.item()`/`.items()`) before the prompt starts, and there is no public
API to re-supply or mutate the candidate list during the interaction. Typing
does filter live (`filter_mode()`), but the matcher is hardcoded
Jaro-Winkler-over-whole-label (`suggest.rs:70–88`, threshold >0.6) with no
match-position information produced, so **matched-character highlighting is
impossible without forking**. A row is just `(label: String, hint: String)`:
the label is `impl Display → to_string()`, so embedded ANSI codes survive and
multi-style rows *render*, but the built-in filter then matches against the raw
string **including the escape codes**, breaking filtering — so styled labels and
the built-in filter are mutually exclusive. There is no per-segment style API
(the theme applies one style to the whole label, `theme.rs:401–431`). The
`Suggest` trait's closure impl (`suggest.rs:49–58`) exists for `Input`
autocomplete only; `Select` hardcodes the `Vec` impl (`select.rs:123`).
Verdict: static scrollable list + fixed-set fuzzy shrink; not a re-filterable
per-keystroke engine.

## 1. Version, license, maintenance — VERIFIED

- Latest version **0.5.6**, released **2026-08-10** (crates.io API,
  <https://crates.io/api/v1/crates/cliclack>).
- License **MIT** (`Cargo.toml` `license = "MIT"`; GitHub license field agrees).
- 367 stars, 10 open issues, not archived, `pushed_at` 2026-08-10
  (<https://github.com/fadeevab/cliclack>).
- Release cadence: 0.5.4 2026-04-08, 0.5.5 2026-07-05, 0.5.6 2026-08-10 —
  active but low-frequency (roughly quarterly).
- **Issue response is slow**: open issue [#114 "Feat: Customizable Filtering
  Logic for Select"](https://github.com/fadeevab/cliclack/issues/114) (opened
  2026-07-31) has **0 comments** as of 2026-09-12. Also open: #109
  "Asynchronous non-blocking search", #110 "filter_mode swallows space"
  (confirmed in code: `filter.rs:78–81` deletes spaces). The existence of #114
  independently confirms filtering logic is not customizable today.
- README gap: the README does not document `filter_mode` at all (grep for
  filter/search/fuzzy over `README.md`: zero hits) — the capability is
  code-only.

## 2. Component list (public API) — VERIFIED (`src/lib.rs:312–448`)

Free functions: `intro`, `outro`, `outro_cancel`, `outro_note`, `note`,
`input`, `password`, `select`, `multiselect`, `confirm`, `spinner`,
`progress_bar`, `multi_progress`, `clear_screen`, `log` module
(`remark`/`info`/`warning`/`error`/`success`-style log groups via
`log::log` + theme `format_log`).
Types: `Input`, `Password`, `Select`, `MultiSelect`, `Confirm`, `ProgressBar`,
`MultiProgress`, `Theme`, `ThemeState`, `set_theme`, `reset_theme`, `termwrap`,
`StringCursor`, `Suggest`, `Validate`.
Matches the expected inventory; nothing beyond it relevant to the picker.

## 3. `Select`'s real capability — VERIFIED from `select.rs` + `filter.rs` + `suggest.rs`

- **Option list fixed at construction?** **Yes, fixed.** Items are added via
  builder `.item(value, label, hint)` / `.items(...)` and captured once at
  `interact()`: `self.filter.set(self.items.to_vec())` (`select.rs:114`).
  The `Rc<RefCell<RadioButton<T>>>` handles are never exposed to the caller,
  so the set cannot be changed mid-prompt. **Cannot be re-supplied per
  keystroke.**
- **Custom multi-style per-row layout?** **Only by smuggling ANSI into the label
  string, and that breaks the filter.** Data model is `RadioButton { value,
  label: String, hint: String }` (`select.rs:18–22`); `label` is
  `impl Display → to_string()`, so pre-styled strings (console/owo-colors)
  render with their embedded codes. But the theme applies **one style to the
  whole label** (`radio_item`, `theme.rs:401–431`: selected → `input_style`,
  unselected → `placeholder_style`), and the hint is shown **only on the
  selected row**, wrapped in `(...)`. There is no per-segment style hook.
  Worse, the built-in filter lowercases and scores the **raw label string**
  (`suggest.rs:28`), so ANSI escapes in the label participate in matching —
  styled labels and a working built-in filter are mutually exclusive.
- **Matched-character highlighting?** **No.** `format_select_item(state,
  selected, label, hint)` (`theme.rs:435`) receives no match positions, and
  the matcher (Jaro-Winkler score) never computes any. Impossible without
  forking.
- **Built-in filter/query input?** **Yes** — `.filter_mode()` (`select.rs:83`)
  enables a type-to-filter input rendered via `theme.format_input`. Matching:
  `strsim::jaro_winkler(label, input)` + bonus if every whitespace-separated
  query word is a substring of the label; keep if score > 0.6; sort descending
  (`suggest.rs:70–88`). Case-insensitive. This is whole-string similarity
  ranking, **not** fzf-style subsequence matching.
- **Custom matcher injectable into Select?** **No.** `Select::on` calls
  `self.filter.on(key, &self.items)` (`select.rs:123`) — hardcoded to the
  `Suggest for Vec<Rc<RefCell<T>>>` impl. The `Suggest for F: Fn(&str) ->
  Vec<T>` closure impl (`suggest.rs:49–58`) is used only by `Input`'s
  autocomplete (`autocomplete.rs:6–25`), not by `Select`.
- **Max practical list size?** **INFERRED** (no benchmarks in repo): the
  filter is O(n · label_len · input_len) per keystroke (Jaro-Winkler per
  item) and rendering is capped at `max_rows` visible lines with
  clear-and-rewrite of the frame only. Thousands of candidates should be fine;
  tens of thousands may add perceptible per-keystroke latency on slow
  terminals. The real ceiling is not rendering but the fixed-set + fixed-matcher
  design.

## 4. Terminal ownership — VERIFIED (`prompt/interaction.rs`)

- **Inline, no alternate screen.** No alternate-screen calls exist anywhere in
  the crate (grep: only `clear_last_lines` / `clear_screen`). The loop renders
  a frame, `term.clear_last_lines(prev_frame_lines)`, `write_all(frame)`,
  flush, then blocks on `read_key_raw` (`interaction.rs:78–101`). Cursor
  hidden during the prompt, restored after (`interaction.rs:65–74`).
- **Default output is stderr** (`Term::stderr()`, `interaction.rs:61`);
  `interact_on(&mut Term)` accepts a custom `Term` (e.g. stdout).
- **Coexistence with a ratatui render loop: not concurrently.** During the
  prompt cliclack owns the blocking read loop and its own clear-and-rewrite
  line accounting; a ratatui inline viewport tracking the same region would
  fight over "lines to erase". Sequential interleaving (cliclack prompt ends →
  ratatui draws, or vice versa) is fine since neither touches the alternate
  screen. For the inline picker, cliclack would own the terminal for the
  prompt's duration. Ratatui inline-viewport mechanics are already verified in
  this repo: `docs/research/rust-inline-picker.md` §1 and
  `docs/research/clack-style.md` §6 — not re-derived here.

## 5. Theming — VERIFIED (`theme.rs`)

- **Changeable without forking**: global swap via
  `set_theme<T: Theme + Sync + Send + 'static>` / `reset_theme()`
  (`theme.rs:823/828`, global `RwLock`). The `Theme` trait exposes ~40
  overridable methods with defaults: `bar_color`, `state_symbol(_color)`,
  `radio_symbol`, `checkbox_symbol`, `input_style`, `placeholder_style`,
  `radio_item`, `format_select_item`, `format_input`, `format_header`,
  `format_footer*`, `format_log*`, etc.
- **Named ANSI + modifiers only: already true.** The default theme uses only
  `console::Style::new().<name>()` calls; grep for `Rgb`/`set_color`/`Color::`
  in `theme.rs` returns **zero** hits. No truecolor anywhere in defaults.
- **No background colours by default**: grep for `.on_` (background setters)
  in `theme.rs` returns **zero** hits — rows carry no background, i.e.
  effectively transparent. A custom theme can keep it that way.
- **Glyphs**: Unicode/ASCII-fallback pairs (`Emoji("◆","*")` etc.,
  `theme.rs:10–40`) are private consts, but every glyph enters output through
  overridable trait methods (`radio_symbol`, `state_symbol`, `info_symbol`, …),
  so glyphs are changeable without forking.

## Summary table

| Capability | Verdict |
|---|---|
| (a) Re-filter large candidate set per keystroke | **No** — set fixed at `interact()`; built-in filter shrinks the fixed set only; matcher not injectable |
| (b) Custom multi-style per-row layout | **Half** — ANSI-in-label renders, but breaks the built-in filter; no per-segment style API; hint only on selected row |
| (c) Matched-character highlighting | **No** — no match positions produced or plumbed |
| (d) Transparent bg, named-ANSI-only | **Yes** — defaults already satisfy this; fully theme-able via `set_theme` |
