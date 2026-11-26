use scr_runtime::{ContextOptions, MapKey, Runtime, RuntimeOptions, Value};
use scr_stdlib::FilesystemPolicy;
use std::fs;

fn runtime(root: &std::path::Path, limit: Option<usize>, policy: FilesystemPolicy) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        context: ContextOptions {
            working_directory: root.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register_with_filesystem_policy(&mut runtime, policy).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    runtime.execute(
        &scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap(),
        &mut Vec::new(),
    )
}
fn failure(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = run(runtime, source).unwrap_err();
    assert!(error.message.starts_with(&format!("{kind}:")), "{error}");
}
fn relative_paths(runtime: &Runtime) -> Vec<String> {
    let Some(Value::List(rows)) = runtime.get_global("rows") else {
        panic!("expected rows");
    };
    rows.iter()
        .map(|row| {
            let Value::Map(fields) = row else {
                panic!("expected map");
            };
            let (_, Value::String(relative)) = fields
                .iter()
                .find(|(key, _)| *key == MapKey::String("relativePath".into()))
                .unwrap()
            else {
                panic!("expected path");
            };
            relative.clone()
        })
        .collect()
}
#[test]
fn nested_hidden_unicode_ordering_depth_and_relative_roots() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("z/inner")).unwrap();
    fs::create_dir(dir.path().join(".hidden")).unwrap();
    for name in [
        "z/inner/é.txt",
        "z/a.txt",
        "a.txt",
        ".hidden/secret.txt",
        ".dot",
    ] {
        fs::write(dir.path().join(name), "data").unwrap();
    }
    let mut rt = runtime(dir.path(), None, FilesystemPolicy::default());
    run(&mut rt, "$rows=Dir::walk('.');").unwrap();
    assert_eq!(
        relative_paths(&rt),
        [
            ".dot",
            ".hidden",
            ".hidden/secret.txt",
            "a.txt",
            "z",
            "z/a.txt",
            "z/inner",
            "z/inner/é.txt"
        ]
    );
    run(
        &mut rt,
        "$rows=Dir::walk('.', ['includeHidden'=>false,'maxDepth'=>1]);",
    )
    .unwrap();
    assert_eq!(relative_paths(&rt), ["a.txt", "z"]);
    run(
        &mut rt,
        "$rows=Dir::walk('.', ['maxDepth'=>0,'maxEntries'=>0]);",
    )
    .unwrap();
    assert!(relative_paths(&rt).is_empty());
    run(
        &mut rt,
        "$rows=Dir::glob('.', '**/*.txt', ['includeHidden'=>false]);",
    )
    .unwrap();
    assert_eq!(relative_paths(&rt), ["a.txt", "z/a.txt", "z/inner/é.txt"]);
    rt.set_global(
        "absolute",
        Value::String(dir.path().to_str().unwrap().into()),
    )
    .unwrap();
    run(
        &mut rt,
        "$rows=Dir::glob($absolute, 'z/?*.txt'); $path=$rows[0]['path'];",
    )
    .unwrap();
    assert_eq!(relative_paths(&rt), ["z/a.txt"]);
    assert_eq!(
        rt.get_global("path"),
        Some(&Value::String(
            dir.path().join("z/a.txt").to_str().unwrap().into()
        ))
    );
}
#[test]
fn empty_trees_entry_limits_and_failed_assignment() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), None, FilesystemPolicy::default());
    run(&mut rt, "$rows=Dir::walk('.', ['maxEntries'=>0]);").unwrap();
    assert!(relative_paths(&rt).is_empty());
    fs::write(dir.path().join("a"), "").unwrap();
    fs::write(dir.path().join(".b"), "").unwrap();
    run(&mut rt, "$rows=Dir::walk('.', ['maxEntries'=>2]);").unwrap();
    let before = rt.get_global("rows").unwrap().clone();
    failure(
        &mut rt,
        "$rows=Dir::walk('.', ['maxEntries'=>1]);",
        "ResourceLimitError",
    );
    assert_eq!(rt.get_global("rows"), Some(&before));
    // Filtered and hidden entries still consume the scan budget.
    failure(
        &mut rt,
        "Dir::glob('.', 'missing', ['maxEntries'=>1,'includeHidden'=>false]);",
        "ResourceLimitError",
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}
#[test]
fn strict_options_patterns_and_filesystem_errors() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("file"), "data").unwrap();
    let mut rt = runtime(dir.path(), None, FilesystemPolicy::default());
    for (source, kind) in [
        ("Dir::walk('.', ['unknown'=>1]);", "TypeError"),
        ("Dir::walk('.', ['maxDepth'=>true]);", "TypeError"),
        ("Dir::walk('.', ['maxDepth'=>257]);", "RangeError"),
        ("Dir::walk('.', ['maxEntries'=>-1]);", "RangeError"),
        ("Dir::walk('.', ['maxEntries'=>1000001]);", "RangeError"),
        ("Dir::walk('.', ['includeHidden'=>1]);", "TypeError"),
        ("Dir::walk('.', []);", "TypeError"),
        ("Dir::walk('missing');", "FileNotFoundError"),
        ("Dir::walk('file');", "FilesystemError"),
    ] {
        failure(&mut rt, source, kind);
    }
    for pattern in [
        "", "/a", "a/", "a//b", "../*", "./*", "a**b", "[a]", "{a,b}",
    ] {
        rt.set_global("pattern", Value::String(pattern.into()))
            .unwrap();
        failure(&mut rt, "Dir::glob('.', $pattern);", "GlobError");
    }
    for (size, succeeds) in [(4096, true), (4097, false)] {
        rt.set_global("pattern", Value::String("a".repeat(size)))
            .unwrap();
        if succeeds {
            run(&mut rt, "Path::matchesGlob('a', $pattern);").unwrap();
        } else {
            failure(&mut rt, "Path::matchesGlob('a', $pattern);", "GlobError");
        }
    }
}
#[test]
fn portable_globs_are_anchored_case_sensitive_and_unicode_scalar_based() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(
        dir.path(),
        None,
        FilesystemPolicy {
            read: false,
            ..Default::default()
        },
    );
    for (candidate, pattern, expected) in [
        ("a.txt", "**/*.txt", true),
        ("a/b/c.txt", "**/*.txt", true),
        ("a/b/c.txt", "*.txt", false),
        ("a/b/c.txt", "a/**/c.txt", true),
        ("a/c.txt", "a/**/c.txt", true),
        ("a/b", "a/*", true),
        ("é", "?", true),
        ("e\u{301}", "?", false),
        (".hidden", "*", true),
        ("A.txt", "a.txt", false),
        ("a\nb", "a?b", true),
        ("a\nb", "**", true),
        (r"a\b", r"a\b", true),
        ("x.txt.more", "*.txt", false),
        ("a/b", "**/**", true),
        ("a", "a/**", true),
        ("a", "a/**/**", true),
    ] {
        rt.set_global("candidate", Value::String(candidate.into()))
            .unwrap();
        rt.set_global("pattern", Value::String(pattern.into()))
            .unwrap();
        run(&mut rt, "$match=Path::matchesGlob($candidate,$pattern);").unwrap();
        assert_eq!(
            rt.get_global("match"),
            Some(&Value::Bool(expected)),
            "{candidate} / {pattern}"
        );
    }
    failure(&mut rt, "Dir::walk('.');", "PermissionError");
}
#[test]
fn logical_result_budget_exact_and_one_byte_below() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a"), "").unwrap();
    let keys = [
        "name",
        "path",
        "relativePath",
        "isDirectory",
        "isFile",
        "isSymlink",
    ];
    let bytes = 2 * std::mem::size_of::<Value>()
        + 2
        + dir.path().join(".").join("a").to_str().unwrap().len()
        + keys
            .iter()
            .map(|key| std::mem::size_of::<MapKey>() + key.len() + std::mem::size_of::<Value>())
            .sum::<usize>();
    for (limit, success) in [(bytes, true), (bytes - 1, false)] {
        let mut rt = runtime(dir.path(), Some(limit), FilesystemPolicy::default());
        if success {
            run(&mut rt, "Dir::walk('.');").unwrap();
        } else {
            failure(&mut rt, "Dir::walk('.');", "ResourceLimitError");
        }
    }
}
#[cfg(unix)]
#[test]
fn links_cycles_and_escapes_are_snapshots_never_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "data").unwrap();
    symlink(outside.path(), dir.path().join("escape")).unwrap();
    symlink(".", dir.path().join("cycle")).unwrap();
    symlink("missing", dir.path().join("dangling")).unwrap();
    let mut rt = runtime(
        dir.path(),
        None,
        FilesystemPolicy {
            roots: Some(vec![dir.path().to_owned()]),
            ..Default::default()
        },
    );
    run(
        &mut rt,
        "$rows=Dir::walk('.'); $link=$rows[0]['isSymlink']; $directory=$rows[0]['isDirectory'];",
    )
    .unwrap();
    assert_eq!(relative_paths(&rt), ["cycle", "dangling", "escape"]);
    assert_eq!(rt.get_global("link"), Some(&Value::Bool(true)));
    assert_eq!(rt.get_global("directory"), Some(&Value::Bool(false)));
    failure(&mut rt, "Dir::walk('cycle');", "FilesystemError");
    failure(&mut rt, "Dir::walk('escape');", "PermissionError");
    failure(&mut rt, "Dir::walk('escape/..');", "PermissionError");
}
#[cfg(unix)]
#[test]
fn inaccessible_descendants_fail_without_partial_assignment() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
    let denied = fs::read_dir(&locked).is_err();
    let mut rt = runtime(dir.path(), None, FilesystemPolicy::default());
    let result = run(&mut rt, "$rows='preserved'; $rows=Dir::walk('.');");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    if denied {
        assert!(result.unwrap_err().message.starts_with("PermissionError:"));
        assert_eq!(
            rt.get_global("rows"),
            Some(&Value::String("preserved".into()))
        );
    } else {
        result.unwrap();
    }
}

#[test]
fn runtime_capabilities_deny_discovery_but_allow_pure_matching() {
    let dir = tempfile::tempdir().unwrap();
    let mut options = RuntimeOptions::default();
    options.context.working_directory = dir.path().to_owned();
    options.context.permissions.allowed.clear();
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register(&mut rt).unwrap();
    failure(&mut rt, "Dir::walk('.');", "PermissionError");
    failure(&mut rt, "Dir::glob('.', '**');", "PermissionError");
    run(&mut rt, "Path::matchesGlob('a.txt','**/*.txt');").unwrap();
}
