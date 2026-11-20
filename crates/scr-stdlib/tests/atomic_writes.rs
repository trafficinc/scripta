use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::FilesystemPolicy;
use std::{fs, path::Path};

fn runtime(base: &Path, policy: FilesystemPolicy) -> Runtime {
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: base.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register_with_filesystem_policy(&mut rt, policy).unwrap();
    rt
}
fn run(rt: &mut Runtime, body: &str) -> Result<u8, RuntimeError> {
    rt.execute_source(&format!("<?scr {body}"), &mut Vec::new())
}
fn error(rt: &mut Runtime, body: &str, kind: &str) {
    let failure = run(rt, body).unwrap_err();
    assert!(
        failure.message.starts_with(&format!("{kind}:")),
        "{body}: {failure}"
    );
}

#[test]
fn atomic_write_creates_replaces_and_truncates_unicode_and_empty_contents() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    run(&mut rt, "$result=File::writeAtomic('report', 'é世界');").unwrap();
    assert_eq!(rt.get_global("result"), Some(&Value::Null));
    assert_eq!(
        fs::read_to_string(dir.path().join("report")).unwrap(),
        "é世界"
    );
    run(&mut rt, "File::writeAtomic('report', 'x');").unwrap();
    assert_eq!(fs::read(dir.path().join("report")).unwrap(), b"x");
    run(&mut rt, "File::writeAtomic('report', '');").unwrap();
    assert_eq!(fs::read(dir.path().join("report")).unwrap(), b"");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn invalid_inputs_and_destinations_fail_without_mutation_or_staging_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("existing"), "original").unwrap();
    fs::create_dir(dir.path().join("directory")).unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    for source in [
        "File::writeAtomic();",
        "File::writeAtomic('existing');",
        "File::writeAtomic('existing', 1);",
        "File::writeAtomic(1, 'x');",
        "File::writeAtomic('existing', 'x', 'extra');",
    ] {
        error(&mut rt, source, "TypeError");
    }
    error(&mut rt, "File::writeAtomic('', 'x');", "PathError");
    error(
        &mut rt,
        "File::writeAtomic('missing/report', 'x');",
        "FileNotFoundError",
    );
    error(
        &mut rt,
        "File::writeAtomic('directory', 'x');",
        "FilesystemError",
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("existing")).unwrap(),
        "original"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn atomic_write_obeys_root_and_write_policy_and_needs_no_read_capability() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir(root.join("allowed")).unwrap();
    fs::write(root.join("outside"), "original").unwrap();
    let policy = FilesystemPolicy {
        roots: Some(vec![root.join("allowed")]),
        ..Default::default()
    };
    let mut rt = runtime(&root.join("allowed"), policy);
    error(
        &mut rt,
        "File::writeAtomic('../outside', 'bad');",
        "PermissionError",
    );
    run(&mut rt, "File::writeAtomic('report', 'ok');").unwrap();
    assert_eq!(
        fs::read_to_string(root.join("allowed/report")).unwrap(),
        "ok"
    );
    let mut rt = runtime(
        &root,
        FilesystemPolicy {
            write: false,
            ..Default::default()
        },
    );
    error(
        &mut rt,
        "File::writeAtomic('outside', 'bad');",
        "PermissionError",
    );
    let mut options = RuntimeOptions {
        context: ContextOptions {
            working_directory: root.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.write");
    let mut denied = Runtime::with_options(options.clone());
    scr_stdlib::register(&mut denied).unwrap();
    error(
        &mut denied,
        "File::writeAtomic('outside', 'bad');",
        "PermissionError",
    );
    options
        .context
        .permissions
        .allowed
        .insert("filesystem.write".into());
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    let mut write_only = Runtime::with_options(options);
    scr_stdlib::register(&mut write_only).unwrap();
    run(
        &mut write_only,
        "File::writeAtomic('outside', 'write only');",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("outside")).unwrap(),
        "write only"
    );
    assert_eq!(fs::read_dir(root.join("allowed")).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
}

#[test]
fn readonly_destinations_are_rejected_and_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report");
    fs::write(&path, "original").unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut readonly = original.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    let failure = run(&mut rt, "File::writeAtomic('report', 'bad');");
    // Restore even when assertions fail so Windows can remove the fixture.
    fs::set_permissions(&path, original).unwrap();
    assert!(failure.unwrap_err().message.starts_with("PermissionError:"));
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn unix_permissions_symlinks_and_old_open_handles_have_explicit_semantics() {
    use std::{
        io::Read,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report");
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    run(&mut rt, "File::writeAtomic('report', 'original');").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o6750)).unwrap();
    let mut old_handle = fs::File::open(&path).unwrap();
    run(&mut rt, "File::writeAtomic('report', 'new');").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o750
    );
    let mut original = String::new();
    old_handle.read_to_string(&mut original).unwrap();
    assert_eq!(original, "original");
    assert_eq!(fs::read(&path).unwrap(), b"new");
    symlink(&path, dir.path().join("link")).unwrap();
    symlink(dir.path().join("missing"), dir.path().join("dangling")).unwrap();
    for source in [
        "File::writeAtomic('link', 'bad');",
        "File::writeAtomic('dangling', 'bad');",
        "File::writeAtomic('/dev/null', 'bad');",
    ] {
        error(&mut rt, source, "FilesystemError");
    }
    assert_eq!(fs::read(&path).unwrap(), b"new");
    assert!(!dir.path().join("missing").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn concurrent_readers_observe_only_complete_old_or_new_files() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report");
    let old = "a".repeat(128 * 1024);
    let new = "b".repeat(128 * 1024);
    fs::write(&path, &old).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let ready = Arc::new(Barrier::new(2));
    let reader = {
        let stop = stop.clone();
        let ready = ready.clone();
        let path = path.clone();
        std::thread::spawn(move || -> Result<usize, String> {
            let first = fs::read(&path).map_err(|e| e.to_string());
            ready.wait();
            let verify = |bytes: &[u8]| {
                bytes.len() == 128 * 1024
                    && (bytes.iter().all(|&b| b == b'a') || bytes.iter().all(|&b| b == b'b'))
            };
            if !verify(&first?) {
                return Err("partial initial file".into());
            }
            let mut reads = 1;
            while !stop.load(Ordering::Relaxed) {
                let bytes = fs::read(&path).map_err(|e| e.to_string())?;
                if !verify(&bytes) {
                    return Err("reader observed partial or mixed contents".into());
                }
                reads += 1;
            }
            Ok(reads)
        })
    };
    ready.wait();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("old", Value::String(old)).unwrap();
    rt.set_global("new", Value::String(new)).unwrap();
    let result = (0..32).try_for_each(|i| {
        run(
            &mut rt,
            if i % 2 == 0 {
                "File::writeAtomic('report', $new);"
            } else {
                "File::writeAtomic('report', $old);"
            },
        )
        .map(|_| ())
    });
    stop.store(true, Ordering::Relaxed);
    let reads = reader.join().unwrap().unwrap();
    result.unwrap();
    assert!(reads >= 1);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn windows_handle_denying_delete_blocks_replacement_without_losing_original() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report");
    fs::write(&path, "original").unwrap();
    // Permit readers/writers but deny delete sharing so staging can complete
    // and the actual replacement must fail.
    let handle = fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&path)
        .unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    let result = run(&mut rt, "File::writeAtomic('report', 'bad');");
    drop(handle);
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), b"original");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    run(&mut rt, "File::writeAtomic('report', 'recovered');").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"recovered");
}
