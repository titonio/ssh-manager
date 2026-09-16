//! Seam tests for the Connection manager.
//!
//! The seam under test is `sshm::connections`: the operations any caller goes
//! through to change the Connection set. These tests go through that public
//! interface only — no rendering, no private helpers — and assert the
//! Connection set that comes back out. The frame's manage paths (#36/#37)
//! are the consumers still to come; until they land this is what keeps the
//! seam honest.

use std::ffi::OsString;

use sshm::config::Config;
use sshm::connections::{self, ConnectionDraft, Ephemeral, Store};

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

#[test]
#[serial_test::serial]
fn user_can_import_connections_from_the_ssh_config() {
    let home = TempHome::new();
    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.4\n  User deploy\n  Port 2222\n\
         \nHost db-01\n  HostName 10.0.0.9\n  User dba\n",
    );
    let mut config = Config::new();

    let report = connections::import(&mut config).expect("import should succeed");

    assert_eq!(report.imported, 2, "both Connections came across");
    assert_eq!(report.failed(), 0);
    assert_eq!(config.connections.len(), 2);
    assert_eq!(config.connections[0].alias, "web-01");
    assert_eq!(config.connections[0].host, "10.0.0.4");
    assert_eq!(config.connections[0].user, "deploy");
    assert_eq!(config.connections[0].port, 2222);
    assert_eq!(
        config.connections[1].port, 22,
        "a Host with no Port line lands on the SSH default"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// The import seam (#38) — what the first-run offer actually does on `y`
// ─────────────────────────────────────────────────────────────────────────────

/// The offer says "Import 2 connections from ~/.ssh/config?" and the user's
/// `y` has to be durable: the folded Connections are on disk, not just in
/// the set the frame was holding.
#[test]
#[serial_test::serial]
fn the_store_persists_the_connections_it_imported() {
    let home = TempHome::new();
    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\n\
         Host db-01\n  HostName 10.0.0.9\n  User dba\n",
    );
    let mut config = Config::new();
    let stored = home.path().join(".ssh").join("connections.json");
    assert!(!stored.exists(), "nothing written before the import");

    let store: &mut dyn Store = &mut config;
    let report = store
        .import_ssh_config(home.path().join(".ssh").join("config").to_str().unwrap())
        .expect("importing a readable ssh config should succeed");

    assert_eq!(report.imported, 2);
    let on_disk = std::fs::read_to_string(&stored).expect("the import should have written the set");
    assert!(
        on_disk.contains("web-01") && on_disk.contains("db-01"),
        "both imported Connections must reach the persisted set: {on_disk}"
    );
}

/// Nothing to add means nothing to write: an import that folded no new
/// Connection must not touch the user's file, the same rule the delete and
/// edit paths hold.
#[test]
#[serial_test::serial]
fn the_store_writes_nothing_when_the_import_adds_nothing() {
    let home = TempHome::new();
    let ssh_config = home.path().join(".ssh").join("config");
    home.write_ssh_config("Host web-01\n  HostName 10.0.0.4\n  User deploy\n");
    let mut config = Config::new();
    connections::add(&mut config, &draft("web-01", "10.0.0.4", "deploy", "22")).unwrap();
    let stored = home.path().join(".ssh").join("connections.json");
    std::fs::remove_file(&stored).expect("the add should have written the set");

    let store: &mut dyn Store = &mut config;
    let report = store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("an import with nothing to add is not a failure");

    assert_eq!(report.imported, 0, "every stanza is already held");
    assert!(
        !stored.exists(),
        "nothing was added, so the set must not be written at all"
    );
}

/// A partial failure is still a success: the good Connections arrive and
/// get persisted, and the ones that could not become Connections are
/// reported rather than silently dropped.
#[test]
#[serial_test::serial]
fn the_store_imports_the_good_stanzas_and_reports_the_ones_that_could_not_arrive() {
    let home = TempHome::new();
    let ssh_config = home.path().join(".ssh").join("config");
    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\n\
         Host *.example.com\n  User wildcard\n\n\
         Host db-01\n  HostName 10.0.0.9\n  User dba\n",
    );
    let mut config = Config::new();

    let store: &mut dyn Store = &mut config;
    let report = store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("a stanza that cannot import must not abort the import");

    assert_eq!(report.imported, 2, "the concrete Hosts still came across");
    assert_eq!(
        report.failures,
        vec!["wildcard host \"*.example.com\""],
        "the stanza that could not become a Connection is named"
    );
    assert_eq!(config.connections.len(), 2);

    let on_disk = std::fs::read_to_string(home.path().join(".ssh").join("connections.json"))
        .expect("a partial import is still an import");
    assert!(
        !on_disk.contains("*.example.com"),
        "the wildcard pattern must not be persisted as a Connection: {on_disk}"
    );
}

/// A second `y` on the same set adds nothing, so it writes nothing — the
/// offer cannot turn into a pile of duplicate Connections by being answered
/// twice.
#[test]
#[serial_test::serial]
fn importing_twice_adds_nothing_the_second_time() {
    let home = TempHome::new();
    let ssh_config = home.path().join(".ssh").join("config");
    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\n\
         Host db-01\n  HostName 10.0.0.9\n  User dba\n",
    );
    let mut config = Config::new();
    let store: &mut dyn Store = &mut config;

    store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("first import");
    let second = store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("a second import is not a failure");

    assert_eq!(second.imported, 0, "everything was already in the set");
    assert_eq!(second.failed(), 0);
    assert_eq!(store.all().len(), 2, "no duplicates were added");
}

/// The visual harness imports through `Ephemeral` too: the folded rows
/// really land in the set the frame is showing, but nothing reaches the
/// user's `connections.json`. Same fold, durability absent.
#[test]
#[serial_test::serial]
fn ephemeral_import_folds_the_set_without_persisting_it() {
    let home = TempHome::new();
    let ssh_config = home.path().join(".ssh").join("config");
    home.write_ssh_config(
        "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\n\
         Host *.example.com\n  User wildcard\n",
    );
    let mut store = Ephemeral::new(Vec::new());

    let report = store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("folding should succeed");

    assert_eq!(report.imported, 1, "the concrete Host arrived");
    assert_eq!(report.failures, vec!["wildcard host \"*.example.com\""]);
    assert_eq!(store.all().len(), 1, "the set really grew");
    assert_eq!(store.all()[0].alias, "web-01");

    let stored = home.path().join(".ssh").join("connections.json");
    assert!(
        !stored.exists(),
        "the harness store must never write the user's connections.json"
    );
}

#[test]
#[serial_test::serial]
fn ephemeral_import_twice_adds_nothing() {
    let home = TempHome::new();
    let ssh_config = home.path().join(".ssh").join("config");
    home.write_ssh_config("Host web-01\n  HostName 10.0.0.4\n  User deploy\n");
    let mut store = Ephemeral::new(Vec::new());

    store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("first fold");
    let second = store
        .import_ssh_config(ssh_config.to_str().unwrap())
        .expect("a second fold is not a failure");

    assert_eq!(second.imported, 0);
    assert_eq!(
        store.all().len(),
        1,
        "no duplicates in the harness set either"
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

fn connection(id: &str, alias: &str) -> sshm::config::Connection {
    sshm::config::Connection {
        id: id.to_string(),
        alias: alias.to_string(),
        host: "10.0.0.4".to_string(),
        user: "deploy".to_string(),
        port: 22,
        key_path: None,
        folder: Some("prod".to_string()),
    }
}
