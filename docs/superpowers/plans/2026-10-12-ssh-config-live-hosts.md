# Live ssh-config hosts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use subagent-driven-development (recommended) or executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every sshm frame lists the usable `Host` stanzas of `~/.ssh/config` live — re-read at frame open, tagged `[ssh-config]`, read-only — and manage can adopt one row into the store.

**Architecture:** A `LiveStore` wraps `Config` at the `connections::Store` seam: it re-reads and merges `~/.ssh/config` at construction and after every mutation, so the frame keeps taking `&[Connection]` unchanged. Provenance is a serde-skipped field on `Connection`; the frame renders the `[ssh-config]` tag from it. The whole-file import machinery retires; adopt is per-row.

**Tech Stack:** Rust, ratatui (frame drawing), serde/serde_json (store), skim (fuzzy), crossterm (PTY e2e in `sshm-e2e`).

**Spec:** `docs/superpowers/specs/2026-10-12-ssh-config-live-hosts-design.md`

**Amendment to the spec (flagged to the user):** the spec left the adopt chord to the plan. The manage hint rail has no room for a sixth hint at 80 columns (see the comment at `frame.rs` `hint_rail_line`), so **adopt is Enter on a config row** — the app's existing "act on the row you are standing on" grammar — not a new Ctrl chord. Enter on a *stored* row keeps its current meaning (leave the frame). The read-only note covers `Ctrl+X`/`Ctrl+E` on config rows. The spec's "Enter in manage mode → read-only note" is superseded by "Enter on a config row → adopt confirm".

## Global Constraints

- Test gates: `./sshm/mutants.sh --in-diff <diff>` green on touched lines; `sshm/coverage.sh` green. TDD: failing test first, every task commits.
- UI rendering goes through the `sshm-design` skill; colour comes from `theme.rs` roles, never a literal. The tag reuses the folder-prefix style (`fg_muted`); the read-only note reuses `plain_note_line`.
- Domain language per `CONTEXT.md`: the unit is a **Connection**; the new term is **Config Host** (task 9 adds it). Never "entry"/"server".
- `~/.ssh/config` is never written. Lenient parsing: unreadable/absent file contributes no rows and no error.
- Collision rule everywhere: a config stanza matches a stored Connection on **host and user** (same match `plan_import` used).
- No new hint-rail segments; existing rails must keep fitting 80 columns (`design_system_test.rs` gates).

## Review Focus

Each line gets its test in the named task:

1. **Absent or unreadable `~/.ssh/config`** — every frame lists stored rows only, no error line, no panic (task 3).
2. **Malformed stanzas** (non-numeric `Port`, keyword before any `Host`, empty `Host` value) — skipped leniently, the frame still opens (task 3).
3. **Stanza with no `User`** — the row renders an empty user exactly as an imported Connection does today (`(@host:22)`), and bare-ssh emit sends only the token (tasks 3, 4).
4. **Same pattern listed twice in the config** — one row, not two (task 3).
5. **Store mutation while the frame is open** (add, delete, adopt) — the merged list rebuilds, the adopted host's config row is gone, the cursor clamps to the new list length (tasks 3, 7).

---

### Task 1: Provenance on Connection and the `[ssh-config]` row tag

**Files:**
- Modify: `sshm/src/config.rs` (Connection struct, top of file)
- Modify: `sshm/src/frame.rs` (`build_row_text`, `compute_field_offsets`, ~1040-1120)
- Modify: `sshm/src/connections.rs` (`connection_from`, draft seeding)
- Test: `sshm/tests/frame_test.rs`, `sshm/tests/filter_test.rs`, `sshm/src/config.rs` (unit mod)

**Interfaces:**
- Produces: `config::Source` — `pub enum Source { #[default] Stored, SshConfig }` (Debug, Clone, Copy, PartialEq, Eq, Default); `Connection.source: Source` with `#[serde(skip, default)]`; `frame::SSH_CONFIG_TAG: &str = "ssh-config"`; `frame::row_prefix(conn: &Connection) -> Option<&str>` — `Some(SSH_CONFIG_TAG)` when `source == Source::SshConfig`, else `conn.folder.as_deref().filter(|f| !f.is_empty())`.
- Consumes: nothing from earlier tasks.

- [ ] **Step 1: Failing tests.** In `frame_test.rs`: a Connection with `source: Source::SshConfig`, alias `build-box`, host `build.internal`, user `me`, port `2222` renders `build_row_text` == `"[ssh-config] build-box (me@build.internal:2222)"`. In `filter_test.rs`: query `"ssh-config"` matches only config-sourced rows; query `"prod"` still matches by folder. In `config.rs` unit mod: a `Connection` with `source: Source::SshConfig` round-trips through serde **without** a `source` key in the JSON and deserializes back as `Stored`.

- [ ] **Step 2: Run, verify fail:** `cargo test -p sshm --test frame_test --test filter_test` — compile error on `Source`/`source`.

- [ ] **Step 3: Implement.** Add `Source` and the `source` field to `Connection` in `config.rs`. Run `cargo build --tests` and fix every flagged struct literal by adding `source: Source::Stored` (~89 sites across src and tests; the compiler enumerates them all — mechanical, no judgement). In `frame.rs`, add `SSH_CONFIG_TAG` and `row_prefix`; use `row_prefix` in `build_row_text` and in `compute_field_offsets` where the `"folder"` haystack segment is built (the tag must be searchable). Leave the confirm/edit header sites (`header_line`, `settled_confirm_line`) on `folder` — they never see a config row.

- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm` — whole suite green (existing literals now compile with `Stored`).

- [ ] **Step 5: Commit:** `git add -A && git commit -m "feat: Connection provenance and the [ssh-config] row tag"`

### Task 2: Multi-pattern `Host a b` expansion

**Files:**
- Modify: `sshm/src/config.rs` (`directive`, `Stanza::set`, `parse_ssh_config`)
- Test: `sshm/src/config.rs` (unit mod)

**Interfaces:**
- Produces: `parse_ssh_config(path) -> Vec<SshConfigEntry>` unchanged in signature; now yields **one entry per pattern** on a `Host` line, each carrying the stanza's user/port/hostname/key_file. Wildcard patterns are still returned (filtering is the merge's job).
- Consumes: nothing.

- [ ] **Step 1: Failing tests.** Fixture config: `Host web db\n  User deploy\n  Port 2222` → two entries (`web`, `db`), both user `deploy`, port `2222`. `Host *` → one entry with host `"*"`. `Host web` with `HostName web.example.com` → entry host `web`, hostname `Some("web.example.com")`. Existing single-pattern tests keep passing.

- [ ] **Step 2: Run, verify fail:** `cargo test -p sshm config::` — `web db` yields one entry.

- [ ] **Step 3: Implement.** In `directive`, take the **rest of the line** as the value (`parts.next()` then `parts.collect::<Vec<_>>().join(" ")` after the keyword) so `Host web db` keeps both tokens. In `Stanza::take`, split the host slot on whitespace and emit one `SshConfigEntry` per pattern (change `take` to return `Vec<SshConfigEntry>`, or expand in `parse_ssh_config` where stanzas flush — one place, not both).

- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm` green (import tests in `config.rs`/`connections_test.rs` that used two-token Host lines, if any, get updated to the honest two-entry expectation).

- [ ] **Step 5: Commit:** `git commit -am "feat: expand multi-pattern Host lines into one entry per pattern"`

### Task 3: `merge_ssh_config`, `LiveStore`, and wiring

**Files:**
- Create: `sshm/src/live.rs`
- Modify: `sshm/src/lib.rs` (add `pub mod live;`)
- Modify: `sshm/src/main.rs` (the three `Config::load()` frame sites, ~183-225)
- Modify: `sshm-e2e/src/fixture.rs` (reuse existing `with_ssh_config`)
- Test: `sshm/tests/live_test.rs` (new), `sshm-e2e/tests/picker_test.rs`

**Interfaces:**
- Produces: `config::merge_ssh_config(stored: &[Connection], entries: &[SshConfigEntry]) -> Vec<Connection>` — skips patterns containing `*` or `?`; skips a pattern whose computed host+user matches a stored Connection **or an earlier config row** (dedup within the config too); builds each Config Host as `Connection { id: format!("ssh-config:{pattern}"), alias: pattern, host: hostname.unwrap_or(pattern), user: user.unwrap_or_default(), port: port.unwrap_or(22), key_path: key_file, folder: None, source: Source::SshConfig }`; returns stored rows ++ config rows sorted by `alias.to_lowercase()` (stable). `live::LiveStore` with `pub fn new(config: Config, ssh_config_path: Option<std::path::PathBuf>) -> Self`, implementing `connections::Store` (`all()` → merged; `add`/`update`/`remove` delegate to the inner `Config` then re-merge; `import_ssh_config` delegates unchanged until task 6 retires it).
- Consumes: task 1 `Source`, task 2 parser.

- [ ] **Step 1: Failing tests** in `live_test.rs` (temp-dir fixture writing a real config file):
  - merge: two stanzas + one stored → three rows, alias-sorted, config rows carry `Source::SshConfig` and `ssh-config:` ids.
  - collision: stored `{host: "10.0.0.1", user: "deploy"}` hides the matching stanza; a stanza differing in either field survives.
  - wildcard: `Host *` and `Host *.example.com` produce no rows.
  - Review Focus 4: `Host web` twice → one row.
  - Review Focus 1: `ssh_config_path: None` and a path that does not exist → merged == the stored rows, in alias-sorted order.
  - Review Focus 2: config with `Port abc`, a `User x` line before any `Host`, and an empty `Host` line → no rows from those, no panic.
  - Review Focus 5: `LiveStore::add` then `all()` shows the new row merged in sorted; `remove` of a stored id drops it and re-merges.
  - Review Focus 3: stanza without `User` → row user empty.
- [ ] **Step 2: Run, verify fail:** `cargo test -p sshm --test live_test` — `live`/`merge_ssh_config` unresolved.
- [ ] **Step 3: Implement** `merge_ssh_config` in `config.rs` (next to `parse_ssh_config`) and `live.rs` per the Interfaces block. In `main.rs`, replace each frame site's `let mut config = config::Config::load();` + `&mut config` with `let mut store = live::LiveStore::new(config::Config::load(), config::ssh_config_path());` + `&mut store`.
- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm` green.
- [ ] **Step 5: E2E:** add to `sshm-e2e/tests/picker_test.rs` — `Fixture::new().with_ssh_config("Host web-01\n  HostName 10.0.0.1\n  User deploy\n")`, spawn `sshm pick`, assert the grid contains `[ssh-config] web-01` and `(deploy@10.0.0.1:22)`. Run `sshm-e2e/run-e2e.sh`.
- [ ] **Step 6: Commit:** `git commit -am "feat: merge ssh-config hosts live into every frame"`

### Task 4: Bare `ssh <token>` for config hosts

**Files:**
- Modify: `sshm/src/ssh.rs` (`build_ssh_args`)
- Test: `sshm/tests/emit_test.rs`

**Interfaces:**
- Produces: `build_ssh_args(conn)` returns exactly `vec![conn.alias]` when `conn.source == Source::SshConfig`; unchanged otherwise.
- Consumes: task 1 `Source`.

- [ ] **Step 1: Failing test** in `emit_test.rs`: `Emit::Execute` resolved on a Picked config-sourced Connection yields `Action::Execute(["build-box"])` — no `-i`, no `-p`. Stored rows keep flag behaviour (existing tests).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** the source check at the top of `build_ssh_args`.
- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm --test emit_test`.
- [ ] **Step 5: Commit:** `git commit -am "feat: connect config hosts as bare ssh <token>"`

### Task 5: Read-only refusal for config rows in manage

**Files:**
- Modify: `sshm/src/manage.rs` (`Trace`, `list_chord_step`)
- Modify: `sshm/src/frame.rs` (`trace_lines` dispatch, ~455-462)
- Test: `sshm/tests/manage_test.rs`, `sshm/tests/manage_frame_test.rs`

**Interfaces:**
- Produces: `Trace::ReadOnly { connection: Connection }`, rendered by `plain_note_line` as `◇ read-only — from ssh-config`.
- Consumes: task 1 `Source`, task 3 merged list.

- [ ] **Step 1: Failing tests** in `manage_test.rs`: `step` with `Ctrl+X` over a `Source::SshConfig` selection returns `ManageState { phase: List, trace: Some(Trace::ReadOnly { .. }), .. }` and **no effects**; same for `Ctrl+E`. A stored selection still opens `ConfirmDelete`/`Form` (existing tests). In `manage_frame_test.rs`: glass contains `◇ read-only — from ssh-config` and the list is still drawn (frame open).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement:** in `list_chord_step`, guard `Ctrl+X`/`Ctrl+E` — when `selected.source == Source::SshConfig`, return `Step::state_only` with `phase: List, trace: Some(Trace::ReadOnly { connection })`. Add the `trace_lines` arm. Exhaustive matches elsewhere get the new variant; the note is a plain note, no colour beyond `fg_muted`.
- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm`.
- [ ] **Step 5: Commit:** `git commit -am "feat: manage refuses edits and deletes on ssh-config rows"`

### Task 6: Retire the whole-file import machinery

**Files (deletions):**
- `sshm/src/config.rs`: `ImportReport`, `plan_import`, `import_from_ssh_config_with_path`, `count_importable`, `import_from_ssh_config` + their unit tests. Keep `ssh_config_path`.
- `sshm/src/connections.rs`: free `import`, `Store::import_ssh_config` (trait + `Config` + `Ephemeral` impls) + `connections_test.rs` import tests.
- `sshm/src/manage.rs`: `Phase::ConfirmImport`, `Effect::Import`, `Trace::Imported`, `Trace::ImportDeclined`, `ImportOutcome`, `settle_import`, `confirm_import_step` + tests.
- `sshm/src/inline.rs`: `perform_import`, the `Effect::Import` arm, `settle_import_error`, `Ledger` test impl.
- `sshm/src/frame.rs`: `ImportOfferFlow`, `FrameFlow::import_offer`, the import header branch, `import_trace_lines`, `display_import_path`, the `flow.import_offer` rail condition.
- `sshm/src/main.rs`: `first_run_import_offer`, `opening_offer`, `importable_ssh_config`; `frame_outcome` seeds `ManageState::with_query(query)` directly; `EmptyStore` test impl; `main_tests` offer tests.
- `sshm-e2e/tests/import_test.rs`: delete the file.

**Interfaces:**
- Produces: `Store` without `import_ssh_config`; `Phase` without `ConfirmImport`; `frame_outcome(emit, store, query, note)` unchanged in signature.
- Consumes: tasks 1-5 (nothing depends on import any more).

- [ ] **Step 1: Delete top-down** starting at `main.rs` callers, then `manage.rs`, `inline.rs`, `frame.rs`, `connections.rs`, `config.rs`, letting the compiler enumerate every site (including test mods and `sshm-e2e`).
- [ ] **Step 2: Verify:** `cargo test -p sshm` green; `cargo build -p sshm-e2e --tests` compiles after deleting `import_test.rs`.
- [ ] **Step 3: Gate check:** `design_system_test.rs` settle-verb gate still passes (no settle verb changed — `adopted` is a note, not a settle).
- [ ] **Step 4: Commit:** `git commit -am "refactor!: retire the whole-file ssh-config import"`

### Task 7: Adopt — Enter on a config row copies it into the store

**Files:**
- Modify: `sshm/src/connections.rs` (`Store::adopt`, `LiveStore`/`Config`/`Ephemeral` impls)
- Modify: `sshm/src/manage.rs` (`Phase::ConfirmAdopt`, `Effect::Adopt`, `Trace::Adopted`, Enter in `list_key_step`, `confirm_adopt_step`, `settle_adopt`)
- Modify: `sshm/src/inline.rs` (`Effect::Adopt` arm, `perform_adopt`)
- Modify: `sshm/src/frame.rs` (`FrameFlow` gains `adopt_confirm: Option<Connection>` — the retired `import_offer`'s slot; `header_line` adopt question; rail condition includes it; `trace_lines` `Adopted` arm)
- Test: `sshm/tests/manage_test.rs`, `sshm/tests/manage_frame_test.rs`, `sshm/tests/connections_test.rs`, `sshm-e2e/tests/adopt_test.rs` (new)

**Interfaces:**
- Produces: `Store::adopt(&mut self, target: &Connection) -> Result<Connection, String>` — `Err` unless `target.source == Source::SshConfig`; builds the stored Connection (fresh uuid, `source: Stored`, same alias/host/user/port/key_path, `folder: None`), persists, re-merges, returns it. `Phase::ConfirmAdopt { target: Connection }`; `Effect::Adopt { target: Connection }`; `Trace::Adopted { connection: Connection }` → `◇ adopted <alias>` via the write-family note lines. Enter on a `SshConfig` row in `Phase::List` → `ConfirmAdopt`; confirm grammar mirrors `confirm_step` (`y` → List + `Effect::Adopt`; `n`/`Esc` → List, `trace: None`; `Ctrl+C` → cancel). Header: `◆ Adopt <alias> into sshm's store? (y/N)` reusing the delete-confirm span grammar, driven by `flow.adopt_confirm`; the rail's `CONFIRMING` condition becomes `confirming.is_some() || adopt_confirm.is_some()`.
- Consumes: tasks 1, 3, 5.

- [ ] **Step 1: Failing tests.** `manage_test.rs`: Enter over a config selection → `ConfirmAdopt`, no effects; `y` inside it → `Effect::Adopt` + `trace: None`; `n` → List, `trace: None`. `connections_test.rs`: `LiveStore::adopt` on a config row → store file gains the Connection, `all()` shows one row for that host (config row hidden by collision — Review Focus 5); `adopt` on a stored row → `Err`. `manage_frame_test.rs`: glass shows `◆ Adopt build-box into sshm's store? (y/N)` and after settle `◇ adopted build-box`.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** per Interfaces. `perform_adopt` mirrors `perform_add`: `Ok(conn)` → `settle_adopt` → `Trace::Adopted`; `Err(msg)` → collapse with the error like the add-error path.
- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm`.
- [ ] **Step 5: E2E** `sshm-e2e/tests/adopt_test.rs`: fixture with one config host, spawn `sshm manage`, cursor on the row, Enter → assert the ask; `y` → assert `◇ adopted` and that `connections.json` holds it and the `[ssh-config]` row is gone. Run `sshm-e2e/run-e2e.sh`.
- [ ] **Step 6: Commit:** `git commit -am "feat: adopt an ssh-config host into the store from manage"`

### Task 8: Reserve the `ssh-config` folder name

**Files:**
- Modify: `sshm/src/connections.rs` (guard in `add`/`edit`)
- Modify: `sshm/src/main.rs` (`build_add_connection`)
- Modify: `sshm/src/manage.rs` (`FormField::Folder` settle)
- Test: `sshm/tests/connections_test.rs`, `sshm/src/main.rs` (unit mod), `sshm/tests/manage_test.rs`

**Interfaces:**
- Produces: `connections::is_reserved_folder(folder: &str) -> bool` — `folder.eq_ignore_ascii_case("ssh-config")`. All three writers refuse a reserved folder: `Store::add`/`Store::update` → `Err("folder \"ssh-config\" is reserved for ssh-config rows")`; `sshm add --folder ssh-config` → same message on stderr; the Form Map Folder row shows `!` and the Submit Row refuses.
- Consumes: task 1 `SSH_CONFIG_TAG` (compare against it, don't re-spell).

- [ ] **Step 1: Failing tests:** `connections::add` with `folder: "SSH-Config"` → `Err`; `build_add_connection` same; `FormField::Folder.settle("ssh-config")` → `Err` (glyph `!` via `map_rows`); `"prod"` still fine everywhere.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement** the three guards on `is_reserved_folder`.
- [ ] **Step 4: Run, verify pass:** `cargo test -p sshm`.
- [ ] **Step 5: Commit:** `git commit -am "feat: reserve the ssh-config folder name"`

### Task 9: Domain language and README

**Files:**
- Modify: `CONTEXT.md` (new term), `README.md` (Features bullets)

- [ ] **Step 1:** Add to `CONTEXT.md` **Language**: **Config Host** — a `Host` stanza from `~/.ssh/config` shown live and read-only in the frame, tagged `[ssh-config]`; adopting it mints a stored Connection. _Avoid_: imported host, ssh entry. Also add **Adopt** — the per-row act of copying a Config Host into the store. _Avoid_: import (retired word).
- [ ] **Step 2:** README: replace the "Import from SSH Config" bullet with "Live ssh-config hosts: every `Host` in `~/.ssh/config` appears in the picker tagged `[ssh-config]` and connects like any Connection; adopt one into the store to folder or edit it."
- [ ] **Step 3: Verify** `grep -rn "import" README.md CONTEXT.md` shows no stale first-run-offer copy. Commit: `git commit -am "docs: Config Host and Adopt in the domain language"`.

### Task 10: Full gate sweep

- [ ] **Step 1:** `cargo test -p sshm && sshm-e2e/run-e2e.sh` — green.
- [ ] **Step 2:** `git diff <branch-point>..HEAD > /tmp/branch.diff && ./sshm/mutants.sh --in-diff /tmp/branch.diff` (`--in-diff` takes a diff *file*, passed through to `cargo mutants`) — no surviving mutants; fix any gap with a test, never an exclusion.
- [ ] **Step 3:** `sshm/coverage.sh` green.
- [ ] **Step 4:** Real-terminal look (`sshm-design` verification loop): open `sshm` and `sshm manage` against a fixture HOME with a config file; confirm the tag, the read-only note, the adopt ask, and both rails at 80 columns. Screenshot/frame text into the PR body.
- [ ] **Step 5:** Commit anything the sweep produced; then `finishing-a-development-branch`.
