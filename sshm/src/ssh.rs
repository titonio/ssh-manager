use crate::config::{Connection, Source};
use std::process;

/// The argv for `ssh`, given the row the user picked.
///
/// The two provenances connect differently and this is the only place that
/// decides it; the check inside says why a config row carries no flags at all.
pub fn build_ssh_args(conn: &Connection) -> Vec<std::ffi::OsString> {
    // A config row is the alias token and nothing else: ssh re-reads the stanza
    // from the same `~/.ssh/config` the row came from and applies it itself —
    // identity file, port, user, and every keyword sshm never parses
    // (`ProxyJump`, `ForwardAgent`, …). Re-sending flags would double-apply them
    // and can contradict a `Match` block the row never showed, so connecting is
    // exactly what the user's own `ssh build-box` would be.
    if conn.source == Source::SshConfig {
        return vec![std::ffi::OsString::from(&conn.alias)];
    }

    let mut args: Vec<std::ffi::OsString> = vec![];

    if let Some(ref key) = conn.key_path {
        args.push(std::ffi::OsStr::new("-i").to_os_string());
        args.push(std::ffi::OsStr::new(key).to_os_string());
    }

    if conn.port != 22 {
        args.push(std::ffi::OsStr::new("-p").to_os_string());
        args.push(std::ffi::OsStr::new(&conn.port.to_string()).to_os_string());
    }

    let user_host = if conn.user.is_empty() {
        conn.host.clone()
    } else {
        format!("{}@{}", conn.user, conn.host)
    };
    args.push(std::ffi::OsStr::new(&user_host).to_os_string());

    args
}

pub fn execute_ssh(args: &[std::ffi::OsString]) -> i32 {
    let status = process::Command::new("ssh").args(args).status();
    status.map(|s| s.code().unwrap_or(1)).unwrap_or(1)
}
