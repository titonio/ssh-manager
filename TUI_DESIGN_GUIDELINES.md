# Terminal UI (TUI) Design Guidelines & Best Practices

**Date:** March 2026  
**Purpose:** Foundation document for creating an AI agent skill for TUI development  
**Focus:** Usability, speed, visual appeal

---

## Table of Contents

1. [Core Design Philosophy](#1-core-design-philosophy)
2. [Layout & Spacing](#2-layout--spacing)
3. [Color Theory & Aesthetics](#3-color-theory--aesthetics)
4. [Typography](#4-typography)
5. [Navigation & Interaction](#5-navigation--interaction)
6. [Information Architecture](#6-information-architecture)
7. [Performance Optimization](#7-performance-optimization)
8. [Error Handling & Feedback](#8-error-handling--feedback)
9. [Accessibility](#9-accessibility)
10. [TUI Frameworks & Tools](#10-tui-frameworks--tools)
11. [Testing](#11-testing)
12. [Design Patterns from Successful TUI Apps](#12-design-patterns-from-successful-tui-apps)
13. [Quick Reference](#13-quick-reference)

---

## 1. Core Design Philosophy

### Fundamental Principles

| Principle | Description |
|-----------|-------------|
| **Progressive Disclosure** | Present essential info first; advanced features through deliberate actions |
| **Least Surprise** | Follow established CLI conventions (vim, emacs, git patterns) |
| **Graceful Degradation** | Function across diverse terminals; enhance where supported |
| **Clear Visual Hierarchy** | Distinguish primary, secondary, tertiary content |
| **Informative Feedback** | Immediate feedback for every action |

### First-Run Experience

- Interactive first-run wizards for configuration
- Sensible defaults that work out-of-the-box
- Helpful onboarding without overwhelming users
- Clear next-step suggestions

### Structured Output for Automation

```bash
# Human-readable (default)
$ mytool status
Server: running
Memory: 2.1 GB

# Machine-readable (--json flag)
$ mytool status --json
{"server": "running", "memory": "2.1 GB"}
```

---

## 2. Layout & Spacing

### The Grid System

Terminal interfaces use a **character grid** (fixed-width):
- Coordinates begin at `(0,0)` top-left
- `x` = columns (horizontal), `y` = rows (vertical)

### Layout Constraints

| Constraint | Description | Use Case |
|------------|-------------|----------|
| `Length` | Fixed characters | Fixed-width sidebars |
| `Min` | Minimum characters | Minimum widget sizes |
| `Max` | Maximum characters | Maximum column widths |
| `Ratio` | Proportional division | Flexible splits (1:3) |
| `Percentage` | % of available space | Responsive layouts |
| `Fill` | Remaining space | Flexible content areas |

### Spacing Scale

- **Element spacing:** 1-2 chars between related items
- **Section spacing:** 3-5 chars between major sections
- **Panel margins:** 1 char from container edges
- **Component gaps:** 1 char between nested components

### Layout Patterns

```
┌─────────────────────────────────────┐
│ Header                              │
├──────────────┬──────────────────────┤
│ Sidebar      │ Main Content          │
│              │                       │
│              │                       │
├──────────────┴──────────────────────┤
│ Footer                              │
└─────────────────────────────────────┘
```

- **Split-Pane:** Master-detail views
- **Dashboard:** Grid-based widget arrangement
- **Container patterns:** Vertical, Horizontal, Grid, Center

---

## 3. Color Theory & Aesthetics

### Terminal Color Systems

| Mode | Colors | Format |
|------|--------|--------|
| ANSI | 16 (8 + 8 bright) | `\x1b[31m` |
| 256-color | 256 | `\x1b[38;5;Nm` |
| Truecolor | 16.7M | `\x1b[38;2;r;g;bm` |

### Semantic Color Usage

| Color | Semantic | Usage |
|-------|----------|-------|
| **Green** | Success | Confirmations, passed tests |
| **Red** | Error | Failures, destructive actions |
| **Yellow** | Warning | Cautions, deprecations |
| **Blue/Cyan** | Info | Informational, links |
| **White/Gray** | Neutral | Primary text |

### Popular Color Schemes

#### Solarized (Dark)
- Background: `#002b36`
- Yellow: `#b58900`, Orange: `#cb4b16`
- Red: `#dc322f`, Magenta: `#d33682`
- Blue: `#268bd2`, Cyan: `#2aa198`, Green: `#859900`

#### Gruvbox (Dark)
- Background: `#282828`
- Red: `#cc241d`, Green: `#98971a`
- Yellow: `#d79921`, Blue: `#458588`

#### Dracula
- Background: `#282A36`
- Purple: `#BD93F9`, Pink: `#FF79C6`
- Green: `#50FA7B`, Cyan: `#8BE9FD`

#### Nord
- Background: `#2E3440`
- Red: `#BF616A`, Yellow: `#EBCB8B`
- Green: `#A3BE8C`, Blue: `#81A1C1`

#### Catppuccin Mocha
- Base: `#1e1e2e`, Text: `#cdd6f4`
- Mauve: `#cba6f7`, Peach: `#fab387`
- Green: `#a6e3a1`, Blue: `#89b4fa`

### Accessibility: Contrast Ratios

| Level | Ratio | Use Case |
|-------|-------|----------|
| AAA | 7:1 | Prolonged reading (ideal) |
| AA | 4.5:1 | Minimum for normal text |
| AA Large | 3:1 | Large text (18pt+) |

---

## 4. Typography

### Monospace Font Selection

**Recommended Fonts:**

| Font | Key Features |
|------|--------------|
| **JetBrains Mono** | Purpose-built for coding, ligatures optional, variable weights |
| **Fira Code** | Programming ligatures, good character distinction |
| **Source Code Pro** | Adobe's open-source, excellent at small sizes |
| **Iosevka** | Highly customizable, many ligature sets |

### Character Distinction Requirements

Must clearly differentiate:
- `0` (zero) vs `O` vs `o`
- `l` (lowercase L) vs `1` (one) vs `I` (capital i)
- `:` vs `;` vs `,`
- `'` (apostrophe) vs `` ` `` (backtick)

### Font Rendering in Terminal

- Terminal DPI settings control rendering, not app settings
- Test in actual terminal emulators
- Ensure Unicode support for box-drawing characters

---

## 5. Navigation & Interaction

### Keyboard-First Design

Standard Navigation Keys:

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Move selection up/down |
| `←`/`→` or `h`/`l` | Move horizontally / collapse-expand |
| `Enter` | Select/activate |
| `Space` | Toggle selection |
| `Tab`/`Shift+Tab` | Move focus between widgets |
| `Esc` | Cancel/close/return |
| `/` or `?` | Open search |

### Focus Management

1. **Visible Focus Indicator** - Highlighted background, brackets, arrows, underline
2. **Logical Focus Order** - Predictable sequence (typically top-to-bottom)
3. **Focus Wrapping** - Optional wrap at list boundaries

### Command Patterns

- **Modal Interfaces:** Full-screen modes (editor, wizard)
- **Command Palettes:** Fuzzy search (`Ctrl+P`)
- **Contextual Help:** `?` or `F1` for current shortcuts
- **Persistent Help Lines:** Footer with available actions

### CLI Argument Conventions (POSIX)

```bash
# Short flags
-a -b

# Long flags
--all --verbose

# Boolean vs value flags
--force (boolean)
--output filename (value)
-o filename (value)
```

---

## 6. Information Architecture

### Data Presentation Strategies

#### Tabular Data
```
NAME        STATUS    MEMORY    CPU
─────────────────────────────────────
server-01   running   2.1 GB    12%
server-02   stopped   0 GB      0%
```

#### Key-Value Pairs
```
Server Configuration:
  Host:       example.com
  Port:       8080
  Protocol:   HTTPS
```

#### Hierarchical (Tree)
```
├── src/
│   ├── main.rs
│   └── lib.rs
├── Cargo.toml
└── README.md
```

### Information Density Guidelines

- **Default to Essential** - Show critical info, opt-in for details
- **Collapse/Expand** - Allow showing/hiding sections
- **Pagination** - For large datasets (Page 1 of 50)
- **Virtual Scrolling** - For very large lists (render only visible)

### Dashboard Design

- Widget-based architecture (each widget = one data type)
- Real-time updates with configurable intervals
- Summary + detail pattern (overview → drill-down)

---

## 7. Performance Optimization

### Rendering Techniques

| Technique | Description |
|-----------|-------------|
| **Diff-Based Rendering** | Only update changed cells |
| **Batch Text Runs** | Combine adjacent cells with identical styling |
| **Dirty Row Tracking** | Track which rows changed |
| **Double Buffering** | Compare buffers, write only differences |

### Input Handling

| Technique | Description |
|-----------|-------------|
| **Non-Blocking Event Loop** | Separate input thread from rendering |
| **Async Event Handling** | Use `tokio::select!` for concurrent events |
| **Event Priority** | Critical inputs (quit) can interrupt rendering |
| **Throttle High-Frequency Events** | Limit scroll events to prevent input starvation |

### Lazy Loading & Memory

| Technique | Description |
|-----------|-------------|
| **Virtual Scrolling** | Render only visible items |
| **Lazy Loading** | Load data on-demand as user scrolls |
| **Pre-allocation** | Use `Vec::with_capacity()` |
| **Bounded History** | Size limits on history/data caches |

### Benchmarking Tools

- **Hyperfine** - CLI benchmarking with statistical analysis
- **termbench-pro** - Terminal backend throughput
- **pprof-tui** - Go profiling in terminal

### Common Pitfalls to Avoid

- ❌ Full screen re-render every frame
- ❌ Polling instead of event-driven (use `usleep()` in loop)
- ❌ Unbuffered terminal writes (flush once per frame)
- ❌ Frequent `clear screen` calls
- ❌ No diff detection (sending identical content)

---

## 8. Error Handling & Feedback

### Error Message Guidelines

1. **Specific** - Explain what happened and what to do
2. **Actionable** - Tell users how to resolve
3. **Consistent** - Same formatting/position for errors

### Feedback Patterns

| Type | Use Case |
|------|----------|
| **Progress Bar** | Known-duration operations |
| **Spinner** | Unknown-duration operations |
| **Toast/Notification** | Background task completion |
| **Status Indicators** | Real-time state (running/stopped) |

### Error Format Example

```
[ERROR] Failed to connect to server
  Reason: Connection timeout
  Action: Check network connectivity
  Code: ERR_CONN_TIMEOUT
  Help: Run 'mytool doctor' for diagnostics
```

---

## 9. Accessibility

### Color Vision Deficiency (CVD)

- ~8% of men, ~0.5% of women have CVD
- **Don't rely on color alone** - combine with icons/text
- **Avoid problematic combinations** - red-green (protanopia/deuteranopia)
- **Test with CVD simulators**

### WCAG Compliance

- Minimum 4.5:1 contrast ratio for text
- Never use color as sole information indicator
- Support screen readers where possible

### Reduced Motion

- Provide options to disable animations
- Respect `prefers-reduced-motion` equivalent settings

---

## 10. TUI Frameworks & Tools

### Framework Comparison

| Framework | Language | Best For | Stars |
|-----------|----------|----------|-------|
| **Ratatui** | Rust | High-performance dashboards | 18.9k |
| **Bubble Tea** | Go | MVU architecture, rapid dev | 40.3k |
| **Textual** | Python | Web deployment, CSS styling | 19k+ |
| **Urwid** | Python | Event loop integration | 2.5k+ |
| **ncurses** | C | Maximum portability | Legacy |

### Recommended Stack by Language

#### Rust
- **Ratatui** - Widgets and layout
- **Crossterm** - Low-level terminal manipulation

#### Go
- **Bubble Tea** - Framework
- **Bubbles** - Pre-built components
- **Lipgloss** - Styling

#### Python
- **Textual** - Full framework (runs in terminal OR browser)
- **Rich** - Text formatting, tables, progress

#### Node.js
- **Ink** - React-like TUI
- **Blessed** - ncurses alternative

### Backend Libraries

- **Crossterm** (Rust) - Cross-platform, async support, 108M+ downloads
- **Termion** (Rust) - Linux/BSD focused
- **ncurses** (C) - Terminal-independent

---

## 11. Testing

### Testing Tools

| Tool | Language | Features |
|------|----------|----------|
| **TUI-Test** | TypeScript | Cross-platform, fast |
| **Ratatui Testlib** | Rust | Headless, snapshot testing |
| **Termwright** | Multiple | Playwright-like automation |
| **TTYtest** | Ruby | Acceptance testing |

### Testing Strategies

- **Snapshot Testing** - Compare render output
- **Headless Testing** - No X11/Wayland needed
- **PTY-based** - Test through pseudo-terminal
- **Integration Tests** - Full user workflows

---

## 12. Design Patterns from Successful TUI Apps

### Modal Interface (Vim)
- Different modes for tasks (normal, insert, visual)
- State machine managing current mode

### Split Views (Vim, Tmux)
- Independent panes for multiple views

### Real-Time Updates (htop)
- Continuous refresh without full redraw
- Delta rendering for changed regions

### Command Palette (Emacs, VSCode)
- Fuzzy search for quick actions
- Overlay with filtered results

### Elm Architecture (Bubble Tea)
- Unidirectional data flow
- Model → View → Update → Model

---

## 13. Quick Reference

### Do's

- ✅ Use keyboard-first navigation
- ✅ Provide `--json` output option
- ✅ Follow POSIX flag conventions
- ✅ Show progress for long operations
- ✅ Handle terminal resize gracefully
- ✅ Test across multiple terminals
- ✅ Use established color schemes
- ✅ Implement virtual scrolling for large lists

### Don'ts

- ❌ Rely solely on color for information
- ❌ Full screen clear on every update
- ❌ Block input during rendering
- ❌ Use polling instead of events
- ❌ Ignore contrast ratios
- ❌ Hardcode terminal assumptions
- ❌ Skip error recovery suggestions
- ❌ Forget `--help` documentation

### Key Shortcuts to Support

```
Movement:  ↑ ↓ ← → / j k h l
Select:    Enter / Space
Navigate:  Tab / Shift+Tab
Search:    / ? 
Cancel:    Esc / q
Help:      ? / F1
```

### Resources

- **Ratatui:** https://ratatui.rs/
- **Textual:** https://textual.textualize.io/
- **Bubble Tea:** https://charm.land/
- **Solarized:** https://ethanschoonover.com/solarized/
- **Nord:** https://nordtheme.com/
- **Catppuccin:** https://catppuccin.com/
- **Hyperfine:** https://github.com/sharkdp/hyperfine

---

## Appendix: Verification Summary

| Claim | Status | Source |
|-------|--------|--------|
| Progressive disclosure improves TUI usability | Supported | Lucas F. Costa, Atlassian |
| Ratatui provides sub-millisecond rendering | Supported | Ratatui documentation |
| 4.5:1 contrast ratio is WCAG AA minimum | Supported | WCAG 2.1, GitHub Blog |
| Virtual scrolling enables 1M+ items | Supported | React Broad Infinite List |
| Bubble Tea uses MVU architecture | Supported | Charmbracelet documentation |
| Truecolor supported in most modern terminals | Supported | GitHub Gist (sindresorhus) |

---

*This document serves as the foundation for an AI agent skill for TUI development. It covers design principles, implementation patterns, and best practices for creating visually appealing, usable, and performant terminal applications.*
