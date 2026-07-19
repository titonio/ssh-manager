# SSH Manager

A terminal UI for managing SSH connections, built with Rust and Ratatui.

## Language

**Connection**:
A single SSH server the user can connect to — an alias, host, user, port, optional key path, and folder. The unit the whole app organizes around.
_Avoid_: entry, server, host (host is a field of a Connection, not the Connection itself)

**Inline Picker**:
The inline, filter-as-you-type selector that lists Connections below the cursor (not fullscreen) and emits the chosen one to the shell. Triggered from a shell widget, distinct from the fullscreen TUI that runs on a bare `sshm`.
_Avoid_: dropdown, popup, completion menu
