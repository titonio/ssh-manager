# The inline manage frame (#36/#37) rendered in a real terminal
#
# Drives the live manage frame — not a `cat` of a captured frame — so what
# lands in this recording is the actual manage mechanics: the frame opening
# in Manage mode, the Ctrl+A add step-sequence walking a field at a time,
# and the Ctrl+X inline delete confirm settling to `◇ deleted`.
#
# It complements demo/sshm-design.tape (which cats the captured frames for a
# deterministic review reel) and demo/inline-frame.tape (the live picker).
#
# The binary is passed through the environment so the tape holds no absolute
# paths (same convention as the other tapes: the inner shell expands it):
#
#   cargo build --example inline_demo
#   SSHMDemo=$PWD/sshm/target/debug/examples/inline_demo \
#     vhs -o /tmp/inline-manage.gif demo/inline-manage.tape
#
# NOTE: as with the other tapes, this vhs (0.11.0) takes pixel dimensions, and
# its `Screenshot` command is a no-op — individual moments are therefore
# extracted from the final GIF afterwards with Pillow. The beats are separated
# by long sleeps so their frames are easy to find; representative stills
# already extracted into demo/rendered/ are manage-add-alias.png,
# manage-delete-confirm.png and manage-deleted.png. E.g.:
#
#   python3 -c "from PIL import Image, ImageSequence; \
#     f=[x.convert('RGB') for x in ImageSequence.Iterator(Image.open('/tmp/inline-manage.gif'))]; \
#     f[-1].save('demo/rendered/manage-deleted.png')"
#
# The demo mutates an Ephemeral store (never the user's connections.json), so
# the delete here is safe by construction.
#
# Requires vhs (https://github.com/charmbracelet/vhs).

Set Width 810
Set Height 460
Set Theme "nord"
Set FontFamily "Noto Sans Mono"
Set FontSize 13
Set Padding 20
Set TypingSpeed 0.03

# Some scrollback above, so "the inline frame leaves the shell visible" reads.
Type "echo cargo build --release"
Enter
Type "git log --oneline -3"
Enter
Sleep 0.2

# Beat 1 — the manage frame opens in Manage mode (the manage rail is visible).
Type "$SSHMDemo manage"
Enter
Sleep 1.5

# Beat 2 — Ctrl+A starts the add sequence: `◆ Alias` is the first step.
Ctrl+A
Sleep 1.2

# Beat 3 — type an alias and Enter to the Host step; the `◇` settle appears.
Type "web-03"
Sleep 0.4
Enter
Sleep 1.2

# Beat 4 — a printable in the Host field filters nothing; then back out with
# Ctrl+C so the sequence closes cleanly (the store was never mutated yet).
Type "10"
Sleep 0.4
Ctrl+C
Sleep 1.5

# Beat 5 — reopen and Ctrl+X on the selected row to raise the inline confirm.
Type "$SSHMDemo manage"
Enter
Sleep 1.5
Ctrl+X
Sleep 1.2

# Beat 6 — `y` at the confirm deletes for real and settles to `◇ deleted`.
Type "y"
Sleep 1.5
