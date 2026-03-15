use crate::config::Connection;
use std::process;

pub fn build_ssh_args(conn: &Connection) -> Vec<std::ffi::OsString> {
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
