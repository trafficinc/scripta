use scr_runtime::{ContextOptions, MapKey, Runtime, RuntimeOptions, Value};
use scr_stdlib::FilesystemPolicy;
use std::{
    fs,
    io::{Cursor, Write},
    path::Path,
};
fn runtime(root: &Path, policy: FilesystemPolicy) -> Runtime {
    let mut rt = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            working_directory: fs::canonicalize(root).unwrap(),
            ..Default::default()
        },
        ..Default::default()
    });
    scr_stdlib::register_with_filesystem_policy(&mut rt, policy).unwrap();
    rt
}
fn run(rt: &mut Runtime, source: &str) {
    rt.execute_source(source, &mut Vec::new()).unwrap();
}
fn error(rt: &mut Runtime, source: &str, kind: &str) {
    let e = rt.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(e.message.starts_with(&format!("{kind}:")), "{source}: {e}");
}
fn binary(data: &[u8]) -> Value {
    Value::Bytes(data.into())
}
fn entry(path: &str, data: Option<&[u8]>) -> Value {
    Value::Map(vec![
        (MapKey::String("path".into()), Value::String(path.into())),
        (
            MapKey::String("data".into()),
            data.map_or(Value::Null, binary),
        ),
    ])
}
fn bytes(rt: &Runtime, key: &str) -> Vec<u8> {
    let Some(Value::Bytes(value)) = rt.get_global(key) else {
        panic!("expected bytes ${key}")
    };
    value.to_vec()
}
fn zip_fixture(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in entries {
        if let Some(data) = data {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        } else {
            writer.add_directory(*name, options).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}
fn tar_fixture(name: &[u8], kind: tar::EntryType, data: &[u8]) -> Vec<u8> {
    let mut header = tar::Header::new_ustar();
    header.as_mut_bytes()[..name.len()].copy_from_slice(name);
    header.set_entry_type(kind);
    header.set_size(data.len() as u64);
    header.set_mode(0o777);
    header.set_cksum();
    let mut writer = tar::Builder::new(Vec::new());
    writer.append(&header, data).unwrap();
    writer.into_inner().unwrap()
}
fn central(data: &[u8]) -> usize {
    data.windows(4).position(|b| b == b"PK\x01\x02").unwrap()
}
fn get16(data: &[u8], offset: usize) -> usize {
    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as usize
}
#[test]
fn zip_tar_roundtrips_preserve_binary_empty_files_directories_order_and_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    let entries = Value::List(vec![
        entry("z/世界.bin", Some(&[0, 255, 1, 0])),
        entry("empty", None),
        entry("z", None),
        entry("blank", Some(&[])),
    ]);
    rt.set_global("entries", entries.clone()).unwrap();
    for format in ["Zip", "Tar"] {
        run(
            &mut rt,
            &format!(
                "$archive={format}::encode($entries); $again={format}::encode($entries); $decoded={format}::decode($archive); {format}::extract($archive,'out-{format}');"
            ),
        );
        assert_eq!(rt.get_global("decoded"), Some(&entries));
        assert_eq!(bytes(&rt, "archive"), bytes(&rt, "again"));
        let root = dir.path().join(format!("out-{format}"));
        assert_eq!(fs::read(root.join("z/世界.bin")).unwrap(), [0, 255, 1, 0]);
        assert!(root.join("empty").is_dir());
        assert_eq!(fs::read(root.join("blank")).unwrap(), b"");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(root.join("blank"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    run(
        &mut rt,
        "$stored=Zip::encode($entries,['compression'=>'stored']); $decoded=Zip::decode($stored);",
    );
    assert_eq!(rt.get_global("decoded"), Some(&entries));
}
#[test]
fn empty_archives_and_gzip_binary_concatenation_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    for format in ["Zip", "Tar"] {
        run(
            &mut rt,
            &format!("$empty={format}::decode({format}::encode([]));"),
        );
        assert_eq!(rt.get_global("empty"), Some(&Value::List(vec![])));
    }
    rt.set_global("data", binary(&[0, 255, 0, 195, 169]))
        .unwrap();
    run(
        &mut rt,
        "$compressed=Gzip::compress($data); $again=Gzip::compress($data); $decoded=Gzip::decompress($compressed); $empty=Gzip::decompress(Gzip::compress(Bytes::encodeUtf8('')),['maxOutputBytes'=>0]);",
    );
    assert_eq!(bytes(&rt, "decoded"), [0, 255, 0, 195, 169]);
    assert_eq!(bytes(&rt, "compressed"), bytes(&rt, "again"));
    assert!(bytes(&rt, "empty").is_empty());
    let mut concat = bytes(&rt, "compressed");
    concat.extend_from_slice(&bytes(&rt, "again"));
    rt.set_global("concat", binary(&concat)).unwrap();
    run(
        &mut rt,
        "$decoded=Gzip::decompress($concat,['maxMembers'=>2]);",
    );
    assert_eq!(
        bytes(&rt, "decoded"),
        [0, 255, 0, 195, 169, 0, 255, 0, 195, 169]
    );
    error(
        &mut rt,
        "Gzip::decompress($concat,['maxMembers'=>1]);",
        "ResourceLimitError",
    );
}
#[test]
fn output_and_input_budgets_count_exact_bytes_and_one_beyond() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("entries", Value::List(vec![entry("a", Some(b"abc"))]))
        .unwrap();
    rt.set_global("data", binary(b"abc")).unwrap();
    for format in ["Zip", "Tar"] {
        run(
            &mut rt,
            &format!("$encoded={format}::encode($entries,['maxInputBytes'=>3]);"),
        );
        let size = bytes(&rt, "encoded").len();
        run(
            &mut rt,
            &format!(
                "{format}::encode($entries,['maxOutputBytes'=>{size}]); {format}::decode($encoded,['maxInputBytes'=>{size},'maxOutputBytes'=>3]);"
            ),
        );
        error(
            &mut rt,
            &format!(
                "{format}::encode($entries,['maxOutputBytes'=>{}]);",
                size - 1
            ),
            "ResourceLimitError",
        );
        error(
            &mut rt,
            &format!("{format}::encode($entries,['maxInputBytes'=>2]);"),
            "ResourceLimitError",
        );
        error(
            &mut rt,
            &format!(
                "{format}::decode($encoded,['maxInputBytes'=>{}]);",
                size - 1
            ),
            "ResourceLimitError",
        );
        error(
            &mut rt,
            &format!("{format}::decode($encoded,['maxOutputBytes'=>2]);"),
            "ResourceLimitError",
        );
    }
    run(
        &mut rt,
        "$encoded=Gzip::compress($data,['maxInputBytes'=>3]);",
    );
    let size = bytes(&rt, "encoded").len();
    run(
        &mut rt,
        &format!(
            "Gzip::compress($data,['maxOutputBytes'=>{size}]); Gzip::decompress($encoded,['maxInputBytes'=>{size},'maxOutputBytes'=>3]);"
        ),
    );
    for source in [
        format!("Gzip::compress($data,['maxOutputBytes'=>{}]);", size - 1),
        "Gzip::compress($data,['maxInputBytes'=>2]);".into(),
        format!(
            "Gzip::decompress($encoded,['maxInputBytes'=>{}]);",
            size - 1
        ),
        "Gzip::decompress($encoded,['maxOutputBytes'=>2]);".into(),
    ] {
        error(&mut rt, &source, "ResourceLimitError");
    }
}
#[test]
fn counts_names_depth_and_options_are_strict() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("entries", Value::List(vec![entry("a/b", Some(b"x"))]))
        .unwrap();
    for format in ["Zip", "Tar"] {
        run(
            &mut rt,
            &format!(
                "$encoded={format}::encode($entries,['maxEntries'=>2,'maxNameBytes'=>3,'maxDepth'=>2]); {format}::decode($encoded,['maxEntries'=>2,'maxNameBytes'=>3,'maxDepth'=>2]);"
            ),
        );
        for operation in [
            format!("{format}::encode($entries,"),
            format!("{format}::decode($encoded,"),
        ] {
            for option in [
                "['maxEntries'=>1]",
                "['maxNameBytes'=>2]",
                "['maxDepth'=>1]",
            ] {
                error(
                    &mut rt,
                    &format!("{operation}{option});"),
                    "ResourceLimitError",
                );
            }
        }
    }
    for (options, kind) in [
        ("['unknown'=>1]", "TypeError"),
        ("['maxOutputBytes'=>true]", "TypeError"),
        ("['maxOutputBytes'=>-1]", "RangeError"),
        ("['maxOutputBytes'=>67108865]", "RangeError"),
        ("['maxEntries'=>10001]", "RangeError"),
        ("['maxDepth'=>129]", "RangeError"),
        ("['maxNameBytes'=>4097]", "RangeError"),
        ("['compression'=>'gzip']", "TypeError"),
    ] {
        error(&mut rt, &format!("Zip::encode($entries,{options});"), kind);
    }
    for source in [
        "Zip::encode([1]);",
        "Zip::encode([['path'=>'a','data'=>'text']]);",
        "Tar::encode([['path'=>'a','data'=>null,'extra'=>1]]);",
        "Gzip::compress('text');",
        "Gzip::compress($encoded,['maxMembers'=>1]);",
        "Tar::encode($entries,['compression'=>'stored']);",
        "Zip::decode($encoded,['compression'=>'stored']);",
    ] {
        error(&mut rt, source, "TypeError");
    }
}
#[test]
fn unsafe_paths_are_rejected_for_creation_decode_and_before_extracting() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    for path in [
        "../escape",
        "a/../escape",
        "/absolute",
        "C:/drive",
        "//server/share",
        r"a\escape",
        "a//b",
        "./a",
        "a/./b",
        "a/",
        "NUL.txt",
        "CON .txt",
        "CONIN$",
        "COM1",
        "LPT².txt",
        "a:stream",
        "trailing.",
        "trailing ",
        "bad\0x",
        "bad\nx",
    ] {
        rt.set_global("entries", Value::List(vec![entry(path, Some(b"data"))]))
            .unwrap();
        for format in ["Zip", "Tar"] {
            error(
                &mut rt,
                &format!("{format}::encode($entries);"),
                "ArchiveError",
            );
        }
        // Trailing slash is a directory marker in ZIP, not a regular-file path.
        if path.ends_with('/') {
            continue;
        }
        rt.set_global("zip", binary(&zip_fixture(&[(path, Some(b"data"))])))
            .unwrap();
        rt.set_global(
            "tar",
            binary(&tar_fixture(
                path.as_bytes(),
                tar::EntryType::Regular,
                b"data",
            )),
        )
        .unwrap();
        for (format, key) in [("Zip", "zip"), ("Tar", "tar")] {
            error(
                &mut rt,
                &format!("{format}::decode(${key});"),
                "ArchiveError",
            );
            error(
                &mut rt,
                &format!("{format}::extract(${key},'output');"),
                "ArchiveError",
            );
            assert!(!dir.path().join("output").exists());
        }
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn duplicates_case_aliases_file_ancestors_and_special_entries_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    for paths in [
        vec![("a", Some(b"x".as_slice())), ("A", Some(b"y".as_slice()))],
        vec![("a", Some(b"x".as_slice())), ("a/b", Some(b"y".as_slice()))],
        vec![("a", Some(b"x".as_slice())), ("a", None)],
    ] {
        rt.set_global("zip", binary(&zip_fixture(&paths))).unwrap();
        error(&mut rt, "Zip::decode($zip);", "ArchiveError");
    }
    let mut duplicate = zip_fixture(&[("a", Some(b"x")), ("b", Some(b"y"))]);
    let first = central(&duplicate);
    let second = first
        + 46
        + get16(&duplicate, first + 28)
        + get16(&duplicate, first + 30)
        + get16(&duplicate, first + 32);
    let local =
        u32::from_le_bytes(duplicate[second + 42..second + 46].try_into().unwrap()) as usize;
    duplicate[second + 46] = b'a';
    duplicate[local + 30] = b'a';
    rt.set_global("bad", binary(&duplicate)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
    for mode in [0o120777u32, 0o010644, 0o060600] {
        let mut special = zip_fixture(&[("link", Some(b"../../outside"))]);
        let offset = central(&special);
        special[offset + 5] = 3;
        special[offset + 38..offset + 42].copy_from_slice(&(mode << 16).to_le_bytes());
        rt.set_global("bad", binary(&special)).unwrap();
        error(&mut rt, "Zip::extract($bad,'out');", "ArchiveError");
        assert!(!dir.path().join("out").exists());
    }
    for kind in [
        tar::EntryType::Symlink,
        tar::EntryType::Link,
        tar::EntryType::Fifo,
        tar::EntryType::GNUSparse,
        tar::EntryType::XHeader,
        tar::EntryType::GNULongName,
    ] {
        rt.set_global("bad", binary(&tar_fixture(b"link", kind, b"")))
            .unwrap();
        error(&mut rt, "Tar::extract($bad,'out');", "ArchiveError");
        assert!(!dir.path().join("out").exists());
    }
}
#[test]
fn malformed_truncated_checksums_and_trailing_junk_preserve_assignments() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    run(&mut rt, "$result=42;");
    let zip = zip_fixture(&[("a", Some(b"abc"))]);
    let tar = tar_fixture(b"a", tar::EntryType::Regular, b"abc");
    for (format, data) in [("Zip", zip.as_slice()), ("Tar", tar.as_slice())] {
        for length in [0, 1, data.len() / 2, data.len() - 1] {
            rt.set_global("bad", binary(&data[..length])).unwrap();
            error(
                &mut rt,
                &format!("$result={format}::decode($bad);"),
                "ArchiveError",
            );
            assert_eq!(rt.get_global("result"), Some(&Value::Int(42)));
        }
    }
    let mut corrupt = zip.clone();
    let start = 30 + get16(&corrupt, 26) + get16(&corrupt, 28);
    corrupt[start] ^= 1;
    rt.set_global("bad", binary(&corrupt)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
    let mut corrupt = tar.clone();
    corrupt[0] ^= 1;
    rt.set_global("bad", binary(&corrupt)).unwrap();
    error(&mut rt, "Tar::decode($bad);", "ArchiveError");
    let mut junk = tar;
    junk.extend_from_slice(&[1; 512]);
    rt.set_global("bad", binary(&junk)).unwrap();
    error(&mut rt, "Tar::decode($bad);", "ArchiveError");
    rt.set_global("data", binary(b"payload")).unwrap();
    run(&mut rt, "$gzip=Gzip::compress($data);");
    let gzip = bytes(&rt, "gzip");
    for length in [0, 1, 10, gzip.len() - 1] {
        rt.set_global("bad", binary(&gzip[..length])).unwrap();
        error(&mut rt, "Gzip::decompress($bad);", "ArchiveError");
    }
    let mut corrupt = gzip.clone();
    let size = corrupt.len();
    corrupt[size - 8] ^= 1;
    rt.set_global("bad", binary(&corrupt)).unwrap();
    error(&mut rt, "Gzip::decompress($bad);", "ArchiveError");
    let mut trailing = gzip;
    trailing.extend_from_slice(b"junk");
    rt.set_global("bad", binary(&trailing)).unwrap();
    error(&mut rt, "Gzip::decompress($bad);", "ArchiveError");
}
#[test]
fn extraction_refuses_existing_destinations_and_obeys_capabilities_and_roots() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("existing")).unwrap();
    fs::write(dir.path().join("existing/keep"), "safe").unwrap();
    fs::write(dir.path().join("file"), "safe").unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("data", binary(&zip_fixture(&[("a", Some(b"new"))])))
        .unwrap();
    for destination in ["existing", "file"] {
        error(
            &mut rt,
            &format!("Zip::extract($data,'{destination}');"),
            "AlreadyExistsError",
        );
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("existing/keep")).unwrap(),
        "safe"
    );
    assert_eq!(fs::read_to_string(dir.path().join("file")).unwrap(), "safe");
    error(
        &mut rt,
        "Zip::extract($data,'missing/out');",
        "FileNotFoundError",
    );
    let mut denied = runtime(
        dir.path(),
        FilesystemPolicy {
            write: false,
            ..Default::default()
        },
    );
    denied
        .set_global("data", binary(&zip_fixture(&[])))
        .unwrap();
    error(
        &mut denied,
        "Zip::extract($data,'denied');",
        "PermissionError",
    );
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut pure = Runtime::with_options(options);
    scr_stdlib::register(&mut pure).unwrap();
    run(
        &mut pure,
        "$archive=Zip::encode([]); Zip::decode($archive); Gzip::decompress(Gzip::compress(Bytes::encodeUtf8('')));",
    );
    error(
        &mut pure,
        "Zip::extract($archive,'out');",
        "PermissionError",
    );
    fs::create_dir(dir.path().join("allowed")).unwrap();
    let mut restricted = runtime(
        dir.path(),
        FilesystemPolicy {
            roots: Some(vec![dir.path().join("allowed")]),
            ..Default::default()
        },
    );
    restricted
        .set_global("data", binary(&zip_fixture(&[])))
        .unwrap();
    error(
        &mut restricted,
        "Zip::extract($data,'outside');",
        "PermissionError",
    );
    assert!(!dir.path().join("outside").exists());
}
#[cfg(unix)]
#[test]
fn existing_symlink_roots_and_parents_are_not_followed() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("keep"), "safe").unwrap();
    symlink(outside.path(), dir.path().join("alias")).unwrap();
    symlink("missing", dir.path().join("dangling")).unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("data", binary(&zip_fixture(&[("a", Some(b"x"))])))
        .unwrap();
    error(
        &mut rt,
        "Zip::extract($data,'alias');",
        "AlreadyExistsError",
    );
    error(
        &mut rt,
        "Zip::extract($data,'dangling');",
        "AlreadyExistsError",
    );
    error(
        &mut rt,
        "Zip::extract($data,'alias/out');",
        "FilesystemError",
    );
    assert!(!outside.path().join("out").exists());
    assert_eq!(
        fs::read_to_string(outside.path().join("keep")).unwrap(),
        "safe"
    );
}

#[test]
fn expansion_budgets_stop_compressed_bombs_and_reject_false_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("data", binary(&vec![0; 256 * 1024])).unwrap();
    rt.set_global(
        "entries",
        Value::List(vec![entry("a", Some(&vec![0; 256 * 1024]))]),
    )
    .unwrap();
    run(
        &mut rt,
        "$gzip=Gzip::compress($data); $zip=Zip::encode($entries);",
    );
    assert!(bytes(&rt, "gzip").len() < 1024);
    assert!(bytes(&rt, "zip").len() < 1024);
    run(
        &mut rt,
        "$round=Zip::decode($zip); $plain=Gzip::decompress($gzip);",
    );
    assert_eq!(rt.get_global("round"), rt.get_global("entries"));
    assert_eq!(bytes(&rt, "plain"), bytes(&rt, "data"));
    error(
        &mut rt,
        "Gzip::decompress($gzip,['maxOutputBytes'=>1024]);",
        "ResourceLimitError",
    );
    error(
        &mut rt,
        "Zip::extract($zip,'out',['maxOutputBytes'=>1024]);",
        "ResourceLimitError",
    );
    assert!(!dir.path().join("out").exists());
    let mut zip = bytes(&rt, "zip");
    let offset = central(&zip);
    zip[22..26].copy_from_slice(&10u32.to_le_bytes());
    zip[offset + 24..offset + 28].copy_from_slice(&10u32.to_le_bytes());
    rt.set_global("bad", binary(&zip)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
}
#[test]
fn gzip_metadata_headers_are_bounded_and_ignored_as_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global("data", binary(b"x")).unwrap();
    run(&mut rt, "$gzip=Gzip::compress($data);");
    let gzip = bytes(&rt, "gzip");
    for (size, works) in [(65536, true), (65537, false)] {
        let mut member = gzip[..10].to_vec();
        member[3] |= 8;
        member.extend(std::iter::repeat_n(b'a', size - 11));
        member.push(0);
        member.extend_from_slice(&gzip[10..]);
        rt.set_global("member", binary(&member)).unwrap();
        if works {
            run(&mut rt, "$plain=Gzip::decompress($member);");
            assert_eq!(bytes(&rt, "plain"), b"x");
        } else {
            error(&mut rt, "Gzip::decompress($member);", "ResourceLimitError");
        }
    }
    let mut member = gzip[..10].to_vec();
    member[3] |= 8;
    member.extend_from_slice(b"../../outside\0");
    member.extend_from_slice(&gzip[10..]);
    rt.set_global("member", binary(&member)).unwrap();
    run(&mut rt, "$plain=Gzip::decompress($member);");
    assert_eq!(bytes(&rt, "plain"), b"x");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn zip_rejects_unsupported_catalogs_and_inconsistent_local_headers() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    let valid = zip_fixture(&[("a", Some(b"abc"))]);
    let offset = central(&valid);
    for (local, central, value) in [
        (4, offset + 6, 45u16),
        (6, offset + 8, 1),
        (6, offset + 8, 8),
        (8, offset + 10, 99),
    ] {
        let mut bad = valid.clone();
        bad[local..local + 2].copy_from_slice(&value.to_le_bytes());
        bad[central..central + 2].copy_from_slice(&value.to_le_bytes());
        rt.set_global("bad", binary(&bad)).unwrap();
        error(&mut rt, "Zip::decode($bad);", "ArchiveError");
    }
    let mut bad = valid.clone();
    bad[30] = b'b';
    rt.set_global("bad", binary(&bad)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
    let mut bad = valid.clone();
    let end = bad.len() - 22;
    bad[end + 8..end + 10].copy_from_slice(&10001u16.to_le_bytes());
    bad[end + 10..end + 12].copy_from_slice(&10001u16.to_le_bytes());
    rt.set_global("bad", binary(&bad)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ResourceLimitError");
    let mut bad = valid;
    let end = bad.len() - 22;
    bad[end + 8..end + 12].copy_from_slice(&[255; 4]);
    rt.set_global("bad", binary(&bad)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
}
#[test]
fn unicode_filesystem_aliases_fail_exclusive_creation_instead_of_merging() {
    let probe = tempfile::tempdir().unwrap();
    fs::create_dir(probe.path().join("é")).unwrap();
    let aliases = match fs::create_dir(probe.path().join("e\u{301}")) {
        Ok(()) => false,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => true,
        Err(e) => panic!("alias probe failed: {e}"),
    };
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    rt.set_global(
        "data",
        binary(&zip_fixture(&[
            ("é/a", Some(b"a")),
            ("e\u{301}/b", Some(b"b")),
        ])),
    )
    .unwrap();
    if aliases {
        error(&mut rt, "Zip::extract($data,'out');", "AlreadyExistsError");
        assert!(!dir.path().join("out").exists());
    } else {
        run(&mut rt, "Zip::extract($data,'out');");
        assert_eq!(fs::read(dir.path().join("out/é/a")).unwrap(), b"a");
        assert_eq!(fs::read(dir.path().join("out/e\u{301}/b")).unwrap(), b"b");
    }
}

#[test]
fn zip_ignores_alternative_footer_candidates_in_comments_and_requires_deflate_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(dir.path(), FilesystemPolicy::default());
    let mut archive = zip_fixture(&[("a", Some(b"abc"))]);
    let end = archive.len() - 22;
    let mut comment = vec![b'x'; 64];
    comment[..4].copy_from_slice(b"PK\x05\x06");
    comment[8..12].copy_from_slice(&[255; 4]);
    comment[16..20].copy_from_slice(&[255; 4]);
    archive[end + 20..end + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    archive.extend_from_slice(&comment);
    rt.set_global("archive", binary(&archive)).unwrap();
    run(&mut rt, "$rows=Zip::decode($archive,['maxEntries'=>1]);");
    assert_eq!(
        rt.get_global("rows"),
        Some(&Value::List(vec![entry("a", Some(b"abc"))]))
    );
    rt.set_global("entries", Value::List(vec![entry("a", Some(b"payload"))]))
        .unwrap();
    run(&mut rt, "$archive=Zip::encode($entries);");
    let mut trailing = bytes(&rt, "archive");
    let cd = central(&trailing);
    let size = u32::from_le_bytes(trailing[18..22].try_into().unwrap());
    trailing.insert(cd, 0);
    let new_cd = cd + 1;
    let trailer = trailing.len() - 22;
    trailing[18..22].copy_from_slice(&(size + 1).to_le_bytes());
    trailing[new_cd + 20..new_cd + 24].copy_from_slice(&(size + 1).to_le_bytes());
    trailing[trailer + 16..trailer + 20].copy_from_slice(&(new_cd as u32).to_le_bytes());
    rt.set_global("trailing", binary(&trailing)).unwrap();
    error(&mut rt, "Zip::decode($trailing);", "ArchiveError");
    let mut archive = bytes(&rt, "archive");
    let original = central(&archive);
    let compressed = u32::from_le_bytes(archive[18..22].try_into().unwrap());
    archive.remove(original - 1);
    let central = original - 1;
    let end = archive.len() - 22;
    archive[18..22].copy_from_slice(&(compressed - 1).to_le_bytes());
    archive[central + 20..central + 24].copy_from_slice(&(compressed - 1).to_le_bytes());
    archive[end + 16..end + 20].copy_from_slice(&(central as u32).to_le_bytes());
    rt.set_global("bad", binary(&archive)).unwrap();
    error(&mut rt, "Zip::decode($bad);", "ArchiveError");
}
