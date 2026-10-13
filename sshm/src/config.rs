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

/// The stanza being read: one slot per keyword the reader understands.
///
/// The slots are per-stanza, not per-line, because ssh_config keywords belong
/// to the `Host` line above them — which is also why a keyword written before
/// any `Host` line rides along to the next stanza instead of becoming an
/// entry of its own: there is no entry without a host to name. The `host` slot
/// holds the whole pattern list the `Host` line named; [`Stanza::take`] is
/// where it fans out into one entry per pattern.
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
    /// the slot empty rather than guessing 22 — the reader reports what the
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

    /// Take the stanza as its entries, leaving the slots empty for the next one.
    ///
    /// One entry per pattern on the `Host` line: `Host web db` is two machines
    /// sharing the stanza's User/Port/IdentityFile, and an entry per pattern is
    /// the only reading that loses nothing. Empty when no `Host` line started a
    /// stanza — which is what lets the reader flush the stanza it has finished
    /// reading at the start of the next one and again at the end of the file
    /// through the same call.
    fn take(&mut self) -> Vec<SshConfigEntry> {
        let Some(patterns) = self.host.take() else {
            return Vec::new();
        };
        let user = self.user.take();
        let port = self.port.take();
        let key_file = self.key_file.take();
        let hostname = self.hostname.take();
        patterns
            .split_whitespace()
            .map(|pattern| SshConfigEntry {
                host: pattern.to_string(),
                user: user.clone(),
                port,
                key_file: key_file.clone(),
                hostname: hostname.clone(),
            })
            .collect()
    }
}

/// One ssh_config line as its lowercased keyword and its value.
///
/// `None` when the line names no keyword or gives it nothing: a blank line, a
/// line of spaces, a bare `Host` with no value. A whole-line comment needs no
/// special case here — its first word is `#something`, and `Stanza::set`
/// ignores every keyword it does not know, which is the same reason an unknown
/// ssh_config keyword is skipped. Guarding on `#` as a keyword would be a
/// second place that decides what a whole-line comment means, and the two
/// could disagree.
///
/// The keyword is lowercased because ssh_config is spelled however the user
/// likes and the reader matches on meaning, not on case.
///
/// The value is the **rest of the line**, not just its first word, because a
/// `Host` line names every pattern it lists: keeping only the first token
/// silently dropped `db` from `Host web db`, and a dropped host is a host the
/// user cannot reach from the list even though `ssh db` works. The split into
/// patterns belongs to [`Stanza::take`], which is where a `Host` value is
/// known to be a pattern list; here every keyword's value is simply "what the
/// line said after the keyword".
///
/// Collecting stops at the first token beginning with `#`, because that is
/// where OpenSSH's `argv_split` ends the argument stream: after an unquoted
/// `#` the rest of the line is the reader's note, not the line's value.
/// Without the stop, `Host web db # staging` names four patterns and
/// `Port 2222 # ssh` stops being a number. A `#` inside a token is not a
/// comment for the same reason `argv_split` does not treat it as one, so only
/// a token's first character is checked.
fn directive(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    // OpenSSH's `strdelim` ends a keyword at `=` as well as at whitespace, so
    // `User=root` is the same line as `User root` and generated configs are
    // written that way. Reading the keyword up to the first whitespace instead
    // turns `User=root` into an unknown keyword `user=root`, and a dropped
    // field is a field adopt then persists as missing.
    let first = parts.next()?;
    let (keyword, attached) = match first.split_once('=') {
        Some((keyword, value)) => (keyword, Some(value)),
        None => (first, None),
    };
    let keyword = keyword.to_lowercase();
    let mut value = attached.unwrap_or_default().to_string();
    for part in parts {
        if part.starts_with('#') {
            break;
        }
        if !value.is_empty() {
            value.push(' ');
        }
        value.push_str(part);
    }
    // The separator may sit on its own instead — `Host = web` — where it is a
    // separator and not the first pattern: a row named `=` is a machine nobody
    // can name, so the `=` goes and the value starts after it.
    let value = value
        .strip_prefix('=')
        .unwrap_or(&value)
        .trim_start()
        .to_string();
    if value.is_empty() {
        return None;
    }
    Some((keyword, value))
}

/// Parse an ssh_config file into one entry per Host pattern it names.
///
/// Lenient by design: a file that cannot be read parses as no entries, and a
/// line that names no known keyword is skipped — the reader reports what
/// the file said, not what it wishes the file had said. A `Host web db` line
/// yields two entries sharing the stanza's fields, because both hosts are
/// real and dropping either is a silent loss. Wildcard patterns are returned
/// as-is; deciding what becomes a row is the merge's job.
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
        // reading is complete the moment the next one starts. A `Match` line
        // closes it for the same reason and then contributes nothing, because
        // its keywords belong to the Match criteria and not to the stanza
        // above: leaving the stanza open would hand a trailing `Match` block's
        // `User` or `IdentityFile` to the previous Host, and adopt would
        // persist that wrong value into the store.
        if keyword == "host" || keyword == "match" {
            entries.extend(stanza.take());
        }
        stanza.set(&keyword, &value);
    }

    entries.extend(stanza.take());

    entries
}

/// Whether `pattern` is a pattern *set* rather than the name of one machine.
///
/// OpenSSH matches a `Host` line with shell-style globs, and the set forms are
/// `*`, `?`, the `[...]` character class, and a **leading** `!` for negation —
/// a `!` anywhere else is a literal character in the glob. None of these can be
/// handed to `ssh`: `Host * !bastion` is a rule about every host except one,
/// and `web[0-9]` is ten machines, so a row drawn from either would be
/// selectable, connectable and adoptable while naming nothing.
fn is_pattern_set(pattern: &str) -> bool {
    pattern.contains(['*', '?', '[', ']']) || pattern.starts_with('!')
}

/// The rows a frame lists: sshm's own Connections with the usable `Host`
/// stanzas of `~/.ssh/config` merged into one flat list.
///
/// Three rules, and each one protects something the user would notice:
///
/// * **A glob is not a target.** `Host *` is a rule about many hosts and `ssh`
///   cannot be pointed at one of them, so a pattern that [`is_pattern_set`]
///   says is a set — a wildcard, a character class, or a negation — contributes
///   nothing: a row names exactly one machine, never a rule about many.
/// * **A stored Connection wins.** A stanza is hidden when its target — the
///   `HostName` if the stanza has one, otherwise the pattern — and its user
///   match a stored Connection, or a stanza earlier in this same file. Two
///   identical-looking rows for one machine is the failure this rule exists to
///   prevent, and it is what makes adopting a stanza collapse its two rows into
///   one instead of leaving the user choosing between them.
/// * **One list, sorted by alias.** The frame keeps input order while no query
///   is typed, so appending the config rows would park them in a block at the
///   foot — the separate section that was rejected. The sort is on the
///   lowercased alias and is stable, so a tie keeps stored rows first: the row
///   the user made by hand is the one they meet first.
///
/// A config row's id is `ssh-config:<pattern>` — the token the user would type,
/// not a uuid: nothing here writes `~/.ssh/config`, so there is no stored record
/// to key a row to between one read and the next, and an id derived from the
/// pattern is what lets a later action name the stanza it is about.
///
/// Lenient like the parser it follows: entries that cannot become a row are
/// skipped in silence, because the file is the user's and one odd line must not
/// cost them the rest of the list.
pub fn merge_ssh_config(stored: &[Connection], entries: &[SshConfigEntry]) -> Vec<Connection> {
    let mut config_rows: Vec<Connection> = Vec::new();

    for entry in entries {
        if is_pattern_set(&entry.host) {
            continue;
        }

        let host = entry.hostname.clone().unwrap_or_else(|| entry.host.clone());
        let user = entry.user.clone().unwrap_or_default();
        let already_listed = stored
            .iter()
            .chain(config_rows.iter())
            .any(|c| c.host == host && c.user == user);
        if already_listed {
            continue;
        }

        config_rows.push(Connection {
            id: format!("ssh-config:{}", entry.host),
            alias: entry.host.clone(),
            host,
            user,
            port: entry.port.unwrap_or(22),
            key_path: entry.key_file.clone(),
            folder: None,
            source: Source::SshConfig,
        });
    }

    let mut merged: Vec<Connection> = stored.to_vec();
    merged.extend(config_rows);
    merged.sort_by_key(|c| c.alias.to_lowercase());
    merged
}

/// Where the user's ssh config lives: `~/.ssh/config`, if a home
/// directory can be resolved at all.
///
/// The one place this path is spelled out. The live merge reads the file
/// through it, and a second copy of `home.join(".ssh").join("config")` is
/// a second thing to change when the answer ever moves.
pub fn ssh_config_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|home| home.join(".ssh").join("config"))
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

    /// A `Host` line yields one entry per pattern it names, not just its first.
    ///
    /// `Host web db` is two machines sharing one stanza's User/Port/
    /// IdentityFile; keeping only `web` silently drops `db`, and a host the
    /// user cannot see in the list is a host they will conclude the parser
    /// is broken about. Each pattern becomes its own entry carrying the
    /// stanza's fields, in reading order. Wildcard patterns stay in the
    /// result — deciding what is importable is the merge's job, not the
    /// parser's, and a parser that filters is a parser the merge cannot
    /// count on.
    #[test]
    fn test_parse_ssh_config_expands_multi_pattern_host_line() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-multi-pattern-test");
        let config_path = temp_dir.join("ssh_config");

        let content = r#"Host web db
    User deploy
    Port 2222
    HostName shared.example.com
    IdentityFile ~/.ssh/id_deploy

Host *
    User default

Host site
    HostName site.example.com
"#;

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, content).unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert_eq!(entries.len(), 4, "one entry per pattern, in reading order");

        assert_eq!(entries[0].host, "web");
        assert_eq!(entries[0].user, Some("deploy".to_string()));
        assert_eq!(entries[0].port, Some(2222));
        assert_eq!(entries[0].hostname, Some("shared.example.com".to_string()));
        assert_eq!(entries[0].key_file, Some("~/.ssh/id_deploy".to_string()));

        assert_eq!(entries[1].host, "db");
        assert_eq!(entries[1].user, Some("deploy".to_string()));
        assert_eq!(entries[1].port, Some(2222));
        assert_eq!(entries[1].hostname, Some("shared.example.com".to_string()));
        assert_eq!(entries[1].key_file, Some("~/.ssh/id_deploy".to_string()));

        assert_eq!(entries[2].host, "*");
        assert_eq!(entries[2].user, Some("default".to_string()));

        assert_eq!(entries[3].host, "site");
        assert_eq!(entries[3].hostname, Some("site.example.com".to_string()));

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// An unquoted `#` ends the value: OpenSSH's argv_split terminates the
    /// argument stream there, so a trailing note belongs to whoever reads the
    /// file and never becomes part of what the line says.
    ///
    /// Taking the rest of the line as the value is right for a pattern list
    /// and wrong for a note at its end: without the truncation `#` and
    /// `staging` become two more patterns, `User deploy # note` stores a user
    /// named `deploy # note`, and `Port 2222 # ssh` stops parsing as a number
    /// at all. The comment is the human's, not the value's.
    #[test]
    fn test_parse_ssh_config_truncates_values_at_inline_comments() {
        let temp_dir = std::env::temp_dir().join("ssh-manager-inline-comment-test");
        let config_path = temp_dir.join("ssh_config");

        let content = r#"Host web db # staging
    User deploy # note
    Port 2222 # ssh
    IdentityFile ~/.ssh/id_rsa # work
"#;

        fs::create_dir_all(&temp_dir).unwrap();
        fs::write(&config_path, content).unwrap();

        let entries = parse_ssh_config(config_path.to_str().unwrap());

        assert_eq!(entries.len(), 2, "the comment names no patterns");

        assert_eq!(entries[0].host, "web");
        assert_eq!(entries[0].user, Some("deploy".to_string()));
        assert_eq!(entries[0].port, Some(2222));
        assert_eq!(entries[0].key_file, Some("~/.ssh/id_rsa".to_string()));

        assert_eq!(entries[1].host, "db");
        assert_eq!(entries[1].user, Some("deploy".to_string()));
        assert_eq!(entries[1].port, Some(2222));
        assert_eq!(entries[1].key_file, Some("~/.ssh/id_rsa".to_string()));

        fs::remove_dir_all(&temp_dir).ok();
    }
}
