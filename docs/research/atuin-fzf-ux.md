# Research: how atuin and fzf achieve the "inline with the CLI" experience

**Status: verified.** Every claim below was checked against the live primary source on 2026-07-18 (atuin `main`, fzf `master`, fzf-tab `master`, docs.atuin.sh). Supersedes an earlier unverified draft produced by a subagent without web access.

## 1. The core architecture: stream split + shell glue widget

Both tools follow the same contract: the picker process draws its UI on the terminal, emits **only the accepted selection** on a capturable stream, and a shell widget splices that selection into the line editor.

- zsh: a ZLE widget manipulates `BUFFER`/`LBUFFER`/`RBUFFER`, then calls `zle reset-prompt`. ([atuin.zsh](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/shell/atuin.zsh), [fzf shell/key-bindings.zsh](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.zsh))
- bash: the handler manipulates `READLINE_LINE`/`READLINE_POINT`. ([fzf shell/key-bindings.bash](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.bash), lines 66–67, 103–139)

## 2. atuin's shell integration

`atuin init zsh` prints the static script `crates/atuin/src/shell/atuin.zsh` and then, unless `ATUIN_NOBIND` is set, prints `bindkey` lines gated by `--disable-ctrl-r` / `--disable-up-arrow`. ([init/zsh.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/init/zsh.rs))

Default bindings: Ctrl-R in emacs + viins keymaps (`atuin-search` / `atuin-search-viins`), `/` in vicmd, and up-arrow in all keymaps (`atuin-up-search`). (same file)

The zsh search widget ([atuin.zsh](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/shell/atuin.zsh)):

```zsh
ATUIN_SHELL=zsh ATUIN_QUERY=$BUFFER atuin search "${search_args[@]}" -i 3>&1 1>&2 2>&3 3>&-
```

- The current buffer is passed via `ATUIN_QUERY` (initial query). `-i` selects **interactive** (TUI) mode.
- The fd swap `3>&1 1>&2 2>&3 3>&-` routes atuin's stdout to the terminal (so the TUI is visible) while the command substitution captures what atuin writes to its stderr — which is where the accepted command is emitted.
- On exit status 0 with non-empty output: `RBUFFER=""; LBUFFER=$output` — insert, not execute — then `zle reset-prompt`.
- On non-zero status: output is printed to `/dev/tty` and the buffer is left untouched (cancel semantics).
- If the output starts with the sentinel `__atuin_accept__:`, the widget strips it and calls `zle accept-line` — this is how execute-immediately is signaled from the TUI to the shell.

The bash script uses the same fd-swap pattern with `ATUIN_QUERY=$READLINE_LINE`. atuin's bash *history-recording* hooks require ble.sh or bash-preexec (tracked via `ATUIN_PREEXEC_BACKEND`); the search keybinding itself works without them. ([atuin.bash](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/shell/atuin.bash), line 323) The bash init writes `__atuin_bind_ctrl_r` / `__atuin_bind_up_arrow` markers the script consumes. ([init/bash.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/init/bash.rs))

Config semantics ([docs.atuin.sh/cli/configuration/config](https://docs.atuin.sh/cli/configuration/config/)):

- `inline_height` — default **40**. "Set the maximum number of lines Atuin's interface should take up. If set to 0, Atuin will always take up as many lines as available (full screen)." So atuin is **inline by default**, fullscreen when set to 0.
- `enter_accept` — default **false** (since v17.0). "When set to true, Atuin will default to immediately executing a command rather than requiring you to press enter twice." Default behavior is insert-don't-execute.

## 3. atuin's Rust rendering

In [interactive.rs](https://github.com/atuinsh/atuin/blob/main/crates/atuin/src/command/client/search/interactive.rs) (lines 1287–1310, 1719–1731):

- A `TerminalWriter` abstraction: uses stdout when stdout is a terminal; falls back to opening `/dev/tty` when stdout is captured (e.g. by `$( )` command substitution).
- Viewport selection:

```rust
let mut terminal = Terminal::with_options(
    CrosstermBackend::new(stdout),
    TerminalOptions {
        viewport: if popup_mode {
            Viewport::Fixed(popup_rect)
        } else if inline_height > 0 {
            Viewport::Inline(inline_height)
        } else {
            Viewport::Fullscreen
        },
    },
)?;
```

## 4. fzf's contract

- Candidates on stdin, selection on stdout, UI + keyboard via the tty: fzf's own shell bindings run it inside `$( )` with an explicit `< /dev/tty` redirect. ([key-bindings.zsh](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.zsh), `__fzf_select`, `fzf-cd-widget`)
- `--height[=HEIGHT%]`: "Display fzf window below the cursor with the given height instead of using the full screen." ([fzf.1 man page](https://github.com/junegunn/fzf/blob/master/man/man1/fzf.1), DISPLAY MODE) The shell bindings default to `--height 40% --min-height 20+` (`__fzf_defaults`), i.e. **inline is the default in shell integration**.
- Exit status: `0` normal, `1` no match, `2` error, `130` interrupted with CTRL-C or ESC. ([fzf.1](https://github.com/junegunn/fzf/blob/master/man/man1/fzf.1), EXIT STATUS) Shell glue only edits the buffer on success.

## 5. fzf's shell integration

([shell/key-bindings.zsh](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.zsh), [shell/key-bindings.bash](https://github.com/junegunn/fzf/blob/master/shell/key-bindings.bash))

- **CTRL-T** (zsh): appends quoted selected paths at the cursor (`LBUFFER="${LBUFFER}$(__fzf_select)"`), then `zle reset-prompt`.
- **CTRL-R** (zsh): dedups history, seeds fzf with `--query=${(qqq)LBUFFER}`, inserts the chosen command into `BUFFER` with `CURSOR=${#BUFFER}` — **never executes**. Two implementation paths: with perl it resolves entries via the `zsh/parameter` `$history` assoc array; the fallback path fetches the real history entry with the `zle .push-line` / `zle vi-fetch-history -n <num>` / `zle .get-line` trick.
- **CTRL-R** (bash): bound as a readline *keyboard macro* containing a backtick command substitution — `bind -m emacs-standard '"\C-r": "\C-e \C-u\C-y\ey\C-u`__fzf_history__`\e\C-e\C-\e("'` (lines 164–166) — not `bind -x`. Inside `__fzf_history__`, fzf is seeded with `--query "$READLINE_LINE"` and the result is assigned to `READLINE_LINE`, with `READLINE_POINT` set to the end (lines 103–139).
- **`**<TAB>` trigger completion**: [shell/completion.zsh](https://github.com/junegunn/fzf/blob/master/shell/completion.zsh) checks whether the tail of `LBUFFER` matches `FZF_COMPLETION_TRIGGER` (default `**`) and routes through fzf instead of normal completion. It is fzf's own trigger token — bare TAB is not hijacked.

## 6. fzf-tab (zsh plugin)

[fzf-tab](https://github.com/Aloxaf/fzf-tab) "doesn't do 'complete', it just shows you the results of the default completion system" ([README](https://github.com/Aloxaf/fzf-tab/blob/master/README.md)). Mechanism: it defines `-ftb-compadd`, which shadows zsh's `compadd` builtin during completion, captures the candidate list with `builtin compadd -A __hits -D __dscr "$@"`, and pipes those candidates through fzf; configured via `zstyle ':fzf-tab:*'`. ([fzf-tab.zsh](https://github.com/Aloxaf/fzf-tab/blob/master/fzf-tab.zsh), lines 13–46)

## 7. The UX qualities that make these tools feel fast and inline

Distilled from the sources above:

1. **No screen disruption.** Inline rendering (`--height 40%`, `inline_height 40`, `Viewport::Inline`) draws below the cursor; scrollback and prompt context stay visible. Fullscreen/alt-screen is the opt-in, not the default, in shell-integration paths.
2. **Seeded query.** The current buffer (`$BUFFER` / `$READLINE_LINE` / `LBUFFER`) becomes the initial filter query — the picker opens already narrowed by what you typed.
3. **Insert, don't execute.** Both tools put the result on the command line and let the user press Enter. Execute-immediately exists only as an explicit opt-in (`enter_accept`, atuin's `__atuin_accept__:` sentinel).
4. **Cancel restores exactly.** Non-zero exit → buffer untouched; the shell widget checks status before editing (`130` for ESC/Ctrl-C in fzf).
5. **Bounded UI.** A capped height (40% / 40 lines) keeps the picker visually subordinate to the terminal rather than taking it over.

## Implications for sshm (verified blueprint)

- `sshm init zsh|bash` emitting widget scripts, modeled on atuin's `init_static` (static script + conditional bindkey lines).
- `sshm pick` subcommand: ratatui `Viewport::Inline`, TUI drawn on stdout-if-terminal else `/dev/tty` (atuin's `TerminalWriter` pattern), accepted selection emitted on the captured stream, distinct exit code for cancel.
- Prefer fzf's cleaner contract (UI→tty, selection→stdout) over atuin's fd-swap if the architecture allows; the fd-swap exists because atuin draws on stdout by default.
- Default: insert the connection's ssh command into the buffer; don't execute.
