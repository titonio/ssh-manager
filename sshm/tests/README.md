# Tests for sshm

Integration tests for the sshm crate. Run them from `sshm/`:

```bash
cargo test
```

**There is no snapshot harness here any more.** This directory used to document
`cargo test --test integration_test`, an `integration_test__<test_name>.snap`
naming convention and a `cargo insta test --accept` recipe for a fullscreen
TUI rendered through ratatui's `TestBackend`. That test file was deleted in
`038c43f` when the fullscreen surface was cut over, and there is no
`tests/snapshots/` directory. What follows describes what actually exists.

## Why the tests assert on values, not on pixels

The inline frame is a pure view-model: `build_frame(connections, query,
selection, mode, canvas)` returns a `Frame` of styled `Line`s. Every test can
therefore assert on spans, styles and text directly — no terminal, no PTY, no
`TestBackend`.

That matters because `TestBackend`'s snapshot writes **glyphs only** and drops
every SGR attribute. Nineteen snapshots sat on top of a 2.34:1 selection
colour and never flinched. The gates below check colour, modifiers and the
bytes the frame emits, which a glyph snapshot cannot.

## What is where

| Target | Gates |
|---|---|
| `src/*` `#[cfg(test)]` (lib target) | Unit tests beside the code: config, the Connection manager, update checking, the theme/ANSI serializer, and the live driver's keystroke routing — private machinery, so it is tested where it lives |
| `src/main.rs` `#[cfg(test)]` (bin target) | CLI dispatch and the generated shell init scripts (insta **inline** snapshots) |
| `tests/connections_test.rs` | The Connection manager seam: add, edit, delete, `~/.ssh/config` import — through the public interface only |
| `tests/design_system_test.rs` | The deterministic design gates: palette purity, the Clack palette's shape, hint fitting, `NO_COLOR` degradation, and the settle-verb state set. Plus the frame dump (below) |
| `tests/emit_test.rs` | The emit axis: `Emit::resolve` turning a frame outcome into an `Action`, and `frame_stream`'s stdout-vs-`/dev/tty` routing |
| `tests/filter_test.rs` | Fuzzy filtering, `build_row_text`, and the row-text pairing: the field offsets must index the text the frame actually renders |
| `tests/frame_test.rs` | The frame seam: the Clack grammar, rows, the hint rail and its fitting, transparency, the empty / no-match states, windowing, and column-accurate width fitting |
| `tests/inline_test.rs` | The live half through the public seam: `diff_rows`, `settle_trace`, `plan_resize`, and that the frame never enters the alternate screen |
| `tests/main_tests.rs` | `ssh` argument building and execution plumbing |
| `tests/pty_test.rs` | The frame in a real pseudo-terminal, driven by `tests/pty_frame_check.py`: it draws and waits, the terminal is raw while it does, and the terminal is cooked again afterwards. The only gate with no seam — it runs the built binary and reads the answer off the terminal |

Typical shape of a green run: **693 passed / 0 failed / 4 ignored**, and it is
green with the network unplugged. The four ignored tests are:

| Ignored test | Why |
|---|---|
| `update::tests::test_cache_duration_threshold`, `test_cache_just_under_threshold` | Date-sensitive: they straddle the 24 h cache boundary |
| `update::tests::test_force_check_for_update_basic`, `test_force_check_for_update_returns_result_variant` | They ask GitHub's `releases/latest` endpoint for real. Run them by hand with `cargo test -- --ignored` to prove the endpoint, the target triple and the asset naming still agree |

No other test may reach the network, and no test may replace a binary. The two
things that make that hold are the injected functions every `dispatch` test is
wired with (`apply_update_must_not_run` panics if a check or a frame path ever
reaches the applier) and `NotACheckout` in `update.rs`, which restores
`CARGO_MANIFEST_DIR` after a test borrowed the absence of it to pose as an
installed binary.

Two tests cross a process boundary on purpose: the update check against a stub
on the loopback interface, and the frame under a pseudo-terminal. Neither
waits on anything outside this machine, which is the property the sentence
above is really protecting — a sweep runs the suite once per mutant, so
anything slow is multiplied by ~800 and anything remote makes the result
depend on the hour.

## Running

```bash
cd sshm
cargo test                       # everything
cargo test --test frame_test     # one target
cargo test -- --nocapture        # see eprintln output (frame dumps, skips)
cargo test -- -- the_rendered_row_is_the_row_text   # filter by name
```

## Mutation testing

`cargo test` says the suite passes. `./mutants.sh` asks whether the suite
*cares*: cargo-mutants rewrites one function body, condition or match arm at a
time into a trivial value, and each rewrite must turn a test red. A mutant that
survives is a behaviour no test guards — an inverted comparison, a `||` where
`&&` was meant, a field nothing reads. The coverage gate and the CRAP gate both
count lines that ran; only a mutant checks that the line matters.

```bash
cd sshm
./mutants.sh                       # the whole crate: 779 mutants, ~40 min on 12 cores
git diff --relative origin/master > /tmp/d.diff
./mutants.sh --in-diff /tmp/d.diff # only the mutants in the lines this branch touched
```

`--relative` is what makes the second form honest: cargo-mutants matches the
diff's paths against its own, rooted at `sshm/`, so a repo-root diff names
`sshm/src/frame.rs`, matches nothing, and passes having tested nothing.

CI runs both: the diff form is a required check on master, the whole-crate sweep
runs weekly and reports. `.github/workflows/mutants.yml` says which is which,
and `sshm/.cargo/mutants.toml` holds the options that both share.

A surviving mutant is a work list, not a verdict, and there is one way off the
list: the test that kills it. Nothing is skipped — no `#[mutants::skip]`, no
`exclude_globs`, no `--exclude-re` — because a skipped mutant is a bug skipped,
and the sweep is what tells you the suite has a hole in it. Nor is the list
tolerable as standing debt: the suite runs once per mutant, about 800 times a
sweep, so a flaky test is a flake with 800 chances to land and a hang costs the
run its signal.

### The five that were outside the suite's reach

The sweep is 779 mutants: 668 caught, 111 unviable, **none missed**. It was not
always: five survived, and each survived because the suite had no way to observe
the thing it changed. They are recorded here because the seams that closed them
are not obvious, and because the next mutant that survives will be in the same
shape — a fact about a terminal, or a fact about a network, sitting behind a
seam the tests never crossed.

| Mutant | Why nothing saw it | What closes it |
| --- | --- | --- |
| `run_frame_command` → `Ok(())` | The whole frame command, doing nothing. With a pipe for stdout and no controlling terminal, the correct code fails for the environment's reason, not the mutant's. | `tests/pty_test.rs` |
| `CrosstermRawMode::enter` → `Ok(())` | Raw mode is a termios state on a real terminal, and a test runner has none — so `enable_raw_mode` failed for the correct code too. | `tests/pty_test.rs` |
| `RawModeGuard::drop` → `()` | The guard's whole job is handing the terminal back, and there was no terminal to hand back. | `tests/pty_test.rs` |
| `latest_release` → `Ok("")`, `Ok("xyzzy")` | The only answer it can get comes from GitHub's `releases/latest`, and a suite run once per mutant cannot afford a request per mutant. | `SSHM_GITHUB_API_URL` and a loopback stub in `src/update.rs` |

The pty harness is the interesting one, because it has no seam: it runs the
built binary under a pseudo-terminal and reads the answer off the terminal
itself. On Linux the line discipline belongs to the pty's slave, so
`tcgetattr` on the master reports the settings the child is running under —
the test asserts what the terminal says, not what the program claims. It
checks four things in one session: the frame draws and stays open, the
terminal is out of cooked mode while it is, Escape ends it with 130, and the
terminal is cooked again afterwards. It costs 50 ms, which is what makes it
affordable 779 times a sweep; every wait in it has a deadline, so a mutant that
wedges the binary is reported by the check that caught it rather than by a
timeout. A fresh pty has no window size and a terminal with no rows is one the
frame correctly refuses to draw in, so the harness sets 24x80 first.

The update seam is a real feature, not a test hook: the dependency already
takes a different API root for GitHub Enterprise, and `SSHM_GITHUB_API_URL`
sets it. The test points it at a one-request server on the loopback interface
that answers `releases/latest` from a canned body and remembers the request
line it served, so the check is exercised end to end without anything leaving
the machine.

`scripts/inline-pty-check.py` remains the manual look-at-the-frame tool; it
predates the harness and checks what the frame draws rather than what the
terminal is.

## The one snapshot that exists

`src/main.rs` asserts the generated shell init scripts with insta **inline**
snapshots — `insta::assert_snapshot!(script, @r###"…"###)` — so the expected
script lives in the source file, not in a `.snap` file. When you change an
init script intentionally:

```bash
cargo insta test --accept    # or: cargo insta review
```

This rewrites the inline snapshot in `src/main.rs`. Read the diff: the init
script is the surface whose failure mode is a broken shell.

## Looking at a frame

`design_system_test.rs::dump_frames_for_review` writes real ANSI frames for
human review, gated on an env var so an ordinary run writes nothing:

```bash
SSHM_DUMP_FRAMES=1 cargo test --test design_system_test
cat target/design-frames/frame-pick-80.ansi
```

It dumps both modes at 80 and 60 columns plus monochrome, the empty and
no-match states. The rail fitting and the `NO_COLOR` downgrade are invisible
to every other gate: one is about a width nobody is looking at, the other is
about bytes no contrast table can measure. This is the step that satisfies
"no change counts as verified until a frame has been looked at".

## Conventions

- **Name the seam, not the implementation.** Test names read as
  specifications (`a_narrow_canvas_keeps_the_escape_hatch_and_drops_the_tail_whole`),
  in the domain words from `CONTEXT.md`: a **Connection**, the **Inline
  Picker**, the **Shell Widget**.
- **Expectations come from the spec, not from the code under test.** A
  hard-coded literal, a worked example from `#31`, or an independent
  computation — never a value recomputed the way the code computes it.
- **Colour is asserted by role.** Tests compare against `Theme::clack()`'s
  roles; the hues themselves are pinned in `design_system_test.rs`, so a
  palette change breaks one place, not forty.
- **A gate that cannot fail is worse than no gate.** Where a test reads its
  own input out of source (the settle-verb gate, the colour-literal grep), it
  asserts the input is non-empty first, so a moved emitter fails loudly
  instead of passing vacuously.

## The flake that was

`update::tests::test_should_check_update_no_cache` used to fail intermittently,
unrelated to any change under review: `should_check_update()` short-circuits
when `CARGO_MANIFEST_DIR` is set, and sibling tests in the same module
`remove_var`-ed that variable to pose as an installed binary and left it
removed for whoever ran next.

It is closed from both ends: that test carries `#[serial]`, and every test that
clears the variable now does it through `NotACheckout`, which puts it back on
the way out. Leaving it cleared was not merely a flake after #46 — with no
`CARGO_MANIFEST_DIR`, a sibling check test could make a real request from what
is meant to be an offline suite.
