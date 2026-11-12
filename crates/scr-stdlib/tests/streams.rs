use scr_runtime::{ContextOptions, Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::FilesystemPolicy;
use std::{fs, io::Write, path::Path};

fn runtime(directory: &Path, limit: Option<usize>, policy: FilesystemPolicy) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        context: ContextOptions {
            working_directory: directory.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register_with_filesystem_policy(&mut runtime, policy).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, body: &str) -> Result<u8, RuntimeError> {
    runtime.execute_source(&format!("<?scr {body}"), &mut Vec::new())
}
fn global(runtime: &Runtime, name: &str, value: Value) {
    assert_eq!(runtime.get_global(name), Some(&value), "{name}");
}

#[test]
fn line_reading_preserves_unicode_and_handles_blank_lines_terminators_and_eof() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("input.txt"),
        "\u{feff}café🦀\r\n\n\r\n裸\rCR\nfinal\r",
    )
    .unwrap();
    fs::write(dir.path().join("empty.txt"), "").unwrap();
    let mut runtime = runtime(dir.path(), None, FilesystemPolicy::default());
    run(&mut runtime, r#"
        $stream = File::openLines('input.txt'); $alias = $stream; $lines = []; $eof = false; $stillOpen = false;
        try {
            $lines[] = File::readLine($stream);
            $lines[] = File::readLine($alias);
            while (($line = File::readLine($stream)) !== null) { $lines[] = $line; }
            $eof = File::readLine($stream); $stillOpen = !$stream->closed;
        } finally { File::close($stream); }
        $closed = $alias->closed; File::close($alias);
        $empty = File::openLines('empty.txt'); $emptyLine = File::readLine($empty); File::close($empty);
    "#).unwrap();
    global(
        &runtime,
        "lines",
        Value::List(
            ["\u{feff}café🦀", "", "", "裸\rCR", "final\r"]
                .map(|s| Value::String(s.into()))
                .into(),
        ),
    );
    global(&runtime, "eof", Value::Null);
    global(&runtime, "emptyLine", Value::Null);
    global(&runtime, "stillOpen", Value::Bool(true));
    global(&runtime, "closed", Value::Bool(true));
    let error = run(&mut runtime, "File::readLine($alias);").unwrap_err();
    assert!(error.message.starts_with("StreamError:"));
}

#[test]
fn line_limits_include_terminators_and_failures_close_shared_state() {
    let dir = tempfile::tempdir().unwrap();
    for (contents, limit, succeeds) in [
        (b"abc".as_slice(), 3, true),
        (b"abc\n".as_slice(), 4, true),
        (b"abc\r\n".as_slice(), 5, true),
        (b"abc\n".as_slice(), 3, false),
        (b"abc\r\n".as_slice(), 4, false),
        (b"abcd".as_slice(), 3, false),
    ] {
        fs::write(dir.path().join("input.txt"), contents).unwrap();
        let mut runtime = runtime(dir.path(), None, FilesystemPolicy::default());
        run(
            &mut runtime,
            &format!("$stream = File::openLines('input.txt', {limit}); $alias = $stream;"),
        )
        .unwrap();
        if succeeds {
            run(&mut runtime, "$line = File::readLine($stream);").unwrap();
            global(&runtime, "line", Value::String("abc".into()));
        } else {
            run(&mut runtime, "$line = 'unchanged'; $caught = false; try { $line = File::readLine($stream); } catch (ResourceLimitError $e) { $caught = true; } $closed = $alias->closed;").unwrap();
            global(&runtime, "caught", Value::Bool(true));
            global(&runtime, "closed", Value::Bool(true));
            global(&runtime, "line", Value::String("unchanged".into()));
        }
        run(&mut runtime, "File::close($alias); $reused = true;").unwrap();
    }
    fs::write(dir.path().join("large.txt"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
    let mut runtime = runtime(dir.path(), None, FilesystemPolicy::default());
    run(&mut runtime, "$stream = File::openLines('large.txt');").unwrap();
    let error = run(&mut runtime, "File::readLine($stream);").unwrap_err();
    assert!(error.message.contains("maxLineBytes"));
    run(&mut runtime, "$closed = $stream->closed;").unwrap();
    global(&runtime, "closed", Value::Bool(true));
}

#[test]
fn utf8_and_value_budget_failures_close_streams_and_allow_runtime_reuse() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("invalid.txt"), [b'x', 0xff, b'\n']).unwrap();
    let mut runtime = runtime(dir.path(), Some(256), FilesystemPolicy::default());
    run(&mut runtime, "$stream = File::openLines('invalid.txt'); $caught = false; try { File::readLine($stream); } catch (EncodingError $e) { $caught = true; } $closed = $stream->closed;").unwrap();
    global(&runtime, "caught", Value::Bool(true));
    global(&runtime, "closed", Value::Bool(true));
    fs::write(dir.path().join("large.txt"), vec![b'x'; 1024]).unwrap();
    run(&mut runtime, "$stream = File::openLines('large.txt', 2048); $caught = false; try { File::readLine($stream); } catch (ResourceLimitError $e) { $caught = true; } $closed = $stream->closed;").unwrap();
    global(&runtime, "caught", Value::Bool(true));
    global(&runtime, "closed", Value::Bool(true));
    fs::write(dir.path().join("valid.txt"), "ok\n").unwrap();
    run(&mut runtime, "$stream = File::openLines('valid.txt'); $line = ''; try { $line = File::readLine($stream); } finally { File::close($stream); }").unwrap();
    global(&runtime, "line", Value::String("ok".into()));
}

#[test]
fn large_files_stream_under_a_small_value_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut file = fs::File::create(dir.path().join("large.txt")).unwrap();
    let line = format!("{}\n", "x".repeat(63));
    for _ in 0..32768 {
        file.write_all(line.as_bytes()).unwrap();
    }
    drop(file);
    let mut runtime = runtime(dir.path(), Some(256), FilesystemPolicy::default());
    run(
        &mut runtime,
        r#"
        $stream = File::openLines('large.txt', 64); $count = 0; $total = 0;
        try {
            while (($line = File::readLine($stream)) !== null) {
                $count += 1; $total += len($line);
            }
        } finally { File::close($stream); }
        $closed = $stream->closed;
        $wholeFailed = false;
        try { File::read('large.txt'); } catch (ResourceLimitError $e) { $wholeFailed = true; }
    "#,
    )
    .unwrap();
    global(&runtime, "count", Value::Int(32768));
    global(&runtime, "total", Value::Int(32768 * 63));
    global(&runtime, "closed", Value::Bool(true));
    global(&runtime, "wholeFailed", Value::Bool(true));
}

#[test]
fn opening_obeys_capabilities_roots_and_regular_file_rules() {
    let dir = tempfile::tempdir().unwrap();
    let allowed = dir.path().join("allowed");
    fs::create_dir(&allowed).unwrap();
    fs::write(dir.path().join("outside.txt"), "outside").unwrap();
    fs::write(allowed.join("inside.txt"), "inside").unwrap();
    let mut runtime = runtime(
        dir.path(),
        None,
        FilesystemPolicy {
            roots: Some(vec![fs::canonicalize(&allowed).unwrap()]),
            ..Default::default()
        },
    );
    for (source, kind) in [
        ("File::openLines('outside.txt');", "PermissionError"),
        ("File::openLines('allowed');", "FilesystemError"),
        ("File::openLines('allowed/missing');", "FileNotFoundError"),
    ] {
        assert!(
            run(&mut runtime, source)
                .unwrap_err()
                .message
                .starts_with(kind)
        );
    }
    run(
        &mut runtime,
        "$stream = File::openLines('allowed/inside.txt'); File::close($stream);",
    )
    .unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(allowed.join("inside.txt"), allowed.join("link.txt")).unwrap();
        assert!(
            run(&mut runtime, "File::openLines('allowed/link.txt');")
                .unwrap_err()
                .message
                .starts_with("FilesystemError")
        );
    }
    let mut denied = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: dir.path().to_owned(),
            permissions: scr_runtime::PermissionConfig {
                allowed: Default::default(),
            },
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register(&mut denied).unwrap();
    assert!(
        run(&mut denied, "File::openLines('outside.txt');")
            .unwrap_err()
            .message
            .starts_with("PermissionError")
    );
    let mut denied = self::runtime(
        dir.path(),
        None,
        FilesystemPolicy {
            read: false,
            ..Default::default()
        },
    );
    assert!(
        run(&mut denied, "File::openLines('outside.txt');")
            .unwrap_err()
            .message
            .starts_with("PermissionError")
    );
}

#[test]
fn handles_keep_the_open_descriptor_and_cannot_cross_registrations() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("input.txt"), "first\nsecond\n").unwrap();
    let mut runtime = runtime(dir.path(), None, FilesystemPolicy::default());
    run(&mut runtime, "$stream = File::openLines('input.txt');").unwrap();
    fs::rename(dir.path().join("input.txt"), dir.path().join("renamed.txt")).unwrap();
    fs::write(dir.path().join("input.txt"), "replacement\n").unwrap();
    run(&mut runtime, "$line = File::readLine($stream);").unwrap();
    global(&runtime, "line", Value::String("first".into()));
    let mut other = self::runtime(dir.path(), None, FilesystemPolicy::default());
    other
        .set_global("stream", runtime.get_global("stream").unwrap().clone())
        .unwrap();
    assert!(
        run(&mut other, "File::readLine($stream);")
            .unwrap_err()
            .message
            .contains("another registration")
    );
    run(
        &mut runtime,
        "$line = File::readLine($stream); File::close($stream);",
    )
    .unwrap();
    global(&runtime, "line", Value::String("second".into()));
}

#[test]
fn stream_signatures_and_maxima_are_strict_and_catchable() {
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = runtime(dir.path(), None, FilesystemPolicy::default());
    for expression in [
        "File::openLines()",
        "File::openLines(1)",
        "File::openLines('x', 1.0)",
        "File::openLines('x', null)",
        "File::openLines('x', 1, 2)",
        "File::readLine(1)",
        "File::readLine()",
        "File::close(null)",
        "File::close()",
        "File::close(1, 2)",
    ] {
        run(&mut runtime, &format!("$caught = false; try {{ {expression}; }} catch (TypeError $e) {{ $caught = true; }}")).unwrap();
        global(&runtime, "caught", Value::Bool(true));
    }
    for max in [0, -1] {
        run(&mut runtime, &format!("$caught = false; try {{ File::openLines('x', {max}); }} catch (RangeError $e) {{ $caught = true; }}")).unwrap();
        global(&runtime, "caught", Value::Bool(true));
    }
}
