//! The live list: sshm's store with `~/.ssh/config` merged into it.
//!
//! Two different things are under test here, and they need different fixtures.
//!
//! [`merge_ssh_config`] is pure: given the stored rows and the stanzas the
//! parser read, it answers which rows a frame shows and in what order. Those
//! tests feed it a real file in a temp dir rather than a hand-built list of
//! `SshConfigEntry`, because the merge's whole job is to sit *behind* the parser
//! — a test that assembled the entries by hand would keep passing when the parser
//! changed shape underneath it, which is the one thing this seam must not do.
//!
//! [`LiveStore`] is the seam a frame holds: it reads the file when it is built
//! and re-merges after every mutation, so a frame never lists a stale config.
//! Those tests go through the `Store` trait exactly as `main.rs` does, with `HOME`
//! pointed at a throwaway directory — a mutation persists, and a test must not be
//! the thing that writes to the developer's `~/.ssh/connections.json`.

use std::ffi::OsString;
use std::path::PathBuf;

use serial_test::serial;

use sshm::config::{self, merge_ssh_config, parse_ssh_config, Config, Connection, Source};
use sshm::connections::{ConnectionDraft, Store};
use sshm::live::LiveStore;

/// Point `HOME` at a throwaway directory so a mutation's write lands there, and
/// put it back on the way out.
struct TempHome {
    dir: tempfile::TempDir,
    previous: Option<OsString>,
}

impl TempHome {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp home");
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", dir.path());
        Self { dir, previous }
    }

    /// Lay down a `~/.ssh/config` inside the throwaway home, replacing whatever
    /// was there — which is also how a test writes "the user edited the file".
    fn write_ssh_config(&self, content: &str) {
        let ssh_dir = self.dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).expect("create .ssh");
        std::fs::write(ssh_dir.join("config"), content).expect("write ~/.ssh/config");
    }

    /// The aliases the store file holds, in file order: proof that a mutation
    /// reached the disk and not merely the merged list.
    fn stored_aliases(&self) -> Vec<String> {
        Config::load()
            .connections
            .iter()
            .map(|c| c.alias.clone())
            .collect()
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(home) => std::env::set_var("HOME", home),
            None => std::env::remove_var("HOME"),
        }
    }
}

/// A throwaway `~/.ssh/config` holding `text`, returned with the directory that
/// owns it so the caller decides when the file stops existing.
fn ssh_config(text: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config");
    std::fs::write(&path, text).expect("write the ssh config");
    (dir, path)
}

/// What a frame would list: `text` parsed as the user's ssh config and merged
/// over `stored`.
fn merged_from(text: &str, stored: &[Connection]) -> Vec<Connection> {
    let (_dir, path) = ssh_config(text);
    let entries = parse_ssh_config(path.to_str().expect("a utf-8 temp path"));
    merge_ssh_config(stored, &entries)
}

/// A Connection already in sshm's store.
fn stored(alias: &str, host: &str, user: &str) -> Connection {
    Connection {
        id: format!("stored-{alias}"),
        alias: alias.to_string(),
        host: host.to_string(),
        user: user.to_string(),
        port: 22,
        key_path: None,
        folder: None,
        source: Source::Stored,
    }
}

/// The list a frame would draw, reduced to the one column every rule here is
/// about.
fn aliases(rows: &[Connection]) -> Vec<&str> {
    rows.iter().map(|c| c.alias.as_str()).collect()
}

fn draft(alias: &str, host: &str, user: &str) -> ConnectionDraft {
    ConnectionDraft {
        alias: alias.to_string(),
        host: host.to_string(),
        user: user.to_string(),
        port: "22".to_string(),
        key_path: String::new(),
        folder: String::new(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// merge_ssh_config — which rows a config contributes, and where they sit
// ─────────────────────────────────────────────────────────────────────────────

/// A stanza becomes a row beside the stored ones, tagged as a config row and
/// sorted into the same list rather than parked below it.
///
/// Three rows from two sources, in one alphabetical run: a block of config hosts
/// at the foot is the separate section the design rejects, and the frame keeps
/// input order when no query is typed, so the order the merge returns *is* the
/// order on screen.
#[test]
fn a_stanza_becomes_a_row_alongside_the_stored_connections() {
    let stored = vec![stored("beta", "10.0.0.2", "deploy")];
    let merged = merged_from(
        "Host web-01\n  HostName 10.0.0.1\n  User deploy\n  Port 2222\n  \
         IdentityFile ~/.ssh/id_deploy\n\nHost db-01\n  HostName 10.0.0.9\n  User dba\n",
        &stored,
    );

    assert_eq!(aliases(&merged), vec!["beta", "db-01", "web-01"]);

    let web = merged
        .iter()
        .find(|c| c.alias == "web-01")
        .expect("the stanza is in the list");
    assert_eq!(
        web.id, "ssh-config:web-01",
        "the id names the Host token the user would type, so it is stable across \
         edits to the stanza and unique per row"
    );
    assert_eq!(web.host, "10.0.0.1", "the HostName is the target");
    assert_eq!(web.user, "deploy");
    assert_eq!(web.port, 2222);
    assert_eq!(web.key_path.as_deref(), Some("~/.ssh/id_deploy"));
    assert_eq!(web.folder, None, "a stanza has no folder to offer");
    assert_eq!(
        web.source,
        Source::SshConfig,
        "the row says where it came from, because that is what makes it read-only"
    );

    let db = merged
        .iter()
        .find(|c| c.alias == "db-01")
        .expect("the second stanza is in the list");
    assert_eq!(
        db.port, 22,
        "a stanza with no Port is ssh's own default, not a row without one"
    );

    let beta = merged
        .iter()
        .find(|c| c.alias == "beta")
        .expect("the stored row survives the merge");
    assert_eq!(beta.id, "stored-beta", "a stored row keeps its own id");
    assert_eq!(beta.source, Source::Stored);
}

/// A `Host` token with no `HostName` is itself the target.
///
/// `ssh web` works, so the row must point at `web`; inventing a hostname here
/// would draw a target the user never named.
#[test]
fn a_stanza_without_a_hostname_points_at_its_own_pattern() {
    let merged = merged_from("Host build-box\n  User ci\n", &[]);

    assert_eq!(aliases(&merged), vec!["build-box"]);
    assert_eq!(merged[0].host, "build-box");
    assert_eq!(merged[0].id, "ssh-config:build-box");
}

/// A multi-pattern `Host` line is one row per pattern, each named for its own
/// token.
///
/// `Host web db` is two machines `ssh` can both reach, and the id is the pattern
/// rather than the shared `HostName` — otherwise the two rows would carry the
/// same id and an adopt could not tell them apart.
#[test]
fn a_multi_pattern_host_line_contributes_one_row_per_pattern() {
    let merged = merged_from("Host web db\n  User deploy\n", &[]);

    assert_eq!(aliases(&merged), vec!["db", "web"]);
    let ids: Vec<&str> = merged.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, vec!["ssh-config:db", "ssh-config:web"]);
}

/// A stanza whose target a stored Connection already describes is hidden.
///
/// This is the collision rule, and it is the adopt story: copying a stanza into
/// sshm's store collapses its two rows into one instead of leaving the user
/// choosing between two identical-looking hosts. Matching on host *and* user is
/// what makes "differs in either field" survive — a different user on the same
/// box is a different target, not a duplicate.
#[test]
fn a_stored_connection_hides_the_stanza_that_describes_the_same_host_and_user() {
    let stored = vec![stored("web-01-adopted", "10.0.0.1", "deploy")];
    let merged = merged_from(
        "Host web-01\n  HostName 10.0.0.1\n  User deploy\n\n\
         Host web-02\n  HostName 10.0.0.2\n  User deploy\n\n\
         Host web-03\n  HostName 10.0.0.1\n  User other\n",
        &stored,
    );

    assert_eq!(
        aliases(&merged),
        vec!["web-01-adopted", "web-02", "web-03"],
        "the adopted stanza collapses into the row that already exists; a stanza \
         differing in host or in user is its own target and stays"
    );
    assert!(
        !merged.iter().any(|c| c.id == "ssh-config:web-01"),
        "the hidden stanza contributes no row at all"
    );
}

/// A wildcard pattern never becomes a row.
///
/// `Host *` is a rule about many hosts, not a target `ssh` could be pointed at,
/// so there is nothing honest to draw. The concrete stanza alongside them is
/// what makes this a test: without it, a merge that dropped every row would pass.
#[test]
fn a_wildcard_pattern_is_a_rule_about_many_hosts_and_contributes_no_row() {
    let merged = merged_from(
        "Host *\n  User default\n\nHost *.example.com\n  User wildcard\n\n\
         Host web-?\n  User wildcard\n\n\
         Host real\n  HostName 10.0.0.3\n  User deploy\n",
        &[],
    );

    assert_eq!(
        aliases(&merged),
        vec!["real"],
        "the globs are skipped — `?` is a wildcard too — and the concrete \
         stanza survives"
    );
}

/// A `Match` block closes the stanza it follows, and contributes nothing.
///
/// sshm cannot evaluate `Match` criteria at list time, so the honest answer for
/// a `Match` block is that it draws no row — but the block also *ends* the
/// stanza above it. Read as a continuation instead, a trailing
/// `Match host db.example.com`'s `User` landed in `Host web`'s stanza, and
/// since adopt persists the parsed fields, that wrong user would become a
/// permanent row in `connections.json`. A mis-parse is no longer a skipped
/// import row.
#[test]
fn a_match_block_closes_the_stanza_it_follows_and_contributes_nothing() {
    let merged = merged_from(
        "Host web\n  HostName web.example.com\n  User webuser\n\n\
         Match host db.example.com\n  User dbuser\n",
        &[],
    );

    assert_eq!(
        aliases(&merged),
        vec!["web"],
        "the Match block names no target, so it draws no row of its own"
    );
    assert_eq!(
        merged[0].user, "webuser",
        "the stanza keeps the user it was written with: a Match block's `User` \
         belongs to the Match, not to the row above it"
    );
}

/// A negation and a bracketed pattern name no reachable machine.
///
/// The glob rule was written for `*` and `?`, but OpenSSH's `Host` line also
/// accepts `!pattern` negation, and a row named `!bastion` is a hostname
/// `ssh` cannot be pointed at. Brackets are not a character class in
/// ssh_config — OpenSSH matches `web[0-9]` literally — but a bracketed
/// token is no resolvable hostname either, so listing it would be the rule
/// the glob already exists to keep.
#[test]
fn a_negated_or_bracketed_pattern_is_a_rule_about_many_hosts_and_contributes_no_row() {
    let merged = merged_from(
        "Host * !bastion\n  User default\n\nHost web[0-9]\n  User ci\n\n\
         Host plain\n  User deploy\n",
        &[],
    );

    assert_eq!(
        aliases(&merged),
        vec!["plain"],
        "the negation and the class are skipped and the concrete stanza survives"
    );
}

/// A `!` in the middle of a pattern is a literal character, not a negation.
///
/// OpenSSH only reads a leading `!` as negation, so `web!1` is one machine with
/// an odd name and it keeps its row; a filter that looked for `!` anywhere would
/// silently drop a real host.
#[test]
fn a_bang_in_the_middle_of_a_pattern_is_a_literal_and_keeps_its_row() {
    let merged = merged_from("Host web!1\n  User deploy\n", &[]);

    assert_eq!(aliases(&merged), vec!["web!1"]);
}

/// `Keyword=value` is the same line as `Keyword value`.
///
/// OpenSSH's `strdelim` ends a keyword at `=` as well as at whitespace, and
/// generated configs are written that way. Read on whitespace alone,
/// `User=root` is an unknown keyword `user=root` and the field is silently
/// dropped — which adopt then persists as a row with no user.
#[test]
fn an_equals_sign_after_the_keyword_is_a_separator_not_part_of_the_value() {
    let merged = merged_from(
        "Host web\n  User=root\n  Port=2222\n  IdentityFile=~/.ssh/id_web\n",
        &[],
    );

    assert_eq!(aliases(&merged), vec!["web"]);
    assert_eq!(merged[0].user, "root", "`User=root` is a user, not a note");
    assert_eq!(merged[0].port, 2222);
    assert_eq!(merged[0].key_path.as_deref(), Some("~/.ssh/id_web"));
}

/// The `=` may sit on either side of the whitespace: `Host = web` is one
/// pattern, not two.
///
/// Read as a value, the lone `=` becomes a pattern named `=` and the file
/// appears to hold a machine nobody can name — and an adopt would offer to
/// persist it.
#[test]
fn an_equals_sign_alone_after_host_names_one_row_not_two() {
    let spaced = merged_from("Host = web\n  User deploy\n", &[]);
    assert_eq!(
        aliases(&spaced),
        vec!["web"],
        "`Host = web` names one machine"
    );

    let tight = merged_from("Host= web\n  User deploy\n", &[]);
    assert_eq!(
        aliases(&tight),
        vec!["web"],
        "and so does `Host= web`, with the `=` against the keyword"
    );

    let padded = merged_from("Host = web\n  User = deploy\n", &[]);
    assert_eq!(
        padded[0].user, "deploy",
        "the value starts after the separator, not at the space behind it"
    );
}

/// A `#` glued to the `=` is OpenSSH's comment starting where the value
/// should be: `User=#root` has no value, and OpenSSH errors on the line and
/// sets nothing. Reading `#root` as the user would persist a user named
/// `#root` through an adopt — a person nobody named.
#[test]
fn a_comment_glued_to_the_separator_leaves_the_field_unset() {
    let merged = merged_from("Host web\n  User=#root\n", &[]);

    assert_eq!(aliases(&merged), vec!["web"]);
    assert_eq!(
        merged[0].user, "",
        "the line has no value, so the stanza keeps none"
    );
}

/// The same stanza twice is one row; the same token twice over two targets is two.
///
/// The dedup is on host+user, not on the alias, and both halves matter: a config
/// that repeats a stanza must not draw it twice, and two `Host web` lines that
/// point at different machines are two real targets that no rule should merge.
#[test]
fn the_same_target_is_one_row_however_often_the_file_repeats_it() {
    let repeated = merged_from(
        "Host web\n  HostName 10.0.0.5\n  User deploy\n\n\
         Host web\n  HostName 10.0.0.5\n  User deploy\n",
        &[],
    );
    assert_eq!(aliases(&repeated), vec!["web"], "one target, one row");

    let split = merged_from(
        "Host web\n  HostName 10.0.0.5\n  User deploy\n\n\
         Host web\n  HostName 10.0.0.6\n  User deploy\n",
        &[],
    );
    assert_eq!(
        aliases(&split),
        vec!["web", "web"],
        "the same token over two targets is two rows: the dedup is about the \
         target, never about the name"
    );
}

/// A stanza with no `User` is a row with an empty one.
///
/// ssh logs in as the local user in that case, which sshm cannot know, so the
/// honest value is the empty string — the same one a stored Connection with no
/// user carries.
#[test]
fn a_stanza_without_a_user_is_a_row_with_an_empty_one() {
    let merged = merged_from("Host bare\n  HostName 10.0.0.4\n", &[]);

    assert_eq!(aliases(&merged), vec!["bare"]);
    assert_eq!(merged[0].user, "");
}

/// Junk in the file costs the rows it touches and nothing else.
///
/// A `Port` that is not a number falls back to ssh's default rather than
/// poisoning the stanza; a keyword written before any `Host` line belongs to no
/// stanza and must not invent one; a `Host` line with no pattern names nothing.
/// The lenient parser is the design's, and the merge is where its leniency has to
/// end up harmless — a panic here would take the whole frame down over one odd
/// line in a file the user edits by hand.
#[test]
fn junk_in_the_file_costs_rows_not_the_whole_list() {
    let merged = merged_from(
        "User orphan\nHost odd\n  Port abc\nHost\n  User nobody\n\
         Host web\n  HostName 10.0.0.9\n  User deploy\n",
        &[],
    );

    assert_eq!(
        aliases(&merged),
        vec!["odd", "web"],
        "the orphan keyword and the empty `Host` line name no target, so they \
         draw no row — and nothing here panicked"
    );
    assert_eq!(
        merged[0].port, 22,
        "`Port abc` is not a port: the row falls back to ssh's default"
    );
}

/// The merged list is sorted by alias ignoring case, and a tie keeps the stored
/// row first.
///
/// Case-insensitive because the frame's own ordering has always been the one a
/// user reads, not the one ASCII defines; stable because the merge is
/// stored-rows-then-config-rows, and on a tie the row the user made by hand is
/// the one that should lead.
#[test]
fn the_merged_list_sorts_by_alias_ignoring_case_and_keeps_a_tie_in_store_order() {
    let stored = vec![
        stored("Zulu", "10.0.0.1", "deploy"),
        stored("beta", "10.0.0.2", "deploy"),
    ];
    let merged = merged_from(
        "Host alpha\n  HostName 10.0.0.3\n  User deploy\n\n\
         Host Beta\n  HostName 10.0.0.4\n  User deploy\n",
        &stored,
    );

    assert_eq!(aliases(&merged), vec!["alpha", "beta", "Beta", "Zulu"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// LiveStore — the seam a frame holds
// ─────────────────────────────────────────────────────────────────────────────

/// With no ssh config to read, the live store is the stored rows in alias order.
///
/// Both ways of having nothing to read — no path resolved at all, and a path that
/// does not exist — because an absent `~/.ssh/config` is the common case and must
/// be an empty contribution, not an error and not an empty frame. The order is
/// the merge's, so a store with no config still lists by alias.
#[test]
fn a_store_with_nothing_to_read_is_the_stored_rows_in_alias_order() {
    let config = Config {
        connections: vec![
            stored("zulu", "10.0.0.8", "ops"),
            stored("alpha", "10.0.0.7", "ops"),
        ],
    };

    let unresolved = LiveStore::new(config.clone(), None);
    assert_eq!(aliases(unresolved.all()), vec!["alpha", "zulu"]);

    let missing = LiveStore::new(
        config,
        Some(PathBuf::from("/nonexistent/path/to/ssh/config")),
    );
    assert_eq!(
        aliases(missing.all()),
        vec!["alpha", "zulu"],
        "a file that is not there contributes nothing and says nothing"
    );
}

/// A mutation re-merges, so the list a frame is holding stays coherent.
///
/// The frame asks the store for its rows once and then acts on them, so an
/// `add` that is not merged in is a row the user made and cannot see, and a
/// `remove` that is not re-merged is a row still on screen after it is gone.
/// The writes are checked against the store file too: the merge is a view, and
/// the store underneath it still has to be the truth.
#[test]
#[serial]
fn a_mutation_shows_up_in_the_list_and_on_the_disk() {
    let home = TempHome::new();
    home.write_ssh_config("Host web-01\n  HostName 10.0.0.1\n  User deploy\n");
    let mut store = LiveStore::new(Config::load(), config::ssh_config_path());
    assert_eq!(aliases(store.all()), vec!["web-01"]);

    let added = store
        .add(&draft("beta", "10.0.0.2", "deploy"))
        .expect("the add succeeds");
    assert_eq!(
        aliases(store.all()),
        vec!["beta", "web-01"],
        "the new row is merged into the sorted list, not appended behind it"
    );
    assert_eq!(home.stored_aliases(), vec!["beta"], "and it was persisted");

    let edited = store
        .update(&added.id, &draft("zulu", "10.0.0.2", "deploy"))
        .expect("the edit succeeds")
        .expect("the Connection was there to edit");
    assert_eq!(edited.alias, "zulu");
    assert_eq!(aliases(store.all()), vec!["web-01", "zulu"]);

    let removed = store
        .remove(&added.id)
        .expect("the delete succeeds")
        .expect("the Connection was there to delete");
    assert_eq!(removed.alias, "zulu");
    assert_eq!(
        aliases(store.all()),
        vec!["web-01"],
        "the deleted row is gone from the list, and the config row it shared the \
         frame with is untouched"
    );
    assert!(
        home.stored_aliases().is_empty(),
        "and the delete was persisted"
    );
}

/// The file is read when the store is built and again after a mutation — and
/// only then.
///
/// This is the freshness contract in both directions. A frame is a short-lived
/// process, so reading at open *is* reading the latest content; re-reading on
/// every `all()` would let the list change under the cursor between two
/// keystrokes, which is why the list is frozen while the frame is open. A
/// mutation is the one moment the frame is allowed to change what it is listing,
/// so that is when the file is read again.
#[test]
#[serial]
fn the_file_is_read_at_open_and_again_after_a_mutation() {
    let home = TempHome::new();
    home.write_ssh_config("Host web-01\n  HostName 10.0.0.1\n  User deploy\n");
    let mut store = LiveStore::new(Config::load(), config::ssh_config_path());

    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.1\n  User deploy\n\n\
         Host db-01\n  HostName 10.0.0.9\n  User dba\n",
    );
    assert_eq!(
        aliases(store.all()),
        vec!["web-01"],
        "a stanza written after the frame opened does not appear under the cursor"
    );

    store
        .add(&draft("beta", "10.0.0.2", "deploy"))
        .expect("the add succeeds");
    assert_eq!(
        aliases(store.all()),
        vec!["beta", "db-01", "web-01"],
        "the mutation re-reads the file, so the stanza the user added while the \
         frame was open is there with it"
    );
}
