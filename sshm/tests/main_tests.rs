use sshm::config::Connection;
use sshm::{build_ssh_args, execute_ssh};

mod test_main {
    use sshm::build_ssh_args;
    use sshm::config::Connection;

    #[test]
    fn test_main_should_connect_false_branch() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert!(!args.is_empty());
    }

    #[test]
    fn test_main_should_connect_true_branch_with_key() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-i"));
    }

    #[test]
    fn test_main_port_not_22_branch() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert!(args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn test_main_port_is_22_branch() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert!(!args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn test_main_user_empty_branch() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert_eq!(args.last().unwrap(), "example.com");
    }

    #[test]
    fn test_main_user_not_empty_branch() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);
        assert_eq!(args.last().unwrap(), "admin@example.com");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_ssh_args_with_key_path() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "/path/to/key");
        assert_eq!(args[2], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_without_key_path() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_with_non_default_port() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "2222");
        assert_eq!(args[2], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_with_default_port() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_with_empty_user() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "example.com");
    }

    #[test]
    fn test_build_ssh_args_full_options() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/home/user/.ssh/id_rsa".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 5);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "/home/user/.ssh/id_rsa");
        assert_eq!(args[2], "-p");
        assert_eq!(args[3], "2222");
        assert_eq!(args[4], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_with_ip_address_host() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "192.168.1.100".to_string(),
            user: "root".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "root@192.168.1.100");
    }

    #[test]
    fn test_build_ssh_args_with_high_port_number() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "user".to_string(),
            port: 65535,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "65535");
        assert_eq!(args[2], "user@example.com");
    }

    #[test]
    fn test_build_ssh_args_port_23() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "user".to_string(),
            port: 23,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "23");
        assert_eq!(args[2], "user@example.com");
    }

    #[test]
    fn test_build_ssh_args_port_21() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "user".to_string(),
            port: 21,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "21");
        assert_eq!(args[2], "user@example.com");
    }

    #[test]
    fn test_build_ssh_args_only_host() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "localhost".to_string(),
            user: "".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "localhost");
    }

    #[test]
    fn test_build_ssh_args_with_key_and_non_default_port() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 2222,
            key_path: Some("/path/to/key".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 5);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "/path/to/key");
        assert_eq!(args[2], "-p");
        assert_eq!(args[3], "2222");
        assert_eq!(args[4], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_key_path_with_spaces() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("/path/to/my key".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "/path/to/my key");
        assert_eq!(args[2], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_with_special_characters_in_host() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "sub-domain.example.co.uk".to_string(),
            user: "user".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "user@sub-domain.example.co.uk");
    }

    #[test]
    fn test_execute_ssh_with_invalid_command() {
        let args: Vec<std::ffi::OsString> = vec![];
        let code = execute_ssh(&args);
        assert_ne!(code, 0);
    }

    #[test]
    fn test_build_ssh_args_empty_key_path_string() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "admin".to_string(),
            port: 22,
            key_path: Some("".to_string()),
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-i");
        assert_eq!(args[1], "");
        assert_eq!(args[2], "admin@example.com");
    }

    #[test]
    fn test_build_ssh_args_user_with_special_chars() {
        let conn = Connection {
            id: "1".to_string(),
            alias: "test".to_string(),
            host: "example.com".to_string(),
            user: "user-name".to_string(),
            port: 22,
            key_path: None,
            folder: None,
        };

        let args = build_ssh_args(&conn);

        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "user-name@example.com");
    }
}

mod ssh_session_exit_code {
    use serial_test::serial;
    use sshm::execute_ssh;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in `ssh` earlier on the `PATH` than the real one, exiting with
    /// the code the test asks about.
    fn stand_in_ssh(dir: &std::path::Path, code: u8) {
        let path = dir.join("ssh");
        std::fs::write(&path, format!("#!/bin/sh\nexit {code}\n")).expect("the stand-in is written");
        let mut perms = std::fs::metadata(&path).expect("the stand-in exists").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("the stand-in is executable");
    }

    /// The exit code of `ssh` is the exit code of `sshm`. The wrapper hands the
    /// terminal back and the reader's next command runs whether the session
    /// connected, timed out or was refused, and a script wrapped around `sshm`
    /// can only tell those apart if the code survives the trip. A fixed answer
    /// — every session reported as 1, or as -1 — makes the tool lie about the
    /// session it just ran.
    #[test]
    #[serial]
    fn the_exit_code_of_ssh_is_the_exit_code_of_sshm() {
        let dir = tempfile::tempdir().expect("a directory for the stand-in ssh");
        stand_in_ssh(dir.path(), 7);

        let previous = std::env::var_os("PATH");
        let mut path = std::ffi::OsString::from(dir.path());
        path.push(":");
        path.push(previous.clone().unwrap_or_default());
        std::env::set_var("PATH", &path);
        let code = execute_ssh(&[]);
        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(code, 7, "ssh answered 7, so sshm must answer 7");
    }

    /// A session that ends in success is reported as success: the code that
    /// matters most is the boring one, and a tool that answers `1` for a
    /// session that closed cleanly teaches its users to ignore the answer.
    #[test]
    #[serial]
    fn a_clean_session_is_reported_as_a_clean_session() {
        let dir = tempfile::tempdir().expect("a directory for the stand-in ssh");
        stand_in_ssh(dir.path(), 0);

        let previous = std::env::var_os("PATH");
        let mut path = std::ffi::OsString::from(dir.path());
        path.push(":");
        path.push(previous.clone().unwrap_or_default());
        std::env::set_var("PATH", &path);
        let code = execute_ssh(&[]);
        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(code, 0, "a session that closed cleanly must not be reported as a failure");
    }
}
