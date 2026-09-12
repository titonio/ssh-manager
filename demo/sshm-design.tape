# sshm design capture
#
# Turns the frames produced by the design matrix into a shareable GIF, so a UI
# review can be looked at rather than described.
#
# Deterministic by construction: it cats a captured frame instead of driving the
# live TUI, so there is no typing-timing flake in CI. The frame itself came from
# ratatui's real render path via theme::ansi::buffer_to_ansi, so what you see is
# what the app draws.
#
# Requires vhs: https://github.com/charmbracelet/vhs
#
#   ./scripts/design-matrix.sh          # writes sshm/target/design-frames/*.ansi
#   vhs demo/sshm-design.tape           # -> demo/sshm-design.gif

Set Width 92
Set Height 18
Set Theme "Nord"
Set FontSize 13
Set Padding 24
Set TypingSpeed 0.02

# Inline picker: selected row with a fuzzy match highlighted.
Type "cat sshm/target/design-frames/picker-Truecolor.ansi"
Enter
Sleep 3
Clear

# Fullscreen TUI: header, search, empty list, footer hints.
Type "cat sshm/target/design-frames/tui-Truecolor.ansi"
Enter
Sleep 3
Clear

# The same picker after the 16-colour downgrade — this is the frame that shows
# whether the design still communicates once the palette is snapped down.
Type "cat sshm/target/design-frames/picker-Ansi16.ansi"
Enter
Sleep 3

# To capture the live app instead (interactive, timing-sensitive — not for CI):
#   Type "sshm pick"
#   Enter
#   Sleep 1
#   Down
#   Sleep 0.5
#   Type "prod"
#   Sleep 2
