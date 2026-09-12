# The verification loop

**Never claim visual verification from code alone.** A diff shows what was written,
not what renders. This project's worst defects were invisible to a green test suite
for its entire life, because the snapshots that looked "visual" captured glyphs and
threw away every colour attribute.

Four steps. All of them, every time a UI change lands.

## 1. The deterministic gate

```bash
cargo test --test design_system_test
```

Twelve rules, no LLM, no network, no judgement — arithmetic and grep. Palette
purity, text contrast, selection perceivability, non-colour signalling, footer
width, and degradation across truecolor/256/16/`NO_COLOR`. This is the step that
catches a bad colour before a human has to.

It is cheap enough to run on every push, which is the point: a gate you can afford
is a gate you actually run.

## 2. Snapshot regression — and actually reading it

```bash
cargo test
```

When snapshots change, **read the diff before accepting**. `git diff` on the
`.snap` files, line by line. Ask: is every change one I intended, and is it the
change I think it is?

```bash
git diff -U0 -- '*.snap' | grep -E '^[+-]' | grep -v '^[+-][+-]' | grep -v assertion_line
```

Accept only after reading:

```bash
INSTA_FORCE_UPDATE=1 cargo test
```

A snapshot accepted unread is a bug promoted to a baseline. The picker-selection
defect survived precisely because the baseline was generated, not reviewed.

## 3. Look at it

The colour-blind snapshot is why `theme::ansi::buffer_to_ansi` exists — it walks a
`Buffer` and re-emits the SGR runs, so a frame can be dumped and viewed in a real
terminal:

```rust
let s = sshm::theme::ansi::buffer_to_ansi(terminal.backend().buffer());
std::fs::write("/tmp/frame.ansi", s).unwrap();
```

```bash
cat /tmp/frame.ansi      # renders with real colour in any truecolour terminal
```

For motion, resize, or anything you want to share, render a real capture with
[`vhs`](https://github.com/charmbracelet/vhs): a `.tape` drives the binary in a
PTY at a fixed size and produces PNG/GIF/MP4 through the actual crossterm
backend, with real font metrics and real box-drawing glyph coverage.

### Doing it with no display

A headless server can satisfy "looked at a frame" — it does not need a monitor,
only something that rasterises the terminal emulator's output. On Fedora:

```bash
sudo dnf install -y chromium vhs     # both are in the default repos
```

Three things worth knowing before you start:

- **vhs 0.11 cannot write PNG.** It accepts only `.gif/.webm/.mp4` and exits 0
  writing nothing for anything else. Render a GIF and take the last frame:
  ```bash
  python3 -c "from PIL import Image, ImageSequence; \
    f=[x.convert('RGB') for x in ImageSequence.Iterator(Image.open('x.gif'))]; \
    f[-1].save('frame.png')"
  ```
- **No browser flag is needed.** go-rod's known-path lookup finds
  `/usr/bin/chromium-browser`. If it ever starts trying to download one, force
  it with `ROD=bin-path=/usr/bin/chromium-browser`.
- **Check the font before trusting an image.** Missing box-drawing coverage
  renders `─ │ ┌` as tofu, which looks like a broken UI rather than a missing
  glyph. Noto Sans Mono covers U+2500; verify with `fc-list : family | grep -i mono`.

`demo/design-matrix.tape` renders every surface in every colour mode into
`demo/rendered/`. The GIF encoder is native to vhs, so ffmpeg is not required.

## 4. The terminal matrix

The TUI equivalent of responsive breakpoints. The same frame, four ways:

| Mode | How | What breaks |
|---|---|---|
| Truecolour | `COLORTERM=truecolor` | baseline |
| 256-colour | `TERM=xterm-256color COLORTERM=` | palette drift, banding |
| 16-colour | `TERM=xterm` | roles collapsing into the same colour — selection can vanish |
| No colour | `NO_COLOR=1` | anything relying on colour alone becomes unreadable |

`every_color_mode_stays_readable` covers the palette arithmetic for all four. The
matrix is for what arithmetic cannot see: whether the design still *communicates*
once the colour is gone.

## What counts as done

A UI change is verified when **all four** are true: the detector is green, the
snapshot diff has been read and is intentional, a frame has been looked at in a
real terminal, and the reduced-colour modes still communicate. Three of four is
not done — that is how a 2.34:1 selection shipped.
