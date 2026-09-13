# Clack-alike Rust crate survey (re-check for #29)

**Verified:** 2026-09-12 (all crates.io / GitHub data checked this date)
**Question:** Has a better Clack-alike Rust crate appeared than the ones the mined branch rejected?
**Method:** crates.io API (`/api/v1/crates/<name>`, `/api/v1/crates/<name>/versions`), crates.io full-text search, GitHub repo metadata. No source-level audits — triage only.

## Baseline being re-checked (mined branch, `origin/clack-style`)

From `clack-style.md` §6: `may-clack` 0.7.2 dormant · `inquire-clack` 0.1.0 single-release fork · `clark`/`clark-cli` 0.2.0 days-old (~50 downloads) · `cliclack` a standalone prompt engine, not a styling layer. Note: #29 dates this "September 2025" and calls it "a year stale" — see the correction below; it is three days old.

## The bar each crate must clear

To retire the hand-rolled option, a crate must support **all three** at once:

1. **Persistent filter-as-you-type list** — the candidate list stays on screen and re-filters per keystroke (not a one-shot prompt).
2. **Custom per-row styling** — we render our own row content (host alias, host, badge) with our own `theme.rs` colours.
3. **Matched-character highlighting** — the filter knows *which* characters matched, so they can be emphasised.

`cliclack` 0.5.6 fails (1) and (3) outright — see `cliclack-capability.md`. That is the bar this survey measures against.

## Survey table

All rows verified 2026-09-12 against the crates.io API (`https://crates.io/api/v1/crates/<name>`).

| Crate | Latest | Last release | Claims Clack/bomb.sh style? | Fit verdict |
|---|---|---|---|---|
| [`may-clack`](https://crates.io/crates/may-clack) | 0.7.2 | 2026-03-26 | Yes — keywords `clack`; "stylish, interactive command line prompts" | **No.** Same version as the mined baseline — ~6 months with no release, 102 recent downloads. Still dormant; no change. |
| [`inquire-clack`](https://crates.io/crates/inquire-clack) | 0.1.0 | 2025-12-10 | Yes — a fork of `inquire` restyled to Clack | **No.** Still a single release, unchanged since the baseline. Fork lives on a `main-clack` branch, not a maintained line. |
| [`clark`](https://crates.io/crates/clark) | 0.2.0 | 2026-09-03 | Yes — "Bombshell's clack prompts ported to Rust" | **No.** 72 total downloads, created 2026-09-02. Cosmetic port, no adoption signal. |
| [`clark-cli`](https://crates.io/crates/clark-cli) | 0.2.0 | 2026-09-03 | Yes — CLI wrapper for the above | **No.** 26 downloads; same author, same repo, same verdict. |
| [`inquire`](https://crates.io/crates/inquire) | 0.9.4 | 2026-02-24 | No — own visual language | **Half.** Healthiest option (5.2M recent downloads) and has dynamic re-filtering list prompts, but not Clack-styled — see note below. |
| [`dialoguer`](https://crates.io/crates/dialoguer) | 0.12.0 | 2025-08-23 | No | **No.** Classic `rustup`-style prompts; no Clack rail, no filter-as-you-type list. |
| [`oneline-cli`](https://crates.io/crates/oneline-cli) | — | — | — | **Does not exist on crates.io** (404 on 2026-09-12). Baseline reference is unverifiable. |
| [`cliclack`](https://crates.io/crates/cliclack) | 0.5.6 | 2026-08-10 | Yes — "inspired by the Clack NPM package" | Out of scope (#27). Listed only to confirm no newer release landed: 0.5.6 is still current. |

## Correction to the premise: the baseline is 3 days old, not a year

#29 says the mined finding "is a year stale". It is not. `origin/clack-style:docs/research/clack-style.md` is dated **2026-09-09** in its own header ("Every mapping below was checked against `sshm/src/style.rs`… on 2026-09-09"), and the crate ages in §6 corroborate it: `clark` was *created* 2026-09-02, which is exactly "days old" relative to 2026-09-09 and impossible relative to September 2025.

So this re-check ran **three days** after the survey it was meant to refresh. The window for something new to have appeared is ~72 hours, not 12 months. Findings below are still reported for completeness, but the prior conclusion was never at risk of being stale.

## Newly discovered candidates (not in the mined §6 list)

Surfaced by crates.io full-text search on `clack`, `bombshell`, `clack style`, `clack prompts`, `inline prompt`, `fuzzy prompt`, `fuzzy select` — 2026-09-12.

| Crate | Latest | Last release | Claims Clack/bomb.sh style? | Fit verdict |
|---|---|---|---|---|
| [`clark-core`](https://crates.io/crates/clark-core) | 0.1.0 | 2026-09-02 | Yes — "State machines and widgets for clark. No I/O." | **No.** Third crate in the same 10-day-old `clark` family; 60 downloads. Splitting state machines out does not make it adoptable. |
| [`cli-ui`](https://crates.io/crates/cli-ui) | 0.1.0 | 2026-07-03 | **Yes — explicitly "clack-style interactive prompts"** | **Unknown without source.** Only crate outside the `clark` family to claim the style outright. 23 downloads, v0.1.0, one release. Flagged below, not audited. |
| [`cliclack-file-autocompletion`](https://crates.io/crates/cliclack-file-autocompletion) | 0.1.0 | 2026-05-08 | Companion to `cliclack` | **No.** Adds path completion to `cliclack` prompts; inherits every `cliclack` limitation from #27. |
| [`cliclack_yaml`](https://crates.io/crates/cliclack_yaml) | 0.1.1 | 2026-03-04 | Companion to `cliclack` | **No.** YAML config layer over `cliclack`; same ceiling. |
| [`nucleo-picker`](https://crates.io/crates/nucleo-picker) | 0.12.2 | 2026-09-04 | No — own TUI, not Clack-styled | **Half, and the strongest non-Clack signal.** 15,442 recent downloads, actively released. Real filter-as-you-type on `nucleo` (Neovim's matcher), which *does* emit match positions. But it owns the screen as its own picker app — not a Clack rail, and not a layer inside our ratatui loop. Flagged below. |
| [`nobubbles`](https://crates.io/crates/nobubbles) | 0.1.1 | 2026-09-07 | No | **Unknown without source.** "Reactive TUI… inline and fullscreen" — inline is the right half of the requirement; nothing says Clack or per-row styling. 175 downloads. |
| [`sparcli`](https://crates.io/crates/sparcli) | 0.4.0 | 2026-07-19 | No | **No.** "Styled CLI output and interactive input widgets" — generic widget toolkit, no Clack claim, 109 downloads. |
| [`promptuity`](https://crates.io/crates/promptuity) | 0.0.5 | 2024-01-14 | No | **No.** Abandoned since 2024. |

### Search trap worth recording

A naive `clack` search on crates.io returns four high-download crates that are **not terminal prompts at all**: `clack-common`, `clack-plugin`, `clack-host`, `clack-extensions` (28.6k / 24.2k / 19.7k / 21.2k recent downloads) are wrappers for the **CLAP audio plugin API**. Any future survey that sorts by downloads and greps for "clack" will hit these first. They are unrelated to bomb.sh Clack.

## The two that are actually interesting

Neither is a drop-in, but both clear part of the bar and neither was in the mined §6.

### `cli-ui` — the only crate outside the `clark` family claiming the style

Closest thing to a Clack port found: it ships the framing primitives (`intro`, `outro`, `note`, `cancel`), a `autocomplete` filter prompt, and a **named-slot theme system** (`accent`, `active`, `input`, `success`, `error`, `cancel`, `dim`, `header`, `title`, `intro_badge`) that maps cleanly onto sshm's `theme.rs` semantic-token rule. [README](https://github.com/NameOfShadow/cli-ui), verified 2026-09-12.

Three strikes, though: **v0.1.0, single release, 23 recent downloads**; the theme slots are *global*, not per-row, so custom row styling is undocumented; and matched-character highlighting is not mentioned anywhere. It is also a whole CLI framework that replaces `clap` with its own derive model — adopting it to get a prompt widget means adopting its argument parser. **Fit: unknown without source, and not adoptable at this maturity.**

### `nucleo-picker` — the strongest engine, wrong shell

The only crate found that explicitly does **matched-character highlighting** ("Match highlighting with automatic scroll-through") *and* **custom per-row rendering** (the `Render` trait, for crate-local and foreign types), on top of `nucleo` — Helix's matcher, which emits match positions rather than just an ordering. 15,442 recent downloads, released 2026-09-04, actively maintained. [README](https://github.com/autobib/nucleo-picker), verified 2026-09-12.

It is not Clack: it renders its own fzf-style picker and owns the terminal. Nothing in its docs addresses the inline constraint that `clack-style.md` §6.1 works around (constant frame height, raw-ANSI settle, `StdoutRedirect` under command substitution). **Fit: half — right matcher, wrong shell.**

## Conclusion

**The mined branch's finding still holds: no off-the-shelf Rust crate can drive a persistent filter-as-you-type list with Clack's visual grammar, custom per-row styling, and matched-character highlighting all at once.** Every crate claiming the Clack look (`may-clack`, `inquire-clack`, `clark`/`clark-cli`/`clark-core`, `cli-ui`) is either dormant, single-release, or sub-100-download; every crate that can actually do the filtering and highlighting (`nucleo-picker`, `inquire`) does not claim the look. Hand-rolled ratatui remains the answer, and the re-check window was 3 days rather than the year #29 assumed.

**One crate could plausibly beat hand-rolled — but it is not a Clack-alike.** `nucleo` (the matcher under `nucleo-picker`, [github.com/helix-editor/nucleo](https://github.com/helix-editor/nucleo)) supplies exactly the thing #27 proved `cliclack` lacks: match *positions*, not just an ordering. That is a component to call from inside our existing hand-rolled frame, not a replacement for it — so it changes the implementation, not the strategy.

## Follow-ups

1. **For #28:** if the picker needs fuzzy matching with matched-character highlighting, evaluate **`nucleo` as a matcher library** under the existing ratatui frame. This is the single actionable item from this survey. It does not require adopting any Clack-alike crate.
2. **Do not** open a ticket for `cli-ui` unless sshm is separately considering replacing its argument parser — at v0.1.0 / 23 downloads it is not a dependency candidate.
3. **Do not** re-run this survey on a calendar schedule. The baseline is 3 days old and the Clack-alike niche shows no signs of consolidation; re-check only if the design requirement changes.
4. **Watch item, no action:** `nucleo-picker` adding an inline/Clack render mode would be the one change that reopens this question. Nothing in its roadmap suggests it.
