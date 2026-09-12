# sshm design matrix — one PNG per colour mode
#
# Renders the ANSI frames dumped by ./scripts/design-matrix.sh (real ratatui
# render output via theme::ansi::buffer_to_ansi) into PNGs a human can review.
#
# The frame to render is passed through the environment: the typed
# `cat $SSHM_FRAME` is expanded by the shell *inside* the emulated terminal,
# which inherits vhs's environment. So this single tape drives all eight
# renders.
#
# NOTE: vhs 0.11.0 only recognises .gif/.webm/.mp4 output extensions — a
# `-o *.png` run exits 0 and writes nothing. So each render goes through a
# GIF (vhs's native encoder, no ffmpeg needed) and the FINAL frame is then
# extracted to PNG with Pillow. Exact commands (run from the repo root):
#
#   SSHM_FRAME=sshm/target/design-frames/picker-Truecolor.ansi \
#     vhs -o /tmp/picker-truecolor.gif demo/design-matrix.tape
#   python3 -c "from PIL import Image, ImageSequence; \
#     f=[x.convert('RGB') for x in ImageSequence.Iterator(Image.open('/tmp/picker-truecolor.gif'))]; \
#     f[-1].save('demo/rendered/picker-truecolor.png')"
#
# Requires vhs (go-rod) + python3-pillow. vhs finds the system chromium at
# /usr/bin/chromium-browser via go-rod's known-path lookup, so it does not
# download its own. If that ever regresses, force it with:
#   ROD=bin-path=/usr/bin/chromium-browser vhs ...

Set Width 760
Set Height 470
Set Theme "nord"
Set FontFamily "Noto Sans Mono"
Set FontSize 13
Set Padding 24
Set TypingSpeed 0

# The frame file is supplied via SSHM_FRAME so one tape covers every mode.
Type "cat $SSHM_FRAME"
Enter
Sleep 1
