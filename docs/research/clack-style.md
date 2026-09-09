# Clack-style inline prompts in sshm

**Status: as-built.** Design/mapping doc for the Nord → Clack restyle. Every mapping below was checked against `sshm/src/style.rs`, `sshm/src/picker.rs`, and `sshm/src/app.rs` on 2026-09-09. Stack: **ratatui 0.30.2**, **crossterm 0.28.1** (`sshm/Cargo.lock`), **no new dependencies**.

## 1. Reference and goal

[vercel-labs/skills](https://github.com/vercel-labs/skills) builds its CLI on [`@clack/prompts`](https://github.com/bombshell-dev/clack) (v1.7.0 at restyle time); its interactive search list is a hand-rolled `searchMultiselect` (`src/prompts/search-multiselect.ts`) on top of Node `readline`. sshm replicates that *visual grammar* — not the JS machinery — in Rust on ratatui. `style.rs` is the single source of truth for the language (glyphs, palette, line builders) and cites `search-multiselect.ts` in its header; `picker.rs` (the inline `sshm pick` frame) and `app.rs` (the fullscreen manager: list, form, note popups, help) are its only two callers.

## 2. Glyph vocabulary

| Glyph | `style.rs` const | Meaning |
|-------|------------------|---------|
| `◆` | `STEP_ACTIVE` | Active step header icon (green) |
| `◇` | `STEP_SUBMIT` | Submitted header icon (settle trace, green) |
| `■` | `STEP_CANCEL` | Cancelled header icon (settle trace, red) |
| `│` | `BAR` | Dim left rail on every content row |
| `└` | `CORNER` | Closing corner, last row of a frame |
| `❯` | `CURSOR` | Cyan cursor column on the current row |
| `●` / `○` | `RADIO_ON` / `RADIO_OFF` | Current / other option dot (green / dim) |

No reserved extras: glyphs held in reserve for possible future screens (`✓`, `•`, `▸`/`▾` disclosure triangles, `◐` tri-state dot, `├─`/`└─` tree connectors) were deliberately dropped pre-commit so `cargo clippy --all-targets -D warnings` stays clean; re-add a const when a screen first uses it.

## 3. Palette — foreground-only

Clack paints with `picocolors` names on the terminal's default background. The `style.rs` mirror uses ratatui named colours + modifiers and **never sets a background** — frames blend into the shell instead of sitting on painted blocks (the old Nord theme's `Rgb` backgrounds inside bordered Blocks are gone).

| `style.rs` helper | picocolors | Used for |
|-------------------|------------|----------|
| `green()` | `green` | Active/submitted step icon, `●` dot, fuzzy highlight |
| `cyan()` | `cyan` | `❯` cursor column |
| `dim()` | `dim` (ANSI faint) | Rail, key hints, placeholders, meta text |
| `bold()` | `bold` | Header message |
| `underline()` | `underline` | Current label |
| `highlight()` | green + bold | Fuzzy-matched characters |

Note: `style.rs` keeps no red, yellow, or strikethrough helper — the red cancel icon and the crossed-out cancelled summary are emitted by `picker::write_settle_trace` directly through crossterm (`SetForegroundColor(Color::Red)`, `SetAttribute(CrossedOut)`) once ratatui is done (see §5).

## 4. Frame anatomy

The picker frame built by `build_picker_lines` (pure, unit-tested):

```text
◆  Pick Connection                    ← header_line(STEP_ACTIVE, green, …)
│  Search: Type to search...          ← rail + dim placeholder
│  ↑↓/j k: Navigate | Enter: Select … ← dim hint rail (PICKER_HINT)
│                                     ← rail_blank
│ ❯ ● [staging] web (u@h.com:22)      ← CURSOR + RADIO_ON + underlined label
│   ○ db (u@h2.com:22)                ← RADIO_OFF + dim `(user@host:port)` hint
│                                     ← rail_blank
└                                     ← corner_line
```

- **Constant height while active.** `pick_frame_height(total) = 6 + clamp(total, 1, MAX_VISIBLE)` — six fixed rows (header, search, hint, two blank rails, corner) plus the list rows: 7–14 lines. Computed once before the inline viewport opens; like inline Clack prompts the frame never grows mid-session, the list window slides instead.
- **Windowing.** `style::visible_window(len, selected, max_visible)` ports skills' `maxVisible` centring: `start = selected.saturating_sub(max_visible / 2)`, snapped back to `len − max_visible` near the end. The picker passes `MAX_VISIBLE = 8`; `app.rs` uses the identical `MAX_LIST_ROWS = 8`.
- **Block caret.** The search row renders `query` + one `Modifier::REVERSED` space — the Rust form of skills' `${query}${inverse(' ')}`; the same reverse-video block marks the active field in the Add/Edit form.

## 5. Settle trace (collapse on exit)

Clack's signature: when a prompt submits, the live frame is erased and a compact summary remains in the scrollback. After the picker loop breaks, `write_settle_trace` runs *before* the ratatui/raw-mode guards drop and leaves two rows at the frame's original position:

- Submit: `◇  Pick Connection` (green icon, bold message) + `│  [staging] web (u@h.com:22)` (dim summary).
- Cancel: `■  Pick Connection` (red icon) + dim, **crossed-out** `│  Cancelled`.

Mechanics: the absolute cursor row is recorded *before* the viewport is created (`frame_top = crossterm::cursor::position()`); the erase is raw ANSI queued to fd 1 — `MoveTo(0, frame_top)` + `Clear(FromCursorDown)` — and the two rows are styled with raw crossterm ANSI — `SetForegroundColor(Green/Red)` for the icon, `SetAttribute(Bold/Dim/CrossedOut)` for the rest — bypassing ratatui (the terminal is done by then). Content comes from the pure `style::settle_plain` builder so it is unit-testable without escape sequences; if the cursor-position request fails, the trace prints in place. The accepted `ssh …` line still goes to the *restored* stdout, so `$(sshm pick)` captures only the command; cancel prints nothing and exits **130** (fzf convention, `main.rs`).

## 6. Constraints and decisions

1. **ratatui 0.30 cannot resize an inline viewport.** `Terminal::set_viewport_height` no longer exists and `set_viewport_area` is `pub(crate)` (verified in the vendored `ratatui-core 0.1.2`, `src/terminal/resize.rs`: "internal helper used by `Terminal::with_options` and `Terminal::resize`"). Growing the frame per keystroke would mean recreating the terminal, leaking blank lines into scrollback each time. Decision: **constant frame height while active** + windowing, and a **raw-ANSI settle after exit** for the collapse.
2. **Main manager stays on the fullscreen alternate screen (v1).** `run_app_inner` keeps `ratatui::init()`; only the visual grammar changed — `App::render` clears with an empty `Block` and draws a `Paragraph` of rail lines, no borders. `cleanup_and_exit`'s hard-reset escape codes (`\x1b[?1049l`, `\x1b[?25h`, `\x1b[1bc`) are unchanged.
3. **No off-the-shelf Rust Clack fits.** crates.io at restyle time: `may-clack` 0.7.2 (dormant), `inquire-clack` 0.1.0 (single-release inquire fork), `clark`/`clark-cli` 0.2.0 (days old, ~50 downloads) — all immature; `cliclack` is popular but a standalone prompt engine, not a ratatui styling layer, so it cannot restyle an existing ratatui render loop. Decision: hand-rolled `style.rs` (one ~308-line module, mostly pure builders + unit tests).
4. **Settling under command substitution.** In `result=$(sshm pick --query …)`, fd 1 is a pipe: crossterm's cursor-position request (`ESC [ 6 n`) writes straight to stdout, never reaches the terminal, and the inline viewport cannot initialize. The `StdoutRedirect` RAII guard points fd 1 at `/dev/tty` for the picker's duration — so the UI *and* the settle trace reach the real terminal — and restores the pipe before `main.rs` prints the chosen `ssh` command (atuin's `TerminalWriter` pattern).

## 7. Clack → sshm mapping

| Clack / skills concept | sshm equivalent |
|------------------------|-----------------|
| Step state → icon (active/submit/cancel) | `STEP_ACTIVE`/`STEP_SUBMIT`/`STEP_CANCEL` + `style::header_line` |
| `bar` symbol on content rows | `BAR` + `rail` / `rail_text` / `rail_blank` |
| Closing corner `└` | `CORNER` + `corner_line` |
| Cyan cursor on active option | `CURSOR` + `rail_row_spans_styled` |
| Active/inactive option dots | `RADIO_ON` / `RADIO_OFF` |
| Underlined active label | `underline()` in `build_picker_row_spans` (picker) / `connection_row_spans` → `rail_row_spans_styled` (manager) |
| `picocolors.*` foreground tokens | `style::green/cyan/dim/bold/underline/highlight` |
| `maxVisible` sliding window | `style::visible_window` + `MAX_VISIBLE` / `MAX_LIST_ROWS` |
| Dim group labels / meta text | dim `[folder]` prefix, dim `(hint)` spans |
| Inverse block caret after query | `Modifier::REVERSED` space span |
| Prompt collapse after submit | `settle_plain` + `picker::write_settle_trace` |
| `isCancel(result)` handling | `PickerOutcome::Cancel` → exit 130, empty stdout |
| Dim hint line under the prompt | `PICKER_HINT` rail row; `footer_help_text()` |

## 8. Where things live

- `sshm/src/style.rs` — glyphs, palette, line builders, `visible_window`, `settle_plain` (all pure + unit-tested).
- `sshm/src/picker.rs` — frame assembly (`pick_frame_height`, `build_picker_lines`, `render_picker_frame`), `StdoutRedirect`, `write_settle_trace`.
- `sshm/src/app.rs` — same grammar on the fullscreen manager: `build_main_lines`, `render_input` (form), `render_popup` / `render_update_popup` / `render_help` (note frames). The older per-region helpers (`render_header`, `render_list`, `render_footer`, `render_search`, `render_search_bar`, `centered_rect`) now survive only as `#[cfg(test)]` fixtures for focused render tests.

Companion research for the inline architecture: [atuin-fzf-ux.md](./atuin-fzf-ux.md), [rust-inline-picker.md](./rust-inline-picker.md).
