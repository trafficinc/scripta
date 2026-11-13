use scr_runtime::{Runtime, RuntimeError, RuntimeOptions, Value};
use scr_stdlib::{FilesystemPolicy, StdlibOptions};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    thread,
};

fn runtime(limit: Option<usize>) -> Runtime {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: limit,
        ..Default::default()
    });
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) -> Result<(), RuntimeError> {
    runtime
        .execute_source(&format!("<?scr {source}"), &mut Vec::new())
        .map(|_| ())
}
fn value(source: &str) -> Value {
    let mut runtime = runtime(None);
    run(&mut runtime, &format!("$result = {source};")).unwrap();
    runtime.get_global("result").unwrap().clone()
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = run(runtime, source).unwrap_err();
    assert!(
        error.message.starts_with(&format!("{kind}:")),
        "{source}: {error}"
    );
}
fn path(runtime: &mut Runtime, value: &Path) {
    runtime
        .set_global("path", Value::String(value.to_str().unwrap().into()))
        .unwrap();
}
fn bytes(data: &[u8]) -> Value {
    Value::Bytes(data.into())
}

#[test]
fn bytes_roundtrip_slicing_equality_and_annotations() {
    let mut runtime = runtime(None);
    run(
        &mut runtime,
        r#"
        function identity($data: bytes): bytes { return $data; }
        $data = identity(Bytes::encodeUtf8("é👍🏽"));
        $text = Bytes::decodeUtf8($data);
        $kind = type($data);
        $length = len($data);
        $slice = Bytes::slice($data, 0, 2);
        $same = $data === Bytes::encodeUtf8($text);
        $empty = Bytes::slice($data, 10, 0);
    "#,
    )
    .unwrap();
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("é👍🏽".into()))
    );
    assert_eq!(
        runtime.get_global("kind"),
        Some(&Value::String("bytes".into()))
    );
    assert_eq!(runtime.get_global("length"), Some(&Value::Int(10)));
    assert_eq!(runtime.get_global("slice"), Some(&bytes("é".as_bytes())));
    assert_eq!(runtime.get_global("same"), Some(&Value::Bool(true)));
    assert_eq!(runtime.get_global("empty"), Some(&bytes(b"")));
    error(
        &mut runtime,
        "Bytes::decodeUtf8(Bytes::slice($data, 0, 1));",
        "EncodingError",
    );
    for source in [
        "Bytes::slice($data, -1);",
        "Bytes::slice($data, 11);",
        "Bytes::slice($data, 9, 2);",
    ] {
        error(&mut runtime, source, "RangeError");
    }
    error(&mut runtime, "Json::encode($data);", "JsonError");
    error(&mut runtime, "str($data);", "TypeError");
    error(&mut runtime, "Bytes::decodeUtf8('text');", "TypeError");
}

#[test]
fn bytes_are_measured_in_nested_values_and_at_host_boundaries() {
    let slot = std::mem::size_of::<Value>();
    let mut runtime = runtime(Some(2 * slot + 3));
    runtime
        .set_global("ok", Value::List(vec![bytes(b"abc")]))
        .unwrap();
    assert!(
        runtime
            .set_global("large", Value::List(vec![bytes(b"abcd")]))
            .is_err()
    );
    assert!(
        runtime
            .set_global("large", bytes(&vec![0; 2 * slot]))
            .is_err()
    );
}

#[test]
fn binary_files_preserve_every_byte_and_reuse_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("archive.bin");
    let all: Vec<u8> = (0..=255).collect();
    let mut runtime = runtime(None);
    path(&mut runtime, &file);
    runtime.set_global("bytes", bytes(&all)).unwrap();
    run(&mut runtime, "File::writeBytes($path, $bytes); File::appendBytes($path, $bytes); $result = File::readBytes($path);").unwrap();
    assert_eq!(runtime.get_global("result"), Some(&bytes(&all.repeat(2))));
    error(&mut runtime, "File::read($path);", "EncodingError");
    error(
        &mut runtime,
        "File::writeBytes($path, 'text');",
        "TypeError",
    );
    let mut denied = Runtime::new();
    scr_stdlib::register_with_options(
        &mut denied,
        StdlibOptions {
            filesystem: FilesystemPolicy {
                roots: Some(vec![]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    path(&mut denied, &file);
    error(&mut denied, "File::readBytes($path);", "PermissionError");
    error(
        &mut denied,
        "File::writeBytes($path, Bytes::encodeUtf8('x'));",
        "PermissionError",
    );
    assert_eq!(std::fs::read(file).unwrap(), all.repeat(2));
}

#[test]
fn regex_is_small_unicode_aware_and_uses_literal_replacements() {
    assert_eq!(
        value(r#"Regex::isMatch("ERROR", "(?i)error")"#),
        Value::Bool(true)
    );
    assert_eq!(
        value(r#"Regex::findAll("café 12 東京", "\\p{L}+")"#),
        Value::List(vec![
            Value::String("café".into()),
            Value::String("東京".into())
        ])
    );
    assert_eq!(
        value(r#"Regex::replace("a1b22", "[0-9]+", "$1")"#),
        Value::String("a$1b$1".into())
    );
    assert_eq!(
        value(r#"Regex::replace("é", "", "-")"#),
        Value::String("-é-".into())
    );
    assert_eq!(
        value(r#"Regex::findAll("é", "")"#),
        Value::List(vec![Value::String("".into()), Value::String("".into())])
    );
    assert_eq!(value(r#"Regex::findAll("a", "z")"#), Value::List(vec![]));
    let mut runtime = runtime(None);
    error(&mut runtime, "Regex::isMatch('x', '[');", "RegexError");
    error(
        &mut runtime,
        r#"Regex::isMatch("x", "(?=x)");"#,
        "RegexError",
    );
    runtime
        .set_global("pattern", Value::String("a".repeat(65537)))
        .unwrap();
    error(&mut runtime, "Regex::isMatch('x', $pattern);", "RegexError");
}

#[test]
fn graphemes_keep_skin_tones_flags_combining_accents_and_zwj_sequences_together() {
    let mut runtime = runtime(None);
    runtime
        .set_global("text", Value::String("👍🏽🇺🇸e\u{301}👨‍👩‍👧‍👦".into()))
        .unwrap();
    run(&mut runtime, "$length = String::graphemeLength($text); $result = String::graphemeSubstring($text, 1, 2); $end = String::graphemeSubstring($text, 4); $zero = String::graphemeSubstring($text, 0, 0);").unwrap();
    assert_eq!(runtime.get_global("length"), Some(&Value::Int(4)));
    assert_eq!(
        runtime.get_global("result"),
        Some(&Value::String("🇺🇸e\u{301}".into()))
    );
    assert_eq!(runtime.get_global("end"), Some(&Value::String("".into())));
    assert_eq!(runtime.get_global("zero"), Some(&Value::String("".into())));
    assert_eq!(value("String::graphemeLength('')"), Value::Int(0));
    for expression in [
        "String::graphemeSubstring($text, -1)",
        "String::graphemeSubstring($text, 5)",
        "String::graphemeSubstring($text, 3, 2)",
    ] {
        error(&mut runtime, &format!("{expression};"), "RangeError");
    }
    assert_eq!(value("len('👍🏽')"), Value::Int(2));
}

#[test]
fn csv_roundtrips_multiline_quotes_empty_fields_and_custom_delimiters() {
    let mut runtime = runtime(None);
    let rows = Value::List(vec![
        Value::List(vec![
            Value::String("name".into()),
            Value::String("note".into()),
            Value::String("empty".into()),
        ]),
        Value::List(vec![
            Value::String("é,東京".into()),
            Value::String("a\r\nb \"quoted\"".into()),
            Value::String("".into()),
        ]),
    ]);
    runtime.set_global("rows", rows.clone()).unwrap();
    run(&mut runtime, r#"$encoded = Csv::encode($rows); $result = Csv::parse($encoded); $custom = Csv::parse(Csv::encode($rows, ["delimiter" => ";"]), ["delimiter" => ";"]);"#).unwrap();
    assert_eq!(runtime.get_global("result"), Some(&rows));
    assert_eq!(runtime.get_global("custom"), Some(&rows));
    assert_eq!(value("Csv::parse('')"), Value::List(vec![]));
    assert_eq!(
        value(r#"Csv::parse("\r\n\n")"#),
        Value::List(vec![Value::List(vec![Value::String("".into())]); 2])
    );
    assert_eq!(
        value("Csv::parse(Csv::encode([['']]))"),
        Value::List(vec![Value::List(vec![Value::String("".into())])])
    );
    assert_eq!(value("Csv::encode([])"), Value::String("".into()));
}

#[test]
fn csv_rejects_malformed_records_inconsistent_widths_and_bad_options() {
    let mut runtime = runtime(None);
    for text in ["\"unterminated", "a\"b,c", "\"a\"junk,b", "a,b\nc"] {
        runtime
            .set_global("text", Value::String(text.into()))
            .unwrap();
        error(&mut runtime, "Csv::parse($text);", "CsvError");
    }
    for source in [
        "Csv::encode([[1]]);",
        "Csv::parse('a', ['delimiter' => 'é']);",
        r#"Csv::parse('a', ['delimiter' => '"']);"#,
        "Csv::parse('a', ['maxRecordBytes' => 0]);",
        "Csv::parse('a', ['unknown' => true]);",
    ] {
        error(&mut runtime, source, "TypeError");
    }

    error(&mut runtime, "Csv::encode([[]]);", "CsvError");
    error(&mut runtime, "Csv::encode([['a'], ['b','c']]);", "CsvError");
    error(
        &mut runtime,
        "Csv::parse('abc', ['maxRecordBytes' => 2]);",
        "ResourceLimitError",
    );
    error(
        &mut runtime,
        "Csv::encode([['abc']], ['maxRecordBytes' => 3]);",
        "ResourceLimitError",
    );
}

#[test]
fn csv_streams_multiline_records_close_aliases_and_close_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data.csv");
    std::fs::write(&file, "a,b\r\n\"first\nsecond\",x\r\nlast,y").unwrap();
    let mut runtime = runtime(None);
    path(&mut runtime, &file);
    run(&mut runtime, "$r = Csv::open($path); $alias = $r; $first = Csv::readRow($r); $second = Csv::readRow($r); $third = Csv::readRow($r); $eof = Csv::readRow($r); Csv::close($r); Csv::close($r); $closed = $alias->closed;").unwrap();
    assert_eq!(
        runtime.get_global("second"),
        Some(&Value::List(vec![
            Value::String("first\nsecond".into()),
            Value::String("x".into())
        ]))
    );
    assert_eq!(runtime.get_global("eof"), Some(&Value::Null));
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    error(&mut runtime, "Csv::readRow($alias);", "StreamError");
    std::fs::write(&file, "\"bad").unwrap();
    run(&mut runtime, "$r = Csv::open($path);").unwrap();
    error(&mut runtime, "Csv::readRow($r);", "CsvError");
    run(&mut runtime, "$closed = $r->closed;").unwrap();
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    std::fs::write(&file, [0xff]).unwrap();
    run(&mut runtime, "$r = Csv::open($path);").unwrap();
    error(&mut runtime, "Csv::readRow($r);", "CsvError");
}

#[test]
fn regex_and_csv_preflight_expanding_output_budgets() {
    let slot = std::mem::size_of::<Value>();
    let mut encoder = runtime(Some(8 * slot));
    encoder
        .set_global(
            "rows",
            Value::List(vec![Value::List(vec![Value::String(
                "\"".repeat(5 * slot),
            )])]),
        )
        .unwrap();
    error(&mut encoder, "Csv::encode($rows);", "ResourceLimitError");
    let mut runtime = runtime(Some(4 * slot));
    run(&mut runtime, "$result = 'unchanged';").unwrap();
    for source in [
        "$result = Regex::findAll('abcdefghijklmnop', '');",
        "$result = Regex::replace('aaaaaaaaaaaaaaaa', 'a', 'xxxxxxxxxxxxxxxx');",
        "$result = Csv::parse('a,b,c,d,e,f,g,h');",
    ] {
        error(&mut runtime, source, "ResourceLimitError");
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::String("unchanged".into()))
        );
    }
}

// One-shot local HTTP fixture that records an arbitrary binary request body.
fn server(
    status: &str,
    body: Vec<u8>,
    declared: Option<usize>,
) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let status = status.to_owned();
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..n]);
            if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nX-Test: yes\r\n\r\n",
            declared.unwrap_or(body.len())
        );
        let _ = socket.write_all(response.as_bytes());
        let _ = socket.write_all(&body);
        request
    });
    (url, worker)
}
fn url(runtime: &mut Runtime, url: String) {
    runtime.set_global("url", Value::String(url)).unwrap();
}

#[test]
fn http_binary_responses_and_uploads_preserve_raw_bytes() {
    let all: Vec<u8> = (0..=255).collect();
    let (address, worker) = server("200 OK", all.clone(), None);
    let mut runtime = runtime(None);
    url(&mut runtime, address);
    runtime.set_global("bytes", bytes(&all)).unwrap();
    run(&mut runtime, r#"$r = Http::post($url, $bytes, ["responseType" => "bytes"]); $body = $r->body; $result = $r->bytes();"#).unwrap();
    assert_eq!(runtime.get_global("result"), Some(&bytes(&all)));
    assert_eq!(runtime.get_global("body"), Some(&bytes(&all)));
    error(&mut runtime, "$r->text();", "EncodingError");
    error(&mut runtime, "$r->json();", "EncodingError");
    let request = worker.join().unwrap();
    assert!(request.ends_with(&all));
    let (address, worker) = server("200 OK", b"hello".to_vec(), None);
    url(&mut runtime, address);
    run(&mut runtime, "$r = Http::request('PUT', $url, ['body' => $bytes]); $text = $r->text(); $result = $r->bytes();").unwrap();
    assert_eq!(
        runtime.get_global("text"),
        Some(&Value::String("hello".into()))
    );
    assert_eq!(runtime.get_global("result"), Some(&bytes(b"hello")));
    assert!(worker.join().unwrap().ends_with(&all));
}

#[test]
fn downloads_stream_publish_metadata_and_refuse_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("archive.zip");
    let body: Vec<u8> = (0..=255).cycle().take(1024 * 1024).collect();
    let (address, worker) = server("200 OK", body.clone(), None);
    // A 1 MiB download succeeds with a value budget far smaller than the file.
    let mut runtime = runtime(Some(4096));
    path(&mut runtime, &file);
    url(&mut runtime, address);
    run(&mut runtime, "$result = Http::download($url, $path); $count = $result['bytesWritten']; $status = $result['status'];").unwrap();
    worker.join().unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), body);
    assert_eq!(runtime.get_global("count"), Some(&Value::Int(1024 * 1024)));
    assert_eq!(runtime.get_global("status"), Some(&Value::Int(200)));
    // This must fail before trying the now-closed server.
    error(
        &mut runtime,
        "Http::download($url, $path);",
        "AlreadyExistsError",
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn download_failures_leave_no_destination_or_temporary_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("archive.zip");
    let mut runtime = runtime(None);
    path(&mut runtime, &file);
    for (status, body, declared, options, kind) in [
        (
            "404 Not Found",
            b"missing".to_vec(),
            None,
            "[]",
            "HttpStatusError",
        ),
        (
            "200 OK",
            b"large".to_vec(),
            None,
            "['maxBodyBytes' => 2]",
            "ResponseLimitError",
        ),
        (
            "200 OK",
            b"partial".to_vec(),
            Some(20),
            "[]",
            "ConnectionError",
        ),
    ] {
        let (address, worker) = server(status, body, declared);
        url(&mut runtime, address);
        error(
            &mut runtime,
            &format!("Http::download($url, $path, {options});"),
            kind,
        );
        worker.join().unwrap();
        assert!(!file.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn binary_http_reuses_limits_default_decoding_and_option_validation() {
    let mut runtime = runtime(None);
    for (body, options, kind) in [
        (vec![0xff], "[]", "EncodingError"),
        (
            vec![0xff; 8],
            "['responseType' => 'bytes', 'maxBodyBytes' => 4]",
            "ResponseLimitError",
        ),
    ] {
        let (address, worker) = server("200 OK", body, None);
        url(&mut runtime, address);
        error(&mut runtime, &format!("Http::get($url, {options});"), kind);
        worker.join().unwrap();
    }
    url(&mut runtime, "http://127.0.0.1:1".into());
    error(
        &mut runtime,
        "Http::get($url, ['responseType' => 'invalid']);",
        "TypeError",
    );
    error(
        &mut runtime,
        "Http::post($url, Bytes::encodeUtf8('x'), ['body' => Bytes::encodeUtf8('y')]);",
        "TypeError",
    );
}

#[test]
fn csv_and_download_require_filesystem_permissions_before_io() {
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.write");
    let mut runtime = Runtime::with_options(options);
    scr_stdlib::register(&mut runtime).unwrap();
    error(&mut runtime, "Csv::open('missing.csv');", "PermissionError");
    error(
        &mut runtime,
        "Http::download('http://127.0.0.1:1', 'missing.zip');",
        "PermissionError",
    );
}

#[test]
fn csv_handles_initial_bom_without_stripping_bom_inside_fields() {
    let mut runtime = runtime(None);
    for text in [
        "\u{feff}\"first\"\n\u{feff}second\n",
        "\u{feff}first\r\u{feff}second",
    ] {
        runtime
            .set_global("text", Value::String(text.into()))
            .unwrap();
        run(&mut runtime, "$result = Csv::parse($text);").unwrap();
        assert_eq!(
            runtime.get_global("result"),
            Some(&Value::List(vec![
                Value::List(vec![Value::String("first".into())]),
                Value::List(vec![Value::String("\u{feff}second".into())]),
            ]))
        );
    }
    let rows = Value::List(vec![Value::List(vec![Value::String(
        "\u{feff}field".into(),
    )])]);
    runtime.set_global("rows", rows.clone()).unwrap();
    run(&mut runtime, "$result = Csv::parse(Csv::encode($rows));").unwrap();
    assert_eq!(runtime.get_global("result"), Some(&rows));
    // Quoting protects a first field's BOM from being interpreted as a document BOM.
    runtime
        .set_global("text", Value::String("\"\u{feff}field\"".into()))
        .unwrap();
    run(&mut runtime, "$result = Csv::parse($text);").unwrap();
    assert_eq!(runtime.get_global("result"), Some(&rows));
}

#[test]
fn csv_exact_record_limits_stream_large_files_under_small_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("large.csv");
    std::fs::write(&file, "\"a\nb\",c\r\n".repeat(10000)).unwrap();
    let mut runtime = runtime(Some(2048));
    path(&mut runtime, &file);
    run(&mut runtime, "$r = Csv::open($path, ['maxRecordBytes' => 10]); $count = 0; while (Csv::readRow($r) !== null) { $count = $count + 1; } Csv::close($r);").unwrap();
    assert_eq!(runtime.get_global("count"), Some(&Value::Int(10000)));
    run(
        &mut runtime,
        "$r = Csv::open($path, ['maxRecordBytes' => 8]);",
    )
    .unwrap();
    error(&mut runtime, "Csv::readRow($r);", "ResourceLimitError");
    run(&mut runtime, "$closed = $r->closed;").unwrap();
    assert_eq!(runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert_eq!(
        value("Csv::parse('abc', ['maxRecordBytes' => 3])"),
        Value::List(vec![Value::List(vec![Value::String("abc".into())])])
    );
}

#[test]
fn binary_file_reads_enforce_exact_payload_budgets() {
    let slot = std::mem::size_of::<Value>();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("data.bin");
    let mut runtime = runtime(Some(slot + 256));
    path(&mut runtime, &file);
    std::fs::write(&file, vec![0xff; 256]).unwrap();
    run(&mut runtime, "$data = File::readBytes($path);").unwrap();
    assert_eq!(runtime.get_global("data"), Some(&bytes(&vec![0xff; 256])));
    std::fs::write(&file, vec![0xff; 257]).unwrap();
    error(
        &mut runtime,
        "$data = File::readBytes($path);",
        "ResourceLimitError",
    );
    assert_eq!(runtime.get_global("data"), Some(&bytes(&vec![0xff; 256])));
}

#[test]
fn binary_http_responses_obey_value_budgets_before_returning() {
    let mut runtime = runtime(Some(2048));
    let (address, worker) = server("200 OK", vec![0xff; 4096], None);
    url(&mut runtime, address);
    error(
        &mut runtime,
        "Http::get($url, ['responseType' => 'bytes']);",
        "ResourceLimitError",
    );
    worker.join().unwrap();
}

#[test]
fn downloads_enforce_host_roots_and_never_follow_destination_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::new();
    scr_stdlib::register_with_options(
        &mut runtime,
        StdlibOptions {
            filesystem: FilesystemPolicy {
                roots: Some(vec![dir.path().to_owned()]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    path(&mut runtime, &outside.path().join("new.zip"));
    error(
        &mut runtime,
        "Http::download('http://127.0.0.1:1', $path);",
        "PermissionError",
    );
    #[cfg(unix)]
    {
        let target = dir.path().join("target");
        std::fs::write(&target, "original").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        path(&mut runtime, &link);
        error(
            &mut runtime,
            "Http::download('http://127.0.0.1:1', $path);",
            "AlreadyExistsError",
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
    }
}

#[test]
fn download_timeouts_and_cancellation_remove_staged_files() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };
    for cancel in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("download.zip");
        let flag = Arc::new(AtomicBool::new(false));
        let mut runtime = Runtime::with_options(RuntimeOptions {
            interrupt_flag: Some(flag.clone()),
            ..Default::default()
        });
        scr_stdlib::register(&mut runtime).unwrap();
        path(&mut runtime, &file);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        url(
            &mut runtime,
            format!("http://{}/file", listener.local_addr().unwrap()),
        );
        let folder = dir.path().to_owned();
        let (done, wait) = mpsc::channel::<()>();
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n")
                .unwrap();
            if cancel {
                let deadline = Instant::now() + Duration::from_secs(3);
                while std::fs::read_dir(&folder).unwrap().next().is_none() {
                    assert!(
                        Instant::now() < deadline,
                        "download did not create staging file"
                    );
                    thread::sleep(Duration::from_millis(1));
                }
                flag.store(true, Ordering::Relaxed);
                let _ = socket.write_all(b"0123456789");
            } else {
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }
        });
        let result = run(
            &mut runtime,
            if cancel {
                "Http::download($url, $path, ['timeoutMs' => 3000]);"
            } else {
                "Http::download($url, $path, ['timeoutMs' => 100]);"
            },
        )
        .unwrap_err();
        if cancel {
            assert!(result.message.contains("interrupted"), "{result}");
        } else {
            assert!(result.message.starts_with("TimeoutError:"), "{result}");
        }
        let _ = done.send(());
        worker.join().unwrap();
        assert!(!file.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}

#[test]
fn csv_readers_cannot_cross_registrations() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("records.csv");
    std::fs::write(&file, "one,two").unwrap();
    let mut first = runtime(None);
    let mut second = runtime(None);
    path(&mut first, &file);
    run(&mut first, "$reader = Csv::open($path);").unwrap();
    second
        .set_global("reader", first.get_global("reader").unwrap().clone())
        .unwrap();
    error(&mut second, "Csv::readRow($reader);", "TypeError");
    run(
        &mut first,
        "$row = Csv::readRow($reader); Csv::close($reader);",
    )
    .unwrap();
    assert_eq!(
        first.get_global("row"),
        Some(&Value::List(vec![
            Value::String("one".into()),
            Value::String("two".into())
        ]))
    );
}
