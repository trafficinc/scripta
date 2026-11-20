use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Command, Output},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scripta"));
    command.current_dir(root).args(["serve", "app.scripta"]);
    for name in [
        "SCR_MAX_STEPS",
        "SCR_MAX_VALUE_BYTES",
        "SCR_MAX_SCOPES",
        "SCR_MAX_SOURCE_BYTES",
    ] {
        command.env_remove(name);
    }
    command
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("app.scripta"),
        "<?scripta File::writeAtomic(\"executed\",\"bad\");",
    )
    .unwrap();
    root
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn inspection_applies_precedence_and_environment_without_listening_or_creating_storage() {
    let root = fixture();
    fs::create_dir(root.path().join("config")).unwrap();
    fs::create_dir(root.path().join("config/nested")).unwrap();
    fs::write(root.path().join("config/server.toml"), "workers = 2\nport = 1234\nqueue = 7\nsession-dir = 'private-secret'\nupload-temp-dir = 'nested/private-spool'\nheader-timeout-ms = 250\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    for args in [
        vec!["--config", "config/server.toml", "--workers", "3"],
        vec!["--workers", "3", "--config", "config/server.toml"],
    ] {
        let text = success(
            command(root.path())
                .args(args)
                .args(["--port", &port, "--print-config"])
                .env("SCR_MAX_STEPS", "4567")
                .output()
                .unwrap(),
        );
        let settings: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(settings["workers"].as_integer(), Some(3));
        assert_eq!(settings["queue"].as_integer(), Some(7));
        assert_eq!(settings["port"].as_integer(), Some(port.parse().unwrap()));
        assert_eq!(settings["header-timeout-ms"].as_integer(), Some(250));
        assert_eq!(settings["connection-timeout-ms"].as_integer(), Some(60000));
        assert_eq!(settings["drain-timeout-ms"].as_integer(), Some(11000));
        assert_eq!(settings["runtime"]["max-steps"].as_str(), Some("4567"));
        assert_eq!(settings["runtime"]["max-scopes"].as_str(), Some("4096"));
        assert!(!text.contains("private-secret"));
        assert!(!text.contains("private-spool"));
        assert!(!text.contains(root.path().to_str().unwrap()));
    }
    success(
        command(root.path())
            .args([
                "--config",
                "config/server.toml",
                "--port",
                &port,
                "--check-config",
            ])
            .output()
            .unwrap(),
    );
    assert!(!root.path().join("executed").exists());
    assert!(!root.path().join("config/private-secret").exists());
    assert!(!root.path().join("config/nested/private-spool").exists());
}

#[test]
fn invalid_files_fail_with_safe_diagnostics_and_no_side_effects() {
    let root = fixture();
    for source in [
        "password = 'DO_NOT_PRINT_ME'",
        "host = 'DO_NOT_PRINT_ME'",
        "workers = 'DO_NOT_PRINT_ME'",
        "workers = 1\nworkers = 'DO_NOT_PRINT_ME'",
        "workers = 65",
        "watch = 'DO_NOT_PRINT_ME'",
        "watch-debounce-ms = 49",
        "watch-debounce-ms = 5001",
        "worker-max-requests = 1000001",
        "worker-max-requests = -1",
        "worker-max-requests = 'DO_NOT_PRINT_ME'",
        "worker-max-requests = 1.5",
        "port = 65536",
        "port = -1",
        "queue = 0",
        "session-dir = ''",
        "upload-temp-dir = ''",
        "session-cookie-secure = 'false'",
        "[runtime]\nmax-steps = 4",
        "header-timeout-ms = 0",
        "drain-timeout-ms = 3600001",
        "connection-timeout-ms = 0",
        "timeout-ms = 3600001",
        "session-ttl-seconds = 0",
        "upload-max-files = 129",
        "session-max-records = 100001",
        "workers = [1]",
        "queue = 1.0",
        "workers = ",
        "log-format = 'DO_NOT_PRINT_ME'",
        "log-format = 1",
        "health-address = '0.0.0.0:9000'",
        "health-address = '[::]:9000'",
        "health-address = 'DO_NOT_PRINT_ME'",
        "host='127.0.0.1'\nport=8080\nhealth-address='127.0.0.1:8080'",
        "trusted-proxies = '127.0.0.1'",
        "trusted-proxies = ['DO_NOT_PRINT_ME']",
        "trusted-proxies = ['::1/129']",
        "allowed-hosts = ['DO_NOT_PRINT_ME/path']",
        "allowed-hosts = [1]",
    ] {
        fs::write(root.path().join("server.toml"), source).unwrap();
        let output = command(root.path())
            .args(["--config", "server.toml", "--check-config"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "accepted {source}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!error.is_empty());
        assert!(!error.contains("DO_NOT_PRINT_ME"));
    }
    fs::write(root.path().join("server.toml"), "#".repeat(65537)).unwrap();
    let output = command(root.path())
        .args(["--config", "server.toml", "--check-config"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("65536"));
    assert!(!root.path().join("executed").exists());
}

#[test]
fn management_settings_are_opt_in_loopback_only_and_overrideable() {
    let root = fixture();
    fs::write(
        root.path().join("server.toml"),
        "log-format='json'\nhealth-address='127.0.0.1:0'",
    )
    .unwrap();
    let settings: toml::Table = toml::from_str(&success(
        command(root.path())
            .args(["--config", "server.toml", "--print-config"])
            .output()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(settings["log-format"].as_str(), Some("json"));
    assert_eq!(settings["health-address"].as_str(), Some("127.0.0.1:0"));
    let settings: toml::Table = toml::from_str(&success(
        command(root.path())
            .args([
                "--config",
                "server.toml",
                "--log-format",
                "text",
                "--health-address",
                "off",
                "--print-config",
            ])
            .output()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(settings["log-format"].as_str(), Some("text"));
    assert_eq!(settings["health-address"].as_str(), Some("off"));
    success(
        command(root.path())
            .args(["--health-address", "[::1]:0", "--check-config"])
            .output()
            .unwrap(),
    );
    assert!(!root.path().join("executed").exists());
}

#[test]
fn proxy_and_host_configuration_uses_bounded_lists_and_cli_replaces_file_lists() {
    let root = fixture();
    fs::write(
        root.path().join("server.toml"),
        "trusted-proxies=['127.0.0.1/32','::1']\nallowed-hosts=['example.com','[::1]:443']",
    )
    .unwrap();
    let text = success(
        command(root.path())
            .args(["--config", "server.toml", "--print-config"])
            .output()
            .unwrap(),
    );
    let settings: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(settings["trusted-proxies"].as_array().unwrap().len(), 2);
    for (flag, field) in [
        ("--trusted-proxies", "trusted-proxies"),
        ("--allowed-hosts", "allowed-hosts"),
    ] {
        let text = success(
            command(root.path())
                .args([flag, "", "--config", "server.toml", "--print-config"])
                .output()
                .unwrap(),
        );
        let settings: toml::Table = toml::from_str(&text).unwrap();
        assert!(settings[field].as_array().unwrap().is_empty());
    }
    for (flag, value) in [
        ("--trusted-proxies", "127.0.0.1,,::1"),
        ("--trusted-proxies", "0.0.0.0/33"),
        ("--allowed-hosts", "*.example.com"),
        ("--allowed-hosts", "host:65536"),
    ] {
        assert!(
            !command(root.path())
                .args([flag, value, "--check-config"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    for count in [128, 129] {
        fs::write(
            root.path().join("server.toml"),
            format!("trusted-proxies=[{}]", vec!["'127.0.0.1'"; count].join(",")),
        )
        .unwrap();
        assert_eq!(
            command(root.path())
                .args(["--config", "server.toml", "--check-config"])
                .output()
                .unwrap()
                .status
                .success(),
            count == 128
        );
    }
}

#[test]
fn inspection_validates_paths_source_flags_and_runtime_environment() {
    let root = fixture();
    for args in [
        vec!["--check-config", "--print-config"],
        vec!["--check-config", "--check-config"],
        vec!["--config", "missing.toml", "--check-config"],
        vec!["--config", "", "--check-config"],
        vec!["--session-dir", "app.scripta", "--check-config"],
        vec!["--session-dir", "app.scripta/nested", "--check-config"],
        vec!["--upload-temp-dir", "missing/nested", "--check-config"],
        vec!["--session-dir", "", "--check-config"],
        vec!["--drain-timeout-ms", "0", "--check-config"],
    ] {
        assert!(
            !command(root.path())
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(
        !command(root.path())
            .arg("--print-config")
            .env("SCR_MAX_STEPS", "DO_NOT_PRINT_ME")
            .output()
            .unwrap()
            .status
            .success()
    );
    // Validate exact timeout bounds without waiting for those durations.
    for value in ["1", "3600000"] {
        success(
            command(root.path())
                .args([
                    "--connection-timeout-ms",
                    value,
                    "--header-timeout-ms",
                    value,
                    "--drain-timeout-ms",
                    value,
                    "--check-config",
                ])
                .output()
                .unwrap(),
        );
    }
    fs::write(
        root.path().join("app.scripta"),
        "<?scripta $credential=\"DO_NOT_PRINT_ME\"; $broken = ;",
    )
    .unwrap();
    let output = command(root.path()).arg("--check-config").output().unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("DO_NOT_PRINT_ME"));
}

#[cfg(unix)]
#[test]
fn inspection_rejects_symlink_and_nonprivate_storage() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = fixture();
    let directory = root.path().join("storage");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    for flag in ["--session-dir", "--upload-temp-dir"] {
        assert!(
            !command(root.path())
                .args([flag, "storage", "--check-config"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&directory, root.path().join("link")).unwrap();
    for flag in ["--session-dir", "--upload-temp-dir"] {
        assert!(
            !command(root.path())
                .args([flag, "link", "--check-config"])
                .output()
                .unwrap()
                .status
                .success()
        );
        success(
            command(root.path())
                .args([flag, "storage", "--check-config"])
                .output()
                .unwrap(),
        );
    }
}

#[test]
fn worker_recycling_settings_preserve_defaults_bounds_and_cli_precedence() {
    let root = fixture();
    let text = success(command(root.path()).arg("--print-config").output().unwrap());
    let settings: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(settings["worker-max-requests"].as_integer(), Some(0));
    fs::write(
        root.path().join("server.toml"),
        "worker-max-requests = 1000000\n",
    )
    .unwrap();
    success(
        command(root.path())
            .args(["--config", "server.toml", "--check-config"])
            .output()
            .unwrap(),
    );
    for value in ["0", "1", "1000000"] {
        let text = success(
            command(root.path())
                .args([
                    "--worker-max-requests",
                    value,
                    "--config",
                    "server.toml",
                    "--print-config",
                ])
                .output()
                .unwrap(),
        );
        let settings: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(
            settings["worker-max-requests"].as_integer(),
            value.parse::<i64>().ok()
        );
    }
    for value in ["-1", "1000001", "18446744073709551616", "secret"] {
        assert!(
            !command(root.path())
                .args(["--worker-max-requests", value, "--check-config"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    assert!(!root.path().join("executed").exists());
}

#[test]
fn watch_is_opt_in_and_cli_can_disable_file_configuration_without_side_effects() {
    let root = fixture();
    fs::write(
        root.path().join("server.toml"),
        "watch=true\nwatch-debounce-ms=50\n",
    )
    .unwrap();
    let text = success(
        command(root.path())
            .args(["--config", "server.toml", "--no-watch", "--print-config"])
            .output()
            .unwrap(),
    );
    let config: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(config["watch"].as_bool(), Some(false));
    assert_eq!(config["watch-debounce-ms"].as_integer(), Some(50));
    let text = success(
        command(root.path())
            .args(["--watch", "--watch-debounce-ms", "5000", "--print-config"])
            .output()
            .unwrap(),
    );
    let config: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(config["watch"].as_bool(), Some(true));
    assert!(
        !command(root.path())
            .args(["--watch", "--no-watch", "--check-config"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!root.path().join("executed").exists());
}

#[test]
fn watch_startup_syntax_diagnostics_are_local_and_explicitly_opt_in() {
    let root = fixture();
    fs::write(
        root.path().join("app.scripta"),
        "<?scripta echo ; // LOCAL_SOURCE_MARKER",
    )
    .unwrap();
    let normal = command(root.path()).arg("--check-config").output().unwrap();
    assert!(!normal.status.success());
    assert!(!String::from_utf8_lossy(&normal.stderr).contains("LOCAL_SOURCE_MARKER"));
    let watched = command(root.path())
        .args(["--watch", "--check-config"])
        .output()
        .unwrap();
    assert!(!watched.status.success());
    let diagnostic = String::from_utf8_lossy(&watched.stderr);
    assert!(diagnostic.contains("app.scripta:1:"), "{diagnostic}");
    assert!(diagnostic.contains("error["));
    assert!(!diagnostic.contains("LOCAL_SOURCE_MARKER"));
}
