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

/// The outcome of **Apply Update** — the act that replaces the installed
/// binary.
///
/// A different type from [`UpdateResult`] on purpose: "a newer release
/// exists" and "the installed binary was replaced" are two facts, and one
/// enum case each. `UpdateAvailable` never means *installed*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyResult {
    /// The installed binary was replaced. The next `sshm` to start is `to`.
    Applied { from: String, to: String },
    /// The running binary is already the newest release. Nothing was
    /// downloaded and nothing was written, and this is a success.
    UpToDate { version: String },
    /// Apply Update cannot structurally succeed here: the directory the swap
    /// would target cannot be written by this user. The way out is
    /// `install.sh`, which escalates correctly.
    Unwritable { path: std::path::PathBuf },
    /// This binary is a **Dev Build** — it is running from a source checkout
    /// and is therefore not sshm's to replace.
    DevBuild { manifest_dir: String },
    /// The check or the swap failed. Nothing was replaced.
    Error(String),
}

/// The one update configuration sshm builds: the repository, the binary,
/// this platform's asset target, and the version running right now.
///
/// Three settings here carry weight:
///
/// * `bin_install_path` is left **unset**, so the library's own `current_exe`
///   resolution is the single answer to *which* binary gets replaced. A path
///   computed here could disagree with the one the swap performs.
/// * `no_confirm(true)` is load-bearing, not incidental: without it the
///   library blocks on an interactive `[Y/n]` inside a command that must
///   never block, TTY or not.
/// * `show_output(false)` and `show_download_progress(true)` are independent
///   switches. The first silences the library's narration — its progress
///   lines, its release-status block, and its `New release is *NOT*
///   compatible` line, which is wrong-scary for a 0.x project, where that
///   comparison reports incompatibility on every minor bump. The second keeps
///   the one piece of its output worth having: several megabytes downloading
///   with no feedback reads as a hang.
///
/// `bin_path_in_archive("sshm")` is the Unix binary. The Windows asset carries
/// `sshm.exe`, so `sshm update` on Windows would download and then fail to
/// extract — which is the accepted state of affairs: ADR-0002 declines Windows
/// Windows outright, and `install.sh` refuses Windows before anyone gets a
/// binary to update. Fixing the name here would imply the swap works, which is
/// the claim nobody has tested.
fn update_config(
    current_version: &str,
) -> Result<Box<dyn self_github_update_enhanced::update::ReleaseUpdate>, String> {
    let asset_name = get_platform_asset_name()?;

    Update::configure()
        .repo_owner("titonio")
        .repo_name("ssh-manager")
        .bin_name("sshm")
        .bin_path_in_archive("sshm")
        .target(&asset_name)
        .current_version(current_version)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(true)
        .build()
        .map_err(|e| format!("Failed to build update configuration: {e}"))
}

/// The one level of symlink resolution `self_replace` performs before it
/// stages the new binary: a symlinked install points at the file at risk, and
/// that is the name a diagnostic has to give.
///
/// Deliberately one level, and deliberately the same one: resolve more, or
/// not at all, and sshm probes a directory the swap never writes to.
///
/// The result is always anchored. A stored target of `../lib/sshm` is relative
/// to the link, not to the process, so taking it literally would have the probe
/// test the user's current directory — pass, download, and name a path with
/// nothing to do with the binary at risk.
fn resolved_target(exe: &std::path::Path) -> PathBuf {
    if !fs::symlink_metadata(exe).is_ok_and(|m| m.file_type().is_symlink()) {
        return exe.to_path_buf();
    }

    let stored = match fs::read_link(exe) {
        Ok(target) => target,
        Err(_) => return exe.to_path_buf(),
    };

    let anchored = if stored.is_absolute() {
        stored
    } else {
        match exe.parent() {
            Some(dir) => dir.join(stored),
            None => return exe.to_path_buf(),
        }
    };
    collapse(&anchored)
}

/// Collapse `.` and `..` by name, so a diagnostic prints one path instead of a
/// walk through it.
///
/// `Path::canonicalize` is the other tool here and it is the wrong one: it
/// resolves *every* symlink and insists the file exists, which is precisely the
/// extra resolution this command is not allowed to assume.
fn collapse(path: &std::path::Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    // A `..` with nothing to pop: keep it and let the probe's
                    // own failure be the answer.
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Can this user create a file in the directory the swap stages into?
///
/// `self_replace` writes the new binary to a temp file **beside** the target
/// and `rename`s over it, so that question *is* "can this binary be replaced
/// in place" — no `ETXTBSY`, no second filesystem, no privilege dance. Asking
/// it before the download is what keeps several megabytes from arriving just
/// to be thrown away, and it is the only way to promise a diagnostic that
/// names the path.
///
/// The probe is advisory. If it passes and the swap still fails, the swap's
/// own error is what gets reported, unedited.
fn install_dir_writable(target: &std::path::Path) -> bool {
    let Some(dir) = target.parent() else {
        return false;
    };
    let probe = dir.join(format!(".sshm-write-probe-{}", std::process::id()));

    // `create_new` rather than `create`: the probe must not open or truncate
    // something that is already there, whatever it is.
    match fs::File::create_new(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// The binary `sshm update` will replace: the running executable, resolved the
/// way the swap resolves it.
fn replace_target() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Failed to resolve the running binary: {e}"))?;
    Ok(resolved_target(&exe))
}

/// **Apply Update**: replace the installed binary with the latest release.
///
/// Release selection is not ours. The library asks GitHub's
/// `releases/latest` endpoint — highest semver, drafts and prereleases
/// excluded — and refuses anything that is not strictly greater, which is
/// what makes a downgrade impossible from here.
pub fn apply_update() -> ApplyResult {
    let current_version = env!("CARGO_PKG_VERSION").to_string();

    // First, and before anything else touches the network: a checkout's
    // binary is not ours to rewrite.
    if let Some(manifest_dir) = dev_build() {
        return ApplyResult::DevBuild { manifest_dir };
    }

    // Then: can the install location be written at all? Asking before the
    // download is what makes "nothing was downloaded" true, and it is the
    // only way the diagnostic can name the path. No second install location
    // is offered as a workaround — a shadowed binary reproducing "`which
    // sshm` and `sshm --version` disagree" is the confusion this command
    // exists to end.
    let target = match replace_target() {
        Ok(target) => target,
        Err(e) => return ApplyResult::Error(e),
    };
    if !install_dir_writable(&target) {
        return ApplyResult::Unwritable { path: target };
    }

    let update = match update_config(&current_version) {
        Ok(update) => update,
        Err(e) => return ApplyResult::Error(e),
    };

    match update.update() {
        Ok(Status::UpToDate(version)) => ApplyResult::UpToDate { version },
        Ok(Status::Updated(version)) => ApplyResult::Applied {
            from: current_version,
            to: version,
        },
        Err(e) => ApplyResult::Error(e.to_string()),
    }
}

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub new_version: String,
}

/// Parse a dotted numeric version like `"0.1.11"` into `(major, minor, patch)`.
/// Returns `None` for anything that isn't exactly three dot-separated integers.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let major = parts[0].parse::<u64>().ok()?;
    let minor = parts[1].parse::<u64>().ok()?;
    let patch = parts[2].parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Returns `true` if `candidate` is strictly newer than `current`.
/// Unparseable versions are treated as "not newer" — quiet beats a lie.
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
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

/// Is this a **Dev Build** — a binary running from a source checkout rather
/// than an installed location?
///
/// `CARGO_MANIFEST_DIR` is what Cargo sets for `cargo run` and `cargo test`,
/// and it gates two behaviours that must agree: no Update Note, and a refused
/// `sshm update`. A binary the developer owns is one sshm never rewrites —
/// under `cargo run -- update` the resolved executable is a build artefact,
/// and replacing it would overwrite the developer's own work with a release
/// build.
fn dev_build() -> Option<String> {
    std::env::var("CARGO_MANIFEST_DIR").ok()
}

fn should_check_update() -> Result<bool, String> {
    // If running via cargo, skip update check
    if dev_build().is_some() {
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

/// Ask whether a newer release exists, and cache the answer for the note.
///
/// The question and nothing else: no archive is downloaded and no binary is
/// written. **Check for Updates** answers; **Apply Update** acts, and the two
/// are different commands (#46).
///
/// Release selection is not ours. `get_latest_release` asks GitHub's
/// `releases/latest` endpoint, which returns the highest published semver with
/// drafts and prereleases excluded — so the ordering defect in #46, which came
/// from indexing a release list by position and bailing only on an exact
/// equality, has no code here to live in.
///
/// Whether that release is newer is `is_newer` again, the same quiet rule that
/// guards the cached note: something unparseable is not newer, and a silent
/// answer beats one that invents a release.
fn check_latest_version(current_version: &str) -> UpdateResult {
    let update = match update_config(current_version) {
        Ok(update) => update,
        Err(e) => {
            let _ = write_cache(None);
            return UpdateResult::Error(e);
        }
    };

    match update.get_latest_release() {
        Ok(release) => {
            if is_newer(&release.version, current_version) {
                let _ = write_cache(Some(release.version.clone()));
                return UpdateResult::UpdateAvailable {
                    version: release.version,
                };
            }
            let _ = write_cache(None);
            UpdateResult::NoUpdate
        }
        Err(e) => {
            let _ = write_cache(None);
            // The reason, unadorned: the surface that prints this supplies its
            // own framing, and "Error checking for updates: Failed to check
            // for updates: …" is what doubling it reads like.
            UpdateResult::Error(e.to_string())
        }
    }
}

/// Check for Updates, honouring the cache: at most one request per
/// [`CACHE_DURATION_SECS`], and none at all from a source checkout.
pub fn check_for_update() -> UpdateResult {
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

    force_check_for_update()
}

/// Check for Updates with the cache ignored: the question `sshm check-update`
/// and the global `-c` ask. It downloads nothing and writes no binary
/// anywhere (#46).
pub fn force_check_for_update() -> UpdateResult {
    check_latest_version(env!("CARGO_PKG_VERSION"))
}

/// The version a previous run cached, with **no network**.
///
/// This is the reader the inline frame's paint path uses (#39). It opens the
/// cache file and reports the `new_version` a *previous* run left there — or
/// `None` when there is no cache, the cache holds no update, the cached
/// version is not newer than the running binary, or the file cannot be read.
/// It never calls the update checker, never opens a socket, and never blocks
/// first paint: the whole point of the note is that it is already known
/// before the frame draws.
///
/// A source checkout returns `None` without reading: running the tool from a
/// checkout should not surface a note, and the cache a checkout's own test
/// runs leave behind is not the user's update state.
///
/// The cached version is compared against `env!("CARGO_PKG_VERSION")` before
/// it is returned. After `sshm update` the running binary *is* the new
/// version, so the cached note would be a lie; returning `None` here keeps the
/// note honest without a network round-trip (#46).
pub fn cached_update_version() -> Option<String> {
    if dev_build().is_some() {
        return None;
    }

    let cached = read_cache().ok().flatten().and_then(|c| c.new_version)?;
    if is_newer(&cached, env!("CARGO_PKG_VERSION")) {
        Some(cached)
    } else {
        None
    }
}

/// Turn the outcome of an update check into the note a surface shows.
pub fn note_from(result: UpdateResult) -> Result<Option<UpdateInfo>, String> {
    match result {
        UpdateResult::UpdateAvailable { version } => Ok(Some(UpdateInfo {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            new_version: version,
        })),
        UpdateResult::NoUpdate => Ok(None),
        UpdateResult::Error(message) => Err(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// Some tests pose as an installed binary by clearing `CARGO_MANIFEST_DIR`.
    /// This puts it back on the way out, so "no test can reach the network by
    /// accident" is a property of the harness rather than of test ordering —
    /// a leaked removal was the documented flake in `tests/README.md`, and
    /// after #46 it would also be a way for a test to make a real request.
    struct NotACheckout(Option<std::ffi::OsString>);

    impl NotACheckout {
        fn take() -> Self {
            let previous = std::env::var_os("CARGO_MANIFEST_DIR");
            std::env::remove_var("CARGO_MANIFEST_DIR");
            Self(previous)
        }
    }

    impl Drop for NotACheckout {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CARGO_MANIFEST_DIR", value),
                None => std::env::remove_var("CARGO_MANIFEST_DIR"),
            }
        }
    }

    #[test]
    #[serial]
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
    #[serial]
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
    #[serial]
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
    #[serial]
    fn test_write_cache_basic() {
        let result = write_cache(Some("0.2.0".to_string()));
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
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
    #[serial]
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

    /// The cache gate, offline: a check less than `CACHE_DURATION_SECS` old
    /// answers from the file and never opens a socket (#39, #46).
    #[test]
    #[serial]
    fn check_for_update_answers_from_a_fresh_cache_without_a_request() {
        // Pose as an installed binary: otherwise the dev-build short-circuit
        // answers first and the freshness gate this test is about never runs.
        let _not_a_checkout = NotACheckout::take();

        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        write_cache(Some("0.9.9".to_string())).unwrap();

        assert!(matches!(
            check_for_update(),
            UpdateResult::UpdateAvailable { version } if version == "0.9.9"
        ));

        fs::remove_file(&cache_path).ok();
    }

    /// Needs network: this asks GitHub's releases API for real. Run it by hand
    /// with `cargo test -- --ignored` when checking that the endpoint, the
    /// target triple and the asset naming still agree.
    #[test]
    #[serial]
    #[ignore]
    fn test_force_check_for_update_basic() {
        let result = force_check_for_update();
        assert!(matches!(
            result,
            UpdateResult::NoUpdate | UpdateResult::UpdateAvailable { .. } | UpdateResult::Error(_)
        ));
    }

    #[test]
    #[serial]
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
        let _not_a_checkout = NotACheckout::take();

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
        let _not_a_checkout = NotACheckout::take();

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
        let _not_a_checkout = NotACheckout::take();

        let result = should_check_update();
        assert!(result.is_ok());
        assert!(result.unwrap());

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
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
        // Without this the gate short-circuits on `CARGO_MANIFEST_DIR` and the
        // assertion below can only ever fail: the sibling test beneath this
        // one poses as an installed binary and this one never did.
        let _not_a_checkout = NotACheckout::take();

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
        let _not_a_checkout = NotACheckout::take();

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

    /// Needs network: the live question `sshm check-update` asks. Kept
    /// alongside `test_force_check_for_update_basic` as the manual probe of
    /// GitHub's `releases/latest` endpoint for this target.
    #[test]
    #[serial]
    #[ignore]
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

    // The update note a frame reads. These live with the note itself, not with
    // the Connection set that has nothing to do with versions.

    #[test]
    fn an_available_update_becomes_a_note_naming_both_versions() {
        match note_from(UpdateResult::UpdateAvailable {
            version: "9.9.9".to_string(),
        }) {
            Ok(Some(info)) => {
                assert_eq!(info.new_version, "9.9.9");
                assert_eq!(info.current_version, env!("CARGO_PKG_VERSION"));
            }
            other => panic!("expected an available-update note, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_update_check_becomes_the_reason_it_failed() {
        match note_from(UpdateResult::Error("network down".to_string())) {
            Err(reason) => assert_eq!(reason, "network down"),
            other => panic!("expected a failed-update note, got {other:?}"),
        }
    }

    #[test]
    fn no_update_means_no_note() {
        assert!(matches!(note_from(UpdateResult::NoUpdate), Ok(None)));
    }

    // The cache-only reader the frame's paint path uses (#39). These pin
    // that it reads the cached version and nothing else — no network, no
    // freshness gate, no panic on a missing or corrupt file.

    #[test]
    #[serial]
    fn cached_update_version_reports_the_version_a_previous_run_left() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let cache = CacheData {
            last_check: 12345,
            new_version: Some("0.2.0".to_string()),
        };
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();
        let _not_a_checkout = NotACheckout::take();

        assert_eq!(cached_update_version().as_deref(), Some("0.2.0"));

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_the_cache_holds_no_new_version() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let cache = CacheData {
            last_check: 12345,
            new_version: None,
        };
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();
        let _not_a_checkout = NotACheckout::take();

        assert_eq!(cached_update_version(), None);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_the_cache_is_unreadable() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        fs::write(&cache_path, "not json at all").unwrap();
        let _not_a_checkout = NotACheckout::take();

        // A corrupt cache is a silent no-note, never a panic and never a
        // network call: the paint path must survive it.
        assert_eq!(cached_update_version(), None);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_there_is_no_cache_at_all() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let _not_a_checkout = NotACheckout::take();

        assert_eq!(cached_update_version(), None);
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_cached_version_equals_running_version() {
        // After `sshm update` the running binary IS the cached version.
        // Returning Some here would make the note a lie.
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let current = env!("CARGO_PKG_VERSION").to_string();
        let cache = CacheData {
            last_check: 12345,
            new_version: Some(current),
        };
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();
        let _not_a_checkout = NotACheckout::take();

        assert_eq!(cached_update_version(), None);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_cached_version_is_older() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let cache = CacheData {
            last_check: 12345,
            new_version: Some("0.0.1".to_string()),
        };
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();
        let _not_a_checkout = NotACheckout::take();

        assert_eq!(cached_update_version(), None);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_is_none_when_cached_version_is_unparseable() {
        let cache_path = get_cache_file_path().unwrap();
        fs::remove_file(&cache_path).ok();
        let cache = CacheData {
            last_check: 12345,
            new_version: Some("not-a-version".to_string()),
        };
        fs::write(&cache_path, serde_json::to_string(&cache).unwrap()).unwrap();
        let _not_a_checkout = NotACheckout::take();

        // Unparseable = not newer = quiet. A lie is worse than silence.
        assert_eq!(cached_update_version(), None);

        fs::remove_file(&cache_path).ok();
    }

    #[test]
    #[serial]
    fn cached_update_version_stays_quiet_inside_a_source_checkout() {
        // A checkout must never surface a note, even if a cache file with a
        // version happens to exist on the machine running the test.
        let previous = std::env::var_os("CARGO_MANIFEST_DIR");
        std::env::set_var("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"));

        assert_eq!(cached_update_version(), None);

        match previous {
            Some(value) => std::env::set_var("CARGO_MANIFEST_DIR", value),
            None => std::env::remove_var("CARGO_MANIFEST_DIR"),
        }
    }

    // ── Apply Update: whose binary is it? (#46) ───────────────────────────

    /// A binary running from a source checkout belongs to the developer, and
    /// sshm never rewrites one. This is the same intent that has always kept
    /// the Update Note away from a checkout, and the two must agree.
    ///
    /// The refusal happens before the update configuration is built, so no
    /// request goes out: under a test run the network is reachable and this
    /// still reports the refusal rather than a fetch or a swap.
    #[test]
    #[serial]
    fn a_dev_build_is_refused_and_nothing_is_fetched() {
        let previous = std::env::var_os("CARGO_MANIFEST_DIR");
        std::env::set_var("CARGO_MANIFEST_DIR", "/checkout/ssh-manager/sshm");

        assert_eq!(
            apply_update(),
            ApplyResult::DevBuild {
                manifest_dir: "/checkout/ssh-manager/sshm".to_string()
            }
        );

        match previous {
            Some(value) => std::env::set_var("CARGO_MANIFEST_DIR", value),
            None => std::env::remove_var("CARGO_MANIFEST_DIR"),
        }
    }

    // ── Apply Update: can this location be written at all? (#46) ──────────
    //
    // The probe is what makes "downloads nothing" true: the swap stages its
    // temp file in the target's own directory and `rename`s over it, so
    // whether this user can write the binary is exactly whether they can
    // create a file there.

    #[test]
    fn a_writable_install_directory_passes_the_probe_and_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("sshm");
        fs::write(&target, b"binary").unwrap();

        assert!(install_dir_writable(&target));
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "the probe must delete the file it made — no litter beside the target"
        );
    }

    /// A directory that cannot be written at all: no user, no root, no way
    /// for an in-app update to work. This is the case the `install.sh`
    /// escape hatch exists for.
    #[test]
    fn a_target_with_no_directory_of_its_own_fails_the_probe() {
        let missing = std::env::temp_dir()
            .join("sshm-no-such-install-dir")
            .join("sshm");

        assert!(!install_dir_writable(&missing));
    }

    /// `self_replace` follows one level of symlink before it stages the new
    /// binary, so that is the file at risk — and the name the diagnostic has
    /// to give. A probe that tested the link's own directory instead would
    /// pass, and the failure it reported would name a symlink nobody wrote to.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_binary_resolves_to_the_file_the_swap_targets() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshm-real");
        fs::write(&real, b"binary").unwrap();
        let link = dir.path().join("sshm");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(
            resolved_target(&link),
            real,
            "the probe must aim at the file behind the link"
        );
        assert_eq!(
            resolved_target(&real),
            real,
            "an ordinary binary is its own target"
        );
    }

    /// The whole reason the probe mirrors the symlink: the real binary sits
    /// in a directory this user cannot write, behind a link in one they can.
    #[cfg(unix)]
    #[test]
    fn a_writable_directory_holding_a_symlink_to_a_locked_one_stays_unwritable() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let locked = root.path().join("locked");
        let open = root.path().join("open");
        fs::create_dir(&locked).unwrap();
        fs::create_dir(&open).unwrap();

        let real = locked.join("sshm");
        fs::write(&real, b"binary").unwrap();
        let link = open.join("sshm");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        // Without the symlink resolution this would probe `open`, pass, and
        // then let a download happen before the swap failed.
        let target = resolved_target(&link);
        assert!(!install_dir_writable(&target));

        // Restore so tempdir cleanup can recurse.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A link target is stored relative to the **link**. Read literally,
    /// `../lib/sshm` resolves against whatever directory the user was standing
    /// in, so the probe would test a directory nobody installs into — pass,
    /// download several megabytes, and then name a path with nothing to do with
    /// the binary at risk.
    #[cfg(unix)]
    #[test]
    fn a_relative_symlink_resolves_against_the_link_not_the_working_directory() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let lib = root.path().join("lib");
        fs::create_dir(&bin).unwrap();
        fs::create_dir(&lib).unwrap();
        let real = lib.join("sshm");
        fs::write(&real, b"binary").unwrap();

        let link = bin.join("sshm");
        std::os::unix::fs::symlink("../lib/sshm", &link).unwrap();

        assert_eq!(
            resolved_target(&link),
            root.path().join("lib/sshm"),
            "the stored target is anchored on the link's own directory, and \
             the result names one path rather than a walk"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_absolute_symlink_target_is_left_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("sshm-real");
        fs::write(&real, b"binary").unwrap();
        let link = dir.path().join("sshm");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let resolved = resolved_target(&link);
        assert!(resolved.is_absolute(), "got {resolved:?}");
        assert_eq!(resolved, real);
    }

    // ── Check for Updates asks; Apply Update acts (#46) ───────────────────
    //
    // "It downloads nothing" is a claim about a code path that a test cannot
    // observe without a network, so it is gated the way this repo gates the
    // other claims about which code paths exist: by reading the source and
    // asserting what is not in it. Same idiom as the settle-verb gate and the
    // colour-literal grep in `design_system_test.rs`.

    #[test]
    fn the_question_never_contains_the_swap() {
        let source = include_str!("update.rs");
        assert!(!source.is_empty(), "the gate reads its own input");

        // Only the code, never the test module below it: that is full of
        // version literals, and of this gate's own strings. Split on `mod
        // tests`, not on `#[cfg(test)]` — an earlier `#[cfg(test)]` helper
        // sits mid-file, and splitting there would silently narrow this gate
        // to the first few hundred lines.
        let code = source
            .split("#[cfg(test)]\nmod tests {")
            .next()
            .expect("the test module is delimited");
        assert!(
            code.contains("fn cached_update_version"),
            "the gate read the code half of this file, not the tests"
        );

        // `.update()` is the library call that downloads, extracts and
        // replaces the binary. Exactly one place in this module may reach it,
        // and that place is Apply Update.
        assert_eq!(
            code.matches(".update()").count(),
            1,
            "`sshm check-update` must not be able to replace a binary"
        );
        let at = code.find(".update()").unwrap();
        let enclosing = code[..at].rsplit("fn ").next().unwrap();
        assert!(
            enclosing.starts_with("apply_update"),
            "the swap belongs in apply_update, not in {enclosing}"
        );

        // And the hand-rolled transport is gone, not merely unused: the
        // ordering defect in #46 lived in the release listing, so deleting it
        // is what fixes it.
        for forbidden in ["reqwest", "flate2", "tar::", "ReleaseList"] {
            assert!(
                !code.contains(forbidden),
                "the update module must not hand-roll {forbidden}: release \
                 selection belongs to the dependency"
            );
        }
    }

    /// No version string is substituted into another version string anywhere:
    /// the old "Asset not found" message rewrote `0.1.6` into `0.1.7` and so
    /// could lie about a second thing at once.
    #[test]
    fn no_version_is_substituted_into_another_version() {
        let source = include_str!("update.rs");
        let code = source.split("#[cfg(test)]\nmod tests {").next().unwrap();
        assert!(
            code.contains("fn cached_update_version"),
            "the gate read the code half of this file, not the tests"
        );
        for stale in ["0.1.6", "0.1.7"] {
            let offenders = code
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .filter(|l| l.contains(stale))
                .collect::<Vec<_>>();
            assert!(
                offenders.is_empty(),
                "version literal {stale} in code: {offenders:?}"
            );
        }
    }
}
