//! The world a child is born into: a throwaway HOME, a seeded store, and the
//! shims that make side effects observable.
//!
//! Every test gets its own directory tree and its own environment, which is
//! what lets the suite run in parallel without a lock and without `serial_test`
//! — and what makes "green on a loaded machine with the network unplugged" mean
//! something. Nothing here reads the developer's real `HOME`, and nothing here
//! reaches past the loopback interface.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The built `sshm` binary under test.
///
/// The suite tests a *binary*, not a library, so the binary has to be built
/// first and named: `run-e2e.sh` builds it and exports this variable, and CI
/// runs the same script, which is the only reason laptop and CI cannot drift.
/// There is deliberately no fallback to a binary found on `PATH` — a suite that
/// silently tests whatever `sshm` happens to be installed is a suite that
/// passes against code it did not build.
pub fn sshm_binary() -> PathBuf {
    match std::env::var_os("SSHM_E2E_BIN") {
        Some(path) => {
            let path = PathBuf::from(path);
            assert!(
                path.exists(),
                "SSHM_E2E_BIN points at {} which does not exist; run the suite through sshm-e2e/run-e2e.sh",
                path.display()
            );
            path
        }
        None => panic!("SSHM_E2E_BIN is not set; run the suite through sshm-e2e/run-e2e.sh"),
    }
}

/// A throwaway HOME with an `.ssh` directory in it, plus the directories a
/// well-behaved program expects to own.
///
/// The `Drop` of `tempfile::TempDir` removes the tree, so a test that fails
/// halfway leaves nothing for the next one to trip over.
pub struct Fixture {
    home: tempfile::TempDir,
    extra_env: Vec<(String, String)>,
    /// Set by [`Fixture::with_ssh_shim`]: the file the `ssh` stub appends to.
    ssh_calls: Option<PathBuf>,
    /// The directory holding the `ssh` stub, prepended to the child's `PATH`.
    shim_dir: Option<PathBuf>,
}

impl Fixture {
    /// An empty HOME whose `.ssh` directory exists but holds no store file —
    /// the state a first-time user is in.
    pub fn new() -> Fixture {
        let home = tempfile::tempdir().expect("a temp HOME");
        fs::create_dir_all(home.path().join(".ssh")).expect("the .ssh directory");
        fs::create_dir_all(home.path().join(".config")).expect("the .config directory");
        Fixture {
            home,
            extra_env: Vec::new(),
            ssh_calls: None,
            shim_dir: None,
        }
    }

    pub fn home_dir(&self) -> &Path {
        self.home.path()
    }

    pub fn ssh_dir(&self) -> PathBuf {
        self.home_dir().join(".ssh")
    }

    /// The store file the app reads and writes: `$HOME/.ssh/connections.json`.
    pub fn store_path(&self) -> PathBuf {
        self.ssh_dir().join("connections.json")
    }

    /// Write the store file directly.
    ///
    /// Seeding through the file rather than through the UI is deliberate: a
    /// test about navigation should not have its setup depend on the
    /// navigation it is testing. The flows that *are* the flow — add, edit,
    /// delete, import — are driven through the keyboard instead.
    pub fn with_store(self, json: &str) -> io::Result<Self> {
        fs::write(self.store_path(), json)?;
        Ok(self)
    }

    /// Seed the store with `connections`, given as JSON objects without ids.
    ///
    /// Ids are filled in here so a test writes only the fields it cares about;
    /// the shape of the file is the app's, not the test's invention.
    pub fn with_connections(self, connections: &[serde_json::Value]) -> io::Result<Self> {
        let filled: Vec<serde_json::Value> = connections
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                let mut obj = c.as_object().expect("a connection is an object").clone();
                obj.entry("id")
                    .or_insert_with(|| serde_json::json!(format!("id-e2e-{idx:02}")));
                serde_json::Value::Object(obj)
            })
            .collect();
        self.with_store(
            &serde_json::to_string_pretty(&serde_json::json!({
                "connections": filled
            }))
            .expect("the fixture serialises"),
        )
    }

    /// The store as the app wrote it, for the assertions that must prove a
    /// keystroke reached the disk and not merely the screen.
    pub fn read_store(&self) -> io::Result<serde_json::Value> {
        Ok(
            serde_json::from_str(&fs::read_to_string(self.store_path())?)
                .expect("the store file is JSON"),
        )
    }

    /// The aliases currently in the store, in file order.
    pub fn stored_aliases(&self) -> io::Result<Vec<String>> {
        let store = self.read_store()?;
        Ok(store["connections"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row["alias"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Whether the store file exists at all — the difference between "wrote an
    /// empty list" and "wrote nothing", which is the difference a decline
    /// makes.
    pub fn store_exists(&self) -> bool {
        self.store_path().exists()
    }

    /// Seed `~/.ssh/config` for the first-run import offer.
    pub fn with_ssh_config(self, text: &str) -> io::Result<Self> {
        fs::write(self.ssh_dir().join("config"), text)?;
        Ok(self)
    }

    /// Put a stub `ssh` earlier on the child's `PATH`.
    ///
    /// The real `Command::new("ssh")` exec path is left completely alone; the
    /// thing it finds is ours. Each call appends one line of argv to the file
    /// [`Fixture::ssh_calls_path`] names, fields separated by the ASCII unit
    /// separator, so a test waits on a file rather than on a process and no
    /// quoting rule can lie about an argument containing a space.
    pub fn with_ssh_shim(mut self) -> io::Result<Self> {
        let bin = self.home_dir().join("shim-bin");
        fs::create_dir_all(&bin)?;
        let log = self.home_dir().join("ssh-calls.log");
        // One record per call: the argv fields joined by the ASCII unit
        // separator (0x1f), the record terminated by NUL. Both are chosen
        // because they cannot occur in an argument — NUL is impossible by the
        // definition of argv, and 0x1f is not a character a path or a hostname
        // contains — so the recording round-trips exactly and no quoting rule
        // can lie about what was passed. A newline-terminated, space-separated
        // log would merge two arguments whenever one of them contained a space,
        // which is the one case worth catching.
        //
        // The `first` flag is there because the obvious trick of remembering the
        // first argument and printing it at the end prints it *last*, silently
        // reordering the argv it is meant to be recording.
        let script = [
            "#!/bin/sh",
            // One `printf` per call, so one call is one `write` to the log. A
            // loop of `printf`s per argument is the obvious version and it is
            // wrong: the reader then sees an argv in the middle of being
            // appended, which is a test that passes or fails on how the
            // scheduler felt. `"$*"` joins the arguments with the first
            // character of `IFS`, so setting `IFS` to the separator is what
            // makes the whole record one string.
            "IFS=$(printf '\\037')",
            "{",
            "  printf '%s\\000' \"$*\"",
            &format!("}} >> '{}'", log.display()),
            "exit 0",
        ]
        .join("\n")
            + "\n";
        let script_path = bin.join("ssh");
        fs::write(&script_path, script)?;
        let mut perms = fs::metadata(&script_path)?.permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
        fs::set_permissions(&script_path, perms)?;

        self.shim_dir = Some(bin);
        self.ssh_calls = Some(log);
        Ok(self)
    }

    /// The argv each `ssh` call received, one `Vec` per call.
    ///
    /// A test waits for this file to appear (with a deadline) before reading
    /// it; there is no sleep anywhere in that sequence.
    pub fn ssh_calls(&self) -> io::Result<Vec<Vec<String>>> {
        let path = self
            .ssh_calls
            .as_ref()
            .expect("with_ssh_shim must be called first");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let bytes = fs::read(path)?;
        Ok(bytes
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .map(|record| {
                record
                    .split(|byte| *byte == 0x1f)
                    .map(|field| String::from_utf8_lossy(field).into_owned())
                    .collect()
            })
            .collect())
    }

    /// Whether every call the shim has taken is finished being recorded.
    ///
    /// The stub writes a whole call in one `write` ending in the terminator, so
    /// a log whose last byte is the terminator holds nothing half-written. A
    /// test that waited for the file to *exist* would be reading an argv while
    /// the stub is still appending it, and would then be asserting against a
    /// truncated one — a failure with nothing wrong in the program.
    pub fn ssh_calls_are_complete(&self) -> bool {
        let Some(path) = self.ssh_calls.as_deref() else {
            return false;
        };
        match fs::read(path) {
            Ok(bytes) => bytes.last() == Some(&0),
            Err(_) => false,
        }
    }

    /// The stub `ssh` itself. Normally found through `PATH`, which is the
    /// point; named here so the shim can be tested in its own right.
    pub fn ssh_stub_path(&self) -> Option<PathBuf> {
        self.shim_dir.as_ref().map(|dir| dir.join("ssh"))
    }

    /// An extra environment variable for the child.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// The complete environment for a child: cleared, then rebuilt from this
    /// fixture.
    ///
    /// `HOME` is what `dirs::home_dir()` reads, and the store path is derived
    /// from it, so this list *is* the isolation the suite runs on. `PATH` is
    /// inherited (plus any shim) because the app shells out to `ssh`;
    /// everything else the app might pick up from the developer's session is
    /// deliberately absent.
    pub fn env(&self) -> Vec<(String, String)> {
        let home = self.home_dir();
        let inherited_path = std::env::var("PATH").unwrap_or_default();
        let path = match &self.shim_dir {
            Some(bin) => format!("{}:{}", bin.display(), inherited_path),
            None => inherited_path,
        };
        let mut env = vec![
            ("HOME".to_string(), home.display().to_string()),
            (
                "XDG_CONFIG_HOME".to_string(),
                home.join(".config").display().to_string(),
            ),
            (
                "XDG_CACHE_HOME".to_string(),
                home.join(".cache").display().to_string(),
            ),
            ("TMPDIR".to_string(), home.display().to_string()),
            ("PATH".to_string(), path),
            ("USER".to_string(), "e2e".to_string()),
            ("LOGNAME".to_string(), "e2e".to_string()),
            // The update note is read from a cache file under the config dir.
            // Pointing the API root at a port nothing listens on keeps a stray
            // check from ever reaching out, and costs nothing: no frame path
            // touches the network anyway.
            (
                "SSHM_GITHUB_API_URL".to_string(),
                "http://127.0.0.1:1".to_string(),
            ),
        ];
        env.extend(self.extra_env.iter().cloned());
        env
    }
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

/// A connection written the way the spec writes one: the fields a test cares
/// about, with the rest at the store's own defaults.
pub fn connection(alias: &str, host: &str, user: &str) -> serde_json::Value {
    serde_json::json!({
        "alias": alias,
        "host": host,
        "user": user,
        "port": 22,
    })
}

/// The same, with a port and a key path — the shape that proves every field
/// reaches `ssh`.
pub fn connection_full(
    alias: &str,
    host: &str,
    user: &str,
    port: u16,
    key_path: &str,
) -> serde_json::Value {
    serde_json::json!({
        "alias": alias,
        "host": host,
        "user": user,
        "port": port,
        "key_path": key_path,
    })
}

/// A ready-made store with a handful of Connections in it — the fixture most
/// picker tests start from.
pub fn seeded_fixture(aliases: &[&str]) -> Fixture {
    let connections: Vec<serde_json::Value> = aliases
        .iter()
        .enumerate()
        .map(|(idx, alias)| connection(alias, &format!("10.0.0.{}", idx + 1), "deploy"))
        .collect();
    Fixture::new().with_connections(&connections).expect("seed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stub `ssh` records what it was handed, in order and unaltered.
    ///
    /// The emit tests assert on this recording, so a shim that reordered or
    /// mangled argv would make them lie. An argument with a space and one with
    /// a newline are in there because those are the two ways a naive
    /// whitespace-separated log would silently merge two arguments into one.
    #[test]
    fn the_ssh_stub_records_argv_exactly_as_it_was_handed_it() {
        let fixture = Fixture::new().with_ssh_shim().expect("the shim");
        let path = fixture.ssh_stub_path().expect("the shim is there");

        let run = |args: &[&str]| {
            std::process::Command::new(&path)
                .args(args)
                .status()
                .expect("the shim runs")
        };
        run(&["-i", "/keys/id", "-p", "2222", "ci@10.0.0.42"]);
        run(&["with a space", "with\na newline"]);

        assert_eq!(
            fixture.ssh_calls().expect("the log reads"),
            vec![
                vec![
                    "-i".to_string(),
                    "/keys/id".to_string(),
                    "-p".to_string(),
                    "2222".to_string(),
                    "ci@10.0.0.42".to_string(),
                ],
                vec!["with a space".to_string(), "with\na newline".to_string()],
            ]
        );
    }
}
