# The verification loop

**Never claim visual verification from code alone.** A diff shows what was
written, not what renders. This project's worst defects were invisible to a
green test suite for its entire life, because the snapshots that looked
"visual" captured glyphs and threw away every colour attribute.

Four steps. All of them, every time a UI change lands.

## 1. The deterministic gate

```bash
cd sshm && cargo test
```

`tests/design_system_test.rs` is the design-specific part: palette purity,
the Clack palette's shape and hues, hint fitting, `NO_COLOR` suppression, and
the settle-verb state set — arithmetic and grep, no LLM, no network, no
judgement. The frame's own structural claims (transparency, monochrome
glyphs, gutter alignment, width fitting) are gated in `tests/frame_test.rs`
at the seam that produces them.

Completion criterion: **0 failures**, and if a gate went from failing to
passing because the gate itself changed, that change is the thing to
scrutinise — a gate that cannot fail is worse than no gate.

## 2. Dump the frames and read them

```bash
SSHM_DUMP_FRAMES=1 cargo test --test design_system_test
ls target/design-frames/
cat target/design-frames/frame-pick-80.ansi
```

`Frame::to_ansi` re-emits the SGR runs a glyph snapshot drops, so a dumped
frame shows the real colours, bold and dim. `cat` renders it in any
truecolour terminal; the files are diffable between runs, so an unintended
palette change shows up as a byte change.

The dump covers both modes at 80 and 60 columns plus monochrome, the empty
state and the no-match state. Completion criterion: every dumped frame has
been read, and every difference from the previous run is one you intended.

## 3. Look at the live thing

A dumped frame is a still. The inline mechanics — the frame opening below the
prompt, the window sliding, the settle-collapse erasing the live list — only
exist in a running terminal:

```bash
cargo run --example inline_demo -- pick
cargo run --example inline_demo -- manage
```

For anything you want to share, `demo/inline-frame.tape` drives the same
binary through a PTY with [`vhs`](https://github.com/charmbracelet/vhs) and
produces a GIF; extract a still with Pillow (vhs 0.11 writes only
`.gif/.webm/.mp4`). Two things bite there: missing box-drawing coverage in
the font renders `─ │ ┌` as tofu, which reads as a broken UI rather than a
missing glyph (`fc-list : family | grep -i mono`; Noto Sans Mono covers
U+2500), and go-rod's browser lookup can be forced with
`ROD=bin-path=/usr/bin/chromium-browser`.

## 4. The terminal matrix

The responsive breakpoints of this medium. The same frame, four ways:

| Mode | How | What breaks |
|---|---|---|
| Truecolour | `COLORTERM=truecolor` | baseline |
| 256-colour | `TERM=xterm-256color COLORTERM=` | palette drift, banding |
| 16-colour | `TERM=xterm` | roles collapsing into one colour — selection can vanish |
| No colour | `NO_COLOR=1` | anything carried by colour alone stops communicating |

`./scripts/design-matrix.sh` runs the gate once per mode by setting the
environment `ColorSupport::detect()` reads. Unsetting `NO_COLOR` on the
colour passes is load-bearing: a non-interactive shell usually carries
`NO_COLOR=1` and `TERM=dumb`, and without the unsets every "colour" pass
silently runs monochrome and the matrix proves nothing.

The named gates that carry the arithmetic —
`named_colours_survive_the_reduced_modes_unmapped`,
`every_canvas_colour_mode_still_renders_the_grammar`,
`clack_degrades_to_16_colours_without_losing_the_state_carrying_roles`,
`the_monochrome_frame_keeps_every_glyph_and_modifier_that_carries_state`,
`a_monochrome_canvas_emits_no_colour_at_all`. The matrix is for what
arithmetic cannot see: whether the design still *communicates* once the
colour is gone.

## The snapshots that remain

There is no `.snap` suite. The only insta snapshots are **inline** in
`src/main.rs`, holding the generated shell init scripts. When one changes,
read the diff before `cargo insta test --accept`: that script's failure mode
is a broken shell, and a snapshot accepted unread is a bug promoted to a
baseline — that is exactly how the 2.34:1 selection shipped.

## What counts as done

A UI change is verified when **all four** are true: the detector is green,
every dumped frame has been read and its diff is intentional, the live frame
has been looked at in a real terminal, and the reduced-colour modes still
communicate. Three of four is not done.
