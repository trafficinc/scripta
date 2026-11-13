use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn scr(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scr"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn ok(root: &Path, args: &[&str]) -> String {
    let result = scr(root, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
fn fails(root: &Path, args: &[&str], message: &str) {
    let result = scr(root, args);
    assert!(!result.status.success(), "{args:?} unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains(message), "{args:?}: {stderr}");
}
fn package(root: &Path, name: &str, version: &str, code: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("scripta.toml"),
        format!("[package]\nname = '{name}'\nversion = '{version}'\nentry = 'main.scr'\n"),
    )
    .unwrap();
    fs::write(root.join("main.scr"), code).unwrap();
}
fn dependency(root: &Path, name: &str, path: &str) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(root.join("scripta.toml"))
        .unwrap();
    writeln!(file, "\n[dependencies.{name}]\npath = '{path}'").unwrap();
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn commit(root: &Path, version: &str, text: &str) -> String {
    package(root, "helpers", version, &format!("<?scr echo '{text}';"));
    git(root, &["add", "."]);
    git(
        root,
        &[
            "-c",
            "user.name=Tests",
            "-c",
            "user.email=tests@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            version,
        ],
    );
    git(root, &["tag", &format!("v{version}")]);
    git(root, &["rev-parse", "HEAD"])
}

#[test]
fn local_install_loading_checksums_and_transactional_failure() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    let lib = temp.path().join("helpers");
    fs::create_dir(&app).unwrap();
    package(
        &lib,
        "helpers",
        "1.0.0",
        "<?scr function greeting() { return 'hello'; }",
    );
    ok(&app, &["init"]);
    fails(&app, &["init"], "error[E0601]");
    ok(&app, &["add", "helpers", "--path", "../helpers"]);
    fs::write(
        app.join("main.scr"),
        "<?scr require_once 'helpers'; require_once 'helpers'; echo greeting();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scr"]), "hello");
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    // Snapshot packages remain executable even when the original source changes.
    fs::write(
        lib.join("main.scr"),
        "<?scr function greeting() { return 'updated'; }",
    )
    .unwrap();
    assert_eq!(ok(&app, &["main.scr"]), "hello");
    fails(
        &app,
        &["install", "--locked"],
        "locked package content changed",
    );
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(ok(&app, &["main.scr"]), "hello");
    ok(&app, &["update", "--offline"]);
    assert_eq!(ok(&app, &["main.scr"]), "updated");
    fs::write(
        app.join(".scripta/packages/helpers/main.scr"),
        "<?scr echo 'tampered';",
    )
    .unwrap();
    fails(&app, &["main.scr"], "checksum mismatch");
    ok(&app, &["install", "--locked"]);
    assert_eq!(ok(&app, &["main.scr"]), "updated");
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    fails(
        &app,
        &["add", "wrongname", "--path", "../helpers"],
        "package declares name",
    );
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(ok(&app, &["main.scr"]), "updated");
}

#[test]
fn private_transitive_dependencies_and_relative_local_loading() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let app = root.join("app");
    fs::create_dir(&app).unwrap();
    package(
        &root.join("a"),
        "a",
        "1.0.0",
        "<?scr require_once 'util'; require 'extra.scr';",
    );
    fs::write(root.join("a/extra.scr"), "<?scr echo '!';").unwrap();
    package(&root.join("b"), "b", "1.0.0", "<?scr require_once 'util';");
    package(&root.join("u1"), "util", "1.0.0", "<?scr echo 'one';");
    package(&root.join("u2"), "util", "2.0.0", "<?scr echo 'two';");
    dependency(&root.join("a"), "util", "../u1");
    dependency(&root.join("b"), "util", "../u2");
    ok(&app, &["init"]);
    ok(&app, &["add", "a", "--path", "../a"]);
    ok(&app, &["add", "b", "--path", "../b"]);
    fs::write(
        app.join("main.scr"),
        "<?scr require_once 'a'; require_once 'b';",
    )
    .unwrap();
    assert_eq!(ok(root, &["run", "app/main.scr"]), "one!two");
    fs::write(app.join("private.scr"), "<?scr require 'util';").unwrap();
    fails(&app, &["private.scr"], "cannot resolve required file");
    // A local file takes precedence over a same-named package.
    fs::write(app.join("a"), "<?scr echo 'local';").unwrap();
    assert_eq!(ok(&app, &["main.scr"]), "localtwo");
    fs::remove_file(app.join("a")).unwrap();
    // The root lock, rather than package-provided lockfiles, owns the transitive graph.
    fs::write(
        app.join(".scripta/packages/a/.scripta/packages/util/main.scr"),
        "<?scr echo 'changed';",
    )
    .unwrap();
    fails(&app, &["main.scr"], "checksum mismatch");
}

#[test]
fn git_semver_pins_updates_reinstall_and_revision_sources() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    let app = temp.path().join("app");
    fs::create_dir(&repo).unwrap();
    fs::create_dir(&app).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, "1.0.0", "one");
    let selected = commit(&repo, "1.2.0", "one.two");
    commit(&repo, "2.0.0", "two");
    ok(&app, &["init"]);
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--version", "^1.0"],
    );
    fs::write(app.join("main.scr"), "<?scr require 'helpers';").unwrap();
    assert_eq!(ok(&app, &["main.scr"]), "one.two");
    let lock = fs::read_to_string(app.join("scripta.lock")).unwrap();
    assert!(lock.contains(&selected));
    commit(&repo, "1.3.0", "one.three");
    // Existing installation is sufficient even if the upstream repository disappears.
    fs::rename(&repo, temp.path().join("hidden")).unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(ok(&app, &["main.scr"]), "one.two");
    fs::remove_dir_all(app.join(".scripta/packages")).unwrap();
    fails(
        &app,
        &["install", "--locked", "--offline"],
        "offline installation cannot fetch",
    );
    fs::rename(temp.path().join("hidden"), &repo).unwrap();
    ok(&app, &["install", "--locked"]);
    assert_eq!(fs::read_to_string(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(ok(&app, &["main.scr"]), "one.two");
    ok(&app, &["update"]);
    assert_eq!(ok(&app, &["main.scr"]), "one.three");
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", "v2.0.0"],
    );
    assert_eq!(ok(&app, &["main.scr"]), "two");
}

#[test]
fn missing_stale_locks_cycles_and_invalid_manifests() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    fs::create_dir(&app).unwrap();
    ok(&app, &["init"]);
    fails(&app, &["install", "--locked"], "missing or out of date");
    package(&temp.path().join("a"), "a", "1.0.0", "<?scr");
    package(&temp.path().join("b"), "b", "1.0.0", "<?scr");
    dependency(&temp.path().join("a"), "b", "../b");
    dependency(&temp.path().join("b"), "a", "../a");
    fails(
        &app,
        &["add", "a", "--path", "../a"],
        "circular package dependency",
    );
    assert!(!app.join("scripta.lock").exists());
    assert!(!app.join(".scripta-install.lock").exists());
    assert_eq!(
        fs::read_to_string(app.join("scripta.toml")).unwrap().trim(),
        "[dependencies]"
    );
    fails(
        &app,
        &["add", "a", "--git", "url"],
        "exactly one of version or rev",
    );
    fails(
        &app,
        &["add", "../escape", "--path", "../a"],
        "invalid dependency name",
    );
    package(&temp.path().join("b"), "b", "1.0.0", "<?scr");
    ok(&app, &["add", "b", "--path", "../b"]);
    fs::write(
        app.join("scripta.toml"),
        "[dependencies.b]\npath = '../b'\nversion = '^2'\n",
    )
    .unwrap();
    fails(&app, &["install", "--locked"], "out of date");
    fails(&app, &["install"], "does not satisfy");
    fs::write(
        app.join("scripta.toml"),
        "[dependencies.b]\npath = '../b'\ngti = 'typo'\n",
    )
    .unwrap();
    fails(&app, &["install"], "unknown field");
}

#[cfg(unix)]
#[test]
fn rejects_package_links_and_entry_traversal() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    let lib = temp.path().join("helpers");
    fs::create_dir(&app).unwrap();
    package(&lib, "helpers", "1.0.0", "<?scr");
    ok(&app, &["init"]);
    symlink(temp.path(), lib.join("escape")).unwrap();
    fails(
        &app,
        &["add", "helpers", "--path", "../helpers"],
        "symlinks are unsupported",
    );
    fs::remove_file(lib.join("escape")).unwrap();
    fs::write(
        lib.join("scripta.toml"),
        "[package]\nname='helpers'\nversion='1.0.0'\nentry='../outside.scr'\n",
    )
    .unwrap();
    fails(
        &app,
        &["add", "helpers", "--path", "../helpers"],
        "without '..'",
    );
}

#[test]
fn resolver_respects_read_permission_and_entry_policy() {
    use scr_runtime::{ContextOptions, NativeError, Runtime, RuntimeOptions};
    use std::{cell::Cell, rc::Rc};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("entry.scr");
    fs::write(&path, "<?scr echo 'must not run';").unwrap();
    let called = Rc::new(Cell::new(false));
    let mut context = ContextOptions {
        script_path: Some(temp.path().join("main.scr")),
        ..Default::default()
    };
    context.permissions.allowed.remove("filesystem.read");
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context,
        ..Default::default()
    });
    let flag = called.clone();
    runtime.set_module_resolver(move |_, _| {
        flag.set(true);
        Ok(None)
    });
    let error = runtime
        .execute_source("<?scr require 'helpers';", &mut Vec::new())
        .unwrap_err();
    assert!(error.message.contains("permission denied"));
    assert!(!called.get());

    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            script_path: Some(temp.path().join("main.scr")),
            ..Default::default()
        },
        ..Default::default()
    });
    let expected = path.clone();
    runtime.set_module_resolver(move |_, name| {
        assert_eq!(name, "helpers");
        Ok(Some(expected.clone()))
    });
    runtime.set_module_policy(move |resolved| {
        assert_eq!(resolved, path);
        Err(NativeError::new(
            "PermissionError",
            "package entry denied by host",
        ))
    });
    let mut output = Vec::new();
    let error = runtime
        .execute_source("<?scr require 'helpers';", &mut output)
        .unwrap_err();
    assert!(error.message.contains("package entry denied by host"));
    assert!(output.is_empty());
}

#[test]
fn package_functions_keep_dependency_resolution_context() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    let lib = temp.path().join("helpers");
    let util = temp.path().join("util");
    fs::create_dir(&app).unwrap();
    package(
        &lib,
        "helpers",
        "1.0.0",
        "<?scr function later() { require 'util'; }",
    );
    package(&util, "util", "1.0.0", "<?scr echo 'from dependency';");
    dependency(&lib, "util", "../util");
    ok(&app, &["init"]);
    ok(&app, &["add", "helpers", "--path", "../helpers"]);
    fs::write(
        app.join("main.scr"),
        "<?scr require_once 'helpers'; later();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["main.scr"]), "from dependency");
}
