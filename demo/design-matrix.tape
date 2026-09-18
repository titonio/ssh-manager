# sshm design matrix — one PNG per rendered frame
#
# Renders an ANSI frame dumped by the design-system gate (real ratatui render
# output via `Frame::to_ansi`) into a PNG a human can review. The new frame set
# is produced by `SSHM_DUMP_FRAMES=1 cargo test --test design_system_test`,
# which names the frames `frame-pick-80`, `frame-manage-add-settled`,
# `frame-empty`, ... — the picker, manage, add sequence and state surfaces.
#
# The frame to render is passed through the environment: the typed
# `cat $SSHM_FRAME` is expanded by the shell *inside* the emulated terminal,
# which inherits vhs's environment. So this single tape drives every render.
#
# NOTE: vhs 0.11.0 only recognises .gif/.webm/.mp4 output extensions — a
# `-o *.png` run exits 0 and writes nothing. So each render goes through a
# GIF (vhs's native encoder, no ffmpeg needed) and the FINAL frame is then
# extracted to PNG with Pillow. Exact commands (run from the repo root):
#
#   cd sshm && SSHM_DUMP_FRAMES=1 cargo test --test design_system_test && cd ..
#   SSHM_FRAME=sshm/target/design-frames/frame-pick-80.ansi \
#     vhs -o /tmp/frame-pick-80.gif demo/design-matrix.tape
#   python3 -c "from PIL import Image, ImageSequence; \
#     f=[x.convert('RGB') for x in ImageSequence.Iterator(Image.open('/tmp/frame-pick-80.gif'))]; \
#     f[-1].save('demo/rendered/frame-pick.png')"
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

# The frame file is supplied via SSHM_FRAME so one tape covers every render.
Type "cat $SSHM_FRAME"
Enter
Sleep 1
