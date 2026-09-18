---
name: tui-design
description: Design craft for terminal user interfaces — character-grid layout, spacing, colour restraint, keybinding conventions, focus visibility, empty/loading/error states, resize resilience, and ratatui architecture. Use when designing, reviewing, or refactoring a terminal screen or component. Not for web or GUI work.
---

# TUI design craft

Vendored from [`pageton/tui-design-skill`][upstream] at commit `53194a4` and trimmed
to what this repo can use: the general craft principles and the reference library.
The upstream framework-comparison tables, non-Rust framework guides, and example
projects were dropped — this project is committed to ratatui. See `VENDOR.md` for
provenance and the security review.

[upstream]: https://github.com/pageton/tui-design-skill

## The principles that carry

**Start monochrome.** Add colour only where it earns its place: status, active
state, the one thing you want the eye to land on. Colour is emphasis, not
decoration, and every colour added is one the user has to learn.

**Generous spacing.** Whitespace groups and separates at terminal scale. Tight
spacing reads as unfinished. One cell of padding does more than a border.

**Borders group, they do not surround.** A border on every element cancels out.
Prefer spacing for separation and reserve borders for a genuine container.

**Keyboard-first, and visible about it.** Every action reachable from the keyboard,
every keybinding discoverable on screen. Focus must be obvious without colour.

**Clear over clever.** If the user has to guess what a glyph means, redesign it.
Conventional symbols beat invented ones.

**Every state is designed.** Empty, loading, error, no-match and overflow are part
of the interface, not error handling bolted on afterwards. The empty state is
usually the first screen a new user sees.

**The terminal is not one size.** Design for a minimum size and say what happens
below it. Resize is not an edge case; it is the base case for anyone using tmux.

## Loading references

Each file stands alone — read the one the task points at, not all of them.

| Reaching for | Read |
|---|---|
| Auditing a screen, or scoring a design review | `references/review-checklist.md` |
| Layout, hierarchy, visual weight | `references/design-principles.md` |
| Palette choices, emphasis, terminal colour limits | `references/color-and-emphasis.md` |
| Keybindings, focus management, navigation | `references/interaction-guide.md` |
| A specific widget — list, form, status bar, popup | `references/component-catalog.md` |
| Empty / loading / error / transient states | `references/states-and-feedback.md` |
| Grids, pagination, filtering, theming, confirm flows | `references/advanced-patterns.md` |
| Resize gates, panic recovery, backpressure, reconnect | `references/stability-and-robustness.md` |
| State, component and app architecture | `references/architecture-patterns.md` |
| ratatui specifics — `Layout`, `Block`, stateful widgets | `frameworks/ratatui-rust.md` |

## Mockups

ASCII mockups must be exactly aligned at their stated width — a misaligned mockup
is worse than none, because the reader cannot tell whether the misalignment is the
design or a typo. `check-mockups.py` in this directory enforces it:

```bash
python3 .dsh/skills/tui-design/check-mockups.py path/to/file.md
```
