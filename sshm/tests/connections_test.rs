//! Seam tests for the Connection manager.
//!
//! The seam under test is `sshm::connections`: the operations any caller goes
//! through to change the Connection set. These tests go through that public
//! interface only — no rendering, no private helpers — and assert the
//! Connection set that comes back out. The frame's manage paths (#36/#37)
//! are the consumers still to come; until they land this is what keeps the
//! seam honest.

use std::ffi::OsString;

use sshm::config::{parse_ssh_config, ssh_config_path, Config, Connection, Source};
use sshm::connections::{self, ConnectionDraft, Ephemeral, Store};
use sshm::live::LiveStore;

/// Point `HOME` at a throwaway directory so a test never touches the real
/// `~/.ssh/connections.json`, and put it back on the way out.
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

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Lay down a `~/.ssh/config` inside the throwaway home.
    fn write_ssh_config(&self, content: &str) {
        let ssh_dir = self.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).expect("create .ssh");
        std::fs::write(ssh_dir.join("config"), content).expect("write ~/.ssh/config");
    }
    /// The aliases the store file holds, in file order: proof that a mutation
    /// reached the disk and not merely the in-memory set.
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

fn draft(alias: &str, host: &str, user: &str, port: &str) -> ConnectionDraft {
    ConnectionDraft {
        alias: alias.to_string(),
        host: host.to_string(),
        user: user.to_string(),
        port: port.to_string(),
        key_path: String::new(),
        folder: String::new(),
    }
}

#[test]
#[serial_test::serial]
fn deleting_an_unknown_id_reports_that_nothing_was_deleted() {
    let _home = TempHome::new();
    let mut config = Config::new();
    connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();

    let removed = connections::remove(&mut config, "no-such-connection")
        .expect("an unknown id is not a failure, there was just nothing to delete");

    assert_eq!(removed, None);
    assert_eq!(config.connections.len(), 1, "the set is left as it was");
}

#[test]
#[serial_test::serial]
fn user_can_delete_a_connection_and_see_what_was_deleted() {
    let _home = TempHome::new();
    let mut config = Config::new();
    let doomed =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    let kept = connections::add(&mut config, &draft("db-01", "10.0.0.9", "dba", "5432")).unwrap();

    let removed = connections::remove(&mut config, &doomed.id)
        .expect("deleting a known Connection should succeed")
        .expect("the Connection was there to be deleted");

    assert_eq!(removed.alias, "web-01");
    assert_eq!(config.connections.len(), 1);
    assert_eq!(
        config.connections[0].id, kept.id,
        "the other Connection survives"
    );
}

#[test]
#[serial_test::serial]
fn editing_an_unknown_id_changes_no_connection() {
    let _home = TempHome::new();
    let mut config = Config::new();
    let kept = connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();

    let edited = connections::edit(
        &mut config,
        "no-such-connection",
        &draft("ghost", "10.9.9.9", "x", "22"),
    )
    .expect("an unknown id is not a failure, there was just nothing to edit");

    assert!(
        edited.is_none(),
        "an unknown id reports that no Connection was edited, so a surface cannot \
         trace an edit that never happened"
    );
    assert_eq!(
        config.connections.len(),
        1,
        "an unknown id must not append a Connection"
    );
    assert_eq!(
        config.connections[0].host, kept.host,
        "the Connection that was not targeted stays as it was"
    );
}

#[test]
#[serial_test::serial]
fn editing_an_unknown_id_writes_nothing() {
    let home = TempHome::new();
    let mut config = Config::new();
    connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    let stored = home.path().join(".ssh").join("connections.json");
    std::fs::remove_file(&stored).expect("the add should have written the set");

    connections::edit(
        &mut config,
        "no-such-connection",
        &draft("ghost", "10.9.9.9", "x", "22"),
    )
    .expect("an unknown id is not a failure");

    assert!(
        !stored.exists(),
        "nothing matched, so the set must not be written at all"
    );
}

#[test]
#[serial_test::serial]
fn user_can_edit_a_connection_in_place() {
    let _home = TempHome::new();
    let mut config = Config::new();
    let original =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();

    let edited = connections::edit(
        &mut config,
        &original.id,
        &draft("web-01", "10.0.0.99", "deploy", "2222"),
    )
    .expect("editing a known Connection should succeed")
    .expect("the Connection was there to be edited");

    assert_eq!(
        config.connections.len(),
        1,
        "an edit replaces the Connection, it does not append one"
    );
    assert_eq!(
        config.connections[0].id, original.id,
        "the id survives the edit"
    );
    assert_eq!(edited.host, "10.0.0.99");
    assert_eq!(edited.port, 2222);
}

#[test]
#[serial_test::serial]
fn a_port_that_is_not_a_number_falls_back_to_the_ssh_default() {
    let _home = TempHome::new();
    let mut config = Config::new();

    let added = connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "ssh"))
        .expect("a mistyped port should not stop the Connection being added");

    assert_eq!(added.port, 22);
}

#[test]
#[serial_test::serial]
fn blank_optional_fields_are_absent_not_empty_strings() {
    let _home = TempHome::new();
    let mut config = Config::new();

    let blank = connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22"))
        .expect("a draft with no key or folder should still add");
    assert_eq!(
        blank.key_path, None,
        "a blank key path is not stored as \"\""
    );
    assert_eq!(blank.folder, None, "a blank folder is not stored as \"\"");

    let filled = ConnectionDraft {
        key_path: "/home/deploy/.ssh/id_ed25519".to_string(),
        folder: "production".to_string(),
        ..draft("web-02", "10.0.0.5", "deploy", "22")
    };
    let filled = connections::add(&mut config, &filled).expect("a filled draft should add");
    assert_eq!(
        filled.key_path,
        Some("/home/deploy/.ssh/id_ed25519".to_string())
    );
    assert_eq!(filled.folder, Some("production".to_string()));
}

#[test]
#[serial_test::serial]
fn user_can_add_a_connection_from_a_draft() {
    let _home = TempHome::new();
    let mut config = Config::new();

    let added = connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "2222"))
        .expect("adding a Connection should succeed");

    assert_eq!(config.connections.len(), 1);
    assert_eq!(added.alias, "web-01");
    assert_eq!(added.host, "10.0.0.4");
    assert_eq!(added.user, "deploy");
    assert_eq!(added.port, 2222);
    assert!(
        !added.id.is_empty(),
        "an added Connection carries an id of its own"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The `Store` seam (#36) — what the frame driver actually holds
// ─────────────────────────────────────────────────────────────────────────────

/// The manage driver deletes through `&mut dyn Store`, never through
/// `Config` directly. This test pins the contract at that seam: the delete
/// removes exactly the named Connection and the listed set is what the
/// frame would show next.
#[test]
#[serial_test::serial]
fn the_store_deletes_exactly_the_named_connection() {
    let _home = TempHome::new();
    let mut config = Config::new();
    let doomed =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    let kept = connections::add(&mut config, &draft("db-01", "10.0.0.9", "dba", "5432")).unwrap();

    let store: &mut dyn Store = &mut config;
    let removed = store
        .remove(&doomed.id)
        .expect("deleting a known Connection should succeed")
        .expect("the Connection was there to be deleted");

    assert_eq!(
        removed.id, doomed.id,
        "the named Connection is what came back"
    );
    assert_eq!(store.all().len(), 1, "exactly one Connection was removed");
    assert_eq!(store.all()[0].id, kept.id, "the neighbour survives");
}

/// The driver's `y` must be durable: a delete through the trait changes the
/// file on disk, because the trait's `Config` impl goes through
/// `connections::remove`, which is the one implementation of "delete a
/// Connection" the module owns.
#[test]
#[serial_test::serial]
fn the_store_persists_the_delete_it_makes() {
    let home = TempHome::new();
    let mut config = Config::new();
    let doomed =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    connections::add(&mut config, &draft("db-01", "10.0.0.9", "dba", "5432")).unwrap();
    let stored = home.path().join(".ssh").join("connections.json");

    let store: &mut dyn Store = &mut config;
    store.remove(&doomed.id).expect("delete should succeed");

    let on_disk = std::fs::read_to_string(&stored).expect("the set is on disk");
    assert!(
        !on_disk.contains("web-01"),
        "the deleted Connection must be gone from the persisted set: {on_disk}"
    );
    assert!(
        on_disk.contains("db-01"),
        "the untouched Connection must survive on disk: {on_disk}"
    );
}

/// The visual harness deletes through `Ephemeral`: the row really leaves
/// the set the frame is showing — otherwise the harness would not be
/// looking at a real delete — but nothing reaches the user's
/// `connections.json`.
#[test]
fn ephemeral_really_removes_the_connection_from_the_set() {
    let mut store = Ephemeral::new(vec![
        connection("id-web-01", "web-01"),
        connection("id-web-02", "web-02"),
    ]);

    let removed = store
        .remove("id-web-01")
        .expect("removing a known Connection should succeed")
        .expect("the Connection was there to be removed");

    assert_eq!(removed.alias, "web-01");
    assert_eq!(store.all().len(), 1, "the set really shrank");
    assert_eq!(store.all()[0].alias, "web-02", "the right one survived");
}

#[test]
fn ephemeral_unknown_id_removes_nothing() {
    let mut store = Ephemeral::new(vec![connection("id-web-01", "web-01")]);

    let removed = store
        .remove("no-such-id")
        .expect("an unknown id is not a failure");

    assert_eq!(removed, None);
    assert_eq!(store.all().len(), 1, "the set is left as it was");
}

#[test]
#[serial_test::serial]
fn ephemeral_deletes_never_reach_the_users_file() {
    let home = TempHome::new();
    let mut store = Ephemeral::new(vec![connection("id-web-01", "web-01")]);

    store.remove("id-web-01").expect("removal should succeed");

    let stored = home.path().join(".ssh").join("connections.json");
    assert!(
        !stored.exists(),
        "the harness store must never write the user's connections.json"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The edit seam (#37) — an edit answers about the Connection it was asked about
// ─────────────────────────────────────────────────────────────────────────────

/// The id is how a surface says *which* Connection, so an edit has to answer
/// with the id it was handed rather than a freshly minted one. Mint a new id
/// here and the row the user just saved is no longer the row they were standing
/// on: the change never reaches the stored Connection at all, and the next edit
/// or delete aimed at the old id finds nothing — the user edits a Connection and
/// ssh keeps using the host and port they just replaced.
#[test]
#[serial_test::serial]
fn an_edit_answers_with_the_id_the_caller_asked_for() {
    let home = TempHome::new();
    let mut config = Config::new();
    let original =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();

    let edited = connections::edit(
        &mut config,
        &original.id,
        &draft("web-01", "10.0.0.99", "deploy", "2222"),
    )
    .expect("editing a known Connection should succeed")
    .expect("the Connection was there to be edited");

    assert_eq!(
        edited.id, original.id,
        "the edited Connection comes back under the id the caller passed in, not a new one"
    );
    assert_eq!(config.connections.len(), 1, "an edit replaces a Connection");
    assert_eq!(
        config.connections[0].id, original.id,
        "the set still holds that Connection under that id"
    );
    assert_eq!(
        config.connections[0].host, "10.0.0.99",
        "the change landed on the Connection the caller named"
    );

    let on_disk = std::fs::read_to_string(home.path().join(".ssh").join("connections.json"))
        .expect("an edit is durable");
    assert!(
        on_disk.contains("10.0.0.99") && on_disk.contains(&original.id),
        "the persisted set holds the edited Connection under its own id: {on_disk}"
    );
}

/// The frame's `◇ edited` note rests on the store answering with the Connection
/// it wrote. A store that answers `None` after a write that landed tells the
/// surface nothing was edited, so the user saves a change, sees no note for it,
/// and has no way to tell a save that happened from one that did not.
#[test]
#[serial_test::serial]
fn the_store_update_answers_with_the_connection_it_wrote() {
    let home = TempHome::new();
    let mut config = Config::new();
    let target =
        connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    let neighbour =
        connections::add(&mut config, &draft("db-01", "10.0.0.9", "dba", "5432")).unwrap();

    let store: &mut dyn Store = &mut config;
    let written = store
        .update(&target.id, &draft("web-01", "10.0.0.99", "deploy", "2222"))
        .expect("editing a known Connection should succeed")
        .expect("the store found that Connection and edited it, so it must say so");

    assert_eq!(written.id, target.id, "the answer is about that Connection");
    assert_eq!(written.host, "10.0.0.99", "the answer is the new value");
    assert_eq!(written.port, 2222);
    assert_eq!(
        store.all().len(),
        2,
        "the set the frame lists has one more edit, not one more row"
    );
    assert_eq!(
        store.all()[1].id,
        neighbour.id,
        "the Connection that was not targeted is untouched"
    );

    let on_disk = std::fs::read_to_string(home.path().join(".ssh").join("connections.json"))
        .expect("the store persists the edit it made");
    assert!(
        on_disk.contains("10.0.0.99"),
        "what the store answered with is what it wrote: {on_disk}"
    );
}

/// The visual harness edits through `Ephemeral`, and the row it hands back has
/// to be the row the real store would have changed — id included. Mint an id
/// here and the harness's edited row becomes a different Connection from the one
/// the frame was listing, so the next keystroke on that row edits nothing.
#[test]
fn an_ephemeral_edit_keeps_the_id_the_caller_asked_for() {
    let mut store = Ephemeral::new(vec![connection("id-web-01", "web-01")]);

    let written = store
        .update("id-web-01", &draft("web-01", "10.0.0.99", "deploy", "2222"))
        .expect("editing a known Connection should succeed")
        .expect("the Connection was there to be edited");

    assert_eq!(
        written.id, "id-web-01",
        "the harness edit answers with the id it was given, not a fresh one"
    );
    assert_eq!(store.all().len(), 1, "an edit replaces a Connection");
    assert_eq!(
        store.all()[0].id,
        "id-web-01",
        "the row the frame lists keeps the id the next edit will ask for"
    );
    assert_eq!(
        store.all()[0].host,
        "10.0.0.99",
        "the change landed on that row"
    );
}

/// The harness edits the row the user is standing on, so the lookup has to find
/// *that* row. Match any other row instead and editing the second Connection
/// silently rewrites the first: one row changes on screen, a different row loses
/// its host, user and port, and two rows end up answering to the same id.
#[test]
fn ephemeral_update_edits_the_connection_with_that_id_and_no_other() {
    let mut store = Ephemeral::new(vec![
        connection("id-web-01", "web-01"),
        connection("id-web-02", "web-02"),
    ]);

    let written = store
        .update("id-web-02", &draft("web-02", "10.0.0.99", "deploy", "2222"))
        .expect("editing a known Connection should succeed")
        .expect("the Connection was there to be edited");

    assert_eq!(
        written.id, "id-web-02",
        "the edit answers about the row it was asked for"
    );
    assert_eq!(store.all().len(), 2, "an edit replaces a Connection");
    assert_eq!(
        store.all()[0].alias,
        "web-01",
        "the Connection that was not targeted keeps its alias"
    );
    assert_eq!(
        store.all()[0].host,
        "10.0.0.4",
        "the Connection that was not targeted keeps its host"
    );
    assert_eq!(
        store.all()[1].host,
        "10.0.0.99",
        "the targeted row is the one that changed"
    );
    assert_eq!(
        store.all().iter().filter(|c| c.id == "id-web-02").count(),
        1,
        "the id still names exactly one row"
    );
}

/// The harness answers the frame through the same trait the persisted store
/// does, so its edit half has to answer too: the Connection it wrote. Answer
/// `None` from the harness and the visual harness never shows its own
/// `◇ edited` note, which is the note the manage path is being designed
/// against.
#[test]
fn ephemeral_update_answers_with_the_connection_it_wrote() {
    let mut store = Ephemeral::new(vec![
        connection("id-web-01", "web-01"),
        connection("id-web-02", "web-02"),
    ]);

    let written = store
        .update("id-web-01", &draft("web-01", "10.0.0.99", "deploy", "2222"))
        .expect("editing a known Connection should succeed")
        .expect("the harness store found that Connection and edited it, so it must say so");

    assert_eq!(written.id, "id-web-01");
    assert_eq!(written.alias, "web-01");
    assert_eq!(written.host, "10.0.0.99");
    assert_eq!(written.port, 2222);
    assert_eq!(store.all().len(), 2, "the set keeps its two rows");
    assert_eq!(store.all()[0].host, "10.0.0.99", "the set really changed");
    assert_eq!(
        store.all()[1].alias,
        "web-02",
        "the other row is left alone"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Reading the saved set — the Connections the user already has
// ─────────────────────────────────────────────────────────────────────────────

/// `Config::load` is how sshm finds out which Connections the user has. Return
/// an empty set here and a real `~/.ssh/connections.json` is silently ignored:
/// the Inline Picker lists nothing, the manage frame has nothing to edit, and
/// the user's saved Connections are gone while the file that holds them sits
/// untouched on disk.
#[test]
#[serial_test::serial]
fn loading_reads_the_connections_the_user_saved() {
    let home = TempHome::new();
    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).expect("create .ssh");
    std::fs::write(
        ssh_dir.join("connections.json"),
        r#"{"connections":[{"id":"id-web-01","alias":"web-01","host":"10.0.0.4","user":"deploy","port":2222,"key_path":"/home/deploy/.ssh/id_ed25519","folder":"production"}]}"#,
    )
    .expect("write the saved set");

    let loaded = Config::load();

    assert_eq!(
        loaded.connections.len(),
        1,
        "the Connection the user saved comes back, it is not silently dropped"
    );
    let web: &Connection = &loaded.connections[0];
    assert_eq!(web.id, "id-web-01", "the id survives the round trip");
    assert_eq!(web.alias, "web-01");
    assert_eq!(web.host, "10.0.0.4");
    assert_eq!(web.user, "deploy");
    assert_eq!(web.port, 2222);
    assert_eq!(
        web.key_path,
        Some("/home/deploy/.ssh/id_ed25519".to_string())
    );
    assert_eq!(web.folder, Some("production".to_string()));
}

// ─────────────────────────────────────────────────────────────────────────────
// What is not a config line — the reader's own floor
// ─────────────────────────────────────────────────────────────────────────────

/// A commented-out stanza stays commented out: `# Host old-box` is a note the
/// user left themselves, not a stanza waiting to be read, and a blank line is a
/// blank line. The reader's contract is that a directive is neither a blank line
/// nor a comment, so a user who commented a box out must not find it back on a
/// list they never asked for.
///
/// Sweep note, so nobody hunts for the missing kill: the `||` → `&&` mutant on
/// that guard is *equivalent*, and this test does not kill it — it passes under
/// the mutant. With `&&` the guard can never fire (a trimmed line is never both
/// empty and starting with `#`), so a comment line does become a directive —
/// but its keyword starts with `#`, which is not `host` and matches no slot of
/// `Stanza::set`, so nothing is folded and no stanza is flushed; a blank line
/// still answers `None` at the keyword's `?`. `parse_ssh_config` is the only
/// caller and it returns the same entries either way: verified by hand and by
/// diffing the parse of every single-line file and every ordered pair drawn from
/// a 38-line pool of blanks, whitespace, comments, wildcards and stanzas, plus
/// 20 000 random six-line files — byte for byte identical, 21 482 cases.
#[test]
#[serial_test::serial]
fn a_comment_or_a_blank_line_never_becomes_a_stanza() {
    let home = TempHome::new();
    home.write_ssh_config(
        "# Host commented-out\n\
         \n\
         # a note with several words\n\
         \t\n\
         Host real-one\n  HostName 10.0.0.1\n  User alice\n\
         #user bob\n#hostname 10.0.0.2\n#port 9999\n#\n",
    );

    let entries = parse_ssh_config(
        home.path()
            .join(".ssh")
            .join("config")
            .to_str()
            .expect("a utf-8 temp path"),
    );

    let hosts: Vec<String> = entries.iter().map(|e| e.host.clone()).collect();
    assert_eq!(
        hosts,
        vec!["real-one".to_string()],
        "only the uncommented Host was read"
    );
    assert_eq!(
        entries[0].hostname.as_deref(),
        Some("10.0.0.1"),
        "and it kept its own fields, not a commented-out neighbour's"
    );
    assert_eq!(entries[0].user.as_deref(), Some("alice"));
}

fn connection(id: &str, alias: &str) -> sshm::config::Connection {
    sshm::config::Connection {
        id: id.to_string(),
        alias: alias.to_string(),
        host: "10.0.0.4".to_string(),
        user: "deploy".to_string(),
        port: 22,
        key_path: None,
        folder: Some("prod".to_string()),
        source: Source::Stored,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Adopt: a Config Host copied into sshm's store
//
// The store is the only thing that can write `connections.json`, so the copy an
// adopt makes is the store's job: it decides what the new row is, whether it
// may exist at all, and what the list looks like once it does. The frame asks;
// these tests are about what answering means.
// ─────────────────────────────────────────────────────────────────────────────

/// A `~/.ssh/config` holding one box, and the live store reading it: the pair a
/// frame is actually handed, built the way `main.rs` builds one.
fn live_store_with_a_config_host() -> (TempHome, LiveStore) {
    let home = TempHome::new();
    home.write_ssh_config(
        "Host build-box\n  HostName build.example.com\n  User ci\n  Port 2222\n  IdentityFile ~/.ssh/id_ci\n",
    );
    let store = LiveStore::new(Config::load(), ssh_config_path());
    (home, store)
}

/// The config row of that fixture, as the frame would hand it back.
fn config_row(store: &LiveStore) -> Connection {
    store
        .all()
        .iter()
        .find(|c| c.source == Source::SshConfig)
        .cloned()
        .expect("the fixture lists a config row")
}

#[test]
#[serial_test::serial]
fn adopting_a_config_row_writes_it_into_sshms_store() {
    let (home, mut store) = live_store_with_a_config_host();
    let target = config_row(&store);

    let adopted = store
        .adopt(&target)
        .expect("a Config Host can be copied into the store");

    assert_eq!(adopted.alias, "build-box");
    assert_eq!(adopted.host, "build.example.com");
    assert_eq!(adopted.user, "ci");
    assert_eq!(adopted.port, 2222);
    assert_eq!(adopted.key_path.as_deref(), Some("~/.ssh/id_ci"));
    assert_eq!(
        adopted.folder, None,
        "a stanza has no folder and an adopt does not invent one"
    );
    assert_eq!(
        adopted.source,
        Source::Stored,
        "the copy is sshm's own from here on, which is what makes it editable"
    );
    assert_ne!(
        adopted.id, target.id,
        "the copy is a new Connection, not the stanza re-labelled"
    );
    assert_eq!(
        home.stored_aliases(),
        vec!["build-box".to_string()],
        "the write reached the file, not just the list"
    );
}

/// An adopt lands unfiled whatever the row it was made from claims.
///
/// A `Host` stanza has no folder, so a config row wearing one is a row the
/// frame is holding from before something else changed. Inheriting it would
/// file the new Connection under a folder that only ever described a different
/// row — the copy would open in the wrong place, and the user would have to
/// know that before they could find it.
#[test]
#[serial_test::serial]
fn an_adopted_row_lands_unfiled_even_if_the_row_it_came_from_carries_a_folder() {
    let (_home, mut store) = live_store_with_a_config_host();
    let target = Connection {
        folder: Some("somewhere-else".into()),
        ..config_row(&store)
    };

    let adopted = store
        .adopt(&target)
        .expect("a Config Host can be copied into the store");

    assert_eq!(
        adopted.folder, None,
        "the copy is unfiled: an adopt never files a Connection the user did \
         not file"
    );
}

/// The collision rule is what makes an adopt visible as *one* row: the stored
/// copy matches the stanza on host and user, so the config row it was made from
/// stops being listed in the same keystroke that wrote the copy. A frame that
/// kept both would show the user a duplicate they just resolved.
#[test]
#[serial_test::serial]
fn adopting_a_config_row_leaves_one_row_for_that_host() {
    let (_home, mut store) = live_store_with_a_config_host();
    let target = config_row(&store);
    assert_eq!(
        store.all().len(),
        1,
        "setup: the fixture lists exactly the config row"
    );

    let adopted = store.adopt(&target).expect("adopted");

    let rows: Vec<&Connection> = store
        .all()
        .iter()
        .filter(|c| c.alias == "build-box")
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the stored copy hides the stanza it was made from: {:?}",
        aliases(store.all())
    );
    assert_eq!(
        rows[0].id, adopted.id,
        "and the row that survives is the copy"
    );
    assert_eq!(rows[0].source, Source::Stored);
}

#[test]
#[serial_test::serial]
fn adopting_a_stored_row_is_refused() {
    let (home, mut store) = live_store_with_a_config_host();
    let kept = store
        .add(&draft("web-01", "10.0.0.4", "deploy", "22"))
        .expect("the fixture's stored row");
    let before = home.stored_aliases();

    let refusal = store
        .adopt(&kept)
        .expect_err("a row that is already sshm's own has nothing to adopt");

    assert!(
        refusal.contains("web-01"),
        "the refusal names the row it refused: {refusal}"
    );
    assert_eq!(
        home.stored_aliases(),
        before,
        "a refusal writes nothing: a second copy of a Connection the user \
         already owns is not what they asked for"
    );
}

#[test]
#[serial_test::serial]
fn a_stored_store_has_nothing_to_adopt() {
    let _home = TempHome::new();
    let mut config = Config::new();
    let row = connection("id-web-01", "web-01");

    let refusal = config.adopt(&row).expect_err(
        "a Config holds only stored rows, so there is nothing \
                     in it to copy in",
    );

    assert!(
        refusal.contains("ssh-config"),
        "the refusal says why: {refusal}"
    );
    assert!(
        config.connections.is_empty(),
        "and the refusal is not a write with an error stapled on"
    );
}

#[test]
fn ephemeral_adoption_adds_the_row_and_writes_nothing() {
    let mut store = Ephemeral::new(vec![config_host("build-box", "build.example.com", "ci")]);
    let target = store.all()[0].clone();

    let adopted = store
        .adopt(&target)
        .expect("the visual harness has to be able to show an adopt");

    assert_eq!(
        store.all().len(),
        2,
        "the copy joins the set the frame is showing"
    );
    assert_eq!(adopted.source, Source::Stored);
    assert_eq!(adopted.folder, None);
    assert_ne!(adopted.id, target.id);
}

#[test]
fn ephemeral_refuses_to_adopt_a_stored_row() {
    let mut store = Ephemeral::new(vec![connection("id-web-01", "web-01")]);
    let target = store.all()[0].clone();

    let refusal = store
        .adopt(&target)
        .expect_err("the harness holds the same rule the real store does");

    assert!(
        refusal.contains("web-01"),
        "the refusal names the row it refused: {refusal}"
    );
    assert_eq!(store.all().len(), 1, "and nothing was added");
}

/// A Config Host as the merge would hand it over: no folder, no uuid.
fn config_host(alias: &str, host: &str, user: &str) -> Connection {
    Connection {
        id: format!("ssh-config:{alias}"),
        alias: alias.to_string(),
        host: host.to_string(),
        user: user.to_string(),
        port: 22,
        key_path: None,
        folder: None,
        source: Source::SshConfig,
    }
}

/// The aliases of a list, for failure messages that read as a list.
fn aliases(rows: &[Connection]) -> Vec<&str> {
    rows.iter().map(|c| c.alias.as_str()).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// The reserved folder name — `ssh-config` is the frame's word, not the user's
//
// A stored row filed under `ssh-config` would draw the same `[ssh-config]`
// prefix a Config Host draws, and so would claim to be a row sshm must not
// edit. The name is therefore refused wherever a folder is written, and
// refused whatever its spelling: `SSH-Config` reads as the same word to a user
// scanning the list, which is exactly the impersonation the rule exists to stop.
// ─────────────────────────────────────────────────────────────────────────────

/// A draft filed under `folder` — the one field this rule is about.
fn filed(alias: &str, folder: &str) -> ConnectionDraft {
    ConnectionDraft {
        folder: folder.to_string(),
        ..draft(alias, "10.0.0.4", "deploy", "22")
    }
}

#[test]
#[serial_test::serial]
fn adding_to_the_reserved_folder_is_refused_and_writes_nothing() {
    let home = TempHome::new();
    let mut config = Config::new();

    let refusal = connections::add(&mut config, &filed("web-01", "SSH-Config"))
        .expect_err("the reserved folder is not a folder a stored row may wear");

    assert!(
        refusal.contains("SSH-Config"),
        "the refusal names the folder it refused: {refusal}"
    );
    assert!(
        refusal.contains("reserved"),
        "and says why it refused: {refusal}"
    );
    assert!(config.connections.is_empty(), "a refusal adds nothing");
    assert!(
        home.stored_aliases().is_empty(),
        "and a refusal writes nothing to the user's file"
    );
}

#[test]
#[serial_test::serial]
fn editing_a_folder_to_the_reserved_name_is_refused_and_changes_nothing() {
    let home = TempHome::new();
    let mut config = Config::new();
    let kept = connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22"))
        .expect("the fixture's stored row");

    let refusal = connections::edit(&mut config, &kept.id, &filed("web-01", "SSH-Config"))
        .expect_err("an edit may not file a row under the reserved name either");

    assert!(
        refusal.contains("SSH-Config"),
        "the refusal names the folder it refused: {refusal}"
    );
    assert_eq!(
        config.connections[0].folder, None,
        "the row keeps the folder it had — here, none"
    );
    let on_disk = std::fs::read_to_string(home.path().join(".ssh").join("connections.json"))
        .expect("the set was already on disk before the refusal");
    assert!(
        !on_disk.contains("SSH-Config"),
        "a refusal writes nothing: the user's file holds no reserved folder\n{on_disk}"
    );
    assert!(
        on_disk.contains("web-01"),
        "and the row they had is still there\n{on_disk}"
    );
}

#[test]
#[serial_test::serial]
fn the_live_store_refuses_the_reserved_folder_on_both_halves_of_its_seam() {
    let (home, mut store) = live_store_with_a_config_host();
    let kept = store
        .add(&draft("web-01", "10.0.0.4", "deploy", "22"))
        .expect("the fixture's stored row");

    let refusal = store
        .add(&filed("web-02", "ssh-config"))
        .expect_err("the frame writes through this seam, so the seam refuses");
    assert!(refusal.contains("reserved"), "got: {refusal}");

    let refusal = store
        .update(&kept.id, &filed("web-01", "SSH-CONFIG"))
        .expect_err("and it refuses on the edit half too, whatever the casing");
    assert!(refusal.contains("reserved"), "got: {refusal}");

    assert_eq!(
        home.stored_aliases(),
        vec!["web-01".to_string()],
        "neither refusal wrote anything"
    );
}

/// The rule is about one word, not about folders: `prod` is the user's own and
/// stays theirs. This is also the test that says the refusal is not a blanket
/// refusal of every folder.
#[test]
#[serial_test::serial]
fn an_ordinary_folder_is_still_an_ordinary_folder() {
    let _home = TempHome::new();
    assert!(
        !connections::is_reserved_folder("prod"),
        "`prod` belongs to the user"
    );
    assert!(
        !connections::is_reserved_folder(""),
        "no folder at all is not the reserved one"
    );
    assert!(
        !connections::is_reserved_folder("ssh-configs"),
        "a folder that merely starts with the reserved name is a different word"
    );
    assert!(
        connections::is_reserved_folder("ssh-config"),
        "the reserved name itself, as the frame spells it"
    );
    assert!(
        connections::is_reserved_folder("SSH-Config"),
        "and any spelling of it, because the list is read by eye"
    );

    let mut config = Config::new();
    let added = connections::add(&mut config, &filed("web-01", "prod"))
        .expect("`prod` is still a folder a stored row may wear");
    assert_eq!(added.folder.as_deref(), Some("prod"));
}
