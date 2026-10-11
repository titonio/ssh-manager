# AGENTS.md

## Agent skills

### Issue tracker

Issues are tracked in this repo's GitHub Issues, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles use their default label strings (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` at the repo root plus `docs/adr/`. See `docs/agents/domain.md`.

### Test gates

A change to `sshm/src` counts as verified when the suite is green *and* nothing
in the diff survives mutation: `./sshm/mutants.sh --in-diff <diff>` tests exactly
the lines the branch touched, which is the same gate CI applies. A surviving
mutant is a line the tests no longer depend on, so the answer is a test, or a
`#[mutants::skip]` with its reason beside it. `sshm/coverage.sh` is the coverage
and CRAP gate; `sshm/tests/README.md` says what each gate proves.

### UI design

Any change to rendering, layout, keybindings, hint text, or colour goes through the
`sshm-design` skill first: semantic colour tokens, the one transparent inline
frame and the three commands that open it, and the verification loop. General
terminal craft is in `tui-design`. Two rules hold regardless: colour comes from a
`theme.rs` role and never a literal, and no change counts as verified until a
frame has been looked at in a real terminal.
