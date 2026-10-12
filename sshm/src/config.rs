use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Where a Connection came from.
///
/// The merged list shows hosts that live in `~/.ssh/config` next to the ones
/// sshm stores itself, and a row has to be able to say which it is: a config row
/// is read-only inside sshm, and it is tagged `[ssh-config]` so that is legible
/// before the keystroke. Provenance is therefore a real property of the row,
/// never a folder stuffed with a magic string — a folder is the user's own data,
/// and a reserved name in it could only impersonate the tag.
///
/// `Stored` is the default because it is the answer for every Connection that
/// came out of `connections.json`, including every file written before this
/// enum existed: provenance is not persisted (see [`Connection::source`]), so
/// the store's shape does not change and a stored row is simply a row nobody
/// merged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// sshm's own store — `~/.ssh/connections.json`.
    #[default]
    Stored,
    /// A `Host` stanza in `~/.ssh/config`, merged into the list live.
    SshConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Connection {
    pub id: String,
    pub alias: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Which list this Connection came from, and so what may be done to it.
    ///
    /// Skipped in both directions: it is a fact about the frame being drawn,
    /// not about the store. Writing it would freeze a live fact into
    /// `connections.json`, and reading it back would let a hand-edited file
    /// claim a provenance nothing can verify — so the only way a Connection
    /// becomes [`Source::SshConfig`] is by being merged from the config.
    #[serde(skip, default)]
    pub source: Source,
}

impl Connection {
    pub fn new(alias: String, host: String, user: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            alias,
            host,
            user,
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub connections: Vec<Connection>,
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(config) => return config,
                    Err(e) => eprintln!("Failed to parse config: {}", e),
                },
                Err(e) => eprintln!("Failed to read config: {}", e),
            }
        }
        Self::new()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn config_path() -> PathBuf {
        let home = dirs::home_dir().expect("Could not find home directory");
        home.join(".ssh").join("connections.json")
    }

    pub fn add_connection(&mut self, connection: Connection) {
        self.connections.push(connection);
    }

    pub fn remove_connection(&mut self, id: &str) {
        self.connections.retain(|c| c.id != id);
    }

    pub fn update_connection(&mut self, connection: Connection) {
        if let Some(idx) = self.connections.iter().position(|c| c.id == connection.id) {
            self.connections[idx] = connection;
        }
    }
}

#[derive(Debug, Clone)]
pub struct SshConfigEntry {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub key_file: Option<String>,
    pub hostname: Option<String>,
}

/// The stanza being read: one slot per keyword the importer understands.
///
/// The slots are per-stanza, not per-line, because ssh_config keywords belong
/// to the `Host` line above them — which is also why a keyword written before
/// any `Host` line rides along to the next stanza instead of becoming an
/// entry of its own: there is no entry without a host to name.
#[derive(Default)]
struct Stanza {
    host: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    key_file: Option<String>,
    hostname: Option<String>,
}

impl Stanza {
    /// Fold one keyword's value into the stanza.
    ///
    /// An unknown keyword is ignored, and a `Port` that is not a number leaves
    /// the slot empty rather than guessing 22 — the importer reports what the
    /// file said, not what it probably meant.
    fn set(&mut self, keyword: &str, value: &str) {
        match keyword {
            "host" => self.host = Some(value.to_string()),
            "user" => self.user = Some(value.to_string()),
            "port" => self.port = value.parse().ok(),
            "identityfile" => self.key_file = Some(value.to_string()),
            "hostname" => self.hostname = Some(value.to_string()),
            _ => {}
        }
    }

    /// Take the stanza as an entry, leaving the slots empty for the next one.
    ///
    /// `None` when no `Host` line started a stanza — which is what lets the
    /// reader flush the stanza it has finished reading at the start of the
    /// next one and again at the end of the file through the same call.
    fn take(&mut self) -> Option<SshConfigEntry> {
        Some(SshConfigEntry {
            host: self.host.take()?,
            user: self.user.take(),
            port: self.port.take(),
            key_file: self.key_file.take(),
            hostname: self.hostname.take(),
        })
    }
}

/// One ssh_config line as its lowercased keyword and its value.
///
/// `None` when the line names no keyword or gives it nothing: a blank line, a
/// line of spaces, a bare `Host` with no value. A comment needs no special
/// case here — its first word is `#something`, and `Stanza::set` ignores every
/// keyword it does not know, which is the same reason an unknown ssh_config
/// keyword is skipped. Guarding on `#` as well would be a second place that
/// decides what a comment means, and the two could disagree.
///
/// The keyword is lowercased because ssh_config is spelled however the user
/// likes and the importer matches on meaning, not on case.
fn directive(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    let keyword = parts.next()?.to_lowercase();
    let value = parts.next()?.to_string();
    Some((keyword, value))
}

pub fn parse_ssh_config(path: &str) -> Vec<SshConfigEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    let mut stanza = Stanza::default();

    for line in content.lines() {
        let Some((keyword, value)) = directive(line) else {
            continue;
        };
        // A `Host` line closes the stanza before it: the entry the user was
        // reading is complete the moment the next one starts.
        if keyword == "host" {
            if let Some(entry) = stanza.take() {
                entries.push(entry);
            }
        }
        stanza.set(&keyword, &value);
    }

    if let Some(entry) = stanza.take() {
        entries.push(entry);
    }

    entries
}

/// What an import from an ssh config actually did to the Connection set.
///
/// The two numbers mean different things to the surface that reports them:
/// `imported` is what the user gained, `failures` is what could not come
/// across and never will. A non-zero `imported` alongside a non-empty
/// `failures` is a partial success — the import still happened, and the
/// trace says so (`◇ imported 9 connections, 3 skipped`) rather than
/// pretending every stanza arrived.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportReport {
    pub imported: usize,
    /// Human-readable reasons for stanzas that could not become Connections.
    pub failures: Vec<String>,
}

impl ImportReport {
    /// How many Host stanzas could not become a Connection.
    pub fn failed(&self) -> usize {
        self.failures.len()
    }
}

/// The one classification every Host stanza gets, shared by the fold and by
/// the offer's count so the two can never disagree about what an import
/// would do.
///
/// A stanza is **skipped** when a Connection with the same host and user is
/// already in the set — including one this same pass added a moment ago, so
/// a config that lists the same host twice still imports once. It is a
/// **failure** when its pattern contains a glob: `*.example.com` is a rule
/// about many hosts, and a Connection names exactly one, so there is
/// nothing honest to import. Everything else becomes a Connection
/// leniently — see [`import_from_ssh_config_with_path`].
fn plan_import(
    existing: &[Connection],
    entries: &[SshConfigEntry],
) -> (Vec<Connection>, Vec<String>) {
    let mut importable = Vec::new();
    let mut failures = Vec::new();

    for entry in entries {
        if entry.host.contains('*') || entry.host.contains('?') {
            failures.push(format!("wildcard host \"{}\"", entry.host));
            continue;
        }

        let host = entry.hostname.clone().unwrap_or_else(|| entry.host.clone());
        let user = entry.user.clone().unwrap_or_default();
        let already_present = existing
            .iter()
            .chain(importable.iter())
            .any(|c| c.host == host && c.user == user);
        if already_present {
            continue;
        }

        let mut conn = Connection::new(entry.host.clone(), host, user);
        conn.port = entry.port.unwrap_or(22);
        conn.key_path = entry.key_file.clone();
        importable.push(conn);
    }

    (importable, failures)
}

/// Parse `path` and fold its Host stanzas into `config`'s in-memory set.
///
/// Does **not** save: the caller owns when the set becomes durable, which
/// is what lets the same fold serve the persisted store and the ephemeral
/// harness alike. Lenient by design — a stanza missing a port, a user or
/// even a Hostname still becomes a Connection on the SSH defaults, because
/// a half-described Connection the user can fix is worth more than an
/// import that stopped at the first odd line.
///
/// A globbed Host pattern is the one thing that cannot cross: it lands in
/// `failures` as `wildcard host "<pattern>"` and adds nothing. A stanza
/// already in the set is skipped, which is not a failure — the user is
/// not being told they are wrong about a Connection they already have.
pub fn import_from_ssh_config_with_path(config: &mut Config, path: &str) -> ImportReport {
    let (importable, failures) = plan_import(&config.connections, &parse_ssh_config(path));
    let imported = importable.len();
    config.connections.extend(importable);

    ImportReport { imported, failures }
}

/// What an import from `path` would add to `existing`, computed without
/// touching anything.
///
/// This is the number the first-run offer shows (#38) before the user has
/// said yes, so it has to be the number the import then delivers. Both go
/// through [`plan_import`]: the offer is not a guess resting on a second
/// copy of the rules, it is the fold's own answer read off a set that is
/// never modified. A path that cannot be read parses as no stanzas, so it
/// counts `0` — an absent `~/.ssh/config` simply means nothing to offer.
pub fn count_importable(existing: &[Connection], path: &str) -> usize {
    plan_import(existing, &parse_ssh_config(path)).0.len()
}

/// Where the user's ssh config lives: `~/.ssh/config`, if a home
/// directory can be resolved at all.
///
/// The one place this path is spelled out. The import wrapper and the
/// manage runner's first-run check both need it, and a second copy of
/// `home.join(".ssh").join("config")` is a second thing to change when
/// the answer ever moves.
pub fn ssh_config_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
}

/// Fold the user's own `~/.ssh/config` into `config` and report what arrived.
///
/// The home-directory wrapper around [`import_from_ssh_config_with_path`]:
/// it decides *which* file is the user's ssh config, and nothing else.
/// Like the wrapper it wraps, it does not save — persistence belongs to
/// [`crate::connections::import`], so a caller that only wants to know
/// what an import would do can fold without writing.
///
/// A home directory that cannot be resolved is an `Err`, not a panic, so a
/// caller advertising a `Result` never panics on the way through.
pub fn import_from_ssh_config(config: &mut Config) -> Result<ImportReport, String> {
    let path = ssh_config_path().ok_or_else(|| "Could not find home directory".to_string())?;

    Ok(import_from_ssh_config_with_path(
        config,
        path.to_str().unwrap_or(""),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_new() {
        let conn = Connection::new(
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
        );
        assert_eq!(conn.alias, "test");
        assert_eq!(conn.host, "192.168.1.1");
        assert_eq!(conn.user, "user");
        assert_eq!(conn.port, 22);
        assert!(conn.key_path.is_none());
        assert!(conn.folder.is_none());
    }

    #[test]
    fn test_connection_with_all_fields() {
        let conn = Connection {
            id: "id1".to_string(),
            alias: "prod".to_string(),
            host: "server.example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/path/to/key".to_string()),
            folder: Some("production".to_string()),
            source: Source::Stored,
        };
        assert_eq!(conn.id, "id1");
        assert_eq!(conn.alias, "prod");
        assert_eq!(conn.host, "server.example.com");
        assert_eq!(conn.user, "admin");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.key_path, Some("/path/to/key".to_string()));
        assert_eq!(conn.folder, Some("production".to_string()));
    }

    /// Provenance is a fact about the row a frame is showing, not a fact about
    /// the store.
    ///
    /// `connections.json` keeps its current shape: a config-sourced Connection
    /// written out carries no `source` key at all, and reading it back answers
    /// `Stored` — the only honest answer for something that came out of the
    /// store. Persisting provenance would freeze a live fact into a file, and
    /// would change the format for a property nothing on disk can be trusted
    /// about.
    #[test]
    fn test_source_is_never_persisted() {
        let conn = Connection {
            id: "id1".to_string(),
            alias: "build-box".to_string(),
            host: "build.internal".to_string(),
            user: "me".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
            source: Source::SshConfig,
        };

        let json = serde_json::to_string(&conn).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            value.get("source").is_none(),
            "provenance leaked into the persisted shape: {json}"
        );

        let back: Connection = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source, Source::Stored);
        assert_eq!(back.alias, "build-box");
        assert_eq!(back.port, 2222);
    }

    /// A hand-written `source` in `connections.json` changes nothing.
    ///
    /// The field is skipped in both directions, so a file cannot claim a
    /// connection came from the config: provenance is only ever set by the code
    /// that merged the stanza into the list, never by the data.
    #[test]
    fn test_source_cannot_be_forged_from_disk() {
        let json = r#"{
            "id": "id1",
            "alias": "build-box",
            "host": "build.internal",
            "user": "me",
            "port": 2222,
            "source": "SshConfig"
        }"#;

        let conn: Connection = serde_json::from_str(json).unwrap();
        assert_eq!(conn.source, Source::Stored);
    }

    #[test]
    fn test_config_new() {
        let config = Config::new();
        assert!(config.connections.is_empty());
    }

    #[test]
    fn test_config_from_connections() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "1".to_string(),
            alias: "a".to_string(),
            host: "h1".to_string(),
            user: "u1".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        assert_eq!(config.connections.len(), 2);
    }

    #[test]
    fn test_add_connection() {
        let mut config = Config::new();
        let conn = Connection::new(
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
        );
        config.add_connection(conn);
        assert_eq!(config.connections.len(), 1);
    }

    #[test]
    fn test_remove_connection() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "test-id".to_string(),
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        assert_eq!(config.connections.len(), 1);

        config.remove_connection("test-id");
        assert_eq!(config.connections.len(), 0);
    }

    #[test]
    fn test_remove_connection_not_found() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "test-id".to_string(),
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.remove_connection("non-existent-id");
        assert_eq!(config.connections.len(), 1);
    }

    #[test]
    fn test_update_connection() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "test-id".to_string(),
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.update_connection(Connection {
            id: "test-id".to_string(),
            alias: "updated".to_string(),
            host: "192.168.1.2".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        assert_eq!(config.connections.len(), 1);
        assert_eq!(config.connections[0].alias, "updated");
        assert_eq!(config.connections[0].host, "192.168.1.2");
    }

    #[test]
    fn test_update_connection_not_found() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "test-id".to_string(),
            alias: "test".to_string(),
            host: "192.168.1.1".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.update_connection(Connection {
            id: "other-id".to_string(),
            alias: "updated".to_string(),
            host: "192.168.1.2".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        assert_eq!(config.connections.len(), 1);
        assert_eq!(config.connections[0].alias, "test");
    }

    #[test]
    fn test_connection_key_path_optional() {
        let conn = Connection::new("test".to_string(), "host".to_string(), "user".to_string());

        let json = serde_json::to_string(&conn).unwrap();

        assert!(!json.contains("key_path"));
    }

    #[test]
    fn test_connection_folder_optional() {
        let conn = Connection::new("test".to_string(), "host".to_string(), "user".to_string());

        let json = serde_json::to_string(&conn).unwrap();

        assert!(!json.contains("folder"));
    }

    #[test]
    fn test_folder_sorted() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "1".to_string(),
            alias: "a".to_string(),
            host: "h1".to_string(),
            user: "u1".to_string(),
            port: 22,
            key_path: None,
            folder: Some("zebra".to_string()),
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: Some("apple".to_string()),
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "3".to_string(),
            alias: "c".to_string(),
            host: "h3".to_string(),
            user: "u3".to_string(),
            port: 22,
            key_path: None,
            folder: Some("banana".to_string()),
            source: Source::Stored,
        });

        // Test that we can access folder information directly from connections
        let folders: Vec<String> = config
            .connections
            .iter()
            .filter_map(|c| c.folder.clone())
            .collect();
        let mut folders_sorted = folders.clone();
        folders_sorted.sort();
        folders_sorted.dedup();

        assert_eq!(folders_sorted, vec!["apple", "banana", "zebra"]);
    }

    #[test]
    fn test_config_deserialize() {
        let json = r#"{"connections":[{"id":"1","alias":"test","host":"localhost","user":"root","port":22}]}"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.connections.len(), 1);
        assert_eq!(config.connections[0].alias, "test");
    }

    #[test]
    fn test_connection_deserialize_with_all_fields() {
        let json = r#"{"id":"1","alias":"prod","host":"server.com","user":"admin","port":2222,"key_path":"/path/key","folder":"prod"}"#;

        let conn: Connection = serde_json::from_str(json).unwrap();

        assert_eq!(conn.alias, "prod");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.key_path, Some("/path/key".to_string()));
        assert_eq!(conn.folder, Some("prod".to_string()));
    }

    #[test]
    fn test_config_serialize_empty() {
        let config = Config::new();

        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("connections"));
    }

    #[test]
    fn test_config_with_multiple_connections_serialize() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "1".to_string(),
            alias: "a".to_string(),
            host: "h1".to_string(),
            user: "u1".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("connections"));
    }

    #[test]
    fn test_ssh_config_entry_debug() {
        let entry = SshConfigEntry {
            host: "test".to_string(),
            user: Some("user".to_string()),
            port: Some(22),
            key_file: None,
            hostname: None,
        };

        let debug = format!("{:?}", entry);

        assert!(debug.contains("test"));
    }

    #[test]
    fn test_connection_debug() {
        let conn = Connection::new("alias".to_string(), "host".to_string(), "user".to_string());

        let debug = format!("{:?}", conn);

        assert!(debug.contains("alias"));
    }

    #[test]
    fn test_config_debug() {
        let config = Config::new();

        let debug = format!("{:?}", config);

        assert!(debug.contains("connections"));
    }

    #[test]
    fn test_config_impl_default() {
        let config = Config::default();

        assert!(config.connections.is_empty());
    }

    #[test]
    fn test_connection_with_key_path_serialize() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "host".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: Some("/key".to_string()),
            folder: None,
            source: Source::Stored,
        };

        let json = serde_json::to_string(&conn).unwrap();

        assert!(json.contains("key_path"));
    }

    #[test]
    fn test_config_save_and_load() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-test");
        let config_path = temp_dir.join("connections.json");

        let mut config = Config::new();
        config.add_connection(Connection {
            id: "test-id".to_string(),
            alias: "test-server".to_string(),
            host: "192.168.1.100".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/home/user/.ssh/id_rsa".to_string()),
            folder: Some("production".to_string()),
            source: Source::Stored,
        });

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let loaded_json = fs::read_to_string(&config_path).unwrap();
        let loaded_config: Config = serde_json::from_str(&loaded_json).unwrap();

        assert_eq!(loaded_config.connections.len(), 1);
        assert_eq!(loaded_config.connections[0].alias, "test-server");
        assert_eq!(loaded_config.connections[0].host, "192.168.1.100");
        assert_eq!(loaded_config.connections[0].port, 2222);
        assert_eq!(
            loaded_config.connections[0].key_path,
            Some("/home/user/.ssh/id_rsa".to_string())
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_config_save_creates_directory() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-new-test");
        let config_path = temp_dir.join("subdir").join("connections.json");

        let config = Config::new();

        let json = serde_json::to_string_pretty(&config).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, json).unwrap();

        assert!(config_path.exists());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_config_load_invalid_json() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-invalid-test");
        let config_path = temp_dir.join("connections.json");

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, "invalid json content").unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let result: Result<Config, _> = serde_json::from_str(&content);

        assert!(result.is_err());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_update_connection_with_existing_id() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "conn-1".to_string(),
            alias: "original".to_string(),
            host: "original.example.com".to_string(),
            user: "original".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "other".to_string(),
            host: "other.example.com".to_string(),
            user: "other".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.update_connection(Connection {
            id: "conn-1".to_string(),
            alias: "updated".to_string(),
            host: "updated.example.com".to_string(),
            user: "updated".to_string(),
            port: 2222,
            key_path: Some("/new/key".to_string()),
            folder: Some("new-folder".to_string()),
            source: Source::Stored,
        });

        assert_eq!(config.connections.len(), 2);
        let updated = config
            .connections
            .iter()
            .find(|c| c.id == "conn-1")
            .unwrap();
        assert_eq!(updated.alias, "updated");
        assert_eq!(updated.host, "updated.example.com");
        assert_eq!(updated.user, "updated");
        assert_eq!(updated.port, 2222);
        assert_eq!(updated.key_path, Some("/new/key".to_string()));
        assert_eq!(updated.folder, Some("new-folder".to_string()));
    }

    #[test]
    fn test_update_connection_preserves_other_connections() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "conn-1".to_string(),
            alias: "original".to_string(),
            host: "original.example.com".to_string(),
            user: "original".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "other".to_string(),
            host: "other.example.com".to_string(),
            user: "other".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.update_connection(Connection {
            id: "conn-1".to_string(),
            alias: "updated".to_string(),
            host: "updated.example.com".to_string(),
            user: "updated".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        let other = config
            .connections
            .iter()
            .find(|c| c.id == "conn-2")
            .unwrap();
        assert_eq!(other.alias, "other");
        assert_eq!(other.host, "other.example.com");
    }

    #[test]
    fn test_remove_connection_with_existing_id() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "conn-1".to_string(),
            alias: "server1".to_string(),
            host: "server1.example.com".to_string(),
            user: "user1".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "server2".to_string(),
            host: "server2.example.com".to_string(),
            user: "user2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });
        config.add_connection(Connection {
            id: "conn-3".to_string(),
            alias: "server3".to_string(),
            host: "server3.example.com".to_string(),
            user: "user3".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.remove_connection("conn-2");

        assert_eq!(config.connections.len(), 2);
        let ids: Vec<&str> = config.connections.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"conn-1"));
        assert!(ids.contains(&"conn-3"));
        assert!(!ids.contains(&"conn-2"));
    }

    #[test]
    fn test_remove_connection_from_empty_config() {
        let mut config = Config::new();
        config.remove_connection("non-existent");
        assert!(config.connections.is_empty());
    }

    #[test]
    fn test_remove_connection_last_connection() {
        let mut config = Config::new();
        config.add_connection(Connection {
            id: "last-conn".to_string(),
            alias: "last".to_string(),
            host: "last.example.com".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        config.remove_connection("last-conn");

        assert!(config.connections.is_empty());
    }

    #[test]
    fn test_parse_ssh_config_basic() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-ssh-test");
        let config_path = temp_dir.join("ssh_config");

        let ssh_config_content = r#"# My SSH config
Host prod
    Hostname prod.example.com
    User deploy
    Port 2222
    IdentityFile ~/.ssh/id_rsa_prod

Host dev
    Hostname dev.example.com
    User developer
    IdentityFile ~/.ssh/id_rsa_dev
"#;

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].host, "prod");
        assert_eq!(entries[0].hostname, Some("prod.example.com".to_string()));
        assert_eq!(entries[0].user, Some("deploy".to_string()));
        assert_eq!(entries[0].port, Some(2222));
        assert_eq!(entries[0].key_file, Some("~/.ssh/id_rsa_prod".to_string()));

        assert_eq!(entries[1].host, "dev");
        assert_eq!(entries[1].hostname, Some("dev.example.com".to_string()));
        assert_eq!(entries[1].user, Some("developer".to_string()));
        assert_eq!(entries[1].port, None);
        assert_eq!(entries[1].key_file, Some("~/.ssh/id_rsa_dev".to_string()));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_parse_ssh_config_empty_file() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-empty-test");
        let config_path = temp_dir.join("ssh_config");

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, "").unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert!(entries.is_empty());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_parse_ssh_config_only_comments() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-comments-test");
        let config_path = temp_dir.join("ssh_config");

        let content = r#"# This is a comment
# Another comment
"#;

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, content).unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert!(entries.is_empty());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_parse_ssh_config_nonexistent_file() {
        let entries = parse_ssh_config("/nonexistent/path/to/ssh/config");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_ssh_config_with_wildcards() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-wildcard-test");
        let config_path = temp_dir.join("ssh_config");

        let content = r#"Host *.example.com
    Hostname wildcard.example.com
    User wildcard

Host *
    User default
    Port 22
"#;

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, content).unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].host, "*.example.com");
        assert_eq!(entries[1].host, "*");

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_with_path_adds_every_importable_stanza_and_reports_the_count() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-report-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host web-01
    HostName web.example.com
    User www

Host db-01
    HostName db.example.com
    User dbadmin
    Port 2222
    IdentityFile ~/.ssh/db_key
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 2, "both Host stanzas came across");
        assert_eq!(report.failed(), 0, "nothing here is unimportable");
        assert_eq!(config.connections.len(), 2);

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_reports_a_wildcard_host_as_a_failure_and_does_not_import_it() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-wildcard-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host web-01
    HostName web.example.com
    User www

Host *.example.com
    User wildcard

Host staging-?
    User qa
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 1, "only the concrete Host arrived");
        assert_eq!(
            report.failures,
            vec![
                "wildcard host \"*.example.com\"",
                "wildcard host \"staging-?\""
            ],
            "each globbed pattern is named, in the order it was read"
        );
        assert_eq!(config.connections.len(), 1);
        assert_eq!(config.connections[0].alias, "web-01");

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_of_an_all_wildcard_config_adds_nothing_and_fails_every_stanza() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-all-wildcard-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host *
    User default
    Port 22

Host *.internal
    User internal
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 0);
        assert_eq!(report.failed(), 2, "every stanza is accounted for");
        assert!(
            config.connections.is_empty(),
            "a pattern is not a Connection: nothing was added"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_skips_already_present_connections_without_calling_them_failures() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-skip-not-failure-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host web-01
    HostName 10.0.0.4
    User deploy

Host db-01
    HostName 10.0.0.9
    User dba
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        config.add_connection(Connection {
            id: "already-here".to_string(),
            alias: "web-01-renamed".to_string(),
            host: "10.0.0.4".to_string(),
            user: "deploy".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(
            report.imported, 1,
            "only the Connection they did not have arrived"
        );
        assert_eq!(
            report.failed(),
            0,
            "having it already is not being wrong: a skip is not a failure"
        );
        assert_eq!(config.connections.len(), 2);
        assert_eq!(
            config.connections[0].id, "already-here",
            "the Connection they had is left alone, alias and all"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_count_importable_is_the_same_number_the_import_then_delivers() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-count-agrees-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host web-01
    HostName 10.0.0.4
    User deploy

Host *.example.com
    User wildcard

Host db-01
    HostName 10.0.0.9
    User dba

Host web-02
    HostName 10.0.0.5
    User deploy
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        config.add_connection(Connection {
            id: "already-here".to_string(),
            alias: "web-01".to_string(),
            host: "10.0.0.4".to_string(),
            user: "deploy".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        let offered = count_importable(&config.connections, config_path.to_str().unwrap());
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(
            offered, report.imported,
            "the offer promised {offered}; the import delivered {}",
            report.imported
        );
        assert_eq!(
            offered, 2,
            "two new Connections, one already held, one wildcard"
        );
        assert_eq!(
            count_importable(&config.connections, config_path.to_str().unwrap()),
            0,
            "once imported, the same set has nothing left to offer"
        );

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_count_importable_on_a_path_that_cannot_be_read_is_zero() {
        assert_eq!(count_importable(&[], "/nonexistent/ssh/config"), 0);
    }

    /// Point `HOME` at a throwaway directory so the home-resolving import
    /// never reads the real `~/.ssh/config`, and put it back on the way out.
    struct TempHome {
        dir: tempfile::TempDir,
        previous: Option<std::ffi::OsString>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp home");
            let previous = std::env::var_os("HOME");
            std::env::set_var("HOME", dir.path());
            Self { dir, previous }
        }

        fn write_ssh_config(&self, content: &str) {
            let ssh_dir = self.dir.path().join(".ssh");
            fs::create_dir_all(&ssh_dir).unwrap();
            fs::write(ssh_dir.join("config"), content).unwrap();
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

    #[test]
    #[serial_test::serial]
    fn test_import_from_home_reports_what_its_ssh_config_held() {
        let home = TempHome::new();
        home.write_ssh_config(
            "Host web-01\n  HostName 10.0.0.4\n  User deploy\n\n\
             Host *.example.com\n  User wildcard\n\n\
             Host db-01\n  HostName 10.0.0.9\n  User dba\n",
        );
        let mut config = Config::new();

        let report =
            import_from_ssh_config(&mut config).expect("a resolvable home is not a failure");

        assert_eq!(report.imported, 2, "both concrete Hosts came across");
        assert_eq!(report.failures, vec!["wildcard host \"*.example.com\""]);
        assert_eq!(config.connections.len(), 2);
    }

    #[test]
    #[serial_test::serial]
    fn test_import_from_home_adds_nothing_when_there_is_no_ssh_config() {
        let _home = TempHome::new();
        let mut config = Config::new();

        let report = import_from_ssh_config(&mut config)
            .expect("a missing ~/.ssh/config is nothing to import, not an error");

        assert_eq!(report.imported, 0);
        assert_eq!(report.failed(), 0);
        assert!(config.connections.is_empty());
    }

    #[test]
    fn test_import_from_ssh_config_basic() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host webserver
    Hostname web.example.com
    User www

Host database
    Hostname db.example.com
    User dbadmin
    Port 2222
    IdentityFile ~/.ssh/db_key
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 2);
        assert_eq!(config.connections.len(), 2);

        let web = config
            .connections
            .iter()
            .find(|c| c.alias == "webserver")
            .unwrap();
        assert_eq!(web.host, "web.example.com");
        assert_eq!(web.user, "www");
        assert_eq!(web.port, 22);

        let db = config
            .connections
            .iter()
            .find(|c| c.alias == "database")
            .unwrap();
        assert_eq!(db.host, "db.example.com");
        assert_eq!(db.user, "dbadmin");
        assert_eq!(db.port, 2222);
        assert_eq!(db.key_path, Some("~/.ssh/db_key".to_string()));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_from_ssh_config_skips_existing() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-skip-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        let ssh_config_content = r#"Host existing
    Hostname existing.example.com
    User existing

Host new
    Hostname new.example.com
    User newuser
"#;

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, ssh_config_content).unwrap();

        let mut config = Config::new();
        config.add_connection(Connection {
            id: "existing-id".to_string(),
            alias: "existing".to_string(),
            host: "existing.example.com".to_string(),
            user: "existing".to_string(),
            port: 22,
            key_path: None,
            folder: None,
            source: Source::Stored,
        });

        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 1);
        assert_eq!(config.connections.len(), 2);
        let existing = config
            .connections
            .iter()
            .find(|c| c.alias == "existing")
            .unwrap();
        assert_eq!(existing.id, "existing-id");

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_from_ssh_config_empty() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-import-empty-test");
        let ssh_dir = temp_dir.join(".ssh");
        let config_path = ssh_dir.join("config");

        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(&config_path, "").unwrap();

        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(report.imported, 0);
        assert!(config.connections.is_empty());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_from_ssh_config_nonexistent() {
        let mut config = Config::new();
        let report = import_from_ssh_config_with_path(&mut config, "/nonexistent/ssh/config");

        assert_eq!(report.imported, 0);
        assert!(config.connections.is_empty());
    }
}
