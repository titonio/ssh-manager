# Live ssh-config hosts in every frame

Date: 2026-10-12
Status: awaiting approval

## Intent

Every frame sshm draws — the bare picker, `sshm pick`, `sshm manage` — lists the
usable `Host` stanzas of `~/.ssh/config` alongside the stored connections,
re-read fresh each time a frame opens. A config-sourced row is selectable and
connects exactly like a stored Connection. The user edits `~/.ssh/config` in
their editor, opens the picker, and the new host is there; delete a stanza and
it is gone. Nothing is copied into sshm's store by merely looking.

The user's own words for the moment this serves: *"I expect to see the new entry
and I can select it to connect to that target."*

## Why now

Today the only bridge to `~/.ssh/config` is the one-time import: it copies
stanzas into `connections.json`, and from that moment the two lists drift.
Hosts added to the config afterwards are invisible until re-imported, and
imported copies go stale. The copy model is the bug; liveness is the feature.

Nobody runs sshm in anger yet, so this is a **breaking** change by agreement:
the first-run import offer and any behaviour that has to bend for it can go.

## Decisions made with the user

These are theirs, not guesses:

1. **sshm keeps its own store** (`connections.json`) and the folder feature.
   There is no cutover to "ssh config as the only store".
2. **Config hosts merge live into the same flat list**, interleaved with the
   stored connections (not a separate section at the foot).
3. **The tag is `[ssh-config]`**, shown where a folder prefix would be.
4. **Import stays, re-meaning as *adopt* — per row**: the cursor sits on a
   config row, a chord copies that one stanza into sshm's store — the only
   way to give a host a folder or edit it in sshm. The whole-file import
   offer goes away with it.

## Decisions left to the builder (my calls, flagged)

Each of these is mine; the user never decided them. A later builder may keep or
change them, but must not silently drop the behaviour they protect.

* **Config hosts are read-only inside sshm.** Add, Edit and Delete never touch
  `~/.ssh/config`. Acting on a config row (delete chord, edit chord) leaves a
  `◇` trace note — `◇ read-only — from ssh-config · Enter adopts` —
  instead of a form or a confirm, and **the frame stays open**: this is a note
  in the `◇ added` / `◇ edited` grammar, not a settle, so a refusal never
  ends the manage session or picks an exit code. The picker and `sshm pick`
  are unaffected: selecting a config row connects or emits normally.
* **Collision rule: stored wins, config copy hidden.** A config stanza is
  dropped from the merged list when a stored Connection matches it on
  host **and** user — the same match `plan_import` already uses. Adopting a
  stanza therefore collapses its two rows into one, which is the whole adopt
  story working as designed.
* **Wildcard stanzas stay skipped** (`Host *`, `Host *.example.com`): they are
  rules about many hosts, not a target. This reuses the existing
  glob-detection rule.
* **The first-run import offer goes away, and with it the whole-file import
  machinery.** Its premise — an empty store means an empty list — is dead: an
  empty store now still shows every config host. Adopt is a per-row chord, so
  `Phase::ConfirmImport { count, path }`, `count_importable`,
  `first_run_import_offer` and the `◇ imported N, M skipped` trace retire;
  the confirm grammar survives as an adopt confirm that names the one host.
  Nothing opens unasked.
* **Provenance is a real property, not a fake folder.** A merged entry knows it
  came from the config; the frame renders `[ssh-config]` from that property,
  never by stuffing the string into `folder`. Provenance is not persisted —
  `connections.json` keeps its current shape. A serde-skipped field on
  `Connection` is the recommended carrier: it is the shape that satisfies
  both "a real property" and "the frame keeps taking `&[Connection]`".
* **`ssh-config` is a reserved folder name** for stored connections, so a
  hand-made folder can never impersonate the tag. The rule sits in the shared
  write seam (`connections`), so the Form Map *and* `sshm add --folder` both
  refuse it; the form shows it as `!` on the Folder row. No migration for
  existing data — nobody has any.
* **Adopt is always exactly one row.** A config row is visible only when it is
  *not* already in the store (the collision rule hides adopted hosts), so the
  chord can never meaningfully answer "adopted 0" — there is always exactly
  one host to take, named in the confirm.
* **Connecting through a config host runs bare `ssh <Host-token>`** — no
  `-i`/`-p` flags. ssh applies the stanza itself, including keywords sshm
  never parses (`ProxyJump`, `ForwardAgent`, …), so the connection is exactly
  what the user's own `ssh build-box` would be. `sshm pick` emits the Host
  token, same rule.
* **Freshness contract: read at frame open, frozen while open.** The frame is a
  short-lived process; re-reading `~/.ssh/config` at open *is* "the latest
  content". The merged list is rebuilt after each store mutation (add, edit,
  delete, adopt) so the list stays coherent within one frame, and no file
  watcher exists. This matches the Update Note's read-once philosophy.
* **The unfiltered list sorts by alias.** The frame's empty-query path keeps
  input order, so "append config rows last" would park them in a block at the
  foot — the section the user rejected. The merged list therefore sorts by
  alias (case-insensitive) when no filter is typed; typing filters re-rank by
  score as today. This also changes the stored-only order from insertion
  order to alias order — breaking, and allowed to be.
* **Multi-pattern `Host a b` lines become one row per pattern.** The current
  parser keeps only the first token, which was tolerable for a one-time
  import but is a silent drop for a live list — `ssh db` works, so `db` must
  appear. Wildcard patterns inside such a line are skipped individually. The
  shared parser expands; each pattern is its own row and its own adopt.
* **Searching `ssh-config` lists the config hosts.** The tag joins the fuzzy
  haystack the way `folder` already does.

## Goals

* A new `Host` stanza appears in every frame with no import step.
* Enter on a config row: bare `sshm` executes `ssh`, `sshm pick` emits the
  token, `sshm manage` says read-only.
* Stored connections and folders behave exactly as today.
* Adopt copies the stanza under the cursor into the store; the adopted host's
  config row disappears, its store row remains and is editable.
* The merged list never shows two rows for the same host+user.

## Non-goals

* Writing to `~/.ssh/config` in any form.
* `Include` directives — `parse_ssh_config` does not follow them today and
  will not start.
* Parsing keywords beyond `Host`/`HostName`/`User`/`Port`/`IdentityFile` for
  display. Connecting still honours them, because ssh does the connecting.
* Watching the file while a frame is open.
* Collapsing near-duplicates that differ in host or user (different aliases for
  the same machine stay two rows; that is honest).

## Anti-goals

* Two identical-looking rows for one machine — the collision rule exists to
  prevent exactly this.
* A config row that looks editable and then fails — the tag plus the read-only
  note make the state legible before the keystroke.
* Silent drops: a stanza that cannot become a row (wildcard) is skipped as
  today — but every pattern a concrete `ssh` target could name must appear.

## Constraints

* Test gates stand: `./sshm/mutants.sh --in-diff <diff>` green on the touched
  lines, `sshm/coverage.sh` green, TDD route.
* UI rendering goes through the `sshm-design` skill: the read-only note and
  the `[ssh-config]` tag take their colour from `theme.rs` roles, and nothing
  counts as verified until a frame has been looked at in a real terminal.
* `CONTEXT.md` gains the new domain term (working name: **Config Host** — a
  `Host` stanza shown live and read-only in the frame, tagged `[ssh-config]`),
  and README's import bullet is re-worded to the adopt meaning.
* The merge lives at the `connections::Store` seam: the frame keeps taking
  `&[Connection]` and keeps its one implementation of add/edit/delete. The
  live store wraps `Config`, re-reads `~/.ssh/config` at construction and
  after each mutation, and is what `main.rs` hands the frame.
* Lenient parsing stays lenient: an unreadable or absent `~/.ssh/config`
  simply contributes no rows.

## What is left to the plan

The shape above is settled; the plan decides:

* the exact Rust carrier for provenance and the `LiveStore`'s re-merge point;
* the manage chord for adopt (and its hint-rail text, via `sshm-design`);
* the read-only note's exact wording and colour role;
* test inventory per the mutation gate, including an e2e case in `sshm-e2e`
  for "stanza added to config appears in picker".
