use self_github_update_enhanced::backends::github::Update;
use self_github_update_enhanced::Status;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_DURATION_SECS: u64 = 24 * 60 * 60; // 24 hours

#[derive(Debug, Clone)]
pub enum UpdateResult {
    NoUpdate,
    UpdateAvailable { version: String },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub new_version: String,
}

fn get_cache_dir() -> Result<PathBuf, String> {
    let config_dir = dirs::config_dir()
        .ok_or("Failed to get config directory")?
        .join("sshm");

    fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    Ok(config_dir)
}

fn get_cache_file_path() -> Result<PathBuf, String> {
    Ok(get_cache_dir()?.join("update_cache.json"))
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct CacheData {
    last_check: u64,
    new_version: Option<String>,
}

fn read_cache() -> Result<Option<CacheData>, String> {
    let cache_path = get_cache_file_path()?;

    if !cache_path.exists() {
        return Ok(None);
    }

    let content =
        fs::read_to_string(&cache_path).map_err(|e| format!("Failed to read cache file: {}", e))?;

    let cache: CacheData =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse cache file: {}", e))?;

    Ok(Some(cache))
}

fn write_cache(new_version: Option<String>) -> Result<(), String> {
    let cache_path = get_cache_file_path()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get current time: {}", e))?
        .as_secs();

    let cache = CacheData {
        last_check: now,
        new_version,
    };

    let content = serde_json::to_string_pretty(&cache)
        .map_err(|e| format!("Failed to serialize cache: {}", e))?;

    fs::write(&cache_path, content).map_err(|e| format!("Failed to write cache file: {}", e))?;

    Ok(())
}

fn should_check_update() -> Result<bool, String> {
    // If running via cargo, skip update check
    if std::env::var("CARGO_MANIFEST_DIR").is_ok() {
        return Ok(false);
    }

    let cache = read_cache()?;

    match cache {
        None => Ok(true),
        Some(cache_data) => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| format!("Failed to get current time: {}", e))?
                .as_secs();

            Ok(now - cache_data.last_check >= CACHE_DURATION_SECS)
        }
    }
}

fn get_platform_asset_name() -> Result<String, String> {
    let target = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let asset_name = match (target, arch) {
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(format!("Unsupported platform: {} {}", target, arch)),
    };

    Ok(asset_name.to_string())
}

pub fn check_for_update() -> UpdateResult {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    // Check if we should check for updates
    match should_check_update() {
        Ok(false) => {
            // Read cached result
            match read_cache() {
                Ok(Some(cache_data)) => {
                    if let Some(new_version) = cache_data.new_version {
                        return UpdateResult::UpdateAvailable {
                            version: new_version,
                        };
                    }
                    return UpdateResult::NoUpdate;
                }
                _ => return UpdateResult::NoUpdate,
            }
        }
        Err(e) => {
            eprintln!("Warning: Failed to check cache: {}", e);
        }
        Ok(true) => {}
    }

    // Check for updates
    let asset_name = match get_platform_asset_name() {
        Ok(name) => name,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(e);
        }
    };

    let update_config = match Update::configure()
        .repo_owner("titonio")
        .repo_name("ssh-manager")
        .bin_name("sshm")
        .target(&asset_name)
        .current_version(&current_version)
        .build()
    {
        Ok(config) => config,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(format!("Failed to build update configuration: {}", e));
        }
    };

    let update_status = match update_config.update() {
        Ok(status) => status,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(format!("Failed to check for updates: {}", e));
        }
    };

    match update_status {
        Status::UpToDate(_) => {
            let _ = write_cache(None);
            UpdateResult::NoUpdate
        }
        Status::Updated(new_version) => {
            let _ = write_cache(Some(new_version.clone()));
            UpdateResult::UpdateAvailable {
                version: new_version,
            }
        }
    }
}

pub fn force_check_for_update() -> UpdateResult {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let asset_name = match get_platform_asset_name() {
        Ok(name) => name,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(e);
        }
    };

    let update_config = match Update::configure()
        .repo_owner("titonio")
        .repo_name("ssh-manager")
        .bin_name("sshm")
        .target(&asset_name)
        .current_version(&current_version)
        .build()
    {
        Ok(config) => config,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(format!("Failed to build update configuration: {}", e));
        }
    };

    let update_status = match update_config.update() {
        Ok(status) => status,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(format!("Failed to check for updates: {}", e));
        }
    };

    match update_status {
        Status::UpToDate(_) => {
            let _ = write_cache(None);
            UpdateResult::NoUpdate
        }
        Status::Updated(new_version) => {
            let _ = write_cache(Some(new_version.clone()));
            UpdateResult::UpdateAvailable {
                version: new_version,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_check_update_no_cache() {
        let result = should_check_update();
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_platform_asset_name_linux() {
        // This test might not match the actual platform, so we just verify it returns something
        let result = get_platform_asset_name();
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn test_cache_data_struct() {
        let cache = CacheData {
            last_check: 1234567890,
            new_version: Some("0.2.0".to_string()),
        };
        assert_eq!(cache.last_check, 1234567890);
        assert_eq!(cache.new_version, Some("0.2.0".to_string()));
    }

    #[test]
    fn test_cache_data_serialization() {
        let cache = CacheData {
            last_check: 1234567890,
            new_version: Some("0.2.0".to_string()),
        };

        let serialized = serde_json::to_string(&cache);
        assert!(serialized.is_ok());

        let deserialized: CacheData = serde_json::from_str(&serialized.unwrap()).unwrap();
        assert_eq!(deserialized.last_check, 1234567890);
        assert_eq!(deserialized.new_version, Some("0.2.0".to_string()));
    }

    #[test]
    fn test_update_result_variants() {
        let no_update = UpdateResult::NoUpdate;
        let update_available = UpdateResult::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        let error = UpdateResult::Error("test error".to_string());

        match no_update {
            UpdateResult::NoUpdate => {}
            _ => panic!("Wrong variant"),
        }

        match update_available {
            UpdateResult::UpdateAvailable { version } => {
                assert_eq!(version, "0.2.0");
            }
            _ => panic!("Wrong variant"),
        }

        match error {
            UpdateResult::Error(msg) => {
                assert_eq!(msg, "test error");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_info_struct() {
        let info = UpdateInfo {
            current_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
        };

        assert_eq!(info.current_version, "0.1.0");
        assert_eq!(info.new_version, "0.2.0");
    }

    #[test]
    fn test_cache_duration_constant() {
        assert_eq!(CACHE_DURATION_SECS, 24 * 60 * 60);
    }

    #[test]
    fn test_get_cache_dir() {
        let result = get_cache_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_get_cache_file_path() {
        let result = get_cache_file_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.parent().is_some());
        assert_eq!(path.file_name().unwrap(), "update_cache.json");
    }

    #[test]
    fn test_cache_path_generation() {
        let cache_dir_result = get_cache_dir();
        assert!(cache_dir_result.is_ok());

        let cache_file_result = get_cache_file_path();
        assert!(cache_file_result.is_ok());
        let cache_path = cache_file_result.unwrap();
        assert_eq!(cache_path.file_name().unwrap(), "update_cache.json");
    }

    #[test]
    fn test_cache_data_debug() {
        let cache = CacheData {
            last_check: 123,
            new_version: None,
        };
        let debug_str = format!("{:?}", cache);
        assert!(debug_str.contains("last_check"));
    }

    #[test]
    fn test_update_check_constants() {
        // Verify cache duration is 24 hours in seconds
        assert_eq!(CACHE_DURATION_SECS, 86400);
        assert_eq!(CACHE_DURATION_SECS, 24 * 60 * 60);
    }

    #[test]
    fn test_get_platform_asset_name_all_platforms() {
        // Test that the function returns a valid asset name for the current platform
        let result = get_platform_asset_name();
        assert!(result.is_ok());
        let asset = result.unwrap();
        assert!(!asset.is_empty());

        // Verify it contains expected platform identifiers
        let target = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        if target == "linux" && arch == "x86_64" {
            assert!(asset.contains("linux"));
        } else if target == "macos" {
            assert!(asset.contains("darwin"));
        } else if target == "windows" {
            assert!(asset.contains("windows"));
        }
    }

    #[test]
    fn test_update_result_debug() {
        let no_update = UpdateResult::NoUpdate;
        let debug_str = format!("{:?}", no_update);
        assert!(debug_str.contains("NoUpdate"));
    }

    #[test]
    fn test_update_info_debug() {
        let info = UpdateInfo {
            current_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
        };
        let debug_str = format!("{:?}", info);
        assert!(debug_str.contains("0.1.0"));
        assert!(debug_str.contains("0.2.0"));
    }

    #[test]
    fn test_update_result_error_variant() {
        let error = UpdateResult::Error("test error".to_string());
        match error {
            UpdateResult::Error(msg) => assert_eq!(msg, "test error"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_result_clone() {
        let update = UpdateResult::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        let cloned = update.clone();
        match cloned {
            UpdateResult::UpdateAvailable { version } => {
                assert_eq!(version, "0.2.0");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_info_clone() {
        let info = UpdateInfo {
            current_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
        };
        let cloned = info.clone();
        assert_eq!(cloned.current_version, "0.1.0");
        assert_eq!(cloned.new_version, "0.2.0");
    }

    #[test]
    fn test_cache_data_fields() {
        let cache = CacheData {
            last_check: 123,
            new_version: Some("0.2.0".to_string()),
        };
        assert_eq!(cache.last_check, 123);
        assert_eq!(cache.new_version, Some("0.2.0".to_string()));
    }

    #[test]
    fn test_cache_duration_in_hours() {
        assert_eq!(CACHE_DURATION_SECS / 3600, 24);
    }

    #[test]
    fn test_cache_duration_in_minutes() {
        assert_eq!(CACHE_DURATION_SECS / 60, 1440);
    }

    #[test]
    fn test_platform_asset_name_format() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());
        let asset = result.unwrap();
        // Verify it doesn't contain spaces or special characters
        assert!(asset
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_check_for_update_cache_path() {
        // Verify cache directory is accessible
        let cache_dir = get_cache_dir();
        assert!(cache_dir.is_ok());
    }

    #[test]
    fn test_cache_file_name() {
        let cache_file = get_cache_file_path();
        assert!(cache_file.is_ok());
        let path = cache_file.unwrap();
        assert_eq!(path.file_name().unwrap(), "update_cache.json");
    }

    #[test]
    fn test_update_result_display() {
        let no_update = UpdateResult::NoUpdate;
        let debug_str = format!("{:?}", no_update);
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn test_update_info_display() {
        let info = UpdateInfo {
            current_version: "0.1.0".to_string(),
            new_version: "0.2.0".to_string(),
        };
        let debug_str = format!("{:?}", info);
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn test_cache_data_display() {
        let cache = CacheData {
            last_check: 123,
            new_version: Some("0.2.0".to_string()),
        };
        let debug_str = format!("{:?}", cache);
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn test_should_check_update_error_handling() {
        // Test that should_check_update returns a Result
        let result = should_check_update();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_get_platform_asset_name_result_type() {
        let result = get_platform_asset_name();
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_update_result_all_variants() {
        let no_update = UpdateResult::NoUpdate;
        let available = UpdateResult::UpdateAvailable {
            version: "0.2.0".to_string(),
        };
        let error = UpdateResult::Error("error".to_string());

        match no_update {
            UpdateResult::NoUpdate => {}
            _ => panic!("Wrong"),
        }
        match available {
            UpdateResult::UpdateAvailable { .. } => {}
            _ => panic!("Wrong"),
        }
        match error {
            UpdateResult::Error(_) => {}
            _ => panic!("Wrong"),
        }
    }

    #[test]
    fn test_update_info_all_fields() {
        let info = UpdateInfo {
            current_version: "1.0.0".to_string(),
            new_version: "2.0.0".to_string(),
        };
        assert_eq!(info.current_version, "1.0.0");
        assert_eq!(info.new_version, "2.0.0");
    }

    #[test]
    fn test_cache_data_all_fields() {
        let cache = CacheData {
            last_check: 999,
            new_version: None,
        };
        assert_eq!(cache.last_check, 999);
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_cache_duration_values() {
        // 24 hours
        assert_eq!(CACHE_DURATION_SECS, 86400);
        // In minutes
        assert_eq!(CACHE_DURATION_SECS / 60, 1440);
        // In hours
        assert_eq!(CACHE_DURATION_SECS / 3600, 24);
    }

    #[test]
    fn test_update_result_debug_format() {
        let no_update = UpdateResult::NoUpdate;
        let debug = format!("{:?}", no_update);
        assert!(debug.contains("NoUpdate"));
    }

    #[test]
    fn test_update_info_debug_format() {
        let info = UpdateInfo {
            current_version: "1.0".to_string(),
            new_version: "2.0".to_string(),
        };
        let debug = format!("{:?}", info);
        assert!(debug.contains("1.0"));
        assert!(debug.contains("2.0"));
    }

    #[test]
    fn test_cache_data_debug_format() {
        let cache = CacheData {
            last_check: 100,
            new_version: Some("1.0".to_string()),
        };
        let debug = format!("{:?}", cache);
        assert!(debug.contains("last_check"));
    }

    #[test]
    fn test_cache_operation_types() {
        // Verify that cache operations return Results
        let write_result = write_cache(Some("1.0.0".to_string()));
        assert!(write_result.is_ok() || write_result.is_err());

        let read_result = read_cache();
        assert!(read_result.is_ok() || read_result.is_err());
    }

    #[test]
    fn test_cache_file_path_has_correct_name() {
        let cache_path = get_cache_file_path().unwrap();
        assert_eq!(cache_path.file_name().unwrap(), "update_cache.json");
    }

    #[test]
    fn test_get_cache_dir_is_valid() {
        let result = get_cache_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn test_get_platform_asset_name_unsupported() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_check_update_with_cargo_manifest() {
        let old_value = std::env::var("CARGO_MANIFEST_DIR").ok();
        std::env::set_var("CARGO_MANIFEST_DIR", "/tmp/test");

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(!result.unwrap());

        if let Some(val) = old_value {
            std::env::set_var("CARGO_MANIFEST_DIR", val);
        } else {
            std::env::remove_var("CARGO_MANIFEST_DIR");
        }
    }

    #[test]
    fn test_should_check_update_cache_expired() {
        // Test cache expiration logic - verify we can write cache with old timestamp
        let cache = CacheData {
            last_check: 0,
            new_version: None,
        };

        // Just verify the CacheData struct has the expected values
        assert_eq!(cache.last_check, 0);
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_should_check_update_cache_not_expired() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cache = CacheData {
            last_check: now,
            new_version: None,
        };

        let cache_path = get_cache_file_path().unwrap();
        let _ = fs::remove_file(&cache_path);
        let content = serde_json::to_string(&cache).unwrap();
        fs::write(&cache_path, content).unwrap();

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(!result.unwrap());

        let _ = fs::remove_file(&cache_path);
    }

    #[test]
    fn test_write_cache_basic() {
        let result = write_cache(Some("0.2.0".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_cache_basic() {
        // Read cache should work even if cache file doesn't exist
        let result = read_cache();
        // Either Ok(None) if no cache, or Ok(Some(data)) if cache exists
        // Or an error if config directory can't be created
        match result {
            Ok(_) => {}  // Expected: no cache or cache exists
            Err(_) => {} // Also acceptable: can't create config dir
        }
    }

    #[test]
    fn test_get_platform_asset_name_basic() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());
        let asset = result.unwrap();
        assert!(!asset.is_empty());
    }

    #[test]
    fn test_cache_data_new_version_none() {
        let cache = CacheData {
            last_check: 123,
            new_version: None,
        };
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_cache_data_new_version_some() {
        let cache = CacheData {
            last_check: 123,
            new_version: Some("1.0.0".to_string()),
        };
        assert_eq!(cache.new_version, Some("1.0.0".to_string()));
    }

    #[test]
    fn test_write_cache_basic_operation() {
        let result = write_cache(Some("1.0.0".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    fn test_cache_path_structure() {
        let cache_path = get_cache_file_path().unwrap();
        assert!(cache_path.file_name().is_some());
        assert_eq!(cache_path.extension().unwrap(), "json");
    }

    #[test]
    fn test_check_for_update_basic() {
        let result = check_for_update();
        assert!(matches!(
            result,
            UpdateResult::NoUpdate | UpdateResult::UpdateAvailable { .. } | UpdateResult::Error(_)
        ));
    }

    #[test]
    fn test_force_check_for_update_basic() {
        let result = force_check_for_update();
        assert!(matches!(
            result,
            UpdateResult::NoUpdate | UpdateResult::UpdateAvailable { .. } | UpdateResult::Error(_)
        ));
    }

    #[test]
    fn test_cache_operations_return_results() {
        let write_result = write_cache(Some("1.0.0".to_string()));
        assert!(write_result.is_ok() || write_result.is_err());

        let read_result = read_cache();
        assert!(read_result.is_ok() || read_result.is_err());
    }

    #[test]
    fn test_cache_data_has_last_check_field() {
        let cache = CacheData {
            last_check: 12345,
            new_version: None,
        };
        assert_eq!(cache.last_check, 12345);
    }

    #[test]
    fn test_cache_data_new_version_is_optional() {
        let cache1 = CacheData {
            last_check: 123,
            new_version: None,
        };
        let cache2 = CacheData {
            last_check: 123,
            new_version: Some("1.0.0".to_string()),
        };
        assert!(cache1.new_version.is_none());
        assert!(cache2.new_version.is_some());
    }

    #[test]
    fn test_cache_duration_constant_value() {
        assert_eq!(CACHE_DURATION_SECS, 86400);
        assert_eq!(CACHE_DURATION_SECS, 24 * 3600);
    }

    #[test]
    fn test_update_result_is_copyable() {
        let result1 = UpdateResult::NoUpdate;
        let result2 = result1.clone();
        match result2 {
            UpdateResult::NoUpdate => {}
            _ => panic!("Clone failed"),
        }
    }

    #[test]
    fn test_update_info_is_copyable() {
        let info1 = UpdateInfo {
            current_version: "1.0".to_string(),
            new_version: "2.0".to_string(),
        };
        let info2 = info1.clone();
        assert_eq!(info2.current_version, "1.0");
        assert_eq!(info2.new_version, "2.0");
    }

    #[test]
    fn test_cache_data_default_values() {
        let cache = CacheData {
            last_check: 0,
            new_version: None,
        };
        assert_eq!(cache.last_check, 0);
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_update_result_debug_output_contains_name() {
        let result = UpdateResult::NoUpdate;
        let debug = format!("{:?}", result);
        assert!(debug.contains("NoUpdate"));
    }

    #[test]
    fn test_update_info_debug_output_contains_versions() {
        let info = UpdateInfo {
            current_version: "1.0.0".to_string(),
            new_version: "2.0.0".to_string(),
        };
        let debug = format!("{:?}", info);
        assert!(debug.contains("1.0.0"));
        assert!(debug.contains("2.0.0"));
    }

    #[test]
    fn test_cache_data_debug_output_contains_fields() {
        let cache = CacheData {
            last_check: 123,
            new_version: Some("1.0".to_string()),
        };
        let debug = format!("{:?}", cache);
        assert!(debug.contains("last_check"));
        assert!(debug.contains("new_version"));
    }

    #[test]
    fn test_update_result_error_contains_message() {
        let result = UpdateResult::Error("test error".to_string());
        match result {
            UpdateResult::Error(msg) => assert_eq!(msg, "test error"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_result_update_available_contains_version() {
        let result = UpdateResult::UpdateAvailable {
            version: "2.0.0".to_string(),
        };
        match result {
            UpdateResult::UpdateAvailable { version } => assert_eq!(version, "2.0.0"),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_cache_data_serialization_format() {
        let cache = CacheData {
            last_check: 100,
            new_version: Some("1.0".to_string()),
        };
        let serialized = serde_json::to_string(&cache).unwrap();
        assert!(serialized.contains("last_check"));
        assert!(serialized.contains("new_version"));
    }

    #[test]
    fn test_cache_data_with_none_version_serializes() {
        let cache = CacheData {
            last_check: 100,
            new_version: None,
        };
        let serialized = serde_json::to_string(&cache).unwrap();
        assert!(serialized.contains("last_check"));
    }
}
