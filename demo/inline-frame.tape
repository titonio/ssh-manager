# The inline frame (#34) rendered in a real terminal
#
# Drives the live inline picker — not a `cat` of a captured frame — so what
# lands in this recording is the actual inline mechanics: the frame opening
# below the prompt, the 8-row window sliding, and the settle-collapse erasing
# the live list.
#
# The binary is passed through the environment so the tape holds no absolute
# paths (same convention as design-matrix.tape: the inner shell expands it):
#
#   cargo build --example inline_demo
#   SSHMDemo=$PWD/sshm/target/debug/examples/inline_demo \
#     vhs -o /tmp/inline-frame.gif demo/inline-frame.tape
#
# NOTE: this vhs (0.11.0) takes pixel dimensions, not character cells —
# 810x460 with this font is a 93x22 terminal — and its `Screenshot` command
# is a no-op (same build quirk design-matrix.tape documents for `-o *.png`).
# Individual moments are therefore extracted from the GIF afterwards with
# Pillow; the four beats below are separated by long sleeps so their frames
# are easy to find:
#
#   python3 -c "from PIL import Image, ImageSequence; \
#     f=[x.convert('RGB') for x in ImageSequence.Iterator(Image.open('/tmp/inline-frame.gif'))]; \
#     f[-1].save('demo/rendered/inline-settled.png')"
#
# For the authoritative text-level check of every beat (open, slide, narrow,
# settle, cancel, resize), use scripts/inline-pty-check.py — it prints the
# screen buffer after each step.
#
# Requires vhs (https://github.com/charmbracelet/vhs).

Set Width 810
Set Height 460
Set Theme "nord"
Set FontFamily "Noto Sans Mono"
Set FontSize 13
Set Padding 20
Set TypingSpeed 0.03

# Some history above, so "the scrollback is still there" is visible rather than
# asserted. Echoes, not real commands: the tape's timing must not depend on a
# build finishing before the next keystroke lands.
Type "echo cargo build --release"
Enter
Type "git log --oneline -3"
Enter
Sleep 0.2

# Beat 1 — open the frame below the prompt. (No decorative `$ ` typed: the
# inner shell would try to execute it. The shell's own prompt is on screen.)
Type "$SSHMDemo pick"
Enter
Sleep 1.5

# Beat 2 — narrow the list: the frame must not change height.
Type "web"
Sleep 1.5

# Beat 3 — widen it back and walk the selection down through the full list.
# (Special keys are standalone commands in this vhs dialect; `Type "backspace"`
# would type the literal word.)
Backspace
Backspace
Sleep 0.4
Down
Down
Down
Down
Down
Down
Sleep 1.5

# Beat 4 — submit: the live list collapses to the settle trace.
Enter
Sleep 1.5
