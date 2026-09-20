# `sshm update` replaces the installed binary

- **Status:** Accepted
- **Date:** 2026-09-20
- **Related:** #46

## Context

Every surface in sshm announces an update and none of them could apply one. The
Update Note tells the user to run `sshm update`, a subcommand that has never
existed; `check-update` downloads a release tarball, unpacks it into a temp
directory, prints "Please restart sshm", and never touches the installed
binary; and the one function that would have swapped it was reachable only from
tests. Users on any version were told to do something impossible, which is the
entirety of #46.

Two fixes were on the table, and they disagree about what sshm is:

**A — ship a real `sshm update`.** Wire the self-replacing path that already
exists in the dependency into a subcommand, so the instruction the frame already
prints becomes true.

**B — stop promising.** Point the Update Note and `check-update` at
`install.sh`, delete the dead code, and downgrade the README's claim of
Automatic Updates. B is a copy fix and a deletion; A is a feature.

B was the smaller change and it was genuinely viable: `install.sh` works today,
and self-updating a binary carries the problems that make people avoid it —
overwriting a running executable, a target directory the user cannot write to,
signing on macOS. It lost for three reasons.

The frame already commits to the `sshm update` wording, and five assertions in
the frame tests pin that string. B does not merely add a command-less note, it
rewrites passing tests to enshrine a weaker product. And README advertises
Automatic Updates with GitHub release integration; under B that line is still
false unless it is downgraded too, leaving `curl | bash` as the permanent update
route for every user.

Most decisively, the hard part is already written. `self-github-update-enhanced`
is a live dependency and already does the whole apply: it asks GitHub's
`/releases/latest` endpoint, refuses a version that is not strictly greater,
downloads, extracts, and replaces through `self-replace`, which stages the new
binary beside the target and renames over it atomically. The version-ordering
defect in #46 lives entirely in sshm's hand-rolled release selection, so deleting
that code removes the bug rather than fixing it.

## Decision

**sshm ships `sshm update`, which applies an update by replacing the installed
binary in place**, delegating release selection, download, extraction and the
swap to `self-github-update-enhanced`. sshm keeps no hand-rolled download or
extract code of its own.

`check-update` becomes purely a question: it reports whether a newer release
exists and points at `sshm update`. It downloads nothing. One command per
intent — Check for Updates answers, Apply Update acts, per `CONTEXT.md`.

An explicit `sshm update` against a Dev Build is refused. The passive opt-out
that has always suppressed the Update Note for a source checkout expresses the
same intent, and the two must agree: a binary the developer owns is one sshm
never rewrites.

When the install directory is not writable, sshm prints the exact command the
user needs to run and exits without downloading anything. It does not
privilege-escalate, and it does not install to a second location that would
shadow the first — `which sshm` and `sshm --version` disagreeing is one of the
confusions in the originating report.

## Deliberate non-decisions

Each of these was considered and declined; do not "fix" them without reopening
this ADR.

- **No Windows self-update.** The release workflow mis-names the Windows asset
  and `install.sh` refuses Windows outright. Both get fixed, because a correctly
  named asset is worth three lines and unblocks the installer. Self-update over a
  running locked `.exe` is a separate problem with its own failure modes and no
  reported user.
- **No code signing step.** The swap copies bytes, so a byte-identical binary
  keeps the signature it already carries. Re-signing after a replace would be a
  new failure mode applied to a problem that does not exist, and a bad
  re-sign bricks the binary that was just installed. The real gap is coverage,
  not code: no CI job exercises the replace path, so it gets a manual smoke test
  on both Mac architectures.
- **No `--version` pinning.** The dependency supports targeting a tag; sshm does
  not expose it. A downgrade path invites "I downgraded and my config format is
  gone" tickets that no one has asked for.
- **No confirmation prompt, and no `--dry-run`.** `check-update` already is the
  dry run. `no_confirm(true)` on the update configuration is load-bearing, not
  incidental: without it the dependency blocks on an interactive prompt inside a
  command that must not prompt.
- **No `install.sh` hint on every failure.** A network blip should be retried.
  The escape hatch is printed only where self-update structurally cannot succeed,
  which is the unwritable-directory case.

## Consequences

- **The chicken-and-egg is permanent and unsolvable in-app.** A binary with no
  working update path cannot acquire one. Users on anything at or before the
  release that lands this must use `install.sh` once. Worth stating in the release
  notes for that version, because it is the answer to #46's original reporter.
- **`CARGO_MANIFEST_DIR` now gates two behaviours**, not one: no Update Note, and
  a refused `sshm update`. Anyone debugging either will need to know it is set.
- **The audit ignores stay.** `ci.yml` already ignores three advisories in the
  update dependency's transitive tree, documented there as unavoidable because it
  is the only published self-update crate for these targets. Choosing A keeps
  that debt for another cycle.
- **A frame must never open as a side effect of either command.** The update
  error paths historically fell through into opening the frame, which is
  acceptable for nobody and absurd after a failed `sshm update`. Both commands
  return, and a failure is a non-zero exit.
- **The Update Note keeps its cache-only invariant.** #39 established that no
  frame path touches the network; adding an adjacent network path is exactly the
  moment that boundary needs a test next to it, not a comment.
