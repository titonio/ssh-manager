# The three surfaces

sshm ships three products in one binary. They share a palette and a fuzzy matcher
and almost nothing else. Treating one like another is where the design goes wrong.

## 1. Fullscreen TUI — `sshm`

Takes over the whole terminal via the alternate screen. Owns every cell.

- **Layout**: four vertical chunks — header (3), search bar (3), list (min 0),
  footer (3). `App::render` in `sshm/src/app.rs`.
- **Modes**: `Normal`, `Add`, `Edit`, `Search`, `Help`, `Update`. Each mode owns
  its footer hint set, and the footer is the only place the user learns what keys
  do right now. A mode with a stale footer is a broken mode.
- **Selection**: `>` prefix built into the row content, plus `highlight` colour and
  bold. Both the marker and the colour are load-bearing.
- **Owns its background.** Every panel sets `t.bg` explicitly, so contrast is
  known and controlled.

**Open gap**: there is no minimum-size gate and no `Event::Resize` handler. At very
small sizes the layout squeezes without complaint and hint text disappears. If you
touch layout, add the gate rather than leaving it implicit.

## 2. Inline Picker — `sshm pick`

Draws *inside the user's live shell session*, above their prompt, and hands back a
single string. This is the surface with the most constraints and the least room to
recover.

- **It does not own the terminal.** It borrows rows from the shell. The prompt line
  and everything above it belong to the user and must survive untouched.
- **It inherits the user's background.** `render_picker_frame` sets no panel
  background, so whatever the user's terminal theme is shows through. Any colour
  used here must be readable against an *unknown* background — which is why the
  selected row sets its own opaque `selection_bg` rather than relying on a tint.
- **Cancel is a promise.** Esc or Ctrl-C must leave the shell buffer exactly as it
  was. Not "mostly". The user's half-typed command is the contract.
- **Stream split.** The accepted selection goes to a capturable stream; the UI
  goes to the terminal. When stdout is captured (`result=$(sshm pick …)`) the TUI
  routes to `/dev/tty` so escape sequences reach the real screen and the pipe
  carries only the selection. Break this and you corrupt the user's command line.
- **Height**: 15 rows by default (`DEFAULT_HEIGHT`), inline — not fullscreen.
  atuin's `inline_height` is the precedent: inline by default, fullscreen only on
  request.
- **Selection marker**: a `> ` `Span` prepended in `build_picker_row_spans`.
  `List::highlight_symbol` is inert here because the list renders statelessly —
  that is how the marker silently vanished for the life of the project.

## 3. Shell Widget — `sshm init zsh|bash`

Not a UI. It is emitted shell code that binds a key and splices the picker's
output into the line editor.

- **Its failure mode is a broken shell.** A bug here does not show a bad screen; it
  makes the user's terminal misbehave. Treat it as the highest-risk surface despite
  containing no rendering.
- **Must fall through.** The `**<TAB>` trigger is zsh-only; when the trigger token
  is absent the widget defers to the normal `.expand-or-complete`. Any change must
  preserve that fallthrough or it hijacks the user's Tab key.
- **Two entry points, one contract.** The bound key (default Ctrl+Alt+S) seeds the
  picker from the current buffer; `**<TAB>` opens it unfiltered. Both insert the
  `ssh` command at the cursor and neither executes it.
- **Configurable binding** via `SSHM_BIND_KEY`, in zsh notation for zsh and
  readline notation for bash — the two are not interchangeable.

## Designing across them

A change that touches a shared component (a row, a hint, a colour) lands on all
three at once. Check each surface separately before calling it done — the picker's
unknown background, the TUI's known one, and the widget's total absence of one
make the same choice behave three different ways.
