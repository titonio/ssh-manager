# sshm — the README hero reel
#
# The capability story in one loop: the inline picker opening below your own
# prompt, filter-as-you-type, the settle-collapse that leaves a single line
# where a live list was, then manage mode's add form-map and inline delete
# confirm.
#
# This drives the REAL `sshm` binary — not a `cat` of captured frames (that
# reel is demo/sshm-design.tape) — so the collapse, the rail swap and the
# ✓-walking field map are the actual mechanics, not a slideshow.
#
# Render it with the wrapper, which builds the crate, resets a sandbox from
# demo/fixtures/connections.json, puts `sshm` on PATH inside it and points
# HOME at it:
#
#   scripts/render-hero.sh                 # -> demo/hero.gif
#   scripts/render-hero.sh /tmp/hero.gif  # anywhere else
#
# Why a sandbox: bare `sshm` *executes* ssh on Enter (Emit::Execute), and
# `sshm manage` writes the real store. The reel therefore uses `sshm pick`
# (Enter emits the alias, executes nothing) and runs under a throwaway HOME,
# so the user's ~/.ssh/connections.json is never opened. The add/delete pair
# is a round trip on the fixture: web-03 goes in, web-03 comes out, the
# store ends exactly where it started — which the wrapper asserts.
#
# Sizing notes for this vhs (0.11.0): it takes pixel dimensions, not
# character cells. 800x430 with this font is a ~95x23 terminal, which holds
# the 11-line frame plus the scrollback that proves the frame is inline
# rather than a takeover — and no more, so the GIF carries no dead space.
#
# The filter query is `prod`, not something shorter, on purpose: the frame
# holds its height when it narrows (that is the design), so a query that
# lands on fewer than the 8 visible rows leaves a hole where the list was.
# Eight production hosts fills the window exactly.
#
# Requires vhs: https://github.com/charmbracelet/vhs

Set Width 800
Set Height 430
Set Theme "nord"
Set FontFamily "Noto Sans Mono"
Set FontSize 14
Set Padding 24
Set TypingSpeed 0.04
# 25fps rather than vhs' 60: a terminal reel has almost no motion between
# frames, and a third of the rate is most of the saved bytes on a GIF that
# has to load on a README.
Set Framerate 25

# Scrollback above the frame. Their job is to make "the shell is still here,
# the frame borrowed a patch of it" visible rather than asserted — and
# `ls ~/.ssh` puts the very file sshm manages on screen before the frame
# that manages it appears.
Type "echo 'sshm — inline SSH connection manager'"
Enter
Type "ls ~/.ssh"
Enter
Sleep 0.5

# ── Beat 1 · the frame opens below the prompt ────────────────────────
# No alternate screen: the lines above stay put and the frame is drawn into
# the lines right under the command.
Type "sshm pick"
Enter
Sleep 2.0

# ── Beat 2 · filter as you type ──────────────────────────────────────
# Seventeen connections down to the production eight, matched on the folder
# field. The frame keeps its height — the window slides, the shell never
# jumps.
Type "prod"
Sleep 1.8

# ── Beat 3 · select: the live list collapses to one line ─────────────
# The signature move. Eight rows of list erase themselves into a single
# `◆ picked` settle and the shell resumes right beneath it.
Down
Sleep 0.6
Enter
Sleep 2.4

# ── Beat 4 · manage mode: same frame, different rail ─────────────────
# One surface, one key set — only the header and the chord hints change.
Type "sshm manage"
Enter
Sleep 1.8

# ── Beat 5 · Ctrl+A opens the add form-map ───────────────────────────
# Every field visible at once, the active one under ◆, the rest waiting.
Ctrl+A
Sleep 1.4

# ── Beat 6 · fill it: ✓ walks down the map as each field lands ───────
Type "web-03"
Enter
Sleep 0.55
Type "10.0.0.6"
Enter
Sleep 0.55
Type "deploy"
Enter
Sleep 0.55

# Port and Key are optional — Enter on an empty field steps past them.
Enter
Enter
Type "prod"
Enter
Sleep 0.7

# The last field's hint reads `Enter save`, and it does: back to the list
# with `◇ added [prod] web-03` and the new row under the cursor.
Enter
Sleep 2.0

# ── Beat 7 · Ctrl+X raises the inline confirm ────────────────────────
# `■ Delete [prod] web-03? (y/N)` — asked in place, no dialog, no popup.
Ctrl+X
Sleep 1.6

# ── Beat 8 · y — deleted, and the settle says so ─────────────────────
# A real delete, off the fixture store. The list closes over the gap.
Type "y"
Sleep 2.6
