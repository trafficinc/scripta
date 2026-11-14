//! Maintainer-only packaging. None of these dependencies are linked into scripta.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TARGETS: [&str; 4] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
];
const MAX_BYTES: u64 = 128 * 1024 * 1024;
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fail(message: &str) -> Box<dyn std::error::Error> {
    message.to_owned().into()
}
fn required<'a>(args: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    args.get(key)
        .map(String::as_str)
        .ok_or_else(|| fail(&format!("missing {key}")))
}
fn main() {
    if let Err(error) = run() {
        eprintln!("release error: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let mut input = env::args().skip(1);
    let command = input
        .next()
        .ok_or("expected package, verify, smoke, checksums, verify-checksums, or version")?;
    let mut args = BTreeMap::new();
    while let Some(key) = input.next() {
        if !key.starts_with("--") {
            return Err(fail("options must use --name value"));
        }
        let value = input.next().ok_or("missing option value")?;
        if args.insert(key, value).is_some() {
            return Err(fail("duplicate option"));
        }
    }
    let allowed: &[&str] = match command.as_str() {
        "package" => &[
            "--binary",
            "--target",
            "--out-dir",
            "--epoch",
            "--tag",
            "--features",
        ],
        "verify" | "smoke" => &["--archive"],
        "checksums" | "verify-checksums" => &["--dir"],
        "version" => &[],
        _ => return Err(fail("unknown release command")),
    };
    if args.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(fail("unknown release option"));
    }
    match command.as_str() {
        "version" => println!("{VERSION}"),
        "package" => {
            let target = required(&args, "--target")?;
            if !TARGETS.contains(&target) {
                return Err(fail("unrecognized release target"));
            }
            if args
                .get("--tag")
                .is_some_and(|tag| tag != &format!("v{VERSION}"))
            {
                return Err(fail("tag must match workspace version"));
            }
            let epoch = args
                .get("--epoch")
                .cloned()
                .or_else(|| env::var("SOURCE_DATE_EPOCH").ok())
                .unwrap_or_else(|| "0".into())
                .parse::<u32>()?;
            let output = package(
                Path::new(required(&args, "--binary")?),
                target,
                Path::new(required(&args, "--out-dir")?),
                epoch,
                args.get("--features").map(String::as_str),
            )?;
            println!("{}", output.display());
        }
        "verify" => {
            let archive = Path::new(required(&args, "--archive")?);
            read_verified(archive)?;
            println!("verified {}", archive.display());
        }
        "smoke" => smoke(Path::new(required(&args, "--archive")?))?,
        "checksums" => checksums(Path::new(required(&args, "--dir")?), false)?,
        "verify-checksums" => checksums(Path::new(required(&args, "--dir")?), true)?,
        _ => unreachable!(),
    }
    Ok(())
}
fn collect(directory: &Path, prefix: &str, files: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "release filename must be UTF-8")?;
        // Local package installations are generated caches, not release inputs.
        if matches!(name.as_str(), ".scripta" | ".scripta-install.lock" | ".git") {
            continue;
        }
        let path = entry.path();
        let key = format!("{prefix}/{name}");
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(fail("symlinks are not allowed in release input"));
        }
        if kind.is_dir() {
            collect(&path, &key, files)?;
        } else if kind.is_file() {
            files.insert(key, fs::read(path)?);
        }
    }
    Ok(())
}
fn supplemental_notices(
    name: &str,
    version: &str,
    package: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<usize> {
    let directory = root()
        .join("release-notices")
        .join(format!("{name}-{version}"));
    let provenance: Value =
        serde_json::from_slice(&fs::read(directory.join("provenance.json")).map_err(|_| {
            fail(&format!(
                "no license text found for {name} {version}; review before release"
            ))
        })?)?;
    let vcs: Value = serde_json::from_slice(&fs::read(package.join(".cargo_vcs_info.json"))?)?;
    if provenance["name"] != name
        || provenance["version"] != version
        || provenance["revision"] != vcs["git"]["sha1"]
    {
        return Err(fail("notice provenance differs from locked dependency"));
    }
    let entries = provenance["files"]
        .as_object()
        .ok_or("notice file inventory missing")?;
    for (filename, entry) in entries {
        safe_path(filename)?;
        let bytes = fs::read(directory.join(filename))?;
        if entry["sha256"].as_str() != Some(digest(&bytes).as_str()) {
            return Err(fail("supplemental license file hash mismatch"));
        }
        files.insert(format!("third-party/{name}-{version}/{filename}"), bytes);
    }
    files.insert(
        format!("third-party/{name}-{version}/provenance.json"),
        fs::read(directory.join("provenance.json"))?,
    );
    Ok(entries.len())
}
fn notices(
    files: &mut BTreeMap<String, Vec<u8>>,
    target: &str,
    features: Option<&str>,
) -> Result<()> {
    let mut command = Command::new("cargo");
    command.current_dir(root()).args([
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--filter-platform",
        target,
    ]);
    if let Some(features) = features {
        command.args(["--features", &format!("scr-cli/{features}")]);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(fail(
            "cargo metadata failed while collecting dependency notices",
        ));
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let packages = metadata["packages"]
        .as_array()
        .ok_or("missing package metadata")?;
    let cli = packages
        .iter()
        .find(|p| p["name"] == "scr-cli")
        .ok_or("missing cli package")?["id"]
        .as_str()
        .ok_or("missing package id")?;
    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("missing dependency graph")?;
    let mut todo = vec![cli.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes
            .iter()
            .find(|n| n["id"] == id)
            .ok_or("missing dependency node")?;
        for dep in node["deps"].as_array().ok_or("missing dependencies")? {
            if dep["dep_kinds"]
                .as_array()
                .ok_or("missing dependency kinds")?
                .iter()
                .any(|k| k["kind"] != "dev")
            {
                todo.push(dep["pkg"].as_str().ok_or("missing dependency id")?.into());
            }
        }
    }
    let mut index = String::from(
        "Dependency notices for the scripta dependency graph (including build tools).\nWorkspace crates use the root MIT license.\n\n",
    );
    let mut selected: Vec<_> = packages
        .iter()
        .filter(|p| seen.contains(p["id"].as_str().unwrap_or("")) && p["source"].is_string())
        .collect();
    selected.sort_by_key(|p| {
        (
            p["name"].as_str().unwrap_or(""),
            p["version"].as_str().unwrap_or(""),
        )
    });
    for package in selected {
        let name = package["name"].as_str().ok_or("missing name")?;
        let version = package["version"].as_str().ok_or("missing version")?;
        let manifest = Path::new(
            package["manifest_path"]
                .as_str()
                .ok_or("missing manifest")?,
        );
        let directory = manifest.parent().ok_or("missing manifest directory")?;
        index.push_str(&format!(
            "{name} {version}: {}\n",
            package["license"].as_str().unwrap_or("see license files")
        ));
        let authors = package["authors"]
            .as_array()
            .ok_or("missing author metadata")?
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        index.push_str(&format!("  Authors (as declared upstream): {authors}\n"));
        let mut count = 0;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let filename = entry.file_name().to_string_lossy().into_owned();
            let lower = filename.to_ascii_lowercase();
            if entry.file_type()?.is_file()
                && ["license", "copying", "notice", "unlicense"]
                    .iter()
                    .any(|prefix| lower.starts_with(prefix))
            {
                files.insert(
                    format!("third-party/{name}-{version}/{filename}"),
                    fs::read(entry.path())?,
                );
                count += 1;
            }
        }
        if let Some(license) = package["license_file"].as_str() {
            let path = directory.join(license);
            files.insert(
                format!("third-party/{name}-{version}/DECLARED-LICENSE"),
                fs::read(path)?,
            );
            count += 1;
        }
        if count == 0 {
            count = supplemental_notices(name, version, directory, files)?;
        }
        if count == 0 {
            return Err(fail("empty supplemental notice inventory"));
        }
    }
    let sysroot = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()?;
    if !sysroot.status.success() {
        return Err(fail("cannot locate Rust library notices"));
    }
    let rustdocs = PathBuf::from(String::from_utf8(sysroot.stdout)?.trim()).join("share/doc/rust");
    files.insert(
        "third-party/rust/COPYRIGHT-library.html".into(),
        fs::read(rustdocs.join("COPYRIGHT-library.html"))?,
    );
    collect(
        &rustdocs.join("licenses"),
        "third-party/rust/licenses",
        files,
    )?;
    files.insert("THIRD_PARTY_NOTICES.txt".into(), index.into_bytes());
    Ok(())
}
fn package(
    binary: &Path,
    target: &str,
    out: &Path,
    epoch: u32,
    features: Option<&str>,
) -> Result<PathBuf> {
    if features.is_some_and(|features| features != "sqlite") {
        return Err(fail("supported optional release features: sqlite"));
    }
    let mut files = BTreeMap::new();
    let name = if target.contains("windows") {
        "scripta.exe"
    } else {
        "scripta"
    };
    let alias = if target.contains("windows") {
        "scr.exe"
    } else {
        "scr"
    };
    let binary_bytes = fs::read(binary)?;
    files.insert(alias.into(), binary_bytes.clone());
    files.insert(name.into(), binary_bytes);
    for name in ["LICENSE", "README.md"] {
        files.insert(name.into(), fs::read(root().join(name))?);
    }
    for directory in ["docs", "examples", "playground"] {
        collect(&root().join(directory), directory, &mut files)?;
    }
    notices(&mut files, target, features)?;
    let inventory: BTreeMap<_, _> = files
        .iter()
        .map(|(name, data)| (name.clone(), digest(data)))
        .collect();
    let extensions = if features == Some("sqlite") {
        vec!["sqlite"]
    } else {
        vec![]
    };
    let manifest = json!({"version":VERSION,"target":target,"binary":name,"compatibility_binary":alias,"binary_sha256":digest(&files[name]),"source_date_epoch":epoch,"extensions":extensions,"files":inventory,"validation":"see the per-target validation record; archive creation alone is not platform support"});
    files.insert(
        "release.json".into(),
        [serde_json::to_vec_pretty(&manifest)?, vec![b'\n']].concat(),
    );
    fs::create_dir_all(out)?;
    let base = format!("scripta-{VERSION}-{target}");
    let path = out.join(format!(
        "{base}.{}",
        if target.contains("windows") {
            "zip"
        } else {
            "tar.gz"
        }
    ));
    let bytes = archive(&files, &base, target.contains("windows"), epoch)?;
    // Use an atomic replacement so an interrupted package step cannot leave a valid-looking partial file.
    let mut temporary = tempfile::NamedTempFile::new_in(out)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    temporary.persist(&path).map_err(|e| e.error)?;
    fs::write(
        path.with_file_name(format!(
            "{}.sha256",
            path.file_name().unwrap().to_string_lossy()
        )),
        format!(
            "{}  {}\n",
            digest(&bytes),
            path.file_name().unwrap().to_string_lossy()
        ),
    )?;
    Ok(path)
}
fn archive(
    files: &BTreeMap<String, Vec<u8>>,
    base: &str,
    windows: bool,
    epoch: u32,
) -> Result<Vec<u8>> {
    if windows {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let timestamp = zip::DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0)
            .map_err(|_| "invalid fixed ZIP date")?;
        for (name, bytes) in files {
            let mode = if matches!(name.as_str(), "scripta.exe" | "scr.exe") {
                0o755
            } else {
                0o644
            };
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored)
                .last_modified_time(timestamp)
                .unix_permissions(mode);
            zip.start_file(format!("{base}/{name}"), options)?;
            zip.write_all(bytes)?;
        }
        Ok(zip.finish()?.into_inner())
    } else {
        let gzip = flate2::GzBuilder::new()
            .mtime(epoch)
            .operating_system(255)
            .write(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        for (name, bytes) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(if matches!(name.as_str(), "scripta" | "scr") {
                0o755
            } else {
                0o644
            });
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(u64::from(epoch));
            header.set_cksum();
            tar.append_data(&mut header, format!("{base}/{name}"), Cursor::new(bytes))?;
        }
        let gzip = tar.into_inner()?;
        Ok(gzip.finish()?)
    }
}
fn safe_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.contains('\\')
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path.contains(':')
    {
        return Err(fail("unsafe archive path"));
    }
    Ok(())
}
fn read_archive(path: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    let mut add = |path: String, reader: &mut dyn Read| -> Result<()> {
        safe_path(&path)?;
        if files.len() >= 4096 {
            return Err(fail("too many archive entries"));
        }
        let mut bytes = Vec::new();
        reader
            .take(MAX_BYTES.saturating_sub(total) + 1)
            .read_to_end(&mut bytes)?;
        total += bytes.len() as u64;
        if total > MAX_BYTES {
            return Err(fail("archive exceeds verification budget"));
        }
        if files.insert(path, bytes).is_some() {
            return Err(fail("duplicate archive path"));
        }
        Ok(())
    };
    if path.extension().is_some_and(|e| e == "zip") {
        let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let name = entry.name().to_owned();
            if entry.is_dir() {
                return Err(fail("unexpected archive directory entry"));
            }
            add(name, &mut entry)?;
        }
    } else {
        let gzip = flate2::read::GzDecoder::new(fs::File::open(path)?);
        let mut archive = tar::Archive::new(gzip);
        for entry in archive.entries()? {
            let mut entry = entry?;
            if !entry.header().entry_type().is_file() {
                return Err(fail("non-regular archive entry"));
            }
            if matches!(
                entry.path()?.file_name().and_then(|n| n.to_str()),
                Some("scripta" | "scr")
            ) && entry.header().mode()? & 0o111 == 0
            {
                return Err(fail("Unix interpreter archive entry is not executable"));
            }
            let name = entry
                .path()?
                .to_str()
                .ok_or("archive path must be UTF-8")?
                .to_owned();
            add(name, &mut entry)?;
        }
    }
    Ok(files)
}
fn read_verified(path: &Path) -> Result<(String, Value, BTreeMap<String, Vec<u8>>)> {
    let files = read_archive(path)?;
    let key = files
        .keys()
        .find(|p| p.ends_with("/release.json"))
        .ok_or("release manifest missing")?;
    let prefix = key
        .strip_suffix("release.json")
        .ok_or("invalid manifest path")?
        .to_owned();
    let manifest: Value = serde_json::from_slice(&files[key])?;
    let version = manifest["version"].as_str().ok_or("version missing")?;
    let target = manifest["target"].as_str().ok_or("target missing")?;
    if !TARGETS.contains(&target) || prefix != format!("scripta-{version}-{target}/") {
        return Err(fail("invalid release identity"));
    }
    if let Some(extensions) = manifest.get("extensions") {
        if extensions != &json!([]) && extensions != &json!(["sqlite"]) {
            return Err(fail("invalid release extension list"));
        }
    }
    let inventory = manifest["files"].as_object().ok_or("inventory missing")?;
    if files.len() != inventory.len() + 1 {
        return Err(fail("release file inventory differs"));
    }
    for (name, hash) in inventory {
        safe_path(name)?;
        let bytes = files
            .get(&format!("{prefix}{name}"))
            .ok_or("release file missing")?;
        if hash.as_str() != Some(digest(bytes).as_str()) {
            return Err(fail("release file hash mismatch"));
        }
    }
    let binary = manifest["binary"].as_str().ok_or("binary missing")?;
    let (primary, alias) = if target.contains("windows") {
        ("scripta.exe", "scr.exe")
    } else {
        ("scripta", "scr")
    };
    if ![primary, alias].contains(&binary)
        || manifest["binary_sha256"]
            != *inventory
                .get(binary)
                .ok_or("binary missing from inventory")?
    {
        return Err(fail("binary identity differs"));
    }
    // Historical archives with the original binary name remain verifiable.
    if binary == primary
        && (manifest["compatibility_binary"] != alias
            || inventory.get(alias) != inventory.get(primary))
    {
        return Err(fail("compatibility executable differs"));
    }
    if binary == alias && manifest.get("compatibility_binary").is_some() {
        return Err(fail("invalid legacy compatibility metadata"));
    }
    Ok((prefix, manifest, files))
}
fn checksums(directory: &Path, verify: bool) -> Result<()> {
    let mut lines = String::new();
    let mut count = 0;
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "non-UTF-8 artifact")?;
        if name.ends_with(".tar.gz") || name.ends_with(".zip") {
            lines.push_str(&format!("{}  {name}\n", digest(&fs::read(entry.path())?)));
            count += 1;
        }
    }
    if count == 0 {
        return Err(fail("no release archives found"));
    }
    let path = directory.join("SHA256SUMS");
    if verify {
        if fs::read_to_string(path)? != lines {
            return Err(fail("SHA256SUMS differs from release archives"));
        }
    } else {
        fs::write(path, lines)?;
    }
    Ok(())
}
fn smoke(path: &Path) -> Result<()> {
    let (prefix, manifest, files) = read_verified(path)?;
    let target = manifest["target"].as_str().ok_or("missing target")?;
    let version = manifest["version"].as_str().ok_or("missing version")?;
    let temporary = tempfile::tempdir()?;
    for (name, bytes) in &files {
        let output = temporary.path().join(name);
        fs::create_dir_all(output.parent().ok_or("missing extraction parent")?)?;
        fs::write(&output, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &output,
                fs::Permissions::from_mode(
                    if [format!("{prefix}scripta"), format!("{prefix}scr")].contains(name) {
                        0o755
                    } else {
                        0o644
                    },
                ),
            )?;
        }
    }
    let directory = temporary.path().join(prefix.trim_end_matches('/'));
    let executable = directory.join(manifest["binary"].as_str().ok_or("missing binary")?);
    let execute_with = |executable: &Path, arguments: &[&str]| -> Result<std::process::Output> {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .current_dir(&directory)
            .env_clear()
            .env("PATH", directory.as_os_str())
            .env("HOME", temporary.path())
            .env("TEMP", temporary.path())
            .env("TMP", temporary.path());
        for name in ["SystemRoot", "WINDIR"] {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(fail(&format!(
                "packaged executable failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(output)
    };
    let legacy = manifest["binary"]
        .as_str()
        .is_some_and(|name| matches!(name, "scr" | "scr.exe"));
    let execute = |arguments: &[&str]| {
        if legacy {
            let arguments: Vec<_> = arguments
                .iter()
                .map(|arg| arg.replace(".scripta", ".scr"))
                .collect();
            let borrowed: Vec<_> = arguments.iter().map(String::as_str).collect();
            execute_with(&executable, &borrowed)
        } else {
            execute_with(&executable, arguments)
        }
    };
    let output = execute(&["version", "--verbose"])?;
    if let Some(alias) = manifest["compatibility_binary"].as_str() {
        let legacy = execute_with(&directory.join(alias), &["version", "--verbose"])?;
        if legacy.stdout != output.stdout {
            return Err(fail("compatibility command output differs"));
        }
    }
    let text = String::from_utf8(output.stdout)?;
    let command_name = if legacy { "scr" } else { "scripta" };
    if !text.starts_with(&format!("{command_name} {version}\n"))
        || !text.lines().any(|l| l == format!("target: {target}"))
    {
        return Err(fail("executable target/version differs from manifest"));
    }
    if let Some(extensions) = manifest["extensions"].as_array() {
        let expected = if extensions.is_empty() {
            "extensions: none"
        } else {
            "extensions: sqlite"
        };
        if !text.lines().any(|line| line == expected) {
            return Err(fail(
                "executable extensions differ from manifest; package with matching --features",
            ));
        }
        if !extensions.is_empty()
            && execute(&["run", "examples/sqlite.scripta"])?.stdout
                != "1: Ada\n2: 世界\n".as_bytes()
        {
            return Err(fail("packaged SQLite example output differs"));
        }
    }
    for (name, expected) in [
        ("hello.scripta", "Hello, Scripta!\n"),
        (
            "variables.scripta",
            "Checking Production...\nDisk usage: 85%\n",
        ),
        ("loops.scripta", "1\n2\n3\n"),
        ("functions.scripta", "Hello, Scripta!\n6\n"),
    ] {
        let name = format!("examples/{name}");
        let output = execute(&["run", &name])?;
        if output.stdout != expected.as_bytes() {
            return Err(fail("packaged example output differs"));
        }
    }
    // Exercise the shipped collection APIs from the extracted package, with
    // the same cleared environment and package-only PATH as the other smoke checks.
    execute(&["run", "playground/collections.scripta"])?;
    let offline = r#"<?scr require_once "examples/lib/helpers.scripta"; $data=Json::decode("{\"status\":\"offline\"}"); File::write("offline.json",Json::encode($data)); echo statusLabel(File::read("offline.json")) . "\n"; echo Date::format(Date::parse("2026-01-01T00:00:00Z"),"%Y") . "\n"; Log::info("local smoke complete");"#;
    let offline = if legacy {
        offline.replace(".scripta", ".scr")
    } else {
        offline.to_owned()
    };
    fs::write(
        directory.join(if legacy {
            "offline.scr"
        } else {
            "offline.scripta"
        }),
        offline,
    )?;
    let output = execute(&["run", "offline.scripta"])?;
    if output.stdout != b"Status: offline\n2026\n" {
        return Err(fail("offline smoke output differs"));
    }
    let validation = json!({"version":version,"target":target,"binary_sha256":manifest["binary_sha256"],"archive_sha256":digest(&fs::read(path)?),"host_os":env::consts::OS,"host_arch":env::consts::ARCH,"offline_smoke":"passed with cleared environment and PATH containing only extracted package, including the collection playground","workspace_tests":"must be checked in the associated native validation record; not inferred from smoke"});
    fs::write(
        path.with_file_name(format!(
            "{}.validation.json",
            path.file_name().unwrap().to_string_lossy()
        )),
        serde_json::to_vec_pretty(&validation)?,
    )?;
    println!("native offline package smoke passed: {target}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_output_is_deterministic_and_inventory_detects_corruption() -> Result<()> {
        for target in TARGETS {
            let binary = if target.contains("windows") {
                "scripta.exe"
            } else {
                "scripta"
            };
            let base = format!("scripta-{VERSION}-{target}");
            let alias = if target.contains("windows") {
                "scr.exe"
            } else {
                "scr"
            };
            let mut files = BTreeMap::from([
                (binary.to_owned(), b"fixture".to_vec()),
                (alias.to_owned(), b"fixture".to_vec()),
            ]);
            let manifest = json!({"version":VERSION,"target":target,"binary":binary,"compatibility_binary":alias,"binary_sha256":digest(b"fixture"),"files":{binary:digest(b"fixture"),alias:digest(b"fixture")}});
            files.insert("release.json".into(), serde_json::to_vec(&manifest)?);
            let first = archive(&files, &base, target.contains("windows"), 1234)?;
            assert_eq!(
                first,
                archive(&files, &base, target.contains("windows"), 1234)?
            );
            let dir = tempfile::tempdir()?;
            let path = dir.path().join(if target.contains("windows") {
                "a.zip"
            } else {
                "a.tar.gz"
            });
            fs::write(&path, first)?;
            read_verified(&path)?;
            for (extensions, valid) in [
                (json!([]), true),
                (json!(["sqlite"]), true),
                (json!(["unknown"]), false),
                (json!("sqlite"), false),
            ] {
                let mut declared = manifest.clone();
                declared["extensions"] = extensions;
                files.insert("release.json".into(), serde_json::to_vec(&declared)?);
                fs::write(
                    &path,
                    archive(&files, &base, target.contains("windows"), 1234)?,
                )?;
                assert_eq!(read_verified(&path).is_ok(), valid);
            }
            files.insert("release.json".into(), serde_json::to_vec(&manifest)?);
            files.insert(binary.into(), b"corrupted".to_vec());
            fs::write(
                &path,
                archive(&files, &base, target.contains("windows"), 1234)?,
            )?;
            assert!(read_verified(&path).is_err());
        }
        Ok(())
    }
    #[test]
    fn historical_archives_verify_and_compatibility_metadata_cannot_redirect_execution()
    -> Result<()> {
        let target = TARGETS[0];
        let base = format!("scripta-{VERSION}-{target}");
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("legacy.tar.gz");
        let mut manifest = json!({"version":VERSION,"target":target,"binary":"scr","binary_sha256":digest(b"fixture"),"files":{"scr":digest(b"fixture")}});
        let mut files = BTreeMap::from([("scr".to_owned(), b"fixture".to_vec())]);
        for (metadata, valid) in [
            (None, true),
            (Some("../outside"), false),
            (Some("scr"), false),
        ] {
            if let Some(name) = metadata {
                manifest["compatibility_binary"] = json!(name);
            }
            files.insert("release.json".into(), serde_json::to_vec(&manifest)?);
            fs::write(&path, archive(&files, &base, false, 0)?)?;
            assert_eq!(read_verified(&path).is_ok(), valid);
        }
        Ok(())
    }
    #[test]
    fn release_collection_excludes_local_package_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("main.scripta"), "<?scr")?;
        fs::create_dir(dir.path().join(".scripta"))?;
        fs::write(dir.path().join(".scripta/cache"), "generated")?;
        fs::write(dir.path().join(".scripta-install.lock"), "generated")?;
        let mut files = BTreeMap::new();
        collect(dir.path(), "examples", &mut files)?;
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["examples/main.scripta"]
        );
        Ok(())
    }
    #[test]
    fn rejects_unsafe_paths() {
        for name in ["../scripta", "/scripta", "C:/scripta", "foo\\scripta"] {
            assert!(safe_path(name).is_err());
        }
    }
    #[test]
    fn checksums_reject_changed_archives() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let archive = dir.path().join("fixture.zip");
        fs::write(&archive, b"abc")?;
        checksums(dir.path(), false)?;
        checksums(dir.path(), true)?;
        assert!(
            fs::read_to_string(dir.path().join("SHA256SUMS"))?
                .starts_with("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        fs::write(archive, b"changed")?;
        assert!(checksums(dir.path(), true).is_err());
        Ok(())
    }
}
