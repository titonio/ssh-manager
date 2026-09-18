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
  segments from the least-needed end. The 87-column Manage rail that #36
  first shipped could not fit an 80-column terminal and clipped mid-word
  (`Ctrl+X dele`); the rail is short enough for 80 now, but the fitting
  rule is what makes that safe at *any* width. Order is the contract:
  escape hatch → Enter → movement → the action. And nothing on the rail
  that does not do what its label says — see the manage bullets below.
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

- **`sshm manage` deletes and edits for real.** `Ctrl+X` raises the inline
  `◆ Delete … ? (y/N)` confirm and a `y` answer persists the removal
  through `connections::Store::remove` — the file on disk changes, and the
  frame returns to the list with the settled
  `■ Delete [prod] web-01? Yes` and the dim `◇ deleted [prod] web-01`
  note. `Ctrl+E` opens the in-place single-field editor (#37): the header
  names the captured target (`◆ Edit [prod] web-01`), one field rides its
  own line seeded from its current value, `←→`/`Tab` move between the five
  fields, and Enter writes that one field through
  `connections::Store::update` — returning to the refreshed list with
  `◇ edited [prod] web-01x`. Esc abandons with
  `◇ edit abandoned — nothing saved`; a target the store no longer has
  settles to `◇ edit failed — no such Connection: …`, never as `edited`.
- **Enter routes; `Ctrl+E` edits.** Enter resolves to
  `Action::Edit(conn)` and leaves the frame with the selection routed to
  the emit path — which writes nothing. That is why `Enter edit` is *not*
  on the manage rail: the route is not a completed change, and a label
  that promises "edit" on a key that only routes teaches a binding that
  does not do what it says. `Ctrl+E` is the chord that keeps the promise,
  and it is on the rail with the behaviour that backs it.
- **`Ctrl+A` walks the five-step add sequence and writes.** The chord
  opens the Clack sequence `◆ Alias` → `◆ Host` → `◆ Port` → `◆ Key` →
  `◆ Folder`, each step settling to a `◇ <label>  <value>` line, with
  per-field validation (Alias/Host required; Port empty means 22,
  out-of-range refused; Key/Folder empty settle absent). The last step's
  Enter persists through `connections::Store::add` and returns to the
  list with `◇ added [prod] web-01`. Backing out leaves
  `◇ add abandoned — nothing saved`.
- **The manage rail lists only keys that work in the current build.**
  `Esc cancel · ↑↓ navigate · Ctrl+X delete · Ctrl+A add · Ctrl+E edit`.
  Two gates hold that: `assert_advertised_chords_are_live` (a hinted chord
  must be one `manage::step` acts on) and
  `the_manage_rail_advertises_only_the_keys_that_work` (a hinted chord
  must do the thing its label names). `Enter edit` is off the rail for
  the reason above: six hints do not fit the 75 columns an 80-column
  terminal leaves, and drop-from-the-end would cut `Ctrl+E` — the chord
  that really edits — first.
- **A failed delete collapses the frame; it does not abandon it.** When the
  store refuses the removal the frame settles to
  `◆ error  [prod] web-01  (deploy@10.0.0.4:22)  — <the store's reason>`
  and the process exits non-zero. Before this the error escaped with
  eleven painted rows and no trace, over a list the store had just proved
  it could not vouch for. `error` is a state in #31's `Settled` set and
  story 34 requires it be designed; this is that design.
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
- **One settle line.** Every exit leaves a trace
  (`◆ picked …` / `◆ cancelled` / `◆ error …`) — one line, for all three
  commands. The emit axis decides what happens to the pick *after* the
  frame is gone; it must not print a second one.

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
  (zsh only) strips its own trigger token and seeds the query from what is
  left. Both put the chosen **alias** on the line and neither executes it —
  the widget inserts, it does not run.
- **The `**<TAB>` fallthrough differs by shell, and the difference is real.**
  In zsh, when the trigger token is absent the widget chains to
  `zle expand-or-complete` — the non-dot widget form, so the completion
  system runs. The dot form (`zle .expand-or-complete`) invokes the raw ZLE
  builtin, bypasses compsys and completes nothing (#23). In bash a
  `bind -x` function cannot chain back to normal completion, so bash never
  rebinds TAB: binding it would cost all of bash's completion for a trigger
  that cannot fall through. The bash limitation is a property of readline,
  not something this code can fix — the fix is not to bind it.
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
