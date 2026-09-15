# AGENTS.md

## Agent skills

### Issue tracker

Issues are tracked in this repo's GitHub Issues, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles use their default label strings (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` at the repo root plus `docs/adr/`. See `docs/agents/domain.md`.

### UI design

Any change to rendering, layout, keybindings, hint text, or colour goes through the
`sshm-design` skill first: semantic colour tokens, the one transparent inline
frame and the three commands that open it, and the verification loop. General
terminal craft is in `tui-design`. Two rules hold regardless: colour comes from a
`theme.rs` role and never a literal, and no change counts as verified until a
frame has been looked at in a real terminal.
