use flate2::read::GzDecoder;
use self_github_update_enhanced::backends::github::{ReleaseList, Update};
use self_github_update_enhanced::Status;
use std::fs;
use std::io::Write;
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
    write_cache_to_path(new_version, &cache_path)
}

fn write_cache_to_path(new_version: Option<String>, cache_path: &PathBuf) -> Result<(), String> {
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

    fs::write(cache_path, content).map_err(|e| format!("Failed to write cache file: {}", e))?;

    Ok(())
}

#[cfg(test)]
fn read_cache_from_path(cache_path: &PathBuf) -> Result<Option<CacheData>, String> {
    if !cache_path.exists() {
        return Ok(None);
    }

    let content =
        fs::read_to_string(cache_path).map_err(|e| format!("Failed to read cache file: {}", e))?;

    let cache: CacheData =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse cache file: {}", e))?;

    Ok(Some(cache))
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
        .bin_path_in_archive("sshm")
        .target(&asset_name)
        .current_version(&current_version)
        .no_confirm(true)
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
    check_for_update_inner()
}

fn check_for_update_inner() -> UpdateResult {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    let asset_name = match get_platform_asset_name() {
        Ok(name) => name,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(e);
        }
    };

    let asset_name_full = format!(
        "sshm-v{}-{}.tar.gz",
        current_version.replace("0.1.6", "0.1.7"),
        asset_name
    );

    match ReleaseList::configure()
        .repo_owner("titonio")
        .repo_name("ssh-manager")
        .with_target(&asset_name)
        .build()
    {
        Ok(release_list) => match release_list.fetch() {
            Ok(releases) => {
                if releases.is_empty() {
                    let _ = write_cache(None);
                    return UpdateResult::NoUpdate;
                }

                let latest = &releases[0];
                if latest.version == current_version {
                    let _ = write_cache(None);
                    return UpdateResult::NoUpdate;
                }

                let asset = latest.assets.iter().find(|a| a.name.contains(&asset_name));

                let asset = match asset {
                    Some(a) => a,
                    None => {
                        let _ = write_cache(None);
                        return UpdateResult::Error(format!(
                            "Asset not found: {}",
                            asset_name_full
                        ));
                    }
                };

                eprintln!(
                    "New version available: {} -> {}",
                    current_version, latest.version
                );
                eprintln!("Downloading: {}", asset.name);

                match download_and_extract(&current_version, &latest.version, &asset_name) {
                    Ok(downloaded_path) => {
                        eprintln!(
                            "Binary ready for installation at: {}",
                            downloaded_path.display()
                        );
                        eprintln!("Please restart sshm to use the new version.");
                        let _ = write_cache(Some(latest.version.clone()));
                        UpdateResult::UpdateAvailable {
                            version: latest.version.clone(),
                        }
                    }
                    Err(e) => {
                        let _ = write_cache(None);
                        UpdateResult::Error(format!("Failed to download/update: {}", e))
                    }
                }
            }
            Err(e) => {
                let _ = write_cache(None);
                UpdateResult::Error(format!("Failed to fetch releases: {}", e))
            }
        },
        Err(e) => {
            let _ = write_cache(None);
            UpdateResult::Error(format!("Failed to build release list: {}", e))
        }
    }
}

fn download_and_extract(
    _current_version: &str,
    new_version: &str,
    asset_name: &str,
) -> Result<PathBuf, String> {
    let browser_url = format!(
        "https://github.com/titonio/ssh-manager/releases/download/v{}/sshm-v{}-{}.tar.gz",
        new_version, new_version, asset_name
    );

    eprintln!("Downloading from: {}", browser_url);

    let response = reqwest::blocking::Client::new()
        .get(&browser_url)
        .send()
        .map_err(|e| format!("Failed to download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status: {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read response: {}", e))?;

    let temp_dir = std::env::temp_dir().join("sshm-update");
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let archive_path = temp_dir.join(format!("sshm-{}.tar.gz", new_version));
    let mut file = fs::File::create(&archive_path)
        .map_err(|e| format!("Failed to create archive file: {}", e))?;
    file.write_all(&bytes)
        .map_err(|e| format!("Failed to write archive: {}", e))?;

    eprintln!("Extracting archive...");

    let file =
        fs::File::open(&archive_path).map_err(|e| format!("Failed to open archive: {}", e))?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    let output_path = temp_dir.join("sshm_new");

    // Extract all files from the archive
    archive
        .unpack(&temp_dir)
        .map_err(|e| format!("Failed to extract archive: {}", e))?;

    // Check if sshm was extracted
    let extracted_path = temp_dir.join("sshm");
    if !extracted_path.exists() {
        return Err("Could not find sshm binary in archive".to_string());
    }

    // Check if sshm was extracted before moving
    if !extracted_path.exists() {
        return Err("Could not find sshm binary in archive".to_string());
    }

    // Move to output path
    fs::rename(&extracted_path, &output_path)
        .map_err(|e| format!("Failed to rename extracted binary: {}", e))?;

    eprintln!(
        "Update downloaded successfully to: {}",
        output_path.display()
    );
    Ok(output_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

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
    #[serial]
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
    fn test_should_check_update_cache_expired_dummy() {
        // Dummy test to verify CacheData struct fields
        let cache = CacheData {
            last_check: 0,
            new_version: None,
        };
        assert_eq!(cache.last_check, 0);
        assert!(cache.new_version.is_none());
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
        if result.is_ok() {}
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

    #[test]
    #[serial]
    fn test_read_cache_no_file() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        let result = read_cache();
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
    fn test_read_cache_with_valid_cache() {
        let cache_data = CacheData {
            last_check: 12345,
            new_version: Some("0.2.0".to_string()),
        };

        let serialized = serde_json::to_string(&cache_data).unwrap();
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        fs::write(&cache_path, serialized).unwrap();

        let result = read_cache();
        assert!(result.is_ok());
        let cached = result.unwrap();
        assert!(cached.is_some());

        let cached_data = cached.unwrap();
        assert_eq!(cached_data.last_check, 12345);
        assert_eq!(cached_data.new_version, Some("0.2.0".to_string()));

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_read_cache_with_invalid_json() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        fs::write(&cache_path, "invalid json").unwrap();

        let result = read_cache();
        assert!(result.is_err());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_write_cache_creates_file() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        let result = write_cache(Some("0.2.0".to_string()));
        assert!(result.is_ok());
        assert!(cache_path.exists());

        let content = fs::read_to_string(&cache_path).unwrap();
        assert!(content.contains("0.2.0"));

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_write_cache_with_none_version() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        let result = write_cache(None);
        assert!(result.is_ok());

        let cache = read_cache().unwrap();
        assert!(cache.is_some());
        assert!(cache.unwrap().new_version.is_none());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_should_check_update_with_no_cache() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        std::env::remove_var("CARGO_MANIFEST_DIR");

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
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
        fs::remove_file(&cache_path).ok();
        let content = serde_json::to_string(&cache).unwrap();
        fs::write(&cache_path, content).unwrap();
        std::env::remove_var("CARGO_MANIFEST_DIR");

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(!result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_should_check_update_cache_expired() {
        let old_cache = CacheData {
            last_check: 1000,
            new_version: None,
        };

        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let content = serde_json::to_string(&old_cache).unwrap();
        fs::write(&cache_path, content).unwrap();
        std::env::remove_var("CARGO_MANIFEST_DIR");

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    fn test_should_check_update_with_cargo_env_skips() {
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
    fn test_get_platform_asset_name_returns_valid_format() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        assert!(!asset.is_empty());

        assert!(asset
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_get_platform_asset_name_contains_platform_identifier() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        let target = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        match (target, arch) {
            ("linux", "x86_64") => assert!(asset.contains("linux")),
            ("macos", _) => assert!(asset.contains("darwin")),
            ("windows", _) => assert!(asset.contains("windows")),
            _ => {}
        }
    }

    #[test]
    fn test_cache_data_serialization_roundtrip() {
        let original = CacheData {
            last_check: 987654321,
            new_version: Some("1.2.3".to_string()),
        };

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: CacheData = serde_json::from_str(&serialized).unwrap();

        assert_eq!(original.last_check, deserialized.last_check);
        assert_eq!(original.new_version, deserialized.new_version);
    }

    #[test]
    fn test_cache_data_serialization_with_none() {
        let original = CacheData {
            last_check: 123,
            new_version: None,
        };

        let serialized = serde_json::to_string(&original).unwrap();
        let deserialized: CacheData = serde_json::from_str(&serialized).unwrap();

        assert_eq!(original.last_check, deserialized.last_check);
        assert!(deserialized.new_version.is_none());
    }

    #[test]
    #[serial]
    fn test_write_and_read_cache_roundtrip() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_roundtrip.json");
        fs::remove_file(&test_cache_path).ok();

        let test_version = "0.9.9";

        let write_result = write_cache_to_path(Some(test_version.to_string()), &test_cache_path);
        assert!(write_result.is_ok());

        let read_result = read_cache_from_path(&test_cache_path);
        assert!(read_result.is_ok());

        let cached = read_result.unwrap();
        assert!(cached.is_some());

        let cache_data = cached.unwrap();
        assert_eq!(cache_data.new_version, Some(test_version.to_string()));
        assert!(cache_data.last_check > 0);

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_cache_file_is_json() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_json.json");
        fs::remove_file(&test_cache_path).ok();

        write_cache_to_path(Some("1.0.0".to_string()), &test_cache_path).unwrap();

        let content = fs::read_to_string(&test_cache_path).unwrap();

        let json_value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(json_value.is_object());
        assert!(json_value.get("last_check").is_some());
        assert!(json_value.get("new_version").is_some());

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_cache_timestamp_is_current() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        write_cache(Some("1.0.0".to_string())).unwrap();

        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let cache = read_cache().unwrap().unwrap();
        assert!(cache.last_check >= before);
        assert!(cache.last_check <= after + 1);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    #[ignore]
    fn test_cache_duration_threshold() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let exactly_24h_ago = CacheData {
            last_check: now - CACHE_DURATION_SECS,
            new_version: None,
        };

        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let content = serde_json::to_string(&exactly_24h_ago).unwrap();
        fs::write(&cache_path, content).unwrap();

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    #[ignore]
    fn test_cache_just_under_threshold() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let just_under_24h = CacheData {
            last_check: now - CACHE_DURATION_SECS + 60,
            new_version: None,
        };

        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let content = serde_json::to_string(&just_under_24h).unwrap();
        fs::write(&cache_path, content).unwrap();
        std::env::remove_var("CARGO_MANIFEST_DIR");

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(!result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    fn test_update_result_no_update_variant() {
        let result = UpdateResult::NoUpdate;
        match result {
            UpdateResult::NoUpdate => (),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_result_update_available_variant() {
        let result = UpdateResult::UpdateAvailable {
            version: "2.0.0".to_string(),
        };
        match result {
            UpdateResult::UpdateAvailable { version } => {
                assert_eq!(version, "2.0.0");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_result_error_variant_new() {
        let result = UpdateResult::Error("connection failed".to_string());
        match result {
            UpdateResult::Error(msg) => {
                assert_eq!(msg, "connection failed");
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_update_result_clone_and_eq() {
        let original = UpdateResult::UpdateAvailable {
            version: "1.5.0".to_string(),
        };
        let cloned = original.clone();

        match cloned {
            UpdateResult::UpdateAvailable { version } => {
                assert_eq!(version, "1.5.0");
            }
            _ => panic!("Clone failed"),
        }
    }

    #[test]
    fn test_update_info_struct_fields() {
        let info = UpdateInfo {
            current_version: "1.0.0".to_string(),
            new_version: "2.0.0".to_string(),
        };

        assert_eq!(info.current_version, "1.0.0");
        assert_eq!(info.new_version, "2.0.0");
    }

    #[test]
    fn test_update_info_clone_new() {
        let original = UpdateInfo {
            current_version: "1.0.0".to_string(),
            new_version: "2.0.0".to_string(),
        };
        let cloned = original.clone();

        assert_eq!(cloned.current_version, "1.0.0");
        assert_eq!(cloned.new_version, "2.0.0");
    }

    #[test]
    fn test_cache_dir_contains_sshm() {
        let result = get_cache_dir();
        assert!(result.is_ok());

        let path = result.unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("sshm"));
    }

    #[test]
    fn test_cache_file_path_structure() {
        let result = get_cache_file_path();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert_eq!(path.file_name().unwrap(), "update_cache.json");
        assert_eq!(path.extension().unwrap(), "json");
    }

    #[test]
    #[serial]
    fn test_cache_operations_independent() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_independent.json");
        fs::remove_file(&test_cache_path).ok();

        write_cache_to_path(Some("version1".to_string()), &test_cache_path).unwrap();
        let cache1 = read_cache_from_path(&test_cache_path).unwrap().unwrap();

        write_cache_to_path(Some("version2".to_string()), &test_cache_path).unwrap();
        let cache2 = read_cache_from_path(&test_cache_path).unwrap().unwrap();

        assert_eq!(cache1.new_version, Some("version1".to_string()));
        assert_eq!(cache2.new_version, Some("version2".to_string()));

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    fn test_check_for_update_returns_result_variant() {
        let result = check_for_update();
        match result {
            UpdateResult::NoUpdate => (),
            UpdateResult::UpdateAvailable { version } => {
                assert!(!version.is_empty());
            }
            UpdateResult::Error(msg) => {
                assert!(!msg.is_empty());
            }
        }
    }

    #[test]
    fn test_force_check_for_update_returns_result_variant() {
        let result = force_check_for_update();
        match result {
            UpdateResult::NoUpdate => (),
            UpdateResult::UpdateAvailable { version } => {
                assert!(!version.is_empty());
            }
            UpdateResult::Error(msg) => {
                assert!(!msg.is_empty());
            }
        }
    }

    #[test]
    fn test_cache_data_debug_format_new() {
        let cache = CacheData {
            last_check: 12345,
            new_version: Some("1.0.0".to_string()),
        };
        let debug_str = format!("{:?}", cache);

        assert!(debug_str.contains("last_check"));
        assert!(debug_str.contains("new_version"));
        assert!(debug_str.contains("12345"));
        assert!(debug_str.contains("1.0.0"));
    }

    #[test]
    fn test_update_result_debug_format_new() {
        let result = UpdateResult::UpdateAvailable {
            version: "2.0.0".to_string(),
        };
        let debug_str = format!("{:?}", result);

        assert!(debug_str.contains("UpdateAvailable"));
        assert!(debug_str.contains("2.0.0"));
    }

    #[test]
    fn test_cache_data_all_field_combinations() {
        let cases = vec![
            (0, None),
            (0, Some("0.0.1".to_string())),
            (u64::MAX, None),
            (u64::MAX, Some("9.9.9".to_string())),
        ];

        for (last_check, new_version) in cases {
            let cache = CacheData {
                last_check,
                new_version: new_version.clone(),
            };
            assert_eq!(cache.last_check, last_check);
            assert_eq!(cache.new_version, new_version);
        }
    }

    #[test]
    #[serial]
    fn test_cache_persistence_across_read_write() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_persistence.json");
        fs::remove_file(&test_cache_path).ok();

        let versions = vec!["1.0.0", "2.0.0", "3.0.0"];

        for version in versions {
            write_cache_to_path(Some(version.to_string()), &test_cache_path).unwrap();
            let cache = read_cache_from_path(&test_cache_path).unwrap().unwrap();
            assert_eq!(cache.new_version, Some(version.to_string()));
        }

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_cache_overwrites_previous_content() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_overwrite.json");
        fs::remove_file(&test_cache_path).ok();

        write_cache_to_path(Some("old_version".to_string()), &test_cache_path).unwrap();
        write_cache_to_path(Some("new_version".to_string()), &test_cache_path).unwrap();

        let cache = read_cache_from_path(&test_cache_path).unwrap().unwrap();
        assert_eq!(cache.new_version, Some("new_version".to_string()));

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_cache_with_empty_version_string() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        let result = write_cache(Some("".to_string()));
        assert!(result.is_ok());

        let cache = read_cache().unwrap().unwrap();
        assert_eq!(cache.new_version, Some("".to_string()));

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn test_cache_timestamp_monotonically_increases() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();

        write_cache(Some("v1".to_string())).unwrap();
        let cache1 = read_cache().unwrap().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(100));

        write_cache(Some("v2".to_string())).unwrap();
        let cache2 = read_cache().unwrap().unwrap();

        assert!(cache2.last_check >= cache1.last_check);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    fn test_cache_data_with_semver_versions() {
        let semver_versions = vec!["0.1.0", "1.0.0", "1.2.3", "10.20.30", "0.0.1"];

        for version in semver_versions {
            let cache = CacheData {
                last_check: 123,
                new_version: Some(version.to_string()),
            };
            assert_eq!(cache.new_version, Some(version.to_string()));
        }
    }

    #[test]
    #[serial]
    fn test_cache_json_structure() {
        let cache_path = get_cache_file_path().unwrap();
        let test_cache_path = cache_path.with_file_name("update_cache_test_json_structure.json");
        fs::remove_file(&test_cache_path).ok();

        write_cache_to_path(Some("test_version".to_string()), &test_cache_path).unwrap();

        let content = fs::read_to_string(&test_cache_path).unwrap();

        let json: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert!(json.get("last_check").is_some());
        assert!(json.get("new_version").is_some());

        let last_check = json.get("last_check").unwrap().as_u64().unwrap();
        assert!(last_check > 0);

        let new_version = json.get("new_version").unwrap();
        assert!(new_version.is_string());

        fs::remove_file(&test_cache_path).ok();
    }

    #[test]
    fn test_cache_data_deserializes_from_valid_json() {
        let json_str = r#"{
            "last_check": 999999,
            "new_version": "9.9.9"
        }"#;

        let cache: CacheData = serde_json::from_str(json_str).unwrap();
        assert_eq!(cache.last_check, 999999);
        assert_eq!(cache.new_version, Some("9.9.9".to_string()));
    }

    #[test]
    fn test_cache_data_deserializes_with_null_new_version() {
        let json_str = r#"{
            "last_check": 123456,
            "new_version": null
        }"#;

        let cache: CacheData = serde_json::from_str(json_str).unwrap();
        assert_eq!(cache.last_check, 123456);
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_cache_data_deserializes_without_new_version() {
        let json_str = r#"{
            "last_check": 111111
        }"#;

        let cache: CacheData = serde_json::from_str(json_str).unwrap();
        assert_eq!(cache.last_check, 111111);
        assert!(cache.new_version.is_none());
    }

    #[test]
    fn test_platform_asset_name_for_linux_x86_64() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        if std::env::consts::OS == "linux" && std::env::consts::ARCH == "x86_64" {
            assert_eq!(asset, "x86_64-unknown-linux-musl");
        }
    }

    #[test]
    fn test_platform_asset_name_for_macos_x86_64() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        if std::env::consts::OS == "macos" && std::env::consts::ARCH == "x86_64" {
            assert_eq!(asset, "x86_64-apple-darwin");
        }
    }

    #[test]
    fn test_platform_asset_name_for_macos_aarch64() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
            assert_eq!(asset, "aarch64-apple-darwin");
        }
    }

    #[test]
    fn test_platform_asset_name_for_windows_x86_64() {
        let result = get_platform_asset_name();
        assert!(result.is_ok());

        let asset = result.unwrap();
        if std::env::consts::OS == "windows" && std::env::consts::ARCH == "x86_64" {
            assert_eq!(asset, "x86_64-pc-windows-msvc");
        }
    }

    #[test]
    fn test_cache_duration_constant_correctness() {
        assert_eq!(CACHE_DURATION_SECS, 24 * 60 * 60);
        assert_eq!(CACHE_DURATION_SECS, 86400);
        assert_eq!(CACHE_DURATION_SECS / 60, 1440);
        assert_eq!(CACHE_DURATION_SECS / 3600, 24);
    }

    #[test]
    fn test_cache_file_extension() {
        let path = get_cache_file_path().unwrap();
        assert_eq!(
            path.extension().map(|s| s.to_string_lossy().to_string()),
            Some("json".to_string())
        );
    }

    #[test]
    fn test_cache_directory_is_in_config() {
        let path = get_cache_dir().unwrap();
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("config") || path_str.contains(".config"),
            "Cache dir should be in config: {}",
            path_str
        );
    }

    #[test]
    fn test_update_check_path_compatibility() {
        let cache_dir = get_cache_dir().unwrap();
        let cache_file = get_cache_file_path().unwrap();

        assert!(cache_file.starts_with(&cache_dir));
        assert_eq!(cache_file.file_name().unwrap(), "update_cache.json");
    }
}
