# inquire-picker prototype — findings (wayfinder #28)

Artifact: `prototypes/inquire-picker/` (standalone throwaway crate, gitignored).
Built and **compiled** against inquire 0.9.4 + crossterm, driven under a real PTY
and under vhs. Records the look-and-feel and — more importantly — the **roadblocks**
the #30 source audit could not see. 2026-09-13.

## TL;DR

Two load-bearing roadblocks, both **compile-verified** (not inferred from source):

1. **A downstream crate cannot implement `inquire::Prompt`.** The trait is public,
   but its mandatory `render(&self, backend: &mut Backend)` and
   `InnerAction::from_key(key: Key, ...)` signatures name `inquire::ui::Backend`
   and `inquire::ui::Key`, **both of which are `pub(crate)`** (E0603 in the
   compile log). Nothing on the public surface re-exports them. **#30's "strategy
   B via a custom `Prompt` impl on inquire's frame plumbing" is therefore not
   possible without forking inquire.** Every architecture that hangs the whole
   frame set on a custom-coded inquire `Prompt` rests on a type it cannot name.
2. **Through the surviving public surface (the built-in `Select`), live
   per-keystroke matched-character highlighting is not reachable.** The row type's
   `Display` impl and the `OptionFormatter` receive only the option, never the live
   filter. `Select::with_scorer` returns a *score*, no offsets. So the Clack green
   fuzzy-hit segments can be drawn statically but not keyed to the filter as the
   user types.

What the built-in `Select` **does** deliver (verified by running, captured below):
filter-as-you-type, the Clack glyph set, transparent fg-only colour, and a genuine
row-diffing settle-collapse. That is the real, reaching surface if we do not fork.

## Probe A — extensibility (compile-verified)

```rust
use inquire::ui::{Backend, Key};   // probe
```
→ `error[E0603]: struct 'Backend' is private` and `enum 'Key' is private`,
pointing at `ui/mod.rs:9` (`pub(crate) use backend::*;`) and `ui/mod.rs:12`
(`pub(crate) use key::*;` — via `api/mod.rs`). `Prompt` is reached through
`pub use crate::prompts::*` but cannot be *implemented* without those names.

## What runs (the reaching surface): the built-in `Select`

The prototype struct:

```text
Connection { alias, user, host, port, folder }        # host is a field (CONTEXT.md)
impl fmt::Display for Connection                       # row = `[folder] alias (user@host:port)`
   emits inline SGR: dim folder/meta (\x1b[2m), bold alias (\x1b[1m)
Scorer: matches on alias|host|user of the TYPED option (not the ANSI string)
RenderConfig::empty() → fg-only, transparent bg, named ANSI only, zero \x1b[0m bg
Select::new(..).with_render_config(..).with_scorer(..).with_page_size(8).prompt()
```

### Emulated-frame capture (first repaint then Enter — no filter)

```text
◆ pick a connection pick a connection
│ ❯ │ [prod] web-01 (deploy@10.0.0.4:22)
│ │  [prod] web-02 (deploy@10.0.0.5:22)
│ │  [prod] db-primary (postgres@10.0.1.10:5432)
│ │  [staging] staging-api (dev@staging.api.internal:22)
│ │  bastion (root@jumpbox.corp:22)
│ │  [home] nas-01 (backup@192.168.1.50:22)
│ [↑↓ to move, enter to select, type to filter]
│ ◆ pick a connection [prod] web-01 (deploy@10.0.0.4:22)     ← collapsed to 1 line on submit
│ 
◆ picked  web-01  (10.0.0.4)                                  ← our stderr settle out-line
```

Evidence:
- **Clack glyphs render** on inquire: `◆` header icon, `│` left rail, `❯` cursor.
- **Row format renders**: `[folder] alias (user@host:port)` with dim meta.
- **Settle-collapse works and is the good kind**: the 6-row list collapses to the
  single `◆ pick a connection [prod] web-01 …` line (row-diff clear).
- **Filter-as-you-type works** (earlier capture: typing `pr` narrowed the list to
  `[prod] db-primary` only — see capture log).
- **Transparent bg by construction**: `RenderConfig::empty()` sets no `bg`; the
  grep-verified sole `with_bg` in the crate (DateSelect calendar) is tangential.
- ANSI styling emitted throughout (150+ SGR sequences in the raw stream).

### The two things it cannot do (the roadblocks)

- **Custom `Prompt`** — closed by Probe A (fork required).
- **Live fuzzy-match character highlights** — the row `Display`/`OptionFormatter`
  never see the filter; scorer returns a score, not positions.

## Implication for #28

The prototype confirms the **look-and-feel** is achievable on inquire's public,
non-forked surface, and that the transparent-bg / settle-collapse / filter
mechanics — the parts a hand-rolled frame would most want for free — genuinely
arrive for free. But it simultaneously cuts the legs out of the **custom-`Prompt`
architecture** that #30's "strategy B" proposed and the Q3 "one frame set, all
three surfaces" answer leaned on: you cannot own the render loop without forking.

Genuine options now:
- **(B1) Built-in `Select` only** — no fork; filter + transparent bg + settle
  collapse, at the cost of: no live match-highlight, a Clack rail squeezed into
  prefix slots, and the manage/add frames forced through `Select`'s fixed
  candidate API.
- **(B2) Fork inquire** (or vendor + patch) just enough to pub-expose
  `Backend`/`Key` and keep the custom-`Prompt` path — re-opens the full custom
  frame, at the cost of owning a `Prompt`-engine fork with bus-factor-1 upstream.
- **(A) Hand-rolled ratatui** as the fallback the prototype keeps pointing at.

The user's Q1 answer was **B**; this evidence narrows what B can be without a fork,
so the go/no-go is the human's call. All three are options on that decision.