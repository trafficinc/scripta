use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn scripta(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scripta"))
        .current_dir(root)
        .env_remove("SCR_PACKAGE_INDEX")
        .args(args)
        .output()
        .unwrap()
}
fn ok(root: &Path, args: &[&str]) -> String {
    let result = scripta(root, args);
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
fn fails(root: &Path, args: &[&str], message: &str) {
    let result = scripta(root, args);
    assert!(!result.status.success(), "{args:?} unexpectedly succeeded");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains(message), "{args:?}: {stderr}");
}
fn package(root: &Path, name: &str, version: &str, code: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("scripta.toml"),
        format!("[package]\nname = '{name}'\nversion = '{version}'\nentry = 'main.scripta'\n"),
    )
    .unwrap();
    fs::write(root.join("main.scripta"), code).unwrap();
}
fn dependency(root: &Path, name: &str, path: &str) {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(root.join("scripta.toml"))
        .unwrap();
    writeln!(file, "\n[dependencies.\"{name}\"]\npath = '{path}'").unwrap();
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
fn git_file_url(root: &Path) -> String {
    let path = git(root, &["rev-parse", "--show-toplevel"]).replace('\\', "/");
    format!(
        "file://{}{}",
        if path.starts_with('/') { "" } else { "/" },
        path
    )
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
        app.join("main.scripta"),
        "<?scr require_once 'helpers'; require_once 'helpers'; echo greeting();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "hello");
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    // Snapshot packages remain executable even when the original source changes.
    fs::write(
        lib.join("main.scripta"),
        "<?scr function greeting() { return 'updated'; }",
    )
    .unwrap();
    assert_eq!(ok(&app, &["main.scripta"]), "hello");
    fails(
        &app,
        &["install", "--locked"],
        "locked package content changed",
    );
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(ok(&app, &["main.scripta"]), "hello");
    ok(&app, &["update", "--offline"]);
    assert_eq!(ok(&app, &["main.scripta"]), "updated");
    fs::write(
        app.join(".scripta/packages/helpers/main.scripta"),
        "<?scr echo 'tampered';",
    )
    .unwrap();
    fails(&app, &["main.scripta"], "checksum mismatch");
    ok(&app, &["install", "--locked"]);
    assert_eq!(ok(&app, &["main.scripta"]), "updated");
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    fails(
        &app,
        &["add", "wrongname", "--path", "../helpers"],
        "package declares name",
    );
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(ok(&app, &["main.scripta"]), "updated");
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
        "<?scr require_once 'util'; require 'extra.scripta';",
    );
    fs::write(root.join("a/extra.scripta"), "<?scr echo '!';").unwrap();
    package(&root.join("b"), "b", "1.0.0", "<?scr require_once 'util';");
    package(&root.join("u1"), "util", "1.0.0", "<?scr echo 'one';");
    package(&root.join("u2"), "util", "2.0.0", "<?scr echo 'two';");
    dependency(&root.join("a"), "util", "../u1");
    dependency(&root.join("b"), "util", "../u2");
    ok(&app, &["init"]);
    ok(&app, &["add", "a", "--path", "../a"]);
    ok(&app, &["add", "b", "--path", "../b"]);
    fs::write(
        app.join("main.scripta"),
        "<?scr require_once 'a'; require_once 'b';",
    )
    .unwrap();
    assert_eq!(ok(root, &["run", "app/main.scripta"]), "one!two");
    fs::write(app.join("private.scripta"), "<?scr require 'util';").unwrap();
    fails(&app, &["private.scripta"], "cannot resolve required file");
    // A local file takes precedence over a same-named package.
    fs::write(app.join("a"), "<?scr echo 'local';").unwrap();
    assert_eq!(ok(&app, &["main.scripta"]), "localtwo");
    fs::remove_file(app.join("a")).unwrap();
    // The root lock, rather than package-provided lockfiles, owns the transitive graph.
    fs::write(
        app.join(".scripta/packages/a/.scripta/packages/util/main.scripta"),
        "<?scr echo 'changed';",
    )
    .unwrap();
    fails(&app, &["main.scripta"], "checksum mismatch");
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
    fs::write(app.join("main.scripta"), "<?scr require 'helpers';").unwrap();
    assert_eq!(ok(&app, &["main.scripta"]), "one.two");
    let lock = fs::read_to_string(app.join("scripta.lock")).unwrap();
    assert!(lock.contains(&selected));
    commit(&repo, "1.3.0", "one.three");
    // Existing installation is sufficient even if the upstream repository disappears.
    fs::rename(&repo, temp.path().join("hidden")).unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(ok(&app, &["main.scripta"]), "one.two");
    fs::remove_dir_all(app.join(".scripta/packages")).unwrap();
    fails(
        &app,
        &["install", "--locked", "--offline"],
        "offline installation cannot fetch",
    );
    fs::rename(temp.path().join("hidden"), &repo).unwrap();
    ok(&app, &["install", "--locked"]);
    assert_eq!(fs::read_to_string(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(ok(&app, &["main.scripta"]), "one.two");
    ok(&app, &["update"]);
    assert_eq!(ok(&app, &["main.scripta"]), "one.three");
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", "v2.0.0"],
    );
    assert_eq!(ok(&app, &["main.scripta"]), "two");
}

#[test]
fn local_git_sources_support_cli_paths_and_canonical_manifest_paths() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo with spaces 世界");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, "1.0.0", "local");

    for handwritten in [false, true] {
        let app = temp.path().join(if handwritten {
            "manifest-app"
        } else {
            "cli-app"
        });
        fs::create_dir(&app).unwrap();
        if handwritten {
            // Windows canonical paths carry the \\?\ prefix that Git must not
            // mistake for a remote host. Old manifests/locks can contain it.
            let source = fs::canonicalize(&repo)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            fs::write(
                app.join("scripta.toml"),
                format!(
                    "[dependencies.helpers]\ngit = {}\nrev = 'v1.0.0'\n",
                    toml::Value::String(source),
                ),
            )
            .unwrap();
            ok(&app, &["install"]);
        } else {
            ok(&app, &["init"]);
            ok(
                &app,
                &[
                    "add",
                    "helpers",
                    "--git",
                    "../repo with spaces 世界",
                    "--rev",
                    "v1.0.0",
                ],
            );
        }
        fs::write(app.join("main.scripta"), "<?scr require 'helpers';").unwrap();
        assert_eq!(ok(&app, &["main.scripta"]), "local");
        ok(&app, &["install", "--locked", "--offline"]);
        assert_eq!(ok(&app, &["main.scripta"]), "local");
    }
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
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"main.scripta\"\n\n[dependencies]"
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
        "[package]\nname='helpers'\nversion='1.0.0'\nentry='../outside.scripta'\n",
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
    let path = temp.path().join("entry.scripta");
    fs::write(&path, "<?scr echo 'must not run';").unwrap();
    let called = Rc::new(Cell::new(false));
    let mut context = ContextOptions {
        script_path: Some(temp.path().join("main.scripta")),
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
            script_path: Some(temp.path().join("main.scripta")),
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
        app.join("main.scripta"),
        "<?scr require_once 'helpers'; later();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["main.scripta"]), "from dependency");
}

#[test]
fn package_aliases_keep_same_named_private_dependency_versions_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let app = root.join("app");
    fs::create_dir(&app).unwrap();
    for (name, util) in [("a", "u1"), ("b", "u2")] {
        package(
            &root.join(name),
            name,
            "1.0.0",
            "<?scr require 'util' as Util; function value() { return Util::value(); }",
        );
        dependency(&root.join(name), "util", &format!("../{util}"));
    }
    package(
        &root.join("u1"),
        "util",
        "1.0.0",
        "<?scr function helper() { return 'one'; } function value() { return helper(); }",
    );
    package(
        &root.join("u2"),
        "util",
        "2.0.0",
        "<?scr function helper() { return 'two'; } function value() { return helper(); }",
    );
    ok(&app, &["init"]);
    ok(&app, &["add", "a", "--path", "../a"]);
    ok(&app, &["add", "b", "--path", "../b"]);
    fs::write(app.join("main.scripta"), "<?scr function value() { return 'caller'; } require_once 'a' as A; require_once 'b' as B; require_once 'a' as Again; echo A::value() . B::value() . Again::value() . value();").unwrap();
    assert_eq!(ok(root, &["run", "app/main.scripta"]), "onetwoonecaller");
    fs::write(
        app.join("private.scripta"),
        "<?scr require_once 'a' as A; Util::value();",
    )
    .unwrap();
    fails(&app, &["private.scripta"], "undefined function Util::value");
    fs::write(
        app.join(".scripta/packages/a/.scripta/packages/util/main.scripta"),
        "<?scr function value() { return 'tampered'; }",
    )
    .unwrap();
    fails(&app, &["main.scripta"], "checksum mismatch");
}

#[test]
fn new_projects_declare_scripta_entries_and_legacy_packages_keep_scr_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let legacy = dir.path().join("legacy");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&legacy).unwrap();
    ok(&app, &["init"]);
    let manifest = fs::read_to_string(app.join("scripta.toml")).unwrap();
    assert!(manifest.contains("entry = \"main.scripta\""), "{manifest}");
    fs::write(
        legacy.join("scripta.toml"),
        "[package]\nname = 'legacy'\nversion = '1.0.0'\n",
    )
    .unwrap();
    fs::write(
        legacy.join("main.scr"),
        "<?scr function message() { return 'legacy'; }",
    )
    .unwrap();
    fs::write(legacy.join("main.scripta"), "<?scr throw 'wrong entry';").unwrap();
    ok(&app, &["add", "legacy", "--path", "../legacy"]);
    fs::write(
        app.join("main.scripta"),
        "<?scr require_once 'legacy'; echo message();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["main.scripta"]), "legacy");
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(ok(&app, &["main.scripta"]), "legacy");
}

#[test]
fn init_creates_a_tag_free_starter_and_preserves_existing_source() {
    let directory = tempfile::tempdir().unwrap();
    ok(directory.path(), &["init"]);
    let source = fs::read_to_string(directory.path().join("main.scripta")).unwrap();
    assert_eq!(source, "echo \"Hello, Scripta!\\n\";\n");
    assert_eq!(
        ok(directory.path(), &["run", "main.scripta"]),
        "Hello, Scripta!\n"
    );
    ok(directory.path(), &["check", "main.scripta"]);
    ok(directory.path(), &["fmt", "--check", "main.scripta"]);
    fails(directory.path(), &["init"], "error[E0601]");
    assert_eq!(
        fs::read_to_string(directory.path().join("main.scripta")).unwrap(),
        source
    );

    let existing = tempfile::tempdir().unwrap();
    let source = "<?scr echo 'existing';";
    fs::write(existing.path().join("main.scripta"), source).unwrap();
    ok(existing.path(), &["init"]);
    assert_eq!(
        fs::read_to_string(existing.path().join("main.scripta")).unwrap(),
        source
    );
}

#[test]
fn tag_free_packages_and_callers_interoperate_with_tagged_dependencies_offline() {
    let directory = tempfile::tempdir().unwrap();
    let app = directory.path().join("app");
    fs::create_dir(&app).unwrap();
    let helper = directory.path().join("helpers");
    package(
        &helper,
        "helpers",
        "1.0.0",
        "require_once 'legacy.scripta'; function message() { return legacy() . ' tag free'; }",
    );
    fs::write(
        helper.join("legacy.scripta"),
        "<?scr function legacy() { return 'tagged'; }",
    )
    .unwrap();
    ok(&app, &["init"]);
    ok(&app, &["add", "helpers", "--path", "../helpers"]);
    fs::write(
        app.join("main.scripta"),
        "require 'helpers' as Helpers; echo Helpers::message();",
    )
    .unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(ok(&app, &["run", "main.scripta"]), "tagged tag free");
    ok(&app, &["fmt", "main.scripta"]);
    assert!(
        !fs::read_to_string(app.join("main.scripta"))
            .unwrap()
            .contains("<?")
    );
    assert_eq!(ok(&app, &["run", "main.scripta"]), "tagged tag free");
}

#[test]
fn namespaced_packages_resolve_private_dependencies_and_reject_unsafe_names() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    package(
        &dir.path().join("template"),
        "scripta/template",
        "1.0.0",
        "require 'scripta/util' as Util; function render() { return Util::value(); }",
    );
    package(
        &dir.path().join("util"),
        "scripta/util",
        "1.0.0",
        "function value() { return 'namespaced'; }",
    );
    dependency(&dir.path().join("template"), "scripta/util", "../util");
    ok(&app, &["init"]);
    ok(&app, &["add", "scripta/template", "--path", "../template"]);
    fs::write(
        app.join("main.scripta"),
        "require 'scripta/template' as Template; echo Template::render();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "namespaced");
    ok(&app, &["install", "--locked", "--offline"]);
    assert!(ok(&app, &["list", "--tree"]).contains("  scripta/util\t1.0.0\tpath"));
    for name in [
        "../template",
        "scripta/../template",
        "/template",
        "scripta/",
        "scripta//template",
        "Scripta/template",
        "scripta\\template",
    ] {
        fails(
            &app,
            &["add", name, "--path", "../template"],
            "invalid dependency name",
        );
    }
}

#[test]
fn local_index_search_add_and_override_work_without_network_or_runtime_index_access() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    package(
        &dir.path().join("template"),
        "scripta/template",
        "1.0.0",
        "function render() { return 'rendered'; }",
    );
    let index = dir.path().join("scripta-index.toml");
    fs::write(&index, "format = 1\n[packages.\"scripta/template\"]\ndescription = 'HTML templates'\nsource = { path = 'template', version = '^1.0' }\n").unwrap();
    let result = ok(
        dir.path(),
        &[
            "search",
            "HTML",
            "--index",
            "scripta-index.toml",
            "--offline",
        ],
    );
    assert!(result.contains("scripta/template\tHTML templates"));
    assert_eq!(
        ok(
            dir.path(),
            &["search", "missing", "--index", "scripta-index.toml"]
        ),
        "No matching packages\n"
    );
    ok(&app, &["init"]);
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(app.join("scripta.toml"))
            .unwrap(),
        "\n[registry]\nindex = '../scripta-index.toml'"
    )
    .unwrap();
    assert!(ok(&app, &["search", "template"]).contains("scripta/template"));
    ok(&app, &["add", "scripta/template", "--offline"]);
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    fails(
        &app,
        &["add", "scripta/template", "--version", "^2"],
        "does not satisfy",
    );
    fails(&app, &["add", "unknown"], "not found in index");
    fails(
        &app,
        &["add", "scripta/template", "--rev", "main"],
        "--rev requires a Git package",
    );
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    fs::remove_file(index).unwrap();
    fs::write(
        app.join("main.scripta"),
        "require 'scripta/template' as Template; echo Template::render();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "rendered");
    ok(&app, &["install", "--locked", "--offline"]);
}

#[test]
fn remove_prunes_only_the_requested_graph_and_reuses_verified_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    for name in ["a", "b"] {
        package(
            &dir.path().join(name),
            name,
            "1.0.0",
            "require 'util' as Util; function value() { return Util::value(); }",
        );
        dependency(&dir.path().join(name), "util", "../util");
    }
    package(
        &dir.path().join("util"),
        "util",
        "1.0.0",
        "function value() { return 'kept'; }",
    );
    ok(&app, &["init"]);
    assert_eq!(ok(&app, &["list"]), "No dependencies\n");
    for name in ["a", "b"] {
        ok(&app, &["add", name, "--path", &format!("../{name}")]);
    }
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    fails(&app, &["remove", "util"], "not a direct dependency");
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    fs::remove_dir_all(dir.path().join("a")).unwrap();
    fs::remove_dir_all(dir.path().join("b")).unwrap();
    fs::remove_dir_all(dir.path().join("util")).unwrap();
    ok(&app, &["remove", "a"]);
    assert!(!app.join(".scripta/packages/a").exists());
    assert!(
        app.join(".scripta/packages/b/.scripta/packages/util/main.scripta")
            .is_file()
    );
    fs::write(
        app.join("main.scripta"),
        "require 'b' as B; echo B::value();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "kept");
    assert!(!ok(&app, &["list"]).contains("util"));
    assert!(ok(&app, &["list", "--tree"]).contains("  util\t1.0.0"));
    ok(&app, &["remove", "b"]);
    assert_eq!(ok(&app, &["list", "--tree"]), "No dependencies\n");
}

#[test]
fn failed_remove_preserves_metadata_and_retained_packages() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    ok(&app, &["init"]);
    for name in ["a", "b"] {
        package(&dir.path().join(name), name, "1.0.0", "echo 'ok';");
        ok(&app, &["add", name, "--path", &format!("../{name}")]);
    }
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    fs::write(
        app.join(".scripta/packages/b/main.scripta"),
        "echo 'tampered';",
    )
    .unwrap();
    fails(&app, &["remove", "a"], "checksum mismatch");
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert!(app.join(".scripta/packages/a/main.scripta").is_file());
    assert!(!app.join(".scripta-install.lock").exists());
}

#[test]
fn outdated_reports_local_content_changes_without_writing_metadata_or_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    let lib = dir.path().join("lib");
    package(&lib, "helpers", "1.0.0", "echo 'original';");
    ok(&app, &["init"]);
    ok(
        &app,
        &["add", "helpers", "--path", "../lib", "--version", "^1"],
    );
    assert_eq!(
        ok(&app, &["outdated", "--offline"]),
        "All dependencies are up to date\n"
    );
    let manifest = fs::read(app.join("scripta.toml")).unwrap();
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    package(&lib, "helpers", "1.0.0", "echo 'changed';");
    assert!(ok(&app, &["outdated"]).contains("content changed"));
    package(&lib, "helpers", "2.0.0", "echo 'new';");
    assert!(
        ok(&app, &["outdated", "--offline"]).contains("helpers\t1.0.0\toutside constraint\t2.0.0")
    );
    assert_eq!(fs::read(app.join("scripta.toml")).unwrap(), manifest);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert_eq!(
        ok(&app, &["list"]),
        "PACKAGE\tVERSION\tSOURCE\nhelpers\t1.0.0\tpath\n"
    );
    fs::write(app.join("main.scripta"), "require 'helpers';").unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "original");
}

#[test]
fn outdated_checks_git_versions_and_moving_revisions_without_changing_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let repo = dir.path().join("repo");
    fs::create_dir(&app).unwrap();
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    let first = commit(&repo, "1.0.0", "one");
    let branch = git(&repo, &["branch", "--show-current"]);
    ok(&app, &["init"]);
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--version", "^1"],
    );
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    commit(&repo, "1.1.0", "minor");
    commit(&repo, "2.0.0", "major");
    assert!(ok(&app, &["outdated"]).contains("helpers\t1.0.0\t1.1.0\t2.0.0\tnew version"));
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    assert!(ok(&app, &["outdated", "--offline"]).contains("Git lookup skipped"));
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", &first],
    );
    assert_eq!(ok(&app, &["outdated"]), "All dependencies are up to date\n");
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", &branch],
    );
    commit(&repo, "2.1.0", "branch moved");
    assert!(ok(&app, &["outdated"]).contains("revision changed"));
    git(&repo, &["tag", "release", &first]);
    git(&repo, &["branch", "release"]);
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", "release"],
    );
    assert_eq!(ok(&app, &["outdated"]), "All dependencies are up to date\n");
    git(
        &repo,
        &[
            "-c",
            "user.name=Tests",
            "-c",
            "user.email=tests@example.invalid",
            "tag",
            "-a",
            "annotated",
            "-m",
            "annotated",
            &first,
        ],
    );
    ok(
        &app,
        &["add", "helpers", "--git", "../repo", "--rev", "annotated"],
    );
    assert_eq!(ok(&app, &["outdated"]), "All dependencies are up to date\n");
}

#[test]
fn git_hosted_index_lookup_installs_pinned_sources_and_honors_offline() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    let repo = dir.path().join("repo");
    let catalog = dir.path().join("catalog");
    for path in [&app, &repo, &catalog] {
        fs::create_dir(path).unwrap();
    }
    git(&repo, &["init", "-q"]);
    commit(&repo, "1.0.0", "from index");
    git(&catalog, &["init", "-q"]);
    let repo_url = git_file_url(&repo);
    fs::write(catalog.join("scripta-index.toml"), format!("format = 1\n[packages.helpers]\ndescription = 'Git library'\nsource = {{ git = '{repo_url}', version = '^1' }}\n")).unwrap();
    git(&catalog, &["add", "."]);
    git(
        &catalog,
        &[
            "-c",
            "user.name=Tests",
            "-c",
            "user.email=tests@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "index",
        ],
    );
    let catalog_url = git_file_url(&catalog);
    assert!(ok(dir.path(), &["search", "--index", &catalog_url]).contains("helpers\tGit library"));
    fails(
        dir.path(),
        &["search", "--index", &catalog_url, "--offline"],
        "offline lookup requires a local",
    );
    ok(&app, &["init"]);
    ok(&app, &["add", "helpers", "--index", &catalog_url]);
    fs::write(app.join("main.scripta"), "require 'helpers';").unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "from index");
    let lock = fs::read(app.join("scripta.lock")).unwrap();
    ok(&app, &["install", "--locked", "--offline"]);
    assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    #[cfg(unix)]
    {
        let index_file = catalog.join("scripta-index.toml");
        fs::remove_file(&index_file).unwrap();
        std::os::unix::fs::symlink(dir.path().join("private-index.toml"), &index_file).unwrap();
        git(&catalog, &["add", "."]);
        git(
            &catalog,
            &[
                "-c",
                "user.name=Tests",
                "-c",
                "user.email=tests@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "reject symlink",
            ],
        );
        fails(
            &app,
            &["search", "--index", &catalog_url],
            "must not be a symlink",
        );
        assert_eq!(fs::read(app.join("scripta.lock")).unwrap(), lock);
    }
}

#[test]
fn index_limits_invalid_sources_and_command_options_fail_before_project_mutation() {
    let dir = tempfile::tempdir().unwrap();
    ok(dir.path(), &["init"]);
    let manifest = fs::read(dir.path().join("scripta.toml")).unwrap();
    fails(
        dir.path(),
        &["add", "helpers"],
        "no package index configured",
    );
    fails(dir.path(), &["search"], "no package index configured");
    for args in [
        vec!["list", "--bad"],
        vec!["outdated", "--bad"],
        vec!["search", "--index"],
        vec!["remove"],
        vec!["remove", "missing"],
        vec!["add", "helpers", "--offline", "--offline"],
    ] {
        let output = scripta(dir.path(), &args);
        assert!(!output.status.success(), "{args:?}");
    }
    let index = dir.path().join("index.toml");
    for (contents, error) in [
        ("format = 2\n[packages]\n", "unsupported package index"),
        (
            "format = 1\n[packages.\"../escape\"]\nsource = {path='lib'}",
            "invalid package index",
        ),
        (
            "format = 1\n[packages.helpers]\nsource = {path='lib', git='repo'}",
            "exactly one",
        ),
        (
            "format = 1\n[packages.helpers]\nsource = {git='repo', version='*', rev='HEAD'}",
            "both version and rev",
        ),
    ] {
        fs::write(&index, contents).unwrap();
        fails(dir.path(), &["search", "--index", "index.toml"], error);
    }
    let mut exact = b"format = 1\n[packages]\n#".to_vec();
    exact.resize(4 * 1024 * 1024, b'x');
    fs::write(&index, &exact).unwrap();
    assert_eq!(
        ok(
            dir.path(),
            &["search", "--index", "index.toml", "--offline"]
        ),
        "No matching packages\n"
    );
    exact.push(b'x');
    fs::write(&index, &exact).unwrap();
    fails(
        dir.path(),
        &["search", "--index", "index.toml"],
        "exceeds 4 MiB",
    );
    assert_eq!(fs::read(dir.path().join("scripta.toml")).unwrap(), manifest);
    assert!(!dir.path().join("scripta.lock").exists());
    assert!(!dir.path().join(".scripta").exists());
}

#[test]
fn package_help_and_environment_index_work_outside_a_project() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.toml");
    fs::write(
        &index,
        "format = 1\n[packages.helpers]\ndescription = 'local helpers'\nsource = {path='lib'}",
    )
    .unwrap();
    for command in [
        "add", "remove", "list", "outdated", "search", "install", "update", "init",
    ] {
        assert!(ok(dir.path(), &[command, "--help"]).contains("Usage: scripta init"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_scripta"))
        .current_dir(dir.path())
        .env("SCR_PACKAGE_INDEX", &index)
        .arg("search")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("helpers\tlocal helpers")
    );
}

#[test]
fn index_entry_and_description_limits_accept_exact_and_reject_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.toml");
    let mut contents = String::from("format = 1\n");
    for i in 0..10_000 {
        contents.push_str(&format!("[packages.pkg{i}]\nsource = {{path='lib'}}\n"));
    }
    fs::write(&index, &contents).unwrap();
    assert_eq!(
        ok(dir.path(), &["search", "no-match", "--index", "index.toml"]),
        "No matching packages\n"
    );
    contents.push_str("[packages.extra]\nsource = {path='lib'}\n");
    fs::write(&index, &contents).unwrap();
    fails(
        dir.path(),
        &["search", "--index", "index.toml"],
        "10000 entries",
    );
    for length in [4096, 4097] {
        fs::write(
            &index,
            format!(
                "format = 1\n[packages.helpers]\ndescription = '{}'\nsource = {{path='lib'}}",
                "x".repeat(length)
            ),
        )
        .unwrap();
        if length == 4096 {
            assert!(ok(dir.path(), &["search", "--index", "index.toml"]).contains("helpers\t"));
        } else {
            fails(
                dir.path(),
                &["search", "--index", "index.toml"],
                "description exceeds 4096 bytes",
            );
        }
    }
}

#[test]
fn unqualified_owner_and_namespaced_packages_have_disjoint_storage() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app");
    fs::create_dir(&app).unwrap();
    let owner = dir.path().join("owner");
    let template = dir.path().join("template");
    package(
        &owner,
        "scripta",
        "1.0.0",
        "function value() { return 'owner'; }",
    );
    fs::create_dir(owner.join("template")).unwrap();
    fs::write(owner.join("template/asset.txt"), "original asset").unwrap();
    package(
        &template,
        "scripta/template",
        "1.0.0",
        "function value() { return 'template'; }",
    );
    ok(&app, &["init"]);
    ok(&app, &["add", "scripta", "--path", "../owner"]);
    ok(&app, &["add", "scripta/template", "--path", "../template"]);
    fs::write(app.join("main.scripta"), "require 'scripta' as Owner; require 'scripta/template' as Template; echo Owner::value() . Template::value();").unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "ownertemplate");
    assert_eq!(
        fs::read_to_string(app.join(".scripta/packages/scripta/template/asset.txt")).unwrap(),
        "original asset"
    );
    ok(&app, &["install", "--locked", "--offline"]);
    ok(&app, &["remove", "scripta"]);
    fs::write(
        app.join("main.scripta"),
        "require 'scripta/template' as Template; echo Template::value();",
    )
    .unwrap();
    assert_eq!(ok(&app, &["run", "main.scripta"]), "template");
    ok(&app, &["remove", "scripta/template"]);
    assert_eq!(ok(&app, &["list"]), "No dependencies\n");
}
