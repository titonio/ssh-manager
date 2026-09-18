# Vendored: tui-design

| | |
|---|---|
| **Source** | https://github.com/pageton/tui-design-skill |
| **Pinned commit** | `53194a47eca31172f0e0377e8ffcba1386c15629` |
| **Upstream author** | Sadiq (`pageton`) |
| **Upstream last commit** | 2026-09-04 — "feat: add advanced/stability references and dbview-style example projects" |
| **Vendored** | 2026-09-12 |
| **Licence** | Check upstream before redistributing; the repo carries no LICENSE file as of the pinned commit. |

## What was taken

- `references/*.md` — all nine reference documents (81 KB). Framework-agnostic
  TUI craft: review checklist, design principles, colour, interaction, component
  catalog, states, advanced patterns, stability, architecture.
- `frameworks/ratatui-rust.md` — the ratatui-specific guide.
- `scripts/check-mockups.py` — ASCII mockup alignment linter.

## What was dropped, and why

| Dropped | Reason |
|---|---|
| `frameworks/bubbletea-go.md`, `textual-python.md`, `ink-react.md` | This project is committed to ratatui. 30 KB of routing noise. |
| `projects/` (132 KB) | Runnable Go and Rust example apps. Not this codebase; would compete for attention. |
| `templates/` (104 KB) | Starter apps for four frameworks. Same reason. |
| `commands/`, `justfile`, `README.md`, `AGENTS.md`, `CLAUDE.md` | Harness-specific wiring for Claude Code / OpenCode. DSH routes through `SKILL.md`. |
| Upstream `SKILL.md` body | Rewritten. It had **no YAML frontmatter**, so it would not route under the agent-skills spec at all, and roughly half of it was framework selection we do not need. |

## Security review

Run against the research checklist in `~/test/ui-ux-agent-skills-research.md` §9
before vendoring. Results:

- **Invisible Unicode** (U+200B–200F, U+202A–202E, U+2060–206F, U+E0000–E007F,
  U+FEFF, U+00AD): **0 occurrences** across every file.
- **Command execution / network**: no `curl`, `wget`, `eval`, `os.system`,
  `subprocess`, `base64`, or URLs in any skill content. The only URL in the tree
  is a documentation link to `github.com/casey/just`.
- **`check-mockups.py`**: read in full. Pure local markdown linter — reads files,
  measures display width of fenced box-drawing runs, exits 0/1. No network, no
  exec, no writes.
- **Bundled binaries**: none.
- **Remote instruction loading**: none.

## Updating

```bash
git -C /tmp clone --quiet https://github.com/pageton/tui-design-skill.git
git -C /tmp/tui-design-skill log --oneline 53194a4..HEAD   # what changed
# re-run the invisible-unicode and command-surface scans, then:
cp /tmp/tui-design-skill/references/*.md .dsh/skills/tui-design/references/
# update the pinned commit above
```

Re-scan on every update. The upstream research found 13–37% of published skills
carry flaws, and a skill's prose is indistinguishable from trusted instructions
once loaded — treat this file as a supply-chain dependency, not documentation.
