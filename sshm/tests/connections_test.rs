//! Seam tests for the Connection manager.
//!
//! The seam under test is `sshm::connections`: the operations every surface
//! (fullscreen TUI today, inline frames tomorrow) calls to change the Connection
//! set. These tests go through that public interface only — no rendering, no
//! private helpers — and assert the Connection set that comes back out.

use std::ffi::OsString;

use sshm::config::Config;
use sshm::connections::{self, ConnectionDraft};

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

    let imported = connections::import(&mut config).expect("import should succeed");

    assert_eq!(imported, 2, "both Connections came across");
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
