# SSH Manager

A terminal UI for managing SSH connections, built with Rust and Ratatui.

## Language

**Connection**:
A single SSH server the user can connect to — an alias, host, user, port, optional key path, and folder. The unit the whole app organizes around.
_Avoid_: entry, server, host (host is a field of a Connection, not the Connection itself)

**Inline Picker**:
The inline, filter-as-you-type selector that lists Connections below the cursor
(line, not fullscreen) and emits the chosen one to the shell. It is the one
frame every command opens (#35): bare `sshm`, `sshm pick` and `sshm manage`
all draw the same Clack grammar, differing only in what Enter means.
_Avoid_: dropdown, popup, completion menu

**Shell Widget**:
A ZLE widget (zsh) or readline function (bash) emitted by `sshm init zsh|bash`
that opens the Inline Picker. Two entry points: a bound key (Ctrl+Alt+S) that
seeds the picker from the buffer, and the `**` completion trigger that opens
the picker unfiltered. The `**<TAB>` route is zsh-only; the widget falls
through to normal `.expand-or-complete` when the trigger token is absent.
_Avoid_: override, hook, completion menu
