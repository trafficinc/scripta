use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::FilesystemPolicy;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        for _ in 0..100 {
            let path = std::env::temp_dir().join(format!(
                "scripta-fs-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(fs::canonicalize(path).unwrap()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create temporary directory: {error}"),
            }
        }
        panic!("temporary directory collision");
    }
    fn runtime(&self) -> Runtime {
        self.with_policy(FilesystemPolicy::default())
    }
    fn with_policy(&self, policy: FilesystemPolicy) -> Runtime {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            context: ContextOptions {
                working_directory: self.0.clone(),
                ..Default::default()
            },
            ..Default::default()
        });
        scr_stdlib::register_with_filesystem_policy(&mut runtime, policy).unwrap();
        runtime
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove test temporary directory");
    }
}
fn run(runtime: &mut Runtime, source: &str) -> Result<u8, RuntimeError> {
    let source = format!("<?scr {source}");
    runtime.execute(
        &scr_parser::parse(&scr_lexer::lex(&source).unwrap()).unwrap(),
        &mut Vec::new(),
    )
}
fn string(runtime: &Runtime, name: &str) -> String {
    let Some(Value::String(value)) = runtime.get_global(name) else {
        panic!("expected string ${name}");
    };
    value.clone()
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) {
    let failure = run(runtime, source).unwrap_err();
    assert!(
        failure.message.starts_with(&format!("{kind}:")),
        "{source}: {failure}"
    );
}

#[test]
fn file_roundtrip_overwrite_append_metadata_copy_move_and_delete() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(
        &mut runtime,
        r#"
        File::write("hello.txt", "long original contents"); File::write("hello.txt", "é");
        File::append("hello.txt", "世界"); $text = File::read("hello.txt");
        $exists = File::exists("hello.txt"); $size = File::size("hello.txt");
        $modified = File::modified("hello.txt"); $extension = File::extension("hello.txt");
        File::copy("hello.txt", "copy.txt"); File::move("copy.txt", "moved.txt");
        $copyExists = File::exists("copy.txt"); $moved = File::read("moved.txt");
        File::delete("hello.txt"); $removed = File::exists("hello.txt");
        File::append("new.txt", "created");
    "#,
    )
    .unwrap();
    assert_eq!(string(&runtime, "text"), "é世界");
    assert_eq!(string(&runtime, "moved"), "é世界");
    assert_eq!(runtime.get_global("size"), Some(&Value::Int(8)));
    assert!(matches!(runtime.get_global("modified"), Some(Value::Int(seconds)) if *seconds > 0));
    assert_eq!(string(&runtime, "extension"), "txt");
    assert_eq!(runtime.get_global("exists"), Some(&Value::Bool(true)));
    for name in ["copyExists", "removed"] {
        assert_eq!(runtime.get_global(name), Some(&Value::Bool(false)));
    }
    assert_eq!(
        fs::read_to_string(temp.0.join("new.txt")).unwrap(),
        "created"
    );
}

#[test]
fn copy_and_move_never_overwrite_existing_names() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(&mut runtime, r#"File::write("source", "source"); File::write("target", "target"); Dir::create("directory");"#).unwrap();
    for source in [
        r#"File::copy("source", "target");"#,
        r#"File::move("source", "target");"#,
        r#"File::copy("source", "source");"#,
        r#"File::move("source", "source");"#,
        r#"File::copy("source", "directory");"#,
        r#"File::move("source", "directory");"#,
    ] {
        error(&mut runtime, source, "AlreadyExistsError");
        assert_eq!(fs::read_to_string(temp.0.join("source")).unwrap(), "source");
        assert_eq!(fs::read_to_string(temp.0.join("target")).unwrap(), "target");
        assert!(temp.0.join("directory").is_dir());
    }
}

#[test]
fn missing_files_raise_exceptions_but_exists_is_false() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(
        &mut runtime,
        r#"$file = File::exists("missing"); $dir = Dir::exists("missing");"#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("file"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("dir"), Some(&Value::Bool(false)));
    for source in [
        r#"File::read("missing");"#,
        r#"File::delete("missing");"#,
        r#"File::size("missing");"#,
        r#"File::modified("missing");"#,
        r#"File::copy("missing", "target");"#,
        r#"File::move("missing", "target");"#,
        r#"Dir::files("missing");"#,
        r#"Dir::remove("missing");"#,
        r#"Dir::change("missing");"#,
        r#"File::write("missing/child", "x");"#,
    ] {
        error(&mut runtime, source, "FileNotFoundError");
    }
    run(&mut runtime, r#"$caught = false; try { File::read("missing"); } catch (FileNotFoundError $e) { $caught = true; }"#).unwrap();
    assert_eq!(runtime.get_global("caught"), Some(&Value::Bool(true)));
    assert!(!temp.0.join("target").exists());
}

#[test]
fn directory_creation_listing_collisions_and_safe_removal() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(
        &mut runtime,
        r#"
        Dir::createAll("nested/empty"); Dir::createAll("nested/empty");
        $empty = Dir::files("nested/empty"); $emptyDirs = Dir::directories("nested/empty");
        Dir::create("z"); Dir::create("a"); File::write("z.txt", "z"); File::write("a.txt", "a");
        $files = Dir::files("."); $dirs = Dir::directories(".");
        $fileIsDir = Dir::exists("a.txt"); $dirIsFile = File::exists("a");
    "#,
    )
    .unwrap();
    assert_eq!(runtime.get_global("empty"), Some(&Value::List(vec![])));
    assert_eq!(runtime.get_global("emptyDirs"), Some(&Value::List(vec![])));
    assert_eq!(runtime.get_global("fileIsDir"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("dirIsFile"), Some(&Value::Bool(false)));
    let Value::List(files) = runtime.get_global("files").unwrap() else {
        panic!();
    };
    let names: Vec<_> = files
        .iter()
        .map(|value| {
            let Value::NativeObject(object) = value else {
                panic!();
            };
            object.get_property("name").unwrap()
        })
        .collect();
    assert_eq!(
        names,
        [Value::String("a.txt".into()), Value::String("z.txt".into())]
    );
    let Value::List(dirs) = runtime.get_global("dirs").unwrap() else {
        panic!();
    };
    assert_eq!(dirs.len(), 3);
    for source in [r#"Dir::create("a.txt");"#, r#"Dir::create("a");"#] {
        error(&mut runtime, source, "AlreadyExistsError");
    }
    for source in [
        r#"File::read("a");"#,
        r#"File::size("a");"#,
        r#"File::delete("a");"#,
        r#"Dir::files("a.txt");"#,
        r#"Dir::createAll("a.txt");"#,
    ] {
        error(&mut runtime, source, "FilesystemError");
    }
    assert!(run(&mut runtime, r#"File::write("a", "bad");"#).is_err());
    assert!(run(&mut runtime, r#"Dir::remove("nested");"#).is_err());
    assert!(temp.0.join("nested/empty").exists());
    error(&mut runtime, r#"Dir::remove(".");"#, "PermissionError");
    error(&mut runtime, r#"Dir::remove("..");"#, "PermissionError");
    error(&mut runtime, r#"Dir::remove("nested", true);"#, "TypeError");
    run(
        &mut runtime,
        r#"Dir::remove("nested/empty"); Dir::remove("nested");"#,
    )
    .unwrap();
    assert!(!temp.0.join("nested").exists());
}

#[test]
fn relative_paths_and_directory_changes_are_local_to_one_runtime() {
    let temp = Temp::new();
    let process_cwd = std::env::current_dir().unwrap();
    let mut first = temp.runtime();
    let mut second = temp.runtime();
    run(&mut first, r#"Dir::create("child"); Dir::change("child"); File::write("relative", "inside"); $cwd = Dir::current(); $absolute = Path::absolute("../root.txt");"#).unwrap();
    assert_eq!(Path::new(&string(&first, "cwd")), temp.0.join("child"));
    assert_eq!(
        Path::new(&string(&first, "absolute")),
        temp.0.join("root.txt")
    );
    assert_eq!(
        fs::read_to_string(temp.0.join("child/relative")).unwrap(),
        "inside"
    );
    run(&mut second, "$cwd = Dir::current();").unwrap();
    assert_eq!(Path::new(&string(&second, "cwd")), temp.0);
    assert_eq!(std::env::current_dir().unwrap(), process_cwd);
    run(&mut first, r#"Dir::change(".."); $cwd = Dir::current();"#).unwrap();
    assert_eq!(Path::new(&string(&first, "cwd")), temp.0);
}

#[test]
fn unicode_entries_are_read_only_and_user_organizer_example_executes() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(&mut runtime, r#"
        Dir::createAll("downloads"); Dir::createAll("documents");
        File::write("downloads/世界.pdf", "pdf"); File::write("downloads/keep.txt", "text");
        foreach (Dir::files("downloads") as $file) {
            if ($file->extension === "pdf") { File::move($file->path, Path::join("documents", $file->name)); }
        }
        $entries = Dir::files("documents"); $entry = $entries[0];
        $name = $entry->name; $path = $entry->path; $extension = $entry->extension;
        $size = $entry->size; $modified = $entry->modified; $isDirectory = $entry->isDirectory;
        $kind = type($entry);
    "#).unwrap();
    assert_eq!(string(&runtime, "name"), "世界.pdf");
    assert_eq!(string(&runtime, "extension"), "pdf");
    assert_eq!(string(&runtime, "kind"), "DirEntry");
    assert_eq!(runtime.get_global("size"), Some(&Value::Int(3)));
    assert_eq!(runtime.get_global("isDirectory"), Some(&Value::Bool(false)));
    assert!(Path::new(&string(&runtime, "path")).is_absolute());
    assert_eq!(
        fs::read_to_string(temp.0.join("documents/世界.pdf")).unwrap(),
        "pdf"
    );
    assert!(temp.0.join("downloads/keep.txt").exists());
    error(&mut runtime, "$entry->name = \"changed\";", "PropertyError");
}

#[test]
fn large_text_and_binary_copy_use_safe_content_semantics() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    let contents = "世界é".repeat(600_000);
    runtime
        .set_global("contents", Value::String(contents.clone()))
        .unwrap();
    run(&mut runtime, r#"File::write("large", $contents); File::append("large", "!"); File::copy("large", "copy"); $read = File::read("copy"); $bytes = File::size("copy");"#).unwrap();
    assert_eq!(string(&runtime, "read"), format!("{contents}!"));
    assert_eq!(
        runtime.get_global("bytes"),
        Some(&Value::Int(contents.len() as i64 + 1))
    );
    fs::write(temp.0.join("binary"), [0xff, 0, 0x80]).unwrap();
    error(&mut runtime, r#"File::read("binary");"#, "EncodingError");
    run(&mut runtime, r#"File::copy("binary", "binary-copy");"#).unwrap();
    assert_eq!(
        fs::read(temp.0.join("binary-copy")).unwrap(),
        [0xff, 0, 0x80]
    );
}

#[test]
fn permission_policies_restrict_operations_roots_and_traversal() {
    let temp = Temp::new();
    let outside = Temp::new();
    fs::create_dir(temp.0.join("allowed")).unwrap();
    fs::write(temp.0.join("allowed/read"), "read").unwrap();
    let mut runtime = temp.with_policy(FilesystemPolicy {
        read: true,
        write: false,
        roots: Some(vec![temp.0.clone()]),
    });
    run(&mut runtime, r#"$read = File::read("allowed/read");"#).unwrap();
    for source in [
        r#"File::write("new", "bad");"#,
        r#"File::append("allowed/read", "bad");"#,
        r#"File::delete("allowed/read");"#,
        r#"Dir::create("new");"#,
    ] {
        error(&mut runtime, source, "PermissionError");
    }
    let mut runtime = temp.with_policy(FilesystemPolicy {
        roots: Some(vec![temp.0.join("allowed")]),
        ..Default::default()
    });
    error(&mut runtime, r#"File::exists("other");"#, "PermissionError");
    error(
        &mut runtime,
        r#"File::write("allowed/missing/../../escape", "bad");"#,
        "PermissionError",
    );
    runtime
        .set_global("outside", Value::String(outside.0.to_str().unwrap().into()))
        .unwrap();
    error(&mut runtime, "Dir::files($outside);", "PermissionError");
    error(&mut runtime, "Dir::change($outside);", "PermissionError");
    run(
        &mut runtime,
        r#"Dir::createAll("allowed/new/child"); File::write("allowed/new/child/file", "ok");"#,
    )
    .unwrap();
    let mut denied = temp.with_policy(FilesystemPolicy {
        roots: Some(vec![]),
        ..Default::default()
    });
    error(
        &mut denied,
        r#"File::exists("missing");"#,
        "PermissionError",
    );
    // Global runtime capabilities also apply, independently of the root policy.
    let mut options = RuntimeOptions::default();
    options.context.working_directory = temp.0.clone();
    options.context.permissions.allowed.clear();
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    error(&mut denied, "Dir::current();", "PermissionError");
    error(
        &mut denied,
        r#"File::write("new", "bad");"#,
        "PermissionError",
    );
    run(&mut denied, r#"$lexical = Path::join("a", "b");"#).unwrap();
    assert!(!temp.0.join("escape").exists());
}

#[test]
fn invalid_arguments_paths_and_source_locations_are_clear() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    for source in [
        "File::read();",
        "File::read(1);",
        "File::write(\"x\", 1);",
        "Dir::current(1);",
        "Path::join(\"x\", 1);",
        "Dir::remove(\"x\", true);",
    ] {
        error(&mut runtime, source, "TypeError");
    }
    error(&mut runtime, r#"File::read("");"#, "PathError");
    runtime
        .set_global("nul", Value::String("bad\0name".into()))
        .unwrap();
    error(&mut runtime, "File::read($nul);", "PathError");
    let source = "<?scr File::read(\"absent\");";
    let program = scr_parser::parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let failure = runtime.execute(&program, &mut Vec::new()).unwrap_err();
    assert_eq!(
        &source[failure.span.start..failure.span.end],
        "File::read(\"absent\")"
    );
    assert_eq!(failure.trace.last().unwrap().function, "File::read");
}

#[test]
fn path_helpers_follow_host_conventions_without_touching_disk() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(&mut runtime, r#"$joined = Path::join("a", "b", "世界.txt"); $name = Path::filename($joined); $ext = Path::extension($joined); $parent = Path::parent($joined); $normal = Path::normalize("a/./b/../c"); $up = Path::normalize("../../x"); $empty = Path::join(); $dot = Path::normalize(""); $hidden = Path::extension(".hidden"); $missing = Path::absolute("missing/../new");"#).unwrap();
    assert_eq!(
        Path::new(&string(&runtime, "joined")),
        Path::new("a").join("b").join("世界.txt")
    );
    assert_eq!(string(&runtime, "name"), "世界.txt");
    assert_eq!(string(&runtime, "ext"), "txt");
    assert_eq!(
        Path::new(&string(&runtime, "parent")),
        Path::new("a").join("b")
    );
    assert_eq!(
        Path::new(&string(&runtime, "normal")),
        Path::new("a").join("c")
    );
    assert_eq!(
        Path::new(&string(&runtime, "up")),
        Path::new("..").join("..").join("x")
    );
    assert_eq!(string(&runtime, "empty"), "");
    assert_eq!(string(&runtime, "dot"), ".");
    assert_eq!(string(&runtime, "hidden"), "");
    assert_eq!(Path::new(&string(&runtime, "missing")), temp.0.join("new"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_excluded_rejected_and_cannot_escape_policy_roots() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let outside = Temp::new();
    fs::write(temp.0.join("real"), "original").unwrap();
    fs::create_dir(temp.0.join("dir")).unwrap();
    symlink(temp.0.join("real"), temp.0.join("link")).unwrap();
    symlink(temp.0.join("dir"), temp.0.join("directory-link")).unwrap();
    symlink("missing", temp.0.join("dangling")).unwrap();
    symlink(".", temp.0.join("cycle")).unwrap();
    let mut runtime = temp.runtime();
    run(&mut runtime, r#"$file = File::exists("link"); $dir = Dir::exists("directory-link"); $files = Dir::files("."); $dirs = Dir::directories(".");"#).unwrap();
    assert_eq!(runtime.get_global("file"), Some(&Value::Bool(false)));
    assert_eq!(runtime.get_global("dir"), Some(&Value::Bool(false)));
    for (name, length) in [("files", 1), ("dirs", 1)] {
        let Value::List(values) = runtime.get_global(name).unwrap() else {
            panic!();
        };
        assert_eq!(values.len(), length);
    }
    for source in [
        r#"File::read("link");"#,
        r#"File::write("link", "bad");"#,
        r#"File::append("link", "bad");"#,
        r#"File::delete("link");"#,
        r#"File::copy("link", "copy");"#,
        r#"File::move("link", "moved");"#,
        r#"Dir::files("directory-link");"#,
        r#"Dir::files("directory-link/");"#,
        r#"Dir::files("directory-link/.");"#,
        r#"Dir::remove("directory-link/");"#,
        r#"Dir::createAll("directory-link/");"#,
        r#"Dir::change("directory-link");"#,
        r#"Dir::remove("directory-link");"#,
        r#"File::write("dangling", "bad");"#,
    ] {
        error(&mut runtime, source, "FilesystemError");
    }
    assert_eq!(fs::read_to_string(temp.0.join("real")).unwrap(), "original");
    symlink(&outside.0, temp.0.join("escape")).unwrap();
    let mut runtime = temp.with_policy(FilesystemPolicy {
        roots: Some(vec![temp.0.clone()]),
        ..Default::default()
    });
    error(
        &mut runtime,
        r#"File::write("escape/new", "bad");"#,
        "PermissionError",
    );
    assert!(!outside.0.join("new").exists());
}

#[cfg(unix)]
#[test]
fn non_utf8_names_raise_encoding_errors_instead_of_lossy_names() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let temp = Temp::new();
    if let Err(error) = fs::write(temp.0.join(OsString::from_vec(vec![b'x', 0xff])), "data") {
        // APFS can reject non-UTF-8 names at creation. Do not mask unrelated
        // failures such as permissions; the conversion itself has a unit test.
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData
            ) || (cfg!(target_os = "macos") && error.raw_os_error() == Some(92)),
            "{error}"
        );
        return;
    }
    let mut runtime = temp.runtime();
    error(&mut runtime, r#"Dir::files(".");"#, "EncodingError");
}

#[cfg(unix)]
#[test]
fn unix_permission_errors_are_not_silently_treated_as_missing() {
    use std::os::unix::fs::PermissionsExt;
    let temp = Temp::new();
    let path = temp.0.join("locked");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("file"), "secret").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
    let denied = fs::read_dir(&path).is_err();
    let mut runtime = temp.runtime();
    let listing = run(&mut runtime, r#"Dir::files("locked");"#);
    let exists = run(&mut runtime, r#"File::exists("locked/file");"#);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    if denied {
        assert!(listing.unwrap_err().message.starts_with("PermissionError:"));
        assert!(exists.unwrap_err().message.starts_with("PermissionError:"));
    } else {
        assert!(
            listing.is_ok(),
            "privileged host can bypass Unix mode restrictions"
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_drive_unc_paths_and_ambiguous_drive_relative_inputs() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    for (input, expected) in [
        (r"C:\one\..\two", r"C:\two"),
        (r"\\server\share\one\..\two", r"\\server\share\two"),
    ] {
        runtime
            .set_global("input", Value::String(input.into()))
            .unwrap();
        run(&mut runtime, "$result = Path::normalize($input);").unwrap();
        assert_eq!(Path::new(&string(&runtime, "result")), Path::new(expected));
    }
    for input in [r"C:relative", r"\rooted"] {
        runtime
            .set_global("input", Value::String(input.into()))
            .unwrap();
        error(&mut runtime, "Path::absolute($input);", "PathError");
    }
}

#[cfg(unix)]
#[test]
fn unix_backslashes_are_filename_characters_and_root_normalization_is_bounded() {
    let temp = Temp::new();
    let mut runtime = temp.runtime();
    run(&mut runtime, r#"$normal = Path::normalize("/../../a");"#).unwrap();
    assert_eq!(string(&runtime, "normal"), "/a");
    runtime
        .set_global("name", Value::String(r"one\two.txt".into()))
        .unwrap();
    run(&mut runtime, "$name = Path::filename($name);").unwrap();
    assert_eq!(string(&runtime, "name"), r"one\two.txt");
}

#[cfg(windows)]
#[test]
fn windows_readonly_file_errors_preserve_contents() {
    let temp = Temp::new();
    let path = temp.0.join("readonly");
    fs::write(&path, "original").unwrap();
    let original_permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original_permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let mut runtime = temp.runtime();
    let result = run(&mut runtime, r#"File::write("readonly", "bad");"#);
    fs::set_permissions(&path, original_permissions).unwrap();
    assert!(result.unwrap_err().message.starts_with("PermissionError:"));
    assert_eq!(fs::read_to_string(path).unwrap(), "original");
}
