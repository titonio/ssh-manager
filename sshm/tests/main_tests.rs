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
