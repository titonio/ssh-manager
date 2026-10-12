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
| `tests/live_test.rs` | The merged list: which `~/.ssh/config` stanzas become rows (globs, collisions, repeats, junk) and where they sort, and `LiveStore`'s freshness contract — read at open, re-merged after every mutation |
| `tests/main_tests.rs` | `ssh` argument building and execution plumbing |
| `tests/pty_test.rs` | The frame in a real pseudo-terminal, driven by the shared `sshm-e2e` harness: it draws and waits, the terminal is raw while it does, and the terminal is cooked again afterwards. One of only two gates with no seam — it runs the built binary and reads the answer off the terminal |
| `sshm-e2e/` (a separate crate) | The shipped binary in a real pseudo-terminal: keystrokes arrive as bytes, the answer is read off the parsed screen grid with its colours intact, and the things a screen cannot show are checked in the world — the store file, the exact `ssh` argv handed to a stub found on `PATH`, what lands on a real zsh's and a real bash's command line. See "The end-to-end suite" below |

Typical shape of a green run: **698 passed / 0 failed / 4 ignored**, and it is
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
terminal is cooked again afterwards. It costs a few milliseconds, which is what
makes it affordable 779 times a sweep; every wait in it has a deadline, so a mutant that
wedges the binary is reported by the check that caught it rather than by a
timeout. A fresh pty has no window size and a terminal with no rows is one the
frame correctly refuses to draw in, so the harness sets 24x80 first.

The update seam is a real feature, not a test hook: the dependency already
takes a different API root for GitHub Enterprise, and `SSHM_GITHUB_API_URL`
sets it. The test points it at a one-request server on the loopback interface
that answers `releases/latest` from a canned body and remembers the request
line it served, so the check is exercised end to end without anything leaving
the machine.

`scripts/inline-pty-check.py` remains the manual look-at-the-frame tool the
design loop uses; it is not part of the test story and checks what the frame
draws rather than what the terminal is. The test harness is Rust only: the
Python one that used to drive `tests/pty_test.rs` is gone, because a second
harness language is a second thing to keep honest.

## The end-to-end suite (`sshm-e2e/`)

Everything above this line tests a library through a seam. This crate tests the
thing that ships: `./sshm-e2e/run-e2e.sh` builds the `sshm` binary, points the
suite at that exact binary through `SSHM_E2E_BIN`, and the suite launches it in a
real pseudo-terminal. Keystrokes are written to the terminal as the bytes a
keyboard sends. The answer is read off a parsed screen grid — a `vt100`
emulator, so cells keep their foreground colour and their bold, which is the
whole reason the harness is not a `TestBackend` snapshot — and the side effects a
screen cannot show are checked in the world: the store file on disk, the argv
handed to a stub `ssh` found through `PATH`, the bytes a captured pipe receives,
the command line a real shell is left holding.

It is a separate crate for three reasons, and the first is the one the gates care
about. There is no root workspace, so `cargo test`, `cargo mutants` and
`cargo llvm-cov` run inside `sshm/` never see this directory: the mutation sweep,
the coverage floor and the CRAP gate are exactly what they were before it
existed, and the numbers quoted above still mean the same thing. That is not an
accident to apologise for. The sweep runs the suite once per mutant, about 800
times; a suite that spawns PTYs and shells is not built to be multiplied by 800,
and pretending otherwise would have bought the sweep a flake instead of the
sweep a second axis. The second reason is that the dependency runs both ways:
this crate dev-depends on `sshm` so a colour assertion can name the `Theme` role
it is checking instead of baking in a hue, and `sshm` dev-depends on this crate
for the harness. Cargo allows a cycle through dev-dependencies and neither
binary pulls the other. The third is honesty about cost: these tests are the
expensive kind, and keeping them out of the sweep is what lets them stay.

| Target | What it is the only witness of |
|---|---|
| `picker_test.rs` | The tracer bullet: the binary starts, the frame draws, the terminal is raw while it is, and the terminal is cooked again afterwards — plus a `Host` stanza in `~/.ssh/config` being on the picker with no import step |
| `navigation_test.rs` | The cursor walking and clamping at both ends, filtering as typing, the empty and no-match states, and the two ways out — Escape and Ctrl+C — each with its own status |
| `form_test.rs` | Ctrl+A and Ctrl+E opening the Form Map, the state glyphs that make a required field differ from an optional one in monochrome, and Enter writing the store |
| `edit_delete_test.rs` | Editing a Connection row by row, the delete question answered three ways, and every route back out of a form |
| `import_test.rs` | A first run on a machine that has a `~/.ssh/config`: the config's hosts are the first screen, nothing is offered, and looking writes no store file |
| `update_test.rs` | A newer release served on loopback appearing as a note above a list that still works |
| `emit_test.rs` | The argv `ssh` is actually handed, and the fact that a captured pipe receives the alias and none of the frame's chrome |
| `shell_test.rs` | The Shell Widget in a real zsh and a real bash: the chord firing, the buffer being replaced, and Escape leaving a half-typed command alone |

### What the harness promises

Every one of these rules exists because breaking it produced a failure that came
and went.

* **A wait is for content, and it has a deadline.** Nothing sleeps. A test drains
  the terminal until a predicate holds, and on expiry the failure names what it
  was waiting for and dumps the grid that refused to show it. A check that cannot
  fail is worse than no check, because it passes by doing nothing.
* **A satisfied wait is a settled frame.** A frame is not painted atomically —
  ratatui queues one write per run of changed cells — so a reader can catch the
  middle of a repaint, where the row that lost the focus glyph has been rewritten
  and the row gaining it has not. Any predicate that reads an *absence* ("the row
  is gone", "no field is focused") is satisfied by that half-paint, so those waits
  additionally require the terminal to go quiet. The frame is event-driven: it
  paints when a key arrives and then stops, so silence is the signal that a paint
  has finished. Where a positive signal exists it is used instead — the submit row
  carries focus as bold, so that is what the test reads.
* **The child is born into a world the test built.** `env_clear` first, then an
  environment built from a list: a temporary `HOME`, `XDG_CONFIG_HOME`,
  `XDG_CACHE_HOME` and `TMPDIR`, a `PATH` that may have the `ssh` stub at its
  head, `TERM` and `LANG` pinned so the emulator and the program agree about what
  they are looking at. Nothing is inherited, which is the sentence that keeps
  someone's `SSH_AUTH_SOCK` or a runner's `NO_COLOR` out of a result. The update
  check is pointed at `http://127.0.0.1:1`, so a check that somehow tries to
  leave the machine fails at once instead of hanging.
* **The terminal answers its queries.** A PTY with nothing on the other end
  answers nothing, and a program that asks "where is the cursor?" and gets
  silence sits there until its own timeout. The harness answers DSR, DA1 and the
  kitty keyboard probe, so a stall is a bug in the program rather than an
  artefact of the harness.
* **Nothing leaves the machine.** The only network in the suite is a
  one-request server bound to the loopback interface. The whole suite is green
  with the network unplugged.
* **Parallel-safe by construction, not by serialisation.** Every test gets its
  own temporary home, its own PTY and its own child, so there is no process-global
  state to fight over and no `serial_test` anywhere in the crate. Every core runs
  a test at once and the whole suite still finishes in under half a second.
* **No retries.** `retries = 0` in `sshm-e2e/.config/nextest.toml`, and that is a
  policy rather than a tuning choice: a test that passes on the second attempt is
  a test that will fail in front of a user after shipping the thing it was meant
  to guard. A flake is a bug.
* **The budget is enforced, not hoped for.** The design budget is 5 s a test and
  90 s for the suite. What holds it is a 4 s deadline on every wait inside the
  harness (`DEFAULT_TIMEOUT`), and a 10 s hard deadline per test in
  `nextest.toml` that terminates a wedged child rather than warning about it. A
  green run is ~0.4 s wall clock with the slowest test at ~0.3 s, which is the
  room a loaded runner spends.
* **Expectations come from the spec.** Test names read as specifications in the
  words from `CONTEXT.md` — a Connection, the Inline Picker, the Form Map, the
  Shell Widget — and the expected text is hard-coded from the design, never
  recomputed the way the code computes it. Colour is asserted against a
  `Theme::clack()` role, never a literal hue. The four golden screens under
  `sshm-e2e/tests/snapshots/` carry text only, never styling: styling is a
  colour question, and colour questions are answered by role assertions.

### Running it

```bash
./sshm-e2e/run-e2e.sh                    # build sshm, then run the whole suite against it
./sshm-e2e/run-e2e.sh shell_test         # one test target
./sshm-e2e/run-e2e.sh -- --nocapture     # past flags on to the harness
```

The script is the only place that decides which binary is under test, which is
the only reason CI and a laptop cannot drift. It uses cargo-nextest when it is
installed — per-test process isolation, so one wedged PTY child fails one named
test instead of taking the run with it — and falls back to libtest so a fresh
machine needs nothing extra.

CI runs it as the `e2e` job in `.github/workflows/ci.yml`: `ubuntu-latest`,
nextest installed, the same script, and a step that installs `zsh` — it is not
in the `ubuntu-latest` image — and then asserts both shells are there, so a
missing one is one clear line rather than a shell test timing out three minutes
later or quietly becoming a skip.
The job is a check like any other; it is not part of the mutation or coverage
gates, and it does not need to be: the mutants in this territory that the sweep
could never reach are already named in the table above, and this suite is the
same evidence at a scale the sweep can afford to skip.

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
