# sshm design review — the full inline frame set (#40)
#
# Turns the frames produced by the design-system dump into a shareable GIF, so
# a UI review can be looked at rather than described.
#
# Deterministic by construction: it cats captured frames instead of driving the
# live TUI, so there is no typing-timing flake. The frames themselves come from
# ratatui's real render path via `build_frame` / `build_frame_with_flow` and
# `Frame::to_ansi` (see `SSHM_DUMP_FRAMES=1 cargo test --test design_system_test`),
# so what you see is what the app draws — the one frame shared by `sshm`,
# `sshm pick` and `sshm manage`.
#
# It covers the surfaces the old reel (fullscreen TUI + old opaque picker)
# was built for, rebuilt for the Clack inline set:
#   - the picker and its states (empty, no-match)
#   - manage and its in-list delete confirm + delete/error settles
#   - the #37 in-place edit sequence and its refusal/error settles
#   - the #36 add step-sequence and its refusal
#
# Requires vhs: https://github.com/charmbracelet/vhs
#
#   cd sshm && SSHM_DUMP_FRAMES=1 cargo test --test design_system_test
#   cd .. && vhs demo/sshm-design.tape      # -> demo/sshm-design.gif

# NOTE: this vhs (0.11.0) takes pixel dimensions, not character cells —
# 760x470 with this font is a ~97x21 terminal, comfortably holding the 11-line
# frame above the shell prompt.
Set Width 760
Set Height 470
Set Theme "nord"
Set FontSize 13
Set Padding 24
Set TypingSpeed 0.02

# The picker: selected row carries the `❯` cursor, the fuzzy hit is green.
Type "cat sshm/target/design-frames/frame-pick-80.ansi"
Enter
Sleep 2
Ctrl+L

# Pick — empty state: the frame's call to action when there is nothing.
Type "cat sshm/target/design-frames/frame-empty.ansi"
Enter
Sleep 2
Ctrl+L

# Pick — no-match state: structure survives a query that matches nothing.
Type "cat sshm/target/design-frames/frame-no-match.ansi"
Enter
Sleep 2
Ctrl+L

# Manage: the same grammar, the five workable chords on the rail.
Type "cat sshm/target/design-frames/frame-manage-80.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — Ctrl+X delete confirm: `◆ Delete [prod] web-01? (y/N)`.
Type "cat sshm/target/design-frames/frame-manage-confirm.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — the settled delete note: `◇ deleted [prod] web-01`.
Type "cat sshm/target/design-frames/frame-manage-deleted.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — #37 in-place edit: header names the target, field rides its line.
Type "cat sshm/target/design-frames/frame-manage-edit.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — delete refused by the store: `◇ delete failed — no such Connection`.
Type "cat sshm/target/design-frames/frame-manage-delete-failed.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — the settled edit note: `◇ edited [production] prod-server`.
Type "cat sshm/target/design-frames/frame-manage-edited.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — edit refused: the warning glyph before the caret, not a lie.
Type "cat sshm/target/design-frames/frame-manage-edit-refused.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — edit refused by the store: `◇ edit failed — no such Connection`.
Type "cat sshm/target/design-frames/frame-manage-edit-failed.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — the `◆ error` settle: a refused write collapses to one clear line.
Type "cat sshm/target/design-frames/frame-manage-error-settle.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — #36 add step-sequence mid-flight: `◆ Alias  web-03_`.
Type "cat sshm/target/design-frames/frame-manage-add.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — the add sequence settled: `◇ <label>  <value>` walks, `—` for absent.
Type "cat sshm/target/design-frames/frame-manage-add-settled.ansi"
Enter
Sleep 2
Ctrl+L

# Manage — add refused: `alias is required`, the `!` the refusal wears.
Type "cat sshm/target/design-frames/frame-manage-add-refused.ansi"
Enter
Sleep 2
Ctrl+L

# The two capability-matrix tiers the design has to survive (see
# scripts/design-matrix.sh): the picker snapped to 60 columns, where the rail
# drops whole labels, and the same picker under NO_COLOR, where every role
# becomes Reset and the glyphs/modifiers carry the state alone.
Type "cat sshm/target/design-frames/frame-pick-60.ansi"
Enter
Sleep 2
Ctrl+L

Type "cat sshm/target/design-frames/frame-pick-80-mono.ansi"
Enter
Sleep 2

# To capture the live app instead (interactive, timing-sensitive — not for CI),
# use demo/inline-frame.tape (pick) and demo/inline-manage.tape (manage).
