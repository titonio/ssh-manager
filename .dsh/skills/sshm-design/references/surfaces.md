# The surfaces

sshm has **one drawing surface and three commands that open it**, plus the
shell glue that opens it for you. The fullscreen TUI is gone: #35 deleted the
`App` render path, `sshm/src/app.rs`, and the alternate screen with them.
Nothing in the binary takes over the terminal any more.

## 1. The inline frame — the only thing that draws

`build_frame(connections, query, selection, mode, canvas)` in
`sshm/src/frame.rs` turns its inputs into a `Frame`: a value, not a terminal
session. It owns the Clack grammar — `◆` step icon, `│` rail, `❯` cursor,
rows as `[folder] alias (user@host:port)`, the empty / no-match / hint-rail
states — and nothing else.

- **It owns no background at all.** Not "sets a dark one" — none. Body text
  is `Reset`, so the terminal's own foreground is what makes the frame
  readable on the terminal's own background. The WCAG table that governed
  Nord cannot govern this surface; the structural rules in `tokens.md`
  replace it, gated by `no_span_in_any_frame_sets_a_background` and
  `the_serialized_frame_asks_for_no_background_colour`.
- **Selection is `❯` plus a bold alias**, never a filled row. There is
  nothing to fill, and a hue alone vanishes under `NO_COLOR`.
- **The hint rail must be width-fitted.** `build_frame` runs it through
  `fit_hints_with` against `canvas.fit_width() - GUTTER`, dropping whole
  segments from the least-needed end. The full Manage rail cannot fit an
  80-column terminal; unfitted it clipped mid-word (`Ctrl+X dele`). Order is
  the contract: escape hatch → Enter → movement → discovery.
- **Colour is an input, not a constant.** The frame draws
  `Theme::clack().resolve(canvas.support)`. Under `NO_COLOR` or `TERM=dumb`
  the whole palette becomes `Reset` before a span exists. Glyphs and
  `BOLD`/`DIM` carry every state, so the frame says the same thing with no
  colour at all.
- **Gutters are shared.** Rows spend four columns before their text
  (`│ ` + cursor + ` `); state and hint lines spend the same four. A line
  that spends two sits visibly left of the rows it belongs to.
- **The row text has one source.** `build_row_text` is the row's display
  text; `compute_field_offsets` indexes it and `row_line` cuts its spans out
  of it at those same boundaries. The matcher's hit offsets and the drawn
  characters are the same bytes — `row_line` used to format the row a second
  time by hand, and the two could drift with nothing noticing.

Glyph inventory and role mapping: `tokens.md` → "Clack grammar glyphs".

### Its live half

`sshm/src/inline.rs` makes the value live inside somebody's shell. Four rules
bind it: never the alternate screen (it never builds a ratatui `Terminal`),
constant height while active, leave the line clean on every exit, and one
stream — everything it writes goes to the writer it was handed, never to a
hardcoded stdout.

**Height and the size gate.** The frame is `FRAME_LINES` = 8 list rows + 3
chrome lines = 11, and it always leaves one line for the user's own prompt.
`fit_visible_rows(terminal_height)` returns `None` when the terminal cannot
hold a frame at all, and both the open path and the resize path fall back to
a clean cancel-and-restore rather than drawing a degenerate frame. (The old
"no minimum-size gate" gap on the fullscreen surface is closed here; there is
no surface left that squeezes silently.)

## 2. The three commands — the `--emit` axis

All three open the same frame and read keys from the terminal. They differ in
exactly one thing: what Enter means. That difference is modelled once, in
`sshm/src/emit.rs`, and `main.rs` never re-derives it.

| Command | `Emit` | Frame | Enter does |
|---|---|---|---|
| `sshm` (bare) | `Execute` | Pick | Runs `ssh` against the selected Connection |
| `sshm pick` | `Insert` | Pick | Writes the alias alone to stdout, for the shell to insert |
| `sshm manage` | `Edit` | Manage | Routes the selection to the edit path |

- **`sshm manage` does not edit anything yet.** Enter resolves to
  `Action::Edit(conn)` and the runner writes nothing: the edit itself is
  #36/#37. The frame's `Enter edit` hint names the route, not a completed
  change, and the runner deliberately adds no settle verb claiming a file
  moved.
- **Cancel is a promise.** Esc or Ctrl-C resolves to `Cancelled` under every
  emit and exits 130. It must leave the shell buffer exactly as it was. Not
  "mostly". The user's half-typed command is the contract.
- **Stream split.** `emit::frame_stream(stdout_is_terminal)` decides where
  the frame draws: stdout when stdout *is* the terminal, `/dev/tty` when it
  is captured. Under `result=$(sshm pick …)` the pipe carries only the
  emitted alias and the escape sequences reach the real screen. Break this
  and you corrupt the user's command line. Keys already come from the
  terminal: crossterm reads stdin when it is a tty and opens `/dev/tty`
  itself when it is not.
- **One settle line.** Both a pick and a cancel leave a trace
  (`◆ picked …` / `◆ cancelled`) — one line, for all three commands. The
  emit axis decides what happens to the pick *after* the frame is gone; it
  must not print a second one.

The remaining commands (`add`, `init <shell>`, `completions <shell>`,
`check-update`) draw nothing.

## 3. Shell Widget — `sshm init zsh|bash`

Not a UI. It is emitted shell code that binds a key and splices the picker's
output into the line editor.

- **Its failure mode is a broken shell.** A bug here does not show a bad
  screen; it makes the user's terminal misbehave. Treat it as the
  highest-risk surface despite containing no rendering.
- **Two entry points, one contract.** The bound key (default Ctrl+Alt+S)
  seeds the picker's query from the current buffer; the `**<TAB>` trigger
  strips its own trigger token and seeds the query from what is left. Both
  put the chosen **alias** on the line and neither executes it — the widget
  inserts, it does not run.
- **The `**<TAB>` fallthrough differs by shell, and the difference is real.**
  In zsh, when the trigger token is absent the widget chains to
  `zle .expand-or-complete`, so Tab still does normal completion. In bash a
  `bind -x` function cannot chain back to normal completion, so Tab is bound
  to the picker and **does nothing** unless the line ends with `**`. Any
  change must preserve the zsh fallthrough; the bash limitation is a property
  of readline, not something this code can fix.
- **Configurable binding** via `SSHM_BIND_KEY`, in zsh notation for zsh
  (`\e^S`) and readline notation for bash (`\e\C-s`) — the two are not
  interchangeable. `--no-bind` (or `SSHM_NO_BIND=1`) suppresses the bind
  lines entirely.

## Designing across them

A change that touches a shared component (a row, a hint, a colour) lands on
every command at once, because there is only one frame. Check the frame at
80×24 and wide, in truecolour and under `NO_COLOR`, and check the emit axis
separately: the same Enter is a session on one command, a string on another,
and a route to code that has not been written on the third.
