# Research: building an atuin/fzf-style inline fuzzy picker in Rust

**Status: verified.** Every claim checked against the live primary source on 2026-07-18. Companion to [atuin-fzf-ux.md](./atuin-fzf-ux.md).

Our stack (from `sshm/Cargo.lock`): **ratatui 0.30.0**, **crossterm 0.28.1**, **clap 4.6.0**.

## 1. ratatui inline viewport (we are on 0.30.0)

From [docs.rs/ratatui/0.30.0](https://docs.rs/ratatui/0.30.0/ratatui/enum.Viewport.html):

```rust
pub enum Viewport {
    Fullscreen,
    Inline(u16),
    Fixed(Rect),
}
```

- `Inline(u16)`: "The viewport is inline with the rest of the terminal. The viewport's height is fixed and specified in number of lines. The width is the same as the terminal's width. **The viewport is drawn below the cursor position.**"
- [`ratatui::init_with_options(options: TerminalOptions) -> DefaultTerminal`](https://docs.rs/ratatui/0.30.0/ratatui/fn.init_with_options.html) exists in 0.30.0 (crossterm feature): enables raw mode and installs the restoring panic hook. Documented example:

```rust
let options = TerminalOptions { viewport: Viewport::Inline(5) };
let terminal = ratatui::init_with_options(options);
```

- For manual control, `Terminal::with_options(backend, TerminalOptions { viewport })` — the exact call atuin makes (see §3).

Known constraints to validate in a prototype: inline viewport behavior on terminal resize, interaction with scrollback when the UI height changes between frames, and cleanup on exit (the viewport area must be left clean so the shell prompt redraws correctly — atuin handles this by clearing before restore; check `Terminal::clear` / drop behavior in 0.30.0).

## 2. The stdout-vs-TTY split

Two verified patterns:

- **fzf's contract (cleanest):** draw UI and read keys on `/dev/tty`; emit only the selection on stdout. fzf's own shell glue runs it as `$(fzf ...) < /dev/tty`. ([key-bindings.zsh](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.zsh))
- **atuin's pattern:** a `TerminalWriter` that writes to stdout when stdout is a terminal and opens `/dev/tty` when stdout is captured by command substitution; the *accepted selection* is written to atu's stderr, and the shell widget uses an fd swap (`3>&1 1>&2 2>&3 3>&-`) so the TUI stays visible while the selection is captured. ([interactive.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/search/interactive.rs), lines 1287–1310; [atuin.zsh](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/shell/atuin.zsh))

Recommendation for sshm: adopt **fzf's contract** — backend over `/dev/tty` (or stderr), selection printed to stdout, cancel = distinct exit code (fzf uses `130`). It needs no fd-swap trickery in the shell script; the widget is just `output=$(sshm pick)`.

## 3. atuin's picker structure (Rust + ratatui, directly analogous)

From the atuin repo:

- Shell scripts are static assets in `crates/atuin/src/shell/atuin.{zsh,bash}`; `atuin init <shell>` prints them plus conditional bindkey lines from [init/zsh.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/init/zsh.rs) / [init/bash.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/init/bash.rs), respecting `ATUIN_NOBIND` and `--disable-ctrl-r` / `--disable-up-arrow`.
- The interactive search (`crates/atuin/src/command/client/search/interactive.rs`) builds the terminal with `Terminal::with_options(..., TerminalOptions { viewport: ... })`, choosing `Fixed` (tmux popup) / `Inline(inline_height)` / `Fullscreen`.
- Execute-vs-insert is negotiated via a sentinel: the TUI prints `__atuin_accept__:<cmd>` when the user asked to execute, and the shell widget strips the prefix and calls `zle accept-line`. A plain accept just sets `LBUFFER`.

## 4. Embed skim vs. bespoke ratatui picker

[skim](https://docs.rs/skim/latest/skim/) (crate `skim` 5.2.0, actively maintained) is a Rust fzf clone usable as a library:

```rust
let options = SkimOptionsBuilder::default()
    .height("50%")
    .multi(true)
    .build()
    .unwrap();
```

Items stream in via `SkimItemSender`/`SkimItemReceiver`; items implement the `SkimItem` trait. It ships its own `tui` module (a separate terminal stack from ratatui).

Trade-off: embedding skim pulls an entire second TUI framework into a binary that already uses ratatui 0.30 — duplicate terminal stacks, theming inconsistency with the existing Nord-themed TUI, and less control over the exact UX (layout, key hints, connection metadata display). A bespoke picker is a bounded build: the fuzzy matcher is a crate (§5), the list UI is a standard ratatui `List` + input box, and our `Connection` structs and `build_ssh_args` are reused directly. The research points bespoke for this codebase; skim-as-library remains the fallback if the bespoke picker balloons.

## 5. Fuzzy-matching crates

- [nucleo](https://docs.rs/nucleo/latest/nucleo/) 0.5.0 — "high level crate that provides a highly effective (parallel) matcher worker … designed to allow quickly plugging a fully featured (and faster) fzf/skim like fuzzy matcher into your TUI application." Background threadpool, snapshot-based results, lock-free streaming of items. "Used in the helix-editor and therefore has a large user base with lots of real world testing." 481k recent downloads.
- [fuzzy-matcher](https://docs.rs/fuzzy-matcher/latest/fuzzy_matcher/trait.FuzzyMatcher.html) 0.3.7 — single-trait, synchronous: `fn fuzzy_indices(&self, choice: &str, pattern: &str) -> Option<(i64, Vec<usize>)>` returns score + matched-character indices (what you need to highlight matches in the list). `SkimMatcherV2` is the main implementor. 8.1M recent downloads; skim's matcher extracted.

For a connection list (tens to low-hundreds of entries), matching is not a parallelism problem — **fuzzy-matcher's `SkimMatcherV2` is the proportionate choice**, and `fuzzy_indices` gives highlight ranges for free. nucleo becomes relevant if the picker ever searches large sets (e.g. shell-history-scale).

## 6. Startup latency

No hard numbers are published by these projects' primary sources for startup budgets; the load-bearing, verifiable facts are structural:

- The picker is invoked on every keypress of the trigger — it must do **no network I/O** before first paint. (sshm today has an update checker with GitHub release integration — see `sshm/src/update.rs`; the pick path must skip it or defer it. Repo fact, needs a code decision.)
- atuin seeds the query from the buffer and opens already-filtered (§2 of the companion doc) — perceived speed comes as much from pre-narrowing as from raw startup time.
- Keep the pick path's startup to: parse args (clap), load config, build candidate list, first paint. Anything else (update checks, telemetry) runs after first paint or not at all.

## Implications for sshm (verified blueprint)

1. New `sshm pick` subcommand: `ratatui::init_with_options(TerminalOptions { viewport: Viewport::Inline(n) })`-style setup (or manual `Terminal::with_options` over a `/dev/tty` backend), a `List` of Connections filtered by `fuzzy_matcher::skim::SkimMatcherV2::fuzzy_indices`, matched chars highlighted.
2. On accept: print `build_ssh_args`-equivalent command to stdout, exit 0. On Esc/Ctrl-C: print nothing, exit 130 (fzf convention).
3. New `sshm init zsh|bash` subcommand emitting static widget scripts + conditional bindkey lines, modeled on atuin's `init_static` (§3).
4. Skip the update checker on the pick path (§6).
5. Prototype checkpoint: inline viewport cleanup on exit + resize behavior on ratatui 0.30.0 (§1 constraints) before committing to the full build.
