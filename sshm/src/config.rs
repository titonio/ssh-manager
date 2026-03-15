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

    #[cfg(test)]
    pub fn new_with_id(
        id: String,
        alias: String,
        host: String,
        user: String,
        port: u16,
        key_path: Option<String>,
        folder: Option<String>,
    ) -> Self {
        Self {
            id,
            alias,
            host,
            user,
            port,
            key_path,
            folder,
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

    #[cfg(test)]
    pub fn save_to_path(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| e.to_string())?;
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

    #[cfg(test)]
    pub fn from_connections(connections: Vec<Connection>) -> Self {
        Self { connections }
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
    use std::io::Write;
    use tempfile::TempDir;

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
        let conn = Connection::new_with_id(
            "id1".to_string(),
            "prod".to_string(),
            "server.example.com".to_string(),
            "admin".to_string(),
            2222,
            Some("/path/to/key".to_string()),
            Some("production".to_string()),
        );
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
        let conn1 = Connection::new_with_id(
            "1".to_string(),
            "a".to_string(),
            "h1".to_string(),
            "u1".to_string(),
            22,
            None,
            None,
        );
        let conn2 = Connection::new_with_id(
            "2".to_string(),
            "b".to_string(),
            "h2".to_string(),
            "u2".to_string(),
            22,
            None,
            None,
        );
        let config = Config::from_connections(vec![conn1, conn2]);
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
        let conn = Connection::new_with_id(
            "test-id".to_string(),
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
            22,
            None,
            None,
        );
        config.add_connection(conn);
        assert_eq!(config.connections.len(), 1);

        config.remove_connection("test-id");
        assert_eq!(config.connections.len(), 0);
    }

    #[test]
    fn test_remove_connection_not_found() {
        let mut config = Config::new();
        let conn = Connection::new_with_id(
            "test-id".to_string(),
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
            22,
            None,
            None,
        );
        config.add_connection(conn);

        config.remove_connection("non-existent-id");
        assert_eq!(config.connections.len(), 1);
    }

    #[test]
    fn test_update_connection() {
        let mut config = Config::new();
        let conn = Connection::new_with_id(
            "test-id".to_string(),
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
            22,
            None,
            None,
        );
        config.add_connection(conn);

        let updated = Connection::new_with_id(
            "test-id".to_string(),
            "updated".to_string(),
            "192.168.1.2".to_string(),
            "admin".to_string(),
            2222,
            None,
            None,
        );
        config.update_connection(updated);

        assert_eq!(config.connections.len(), 1);
        assert_eq!(config.connections[0].alias, "updated");
        assert_eq!(config.connections[0].host, "192.168.1.2");
    }

    #[test]
    fn test_update_connection_not_found() {
        let mut config = Config::new();
        let conn = Connection::new_with_id(
            "test-id".to_string(),
            "test".to_string(),
            "192.168.1.1".to_string(),
            "user".to_string(),
            22,
            None,
            None,
        );
        config.add_connection(conn);

        let updated = Connection::new_with_id(
            "other-id".to_string(),
            "updated".to_string(),
            "192.168.1.2".to_string(),
            "admin".to_string(),
            2222,
            None,
            None,
        );
        config.update_connection(updated);

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
        let conn1 = Connection::new_with_id(
            "1".to_string(),
            "a".to_string(),
            "h1".to_string(),
            "u1".to_string(),
            22,
            None,
            Some("zebra".to_string()),
        );
        let conn2 = Connection::new_with_id(
            "2".to_string(),
            "b".to_string(),
            "h2".to_string(),
            "u2".to_string(),
            22,
            None,
            Some("apple".to_string()),
        );
        let conn3 = Connection::new_with_id(
            "3".to_string(),
            "c".to_string(),
            "h3".to_string(),
            "u3".to_string(),
            22,
            None,
            Some("banana".to_string()),
        );

        let config = Config::from_connections(vec![conn1, conn2, conn3]);

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
        let conn1 = Connection::new_with_id(
            "1".to_string(),
            "a".to_string(),
            "h1".to_string(),
            "u1".to_string(),
            22,
            None,
            None,
        );
        let conn2 = Connection::new_with_id(
            "2".to_string(),
            "b".to_string(),
            "h2".to_string(),
            "u2".to_string(),
            22,
            None,
            None,
        );

        let config = Config::from_connections(vec![conn1, conn2]);
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
        let conn = Connection::new_with_id(
            "1".to_string(),
            "test".to_string(),
            "host".to_string(),
            "user".to_string(),
            22,
            Some("/key".to_string()),
            None,
        );

        let json = serde_json::to_string(&conn).unwrap();

        assert!(json.contains("key_path"));
    }
}
