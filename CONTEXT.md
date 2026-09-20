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
through to the standard `expand-or-complete` widget (non-dot form, so the
completion system runs) when the trigger token is absent. bash never rebinds
TAB — `bind -x` cannot chain back, so the binding would cost all of bash's
completion.
_Avoid_: override, hook, completion menu

**Form Map**:
The six-row surface that both `Ctrl+A` and `Ctrl+E` open — Alias, Host, User,
Port, Key, Folder, each row carrying a glyph that states its condition
(`✓` valid, `●` changed from what is stored, `○` required and empty, `!`
will not validate, `·` optional and empty), with the cursor row wearing `◆`.
`Ctrl+A` opens it blank; `Ctrl+E` opens it seeded from the selected
Connection. It is one surface with two starting drafts, not two forms — the
distinction is what keeps the keymap honest.
_Avoid_: stepper, wizard, form, multi-step prompt, edit screen

**Submit Row**:
The `▶` row at the foot of the Form Map, and the only place the form can
commit. It is a row the cursor can be *on*, which is what lets Enter mean
"act on the row you are standing on" everywhere. `▶ Add connection` on a
new Connection, `▶ Save changes to <alias>` on an existing one.
_Avoid_: button, footer action, confirm key

**Live Draft**:
The Form Map's content model: every keystroke writes straight into the field
under the cursor, so a drawn row *is* the value rather than a view of a
value that must be settled first. Normalisation (trim, the port's
empty-to-`22`) is deferred to the Submit Row, so the map never displays a
value the user did not type.
_Avoid_: pending input, staged field, settled line

**Update Note**:
The `◆ update available: v… — run sshm update` line a frame draws above
itself. Read once from the cache before the frame opens, never from the
network.
_Avoid_: banner, notification, popup, reminder

**Check for Updates** / **Apply Update**:
The two acts the bare word "update" has been covering. Checking answers
_is there a newer release?_ and changes nothing. Applying replaces the
installed binary, and is the only act `sshm update` performs. Never write
"update" unqualified.
_Avoid_: auto-update, self-update, upgrade

**Dev Build**:
A binary running from a source checkout rather than an installed location.
It is out of sshm's custody: it is never replaced, and it is never offered an
Update Note.
_Avoid_: debug build, local build, cargo build
