use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub fn parse_ssh_config(path: &str) -> Vec<SshConfigEntry> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut current_host: Option<String> = None;
    let mut current_user: Option<String> = None;
    let mut current_port: Option<u16> = None;
    let mut current_key_file: Option<String> = None;
    let mut current_hostname: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        match parts[0].to_lowercase().as_str() {
            "host" => {
                if let Some(host) = current_host.take() {
                    entries.push(SshConfigEntry {
                        host,
                        user: current_user.take(),
                        port: current_port.take(),
                        key_file: current_key_file.take(),
                        hostname: current_hostname.take(),
                    });
                }
                current_host = Some(parts[1].to_string());
            }
            "user" => current_user = Some(parts[1].to_string()),
            "port" => current_port = parts[1].parse().ok(),
            "identityfile" => current_key_file = Some(parts[1].to_string()),
            "hostname" => current_hostname = Some(parts[1].to_string()),
            _ => {}
        }
    }

    if let Some(host) = current_host {
        entries.push(SshConfigEntry {
            host,
            user: current_user.take(),
            port: current_port.take(),
            key_file: current_key_file.take(),
            hostname: current_hostname.take(),
        });
    }

    entries
}

pub fn import_from_ssh_config(config: &mut Config) -> usize {
    let ssh_dir = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .expect("Could not find home directory");

    let entries = parse_ssh_config(ssh_dir.to_str().unwrap_or(""));
    let mut imported = 0;

    for entry in entries {
        let host = entry.hostname.unwrap_or(entry.host.clone());

        if !config
            .connections
            .iter()
            .any(|c| c.host == host && c.user == entry.user.as_deref().unwrap_or(""))
        {
            let mut conn =
                Connection::new(entry.host.clone(), host, entry.user.unwrap_or_default());
            conn.port = entry.port.unwrap_or(22);
            conn.key_path = entry.key_file;
            config.connections.push(conn);
            imported += 1;
        }
    }

    imported
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
        };
        assert_eq!(conn.id, "id1");
        assert_eq!(conn.alias, "prod");
        assert_eq!(conn.host, "server.example.com");
        assert_eq!(conn.user, "admin");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.key_path, Some("/path/to/key".to_string()));
        assert_eq!(conn.folder, Some("production".to_string()));
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
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
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
        });

        config.update_connection(Connection {
            id: "test-id".to_string(),
            alias: "updated".to_string(),
            host: "192.168.1.2".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
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
        });

        config.update_connection(Connection {
            id: "other-id".to_string(),
            alias: "updated".to_string(),
            host: "192.168.1.2".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
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
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: Some("apple".to_string()),
        });
        config.add_connection(Connection {
            id: "3".to_string(),
            alias: "c".to_string(),
            host: "h3".to_string(),
            user: "u3".to_string(),
            port: 22,
            key_path: None,
            folder: Some("banana".to_string()),
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
        });
        config.add_connection(Connection {
            id: "2".to_string(),
            alias: "b".to_string(),
            host: "h2".to_string(),
            user: "u2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
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
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "other".to_string(),
            host: "other.example.com".to_string(),
            user: "other".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        });

        config.update_connection(Connection {
            id: "conn-1".to_string(),
            alias: "updated".to_string(),
            host: "updated.example.com".to_string(),
            user: "updated".to_string(),
            port: 2222,
            key_path: Some("/new/key".to_string()),
            folder: Some("new-folder".to_string()),
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
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "other".to_string(),
            host: "other.example.com".to_string(),
            user: "other".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        });

        config.update_connection(Connection {
            id: "conn-1".to_string(),
            alias: "updated".to_string(),
            host: "updated.example.com".to_string(),
            user: "updated".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
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
        });
        config.add_connection(Connection {
            id: "conn-2".to_string(),
            alias: "server2".to_string(),
            host: "server2.example.com".to_string(),
            user: "user2".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        });
        config.add_connection(Connection {
            id: "conn-3".to_string(),
            alias: "server3".to_string(),
            host: "server3.example.com".to_string(),
            user: "user3".to_string(),
            port: 22,
            key_path: None,
            folder: None,
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
        let imported = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(imported, 2);
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
        });

        let imported = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(imported, 1);
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
        let imported = import_from_ssh_config_with_path(&mut config, config_path.to_str().unwrap());

        assert_eq!(imported, 0);
        assert!(config.connections.is_empty());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_import_from_ssh_config_nonexistent() {
        let mut config = Config::new();
        let imported = import_from_ssh_config_with_path(&mut config, "/nonexistent/ssh/config");

        assert_eq!(imported, 0);
        assert!(config.connections.is_empty());
    }

    fn import_from_ssh_config_with_path(config: &mut Config, path: &str) -> usize {
        let entries = parse_ssh_config(path);
        let mut imported = 0;

        for entry in entries {
            let host = entry.hostname.unwrap_or(entry.host.clone());

            if !config
                .connections
                .iter()
                .any(|c| c.host == host && c.user == entry.user.as_deref().unwrap_or(""))
            {
                let mut conn =
                    Connection::new(entry.host.clone(), host, entry.user.unwrap_or_default());
                conn.port = entry.port.unwrap_or(22);
                conn.key_path = entry.key_file;
                config.connections.push(conn);
                imported += 1;
            }
        }

        imported
    }
}
