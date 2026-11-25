use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn command(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scripta"))
        .current_dir(root)
        .env_remove("SCR_PACKAGE_INDEX")
        .args(args)
        .output()
        .unwrap()
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn failure(output: Output, expected: &str) {
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains(expected), "{text}");
}
fn package(root: &Path, name: &str, extra: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("scripta.toml"),
        format!("[package]\nname='{name}'\nversion='1.0.0'\nentry='main.scripta'\n{extra}\n"),
    )
    .unwrap();
    fs::write(
        root.join("main.scripta"),
        "function greeting() { return 'installed'; }\n",
    )
    .unwrap();
}
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, at: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(at).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
fn git(root: &Path, args: &[&str]) {
    let hooks = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Extension Tests",
            "-c",
            "user.email=tests@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
        ])
        .arg(format!("core.hooksPath={}", hooks.path().display()))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn repository(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "fixture"]);
}

#[test]
fn root_failures_preserve_lf_crlf_manifest_lock_and_installed_files_before_any_script_runs() {
    for newline in ["\n", "\r\n"] {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path();
        let manifest="[package]\nname='app'\nversion='1.0.0'\nentry='main.scripta'\n[extensions]\nmissingconnector={version='^1',capabilities=['prepare.native']}\n[dependencies]\n".replace('\n',newline);
        fs::write(app.join("scripta.toml"), manifest.as_bytes()).unwrap();
        fs::write(
            app.join("scripta.lock"),
            "format = 1\n[packages]\n".replace('\n', newline).as_bytes(),
        )
        .unwrap();
        fs::write(
            app.join("main.scripta"),
            "File::write('executed','bad'); echo 'must not run';",
        )
        .unwrap();
        fs::create_dir_all(app.join(".scripta/packages")).unwrap();
        fs::write(app.join(".scripta/packages/sentinel"), b"unchanged\0").unwrap();
        let before = files(app);
        for args in [
            &["install", "--locked", "--offline"][..],
            &["install"][..],
            &["update", "--offline"][..],
            &[
                "add",
                "unavailable",
                "--git",
                "../no-repository",
                "--rev",
                "HEAD",
            ][..],
        ] {
            failure(command(app, args), "required extension 'missingconnector'");
            assert_eq!(files(app), before);
        }
        let output = command(app, &["run", "main.scripta"]);
        assert!(output.stdout.is_empty());
        failure(output, "required extension 'missingconnector'");
        assert!(!app.join("executed").exists());
        success(command(app, &["check", "main.scripta"]));
        assert_eq!(files(app), before);
    }
}

#[test]
fn failed_local_and_git_transitive_requirements_preserve_previous_installation() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    fs::create_dir(&app).unwrap();
    package(&temp.path().join("safe"), "safe", "");
    success(command(&app, &["init"]));
    success(command(&app, &["add", "safe", "--path", "../safe"]));
    let before = files(&app);
    for (name, git_source) in [("localouter", false), ("gitouter", true)] {
        let parent = temp.path().join(name);
        package(
            &parent,
            name,
            "[dependencies]\nconnector={path='connector'}",
        );
        package(
            &parent.join("connector"),
            "connector",
            "[extensions]\nmissingconnector={version='1'}",
        );
        if git_source {
            repository(&parent);
        }
        let source = parent.to_str().unwrap();
        let args = if git_source {
            vec!["add", name, "--git", source, "--rev", "HEAD"]
        } else {
            vec!["add", name, "--path", source]
        };
        let output = command(&app, &args);
        failure(
            output,
            "package 'connector@1.0.0': required extension 'missingconnector'",
        );
        assert_eq!(files(&app), before);
    }
    assert!(success(command(&app, &["run", "main.scripta"])).contains("Hello, Scripta!"));
}

#[cfg(any(feature = "sqlite", feature = "mysql"))]
fn enabled() -> (&'static str, &'static str) {
    if cfg!(feature = "sqlite") {
        ("sqlite", "query.assoc")
    } else {
        ("mysql", "prepare.native")
    }
}
#[test]
#[cfg(any(feature = "sqlite", feature = "mysql"))]
fn mismatched_versions_and_capabilities_fail_before_add_changes_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    fs::create_dir(&app).unwrap();
    success(command(&app, &["init"]));
    let (name, capability) = enabled();
    for (constraint, cap, expected) in [
        (">=999.0.0", capability, "does not satisfy"),
        ("*", "unimplemented.feature", "lacks required capability"),
    ] {
        package(
            &temp.path().join("library"),
            "library",
            &format!("[extensions]\n{name}={{version='{constraint}',capabilities=['{cap}']}}"),
        );
        let before = files(&app);
        failure(
            command(&app, &["add", "library", "--path", "../library"]),
            expected,
        );
        assert_eq!(files(&app), before);
    }
}

#[test]
#[cfg(any(feature = "sqlite", feature = "mysql"))]
fn locked_offline_reuse_rechecks_host_metadata_without_git_or_index_access() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    fs::create_dir(&app).unwrap();
    success(command(&app, &["init"]));
    let (name, capability) = enabled();
    let lib = temp.path().join("library");
    package(
        &lib,
        "library",
        &format!("[extensions]\n{name}={{version='*',capabilities=['{capability}']}}"),
    );
    repository(&lib);
    success(command(
        &app,
        &["add", "library", "--git", "../library", "--rev", "HEAD"],
    ));
    fs::write(
        app.join("main.scripta"),
        "require 'library' as Library; echo Library::greeting();",
    )
    .unwrap();
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    assert!(
        std::str::from_utf8(&lock)
            .unwrap()
            .starts_with("format = 1")
    );
    assert!(!std::str::from_utf8(&lock).unwrap().contains("extensions"));
    fs::rename(&lib, temp.path().join("source-moved")).unwrap();
    let no_tools = temp.path().join("no-tools");
    fs::create_dir(&no_tools).unwrap();
    success(
        Command::new(env!("CARGO_BIN_EXE_scripta"))
            .current_dir(&app)
            .env("PATH", &no_tools)
            .env("SCR_PACKAGE_INDEX", "not-present")
            .args(["install", "--locked", "--offline"])
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(
        success(command(&app, &["run", "main.scripta"])),
        "installed"
    );
    let before = files(&app);
    let error = scr_cli::packages::check_execution_with_extensions(&app, &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("package 'library@1.0.0': required extension"),
        "{error}"
    );
    assert!(scr_cli::packages::resolve_with_extensions(&app, "library", &[]).is_err());
    assert_eq!(files(&app), before);
    let mut manifest: toml::Value =
        toml::from_str(&fs::read_to_string(app.join("scripta.toml")).unwrap()).unwrap();
    let extensions = toml::Value::Table(toml::map::Map::from_iter([(
        name.into(),
        toml::Value::Table(toml::map::Map::from_iter([(
            "version".into(),
            toml::Value::String(">=999".into()),
        )])),
    )]));
    manifest
        .as_table_mut()
        .unwrap()
        .insert("extensions".into(), extensions);
    fs::write(
        app.join("scripta.toml"),
        toml::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let before = files(&app);
    failure(
        command(&app, &["install", "--locked", "--offline"]),
        "does not satisfy",
    );
    failure(command(&app, &["run", "main.scripta"]), "does not satisfy");
    assert_eq!(files(&app), before);
}

#[test]
#[cfg(not(feature = "mysql"))]
fn mysql_disabled_binary_reports_the_source_feature_hint_during_install() {
    let temp = tempfile::tempdir().unwrap();
    package(
        temp.path(),
        "app",
        "[extensions]\nmysql={version='^0.2',capabilities=['prepare.native']}",
    );
    failure(command(temp.path(), &["install"]), "--features mysql");
}

#[test]
#[ignore = "requires compatible and minimal binaries in SCR_EXTENSION_COMPATIBLE_BINARY and SCR_EXTENSION_INCOMPATIBLE_BINARY"]
fn native_requirements_are_revalidated_when_an_installed_project_moves_to_another_binary() {
    let enabled =
        std::env::var_os("SCR_EXTENSION_COMPATIBLE_BINARY").expect("provide MySQL-enabled binary");
    let disabled =
        std::env::var_os("SCR_EXTENSION_INCOMPATIBLE_BINARY").expect("provide minimal binary");
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    fs::create_dir(&app).unwrap();
    package(
        &temp.path().join("library"),
        "library",
        "[extensions]\nmysql={version='^0.2',capabilities=['prepare.native','transactions']}",
    );
    package(&app, "app", "[dependencies]\nlibrary={path='../library'}");
    fs::write(
        app.join("main.scripta"),
        "File::write('executed','bad'); require 'library' as Library; echo Library::greeting();",
    )
    .unwrap();
    success(
        Command::new(&enabled)
            .current_dir(&app)
            .arg("install")
            .output()
            .unwrap(),
    );
    let before = files(&app);
    failure(
        Command::new(&disabled)
            .current_dir(&app)
            .args(["install", "--locked", "--offline"])
            .output()
            .unwrap(),
        "required extension 'mysql'",
    );
    for args in [
        &["run", "main.scripta"][..],
        &["serve", "main.scripta", "--check-config"][..],
    ] {
        let output = Command::new(&disabled)
            .current_dir(&app)
            .args(args)
            .output()
            .unwrap();
        assert!(output.stdout.is_empty());
        failure(output, "required extension 'mysql'");
    }
    success(
        Command::new(&disabled)
            .current_dir(&app)
            .args(["check", "main.scripta"])
            .output()
            .unwrap(),
    );
    assert_eq!(files(&app), before);
    success(
        Command::new(&disabled)
            .current_dir(&app)
            .args(["remove", "library"])
            .output()
            .unwrap(),
    );
    assert!(!app.join(".scripta/packages/library").exists());
}

#[test]
fn custom_embedding_catalog_is_used_for_installation_and_execution_checks() {
    let temp = tempfile::tempdir().unwrap();
    package(
        temp.path(),
        "app",
        "[extensions]
embedded={version='^1.2',capabilities=['query.native']}",
    );
    let catalog = [scr_cli::extensions::ExtensionInfo {
        name: "embedded",
        version: "1.2.3",
        capabilities: &["query.native"],
    }];
    let args = ["install".into(), "--offline".into()];
    scr_cli::packages::execute_with_extensions(&args, temp.path(), &catalog).unwrap();
    scr_cli::packages::check_execution_with_extensions(temp.path(), &catalog).unwrap();
    let before = files(temp.path());
    assert!(
        scr_cli::packages::execute_with_extensions(&args, temp.path(), &[])
            .unwrap_err()
            .to_string()
            .contains("required extension 'embedded'")
    );
    assert_eq!(files(temp.path()), before);
}

#[test]
fn local_and_cached_module_loads_revalidate_requirements_before_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path();
    package(app, "app", "");
    package(&app.join("nested"), "nested", "");
    fs::write(
        app.join("nested/main.scripta"),
        "File::write('child-ran','once'); function greeting(){return 'ok';}",
    )
    .unwrap();
    let incompatible = "[package]\nname='nested'\nversion='1.0.0'\nentry='main.scripta'\n[extensions]\nmissingconnector={version='1'}\n";
    fs::write(app.join("main.scripta"),format!("require 'nested/main.scripta' as First; File::write('nested/scripta.toml', {incompatible:?}); require 'nested/main.scripta' as Second; echo 'must not return cached alias';")).unwrap();
    let output = command(app, &["run", "main.scripta"]);
    assert!(output.stdout.is_empty());
    failure(output, "required extension 'missingconnector'");
    assert_eq!(fs::read_to_string(app.join("child-ran")).unwrap(), "once");
    fs::remove_file(app.join("child-ran")).unwrap();
    fs::write(
        app.join("main.scripta"),
        "require 'nested/main.scripta' as First;",
    )
    .unwrap();
    failure(
        command(app, &["run", "main.scripta"]),
        "required extension 'missingconnector'",
    );
    assert!(!app.join("child-ran").exists());
}

#[test]
fn test_and_server_preflight_do_not_initialize_an_incompatible_project() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path();
    package(app, "app", "[extensions]\nmissingconnector={version='1'}");
    fs::create_dir(app.join("tests")).unwrap();
    fs::write(
        app.join("tests/native_test.scripta"),
        "File::write('test-initialized','bad'); function test_noop(){} ",
    )
    .unwrap();
    let listed = success(command(app, &["test", "--list"]));
    assert!(listed.contains("test_noop"));
    failure(
        command(app, &["test"]),
        "required extension 'missingconnector'",
    );
    failure(
        command(app, &["serve", "main.scripta", "--check-config"]),
        "required extension 'missingconnector'",
    );
    assert!(!app.join("test-initialized").exists());
}

#[test]
fn postgres_manifest_uses_actual_compiled_version_and_capabilities() {
    let root = tempfile::tempdir().unwrap();
    package(
        root.path(),
        "postgres-consumer",
        "[extensions]\npgsql={version='^0.2', capabilities=['prepare.native','dml.returning','lastInsertId.sequence']}",
    );
    let output = command(root.path(), &["install", "--offline"]);
    if cfg!(feature = "postgres") {
        success(output);
    } else {
        failure(output, "--features postgres");
    }
}
