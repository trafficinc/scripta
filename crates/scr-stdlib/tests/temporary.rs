use scr_runtime::{
    NativeFunction, NativeObject, NativeSignature, NativeType, Runtime, RuntimeOptions, Value,
};
use scr_stdlib::FilesystemPolicy;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn runtime(root: &Path, options: RuntimeOptions, policy: FilesystemPolicy) -> Runtime {
    let mut options = options;
    options.context.working_directory = fs::canonicalize(root).unwrap();
    let mut rt = Runtime::with_options(options);
    scr_stdlib::register_with_filesystem_policy(&mut rt, policy).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) -> Result<u8, scr_runtime::RuntimeError> {
    rt.execute_source(source, &mut Vec::new())
}
fn failure(rt: &mut Runtime, source: &str, kind: &str) {
    let error = run(rt, source).unwrap_err();
    assert!(error.message.starts_with(&format!("{kind}:")), "{error}");
}
fn string(rt: &Runtime, name: &str) -> String {
    let Some(Value::String(s)) = rt.get_global(name) else {
        panic!("expected ${name}");
    };
    s.clone()
}
fn closed(value: &Value) -> bool {
    let Value::NativeObject(h) = value else {
        panic!();
    };
    h.get_property("closed").unwrap() == Value::Bool(true)
}
#[test]
fn temporary_resources_are_private_exclusive_aliased_and_execution_scoped() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("existing"), "unchanged").unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    run(
        &mut rt,
        r#"
        $d=Temp::directory('.'); $directory=$d->path;
        $f=Temp::file($d->path); $file=$f->path; $alias=$f;
        File::write($f->path,'hello'); $text=File::read($alias->path);
        $other=Temp::file($d->path); $different=$other->path !== $f->path;
        $lock=File::lock($f->path,'exclusive');
    "#,
    )
    .unwrap();
    assert_eq!(string(&rt, "text"), "hello");
    assert_eq!(rt.get_global("different"), Some(&Value::Bool(true)));
    for name in ["d", "f", "alias", "other", "lock"] {
        assert!(closed(rt.get_global(name).unwrap()));
    }
    assert!(!Path::new(&string(&rt, "directory")).exists());
    assert!(!Path::new(&string(&rt, "file")).exists());
    run(
        &mut rt,
        "Temp::close($alias); Temp::close($d); File::unlock($lock);",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("existing")).unwrap(),
        "unchanged"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn explicit_close_removes_paths_and_unreferenced_resources_close_at_execution_end() {
    let root = tempfile::tempdir().unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    run(&mut rt,"$t=Temp::file('.'); $p=$t->path; $alias=$t; Temp::close($t); $closed=$alias->closed; $gone=!File::exists($p); Temp::close($alias); $d=Temp::directory('.'); $dp=$d->path; $d=null; $retained=Dir::exists($dp);").unwrap();
    for name in ["closed", "gone", "retained"] {
        assert_eq!(rt.get_global(name), Some(&Value::Bool(true)));
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn uncaught_errors_step_limits_and_exit_cleanup_host_retained_aliases() {
    for (end, options, is_error) in [
        ("File::read('missing');", RuntimeOptions::default(), true),
        (
            "while (true) {}",
            RuntimeOptions {
                instruction_limit: Some(1000),
                ..Default::default()
            },
            true,
        ),
        ("exit 3;", RuntimeOptions::default(), false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut rt = runtime(root.path(), options, FilesystemPolicy::default());
        let result = run(
            &mut rt,
            &format!(
                "$t=Temp::directory('.'); $path=$t->path; File::write(Path::join($path,'child'),'data'); $lock=File::lock(Path::join($path,'child'),'exclusive'); {end}"
            ),
        );
        assert_eq!(result.is_err(), is_error);
        let alias = rt.get_global("t").unwrap().clone();
        assert!(closed(&alias));
        assert!(!Path::new(&string(&rt, "path")).exists());
        drop(rt);
        assert!(closed(&alias));
    }
}
#[cfg(unix)]
#[test]
fn temporary_cleanup_unlinks_contained_links_without_touching_targets() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), "safe").unwrap();
    let target = outside.path().to_owned();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    rt.register(
        "makeLinks",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Null),
            move |_, args| {
                let Value::String(path) = &args[0] else {
                    unreachable!()
                };
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o700
                );
                symlink(&target, Path::new(path).join("escape")).unwrap();
                symlink(".", Path::new(path).join("cycle")).unwrap();
                symlink("missing", Path::new(path).join("dangling")).unwrap();
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    rt.register(
        "checkMode",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Null),
            |_, args| {
                let Value::String(path) = &args[0] else {
                    unreachable!()
                };
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    run(
        &mut rt,
        "$d=Temp::directory('.'); makeLinks($d->path); $f=Temp::file('.'); checkMode($f->path);",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(outside.path().join("keep")).unwrap(),
        "safe"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
#[test]
fn replacing_a_temporary_path_refuses_cleanup_of_replacement_data() {
    for directory in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut rt = runtime(
            root.path(),
            RuntimeOptions::default(),
            FilesystemPolicy::default(),
        );
        rt.register(
            "replace",
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::String], NativeType::Null),
                move |_, args| {
                    let Value::String(path) = &args[0] else {
                        unreachable!()
                    };
                    fs::rename(path, Path::new(path).with_extension("moved")).unwrap();
                    if directory {
                        fs::create_dir(path).unwrap();
                        fs::write(Path::new(path).join("replacement"), "safe").unwrap();
                    } else {
                        fs::write(path, "safe").unwrap();
                    }
                    Ok(Value::Null)
                },
            ),
        )
        .unwrap();
        let create = if directory { "directory" } else { "file" };
        failure(
            &mut rt,
            &format!("$t=Temp::{create}('.'); $path=$t->path; replace($path); Temp::close($t);"),
            "TempError",
        );
        let path = PathBuf::from(string(&rt, "path"));
        let replacement = if directory {
            path.join("replacement")
        } else {
            path.clone()
        };
        assert_eq!(fs::read_to_string(replacement).unwrap(), "safe");
        assert!(path.with_extension("moved").exists());
        // Restoring the owned path allows a later explicit cleanup retry.
        if directory {
            fs::remove_dir_all(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        fs::rename(path.with_extension("moved"), &path).unwrap();
        run(&mut rt, "Temp::close($t);").unwrap();
        assert!(!path.exists());
    }
}
#[test]
fn permissions_options_foreign_and_forged_handles_fail_without_modifying_files() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("data"), "safe").unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    for (source, kind) in [
        ("Temp::file('missing');", "FileNotFoundError"),
        ("Temp::directory('data');", "FilesystemError"),
        ("Temp::file('');", "PathError"),
        ("File::lock('missing','exclusive');", "FileNotFoundError"),
        ("File::lock('data','bad');", "TypeError"),
        ("File::lock('data','shared',-1);", "RangeError"),
        ("File::lock('data','shared',30001);", "RangeError"),
        ("Temp::close(1);", "TypeError"),
    ] {
        failure(&mut rt, source, kind);
    }
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn type_name(&self) -> &str {
            "TempFile"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("no properties".into())
        }
    }
    rt.set_global("fake", Value::NativeObject(Rc::new(Fake)))
        .unwrap();
    failure(&mut rt, "Temp::close($fake);", "TypeError");
    let other = std::cell::RefCell::new(runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    ));
    rt.register(
        "checkForeign",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::Any], NativeType::Null),
            move |_, args| {
                let mut other = other.borrow_mut();
                other.set_global("foreign", args[0].clone()).unwrap();
                let Value::NativeObject(handle) = &args[0] else {
                    unreachable!()
                };
                let source = if handle.type_name() == "FileLock" {
                    "File::unlock($foreign);"
                } else {
                    "Temp::close($foreign);"
                };
                failure(&mut other, source, "TypeError");
                assert!(!closed(&args[0]));
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    run(
        &mut rt,
        "$t=Temp::file('.'); checkForeign($t); File::write($t->path,'still usable'); $lock=File::lock($t->path,'exclusive'); checkForeign($lock);",
    )
    .unwrap();
    let mut denied = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy {
            write: false,
            ..Default::default()
        },
    );
    failure(&mut denied, "Temp::file('.');", "PermissionError");
    failure(
        &mut denied,
        "File::lock('data','exclusive');",
        "PermissionError",
    );
    run(&mut denied, "$lock=File::lock('data','shared');").unwrap();
    assert_eq!(
        fs::read_to_string(root.path().join("data")).unwrap(),
        "safe"
    );
}
#[test]
fn exact_resource_count_limit_and_reuse_after_close() {
    let root = tempfile::tempdir().unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    let fill = "$handles=[]; for ($i=0; $i<64; $i=$i+1) {$handles[]=Temp::file('.');}";
    run(
        &mut rt,
        &format!("{fill} Temp::close($handles[0]); $replacement=Temp::directory('.');"),
    )
    .unwrap();
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    failure(
        &mut rt,
        &format!("{fill} $extra=Temp::file('.');"),
        "ResourceLimitError",
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}

// Spawn this test executable as a fixture process so lock contention is tested
// across processes on every host, without Python or platform-specific commands.
#[test]
fn lock_fixture_process() {
    let Some(path) = std::env::var_os("SCRIPTA_LOCK_FIXTURE_PATH") else {
        return;
    };
    let mode = std::env::var("SCRIPTA_LOCK_FIXTURE_MODE").unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    if mode == "shared" {
        fs2::FileExt::lock_shared(&file).unwrap();
    } else {
        fs2::FileExt::lock_exclusive(&file).unwrap();
    }
    println!("LOCK_READY");
    std::io::stdout().flush().unwrap();
    let mut release = [0];
    std::io::stdin().read_exact(&mut release).unwrap();
}
struct Holder {
    child: Child,
    _output: BufReader<std::process::ChildStdout>,
}
impl Holder {
    fn new(path: &Path, mode: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "lock_fixture_process", "--nocapture"])
            .env("SCRIPTA_LOCK_FIXTURE_PATH", path)
            .env("SCRIPTA_LOCK_FIXTURE_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert!(
                output.read_line(&mut line).unwrap() > 0,
                "lock fixture exited before ready"
            );
            if line.trim() == "LOCK_READY" {
                break;
            }
        }
        Self {
            child,
            _output: output,
        }
    }
}
impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn shared_exclusive_contention_timeouts_and_release_use_fixture_processes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    fs::write(&path, "safe").unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    let holder = Holder::new(&path, "shared");
    run(&mut rt,"$shared=File::lock('lock','shared'); $exclusive=File::lock('lock','exclusive'); $mode=$shared->mode;").unwrap();
    assert!(matches!(
        rt.get_global("shared"),
        Some(Value::NativeObject(_))
    ));
    assert_eq!(string(&rt, "mode"), "shared");
    assert_eq!(rt.get_global("exclusive"), Some(&Value::Null));
    let start = Instant::now();
    failure(
        &mut rt,
        "File::lock('lock','exclusive',30);",
        "TimeoutError",
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    drop(holder);
    let mut holder = Holder::new(&path, "exclusive");
    run(&mut rt, "$busy=File::lock('lock','shared');").unwrap();
    assert_eq!(rt.get_global("busy"), Some(&Value::Null));
    let mut release = holder.child.stdin.take().unwrap();
    let releasing = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        release.write_all(b"x").unwrap();
    });
    run(&mut rt,"$exclusive=File::lock('lock','exclusive',2000); File::unlock($exclusive); File::unlock($exclusive);").unwrap();
    releasing.join().unwrap();
    assert!(holder.child.wait().unwrap().success());
    assert_eq!(fs::read_to_string(path).unwrap(), "safe");
}
#[test]
fn cancellation_during_lock_wait_cleans_up_prior_temporary_resources() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("lock");
    fs::write(&path, "safe").unwrap();
    let holder = Holder::new(&path, "exclusive");
    let flag = Arc::new(AtomicBool::new(false));
    let mut rt = runtime(
        root.path(),
        RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        },
        FilesystemPolicy::default(),
    );
    let (start, ready) = std::sync::mpsc::channel();
    rt.register(
        "startCancellation",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Null),
            move |_, _| {
                start.send(()).unwrap();
                Ok(Value::Null)
            },
        ),
    )
    .unwrap();
    let cancelling = thread::spawn(move || {
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        thread::sleep(Duration::from_millis(50));
        flag.store(true, Ordering::Release);
    });
    let result = run(
        &mut rt,
        "$t=Temp::directory('.'); $path=$t->path; startCancellation(); File::lock('lock','exclusive',2000);",
    );
    cancelling.join().unwrap();
    assert!(
        result
            .unwrap_err()
            .message
            .contains("execution interrupted")
    );
    assert!(closed(rt.get_global("t").unwrap()));
    assert!(!Path::new(&string(&rt, "path")).exists());
    drop(holder);
    let probe = fs::File::open(path).unwrap();
    fs2::FileExt::try_lock_exclusive(&probe).unwrap();
}

#[cfg(unix)]
#[test]
fn inaccessible_owned_contents_report_cleanup_failure_and_allow_retry() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let mut rt = runtime(
        root.path(),
        RuntimeOptions::default(),
        FilesystemPolicy::default(),
    );
    rt.register(
        "makeInaccessible",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Bool),
            |_, args| {
                let Value::String(path) = &args[0] else {
                    unreachable!()
                };
                let locked = Path::new(path).join("locked");
                fs::create_dir(&locked).unwrap();
                fs::write(locked.join("data"), "data").unwrap();
                fs::set_permissions(&locked, fs::Permissions::from_mode(0o0)).unwrap();
                Ok(Value::Bool(fs::read_dir(locked).is_err()))
            },
        ),
    )
    .unwrap();
    let result = run(
        &mut rt,
        "$t=Temp::directory('.'); $path=$t->path; $denied=makeInaccessible($path); Temp::close($t);",
    );
    if rt.get_global("denied") == Some(&Value::Bool(true)) {
        assert!(result.unwrap_err().message.starts_with("PermissionError:"));
        assert!(!closed(rt.get_global("t").unwrap()));
        fs::set_permissions(
            Path::new(&string(&rt, "path")).join("locked"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        run(&mut rt, "Temp::close($t);").unwrap();
    } else {
        result.unwrap();
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
