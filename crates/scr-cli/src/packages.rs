//! Project-local packages. Installation is explicit; execution never uses Git or the network.
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = "scripta.toml";
const LOCK: &str = "scripta.lock";
const STORE: &str = ".scripta/packages";
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<Package>,
    #[serde(default)]
    dependencies: BTreeMap<String, Dependency>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Package {
    name: String,
    version: String,
    #[serde(default = "default_entry")]
    entry: String,
}
fn default_entry() -> String {
    "main.scr".into()
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Dependency {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Lockfile {
    format: u32,
    #[serde(default)]
    packages: BTreeMap<String, Locked>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Locked {
    source: Dependency,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<String>,
    checksum: String,
    #[serde(default)]
    dependencies: BTreeMap<String, Locked>,
}

fn fail(message: impl Into<String>) -> Box<dyn std::error::Error> {
    io::Error::other(message.into()).into()
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join(MANIFEST);
    let manifest: Manifest = toml::from_str(&fs::read_to_string(&path)?)
        .map_err(|e| fail(format!("{}: {e}", path.display())))?;
    if let Some(package) = &manifest.package {
        if !valid_name(&package.name) {
            return Err(fail(
                "package.name must use lowercase letters, digits, '-' or '_' and start with a letter (max 64 characters)",
            ));
        }
        Version::parse(&package.version)?;
        let entry = Path::new(&package.entry);
        if entry.as_os_str().is_empty()
            || !entry
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(fail(
                "package.entry must be a relative file path without '..'",
            ));
        }
    }
    for (name, dep) in &manifest.dependencies {
        if !valid_name(name) {
            return Err(fail(format!("invalid dependency name '{name}'")));
        }
        match (&dep.path, &dep.git) {
            (Some(path), None) if !path.is_empty() && dep.rev.is_none() => {}
            (None, Some(url)) if !url.is_empty() && !url.starts_with('-') => {
                if dep.version.is_some() == dep.rev.is_some() {
                    return Err(fail(format!(
                        "{name}: Git dependencies need exactly one of version or rev"
                    )));
                }
            }
            _ => return Err(fail(format!("{name}: specify exactly one of path or git"))),
        }
        if let Some(version) = &dep.version {
            VersionReq::parse(version)?;
        }
        if dep
            .rev
            .as_ref()
            .is_some_and(|r| r.is_empty() || r.starts_with('-'))
        {
            return Err(fail(format!("{name}: invalid Git revision")));
        }
    }
    Ok(manifest)
}
fn project(start: &Path) -> Result<PathBuf> {
    let start = fs::canonicalize(start)?;
    start
        .ancestors()
        .find(|p| p.join(MANIFEST).is_file())
        .map(Path::to_owned)
        .ok_or_else(|| fail("no scripta.toml found; run 'scr init' first"))
}
fn read_lock(root: &Path) -> Result<Option<Lockfile>> {
    let text = match fs::read_to_string(root.join(LOCK)) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let lock: Lockfile = toml::from_str(&text)?;
    if lock.format != 1 {
        return Err(fail("unsupported scripta.lock format"));
    }
    Ok(Some(lock))
}
fn lock_matches(manifest: &Manifest, lock: &BTreeMap<String, Locked>) -> bool {
    manifest.dependencies.len() == lock.len()
        && manifest
            .dependencies
            .iter()
            .all(|(name, dep)| lock.get(name).is_some_and(|l| l.source == *dep))
}
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Package subcommands receive native OS arguments; manifest strings must be UTF-8.
pub fn execute(args: &[std::ffi::OsString], cwd: &Path) -> Result<String> {
    let args: Vec<&str> = args
        .iter()
        .map(|s| {
            s.to_str()
                .ok_or_else(|| fail("package arguments must be UTF-8"))
        })
        .collect::<Result<_>>()?;
    match args.as_slice() {
        ["init"] => {
            let manifest = Manifest::default();
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(cwd.join(MANIFEST))?;
            file.write_all(toml::to_string_pretty(&manifest)?.as_bytes())?;
            Ok("Created scripta.toml".into())
        }
        ["install", rest @ ..] | ["update", rest @ ..] => {
            let update = args[0] == "update";
            if rest.iter().any(|a| !matches!(*a, "--locked" | "--offline"))
                || update && rest.contains(&"--locked")
            {
                return Err(fail(
                    "usage: scr install [--locked] [--offline] | scr update [--offline]",
                ));
            }
            let root = project(cwd)?;
            let manifest = read_manifest(&root)?;
            install(
                &root,
                &manifest,
                update,
                rest.contains(&"--locked"),
                rest.contains(&"--offline"),
                None,
            )?;
            Ok("Installed dependencies".into())
        }
        ["add", name, rest @ ..] => {
            if !valid_name(name) {
                return Err(fail("invalid dependency name"));
            }
            let mut dep = Dependency {
                path: None,
                git: None,
                version: None,
                rev: None,
            };
            for pair in rest.chunks(2) {
                let [flag, value] = pair else {
                    return Err(fail("dependency options need values"));
                };
                let slot = match *flag {
                    "--path" => &mut dep.path,
                    "--git" => &mut dep.git,
                    "--version" => &mut dep.version,
                    "--rev" => &mut dep.rev,
                    _ => return Err(fail(format!("unknown dependency option {flag}"))),
                };
                if slot.replace((*value).into()).is_some() {
                    return Err(fail(format!("duplicate dependency option {flag}")));
                }
            }
            let root = project(cwd)?;
            // CLI paths are relative to the caller; manifest paths are relative to the project.
            if let Some(path) = &mut dep.path {
                *path = fs::canonicalize(cwd.join(&*path))?
                    .to_str()
                    .ok_or_else(|| fail("package paths must be UTF-8"))?
                    .into();
                if let Ok(relative) = Path::new(path).strip_prefix(&root) {
                    *path = relative.to_string_lossy().replace('\\', "/");
                }
            }
            if let Some(url) = &mut dep.git {
                if cwd.join(&*url).is_dir() {
                    *url = fs::canonicalize(cwd.join(&*url))?
                        .to_string_lossy()
                        .into_owned();
                }
            }
            let mut manifest = read_manifest(&root)?;
            manifest.dependencies.insert((*name).into(), dep);
            let text = toml::to_string_pretty(&manifest)?;
            // Validate before starting installation, using the same checks as file manifests.
            let validation = tempfile::tempdir()?;
            fs::write(validation.path().join(MANIFEST), &text)?;
            read_manifest(validation.path())?;
            install(&root, &manifest, false, false, false, Some(text))?;
            Ok(format!("Added {name} and installed dependencies"))
        }
        _ => Err(fail(
            "usage: scr init | scr add <name> (--path <dir> | --git <url> (--version <constraint> | --rev <ref>)) | scr install [--locked] [--offline] | scr update [--offline]",
        )),
    }
}

struct InstallGuard(PathBuf);
impl Drop for InstallGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
struct Installer {
    update: bool,
    offline: bool,
    active: HashSet<String>,
    count: usize,
}
fn install(
    root: &Path,
    manifest: &Manifest,
    update: bool,
    locked: bool,
    offline: bool,
    new_manifest: Option<String>,
) -> Result<()> {
    let guard_path = root.join(".scripta-install.lock");
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&guard_path)
        .map_err(|e| {
            fail(format!(
                "cannot acquire installation lock (another install may be running): {e}"
            ))
        })?;
    let _guard = InstallGuard(guard_path);
    let old_lock = read_lock(root)?;
    if locked
        && !old_lock
            .as_ref()
            .is_some_and(|l| lock_matches(manifest, &l.packages))
    {
        return Err(fail(
            "scripta.lock is missing or out of date; run 'scr install' without --locked",
        ));
    }
    let store_parent = root.join(".scripta");
    if fs::symlink_metadata(&store_parent).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(fail(".scripta must not be a symlink"));
    }
    fs::create_dir_all(&store_parent)?;
    let stage = tempfile::tempdir_in(&store_parent)?;
    let mut installer = Installer {
        update,
        offline,
        active: HashSet::new(),
        count: 0,
    };
    let packages = installer.dependencies(
        root,
        manifest,
        old_lock.as_ref().map(|l| &l.packages),
        &stage.path().join("packages"),
        &root.join(STORE),
    )?;
    let text = toml::to_string_pretty(&Lockfile {
        format: 1,
        packages,
    })?;
    // Stage everything before replacing the installed tree. Keep the previous tree
    // and metadata until both atomic file replacements have succeeded.
    let store = root.join(STORE);
    let backup = stage.path().join("previous");
    let had_store = store.exists();
    if had_store {
        fs::rename(&store, &backup)?;
    }
    if let Err(e) = fs::rename(stage.path().join("packages"), &store) {
        if had_store {
            fs::rename(&backup, &store)?;
        }
        return Err(e.into());
    }
    let old_lock_bytes = fs::read(root.join(LOCK)).ok();
    let commit = (|| -> Result<()> {
        write_atomic(&root.join(LOCK), text.as_bytes())?;
        if let Some(text) = &new_manifest {
            write_atomic(&root.join(MANIFEST), text.as_bytes())?;
        }
        Ok(())
    })();
    if commit.is_err() {
        fs::remove_dir_all(&store)?;
        if had_store {
            fs::rename(&backup, &store)?;
        }
        if let Some(bytes) = old_lock_bytes {
            write_atomic(&root.join(LOCK), &bytes)?;
        } else {
            let _ = fs::remove_file(root.join(LOCK));
        }
    }
    commit
}
impl Installer {
    fn dependencies(
        &mut self,
        source_root: &Path,
        manifest: &Manifest,
        locked: Option<&BTreeMap<String, Locked>>,
        dest: &Path,
        installed: &Path,
    ) -> Result<BTreeMap<String, Locked>> {
        fs::create_dir_all(dest)?;
        let mut result = BTreeMap::new();
        for (name, dep) in &manifest.dependencies {
            self.count += 1;
            if self.count > 256 || self.active.len() >= 32 {
                return Err(fail("dependency graph exceeds 256 packages or 32 levels"));
            }
            let pin = if self.update {
                None
            } else {
                locked
                    .and_then(|l| l.get(name))
                    .filter(|l| l.source == *dep)
            };
            let destination = dest.join(name);
            let old = installed.join(name);
            let temp = tempfile::tempdir()?;
            let (source, commit, identity) = if let Some(path) = &dep.path {
                let source = fs::canonicalize(source_root.join(path))?;
                let identity = format!("path:{}", source.display());
                (source, None, identity)
            } else if let Some(pin) =
                pin.filter(|p| old.is_dir() && checksum(&old).is_ok_and(|c| c == p.checksum))
            {
                let commit = pin
                    .commit
                    .clone()
                    .ok_or_else(|| fail("Git lock entry has no commit"))?;
                let identity = format!("git:{}@{commit}", dep.git.as_ref().unwrap());
                (old.clone(), Some(commit), identity)
            } else {
                if self.offline {
                    return Err(fail(format!(
                        "{name} is not installed with its locked checksum; offline installation cannot fetch Git packages"
                    )));
                }
                let repo = temp.path().join("repo");
                let url = dep.git.as_ref().unwrap();
                // Relative Git paths in hand-written manifests use the manifest directory.
                let url = if source_root.join(url).is_dir() {
                    fs::canonicalize(source_root.join(url))?
                        .to_string_lossy()
                        .into_owned()
                } else {
                    url.clone()
                };
                git(
                    None,
                    &[
                        "clone",
                        "--quiet",
                        "--no-checkout",
                        "--no-hardlinks",
                        "--",
                        &url,
                        repo.to_str()
                            .ok_or_else(|| fail("temporary path is not UTF-8"))?,
                    ],
                )?;
                let reference = if let Some(pin) = pin {
                    pin.commit
                        .clone()
                        .ok_or_else(|| fail("Git lock entry has no commit"))?
                } else if let Some(rev) = &dep.rev {
                    rev.clone()
                } else {
                    let req = VersionReq::parse(dep.version.as_ref().unwrap())?;
                    let tags = git(Some(&repo), &["tag", "--list"])?;
                    let mut candidates: Vec<_> = tags
                        .lines()
                        .filter_map(|tag| {
                            Version::parse(tag.strip_prefix('v').unwrap_or(tag))
                                .ok()
                                .filter(|v| req.matches(v))
                                .map(|v| (v, tag.to_owned()))
                        })
                        .collect();
                    candidates.sort();
                    candidates
                        .pop()
                        .ok_or_else(|| fail(format!("{name}: no Git version tag matches {req}")))?
                        .1
                };
                let commit = git(
                    Some(&repo),
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{reference}^{{commit}}"),
                    ],
                )?
                .trim()
                .to_owned();
                if !is_commit(&commit) {
                    return Err(fail("Git returned an invalid commit ID"));
                }
                git(
                    Some(&repo),
                    &[
                        "-c",
                        "advice.detachedHead=false",
                        "checkout",
                        "--quiet",
                        "--detach",
                        &commit,
                        "--",
                    ],
                )?;
                let identity = format!("git:{}@{commit}", dep.git.as_ref().unwrap());
                (repo, Some(commit), identity)
            };
            if !self.active.insert(identity.clone()) {
                return Err(fail(format!("circular package dependency at {name}")));
            }
            let package_manifest = read_manifest(&source)?;
            let package = package_manifest
                .package
                .as_ref()
                .ok_or_else(|| fail(format!("{name}: dependency needs a [package] section")))?;
            if package.name != *name {
                return Err(fail(format!(
                    "{name}: package declares name '{}'",
                    package.name
                )));
            }
            let version = Version::parse(&package.version)?;
            if let Some(req) = &dep.version {
                if !VersionReq::parse(req)?.matches(&version) {
                    return Err(fail(format!(
                        "{name}: package version {version} does not satisfy {req}"
                    )));
                }
            }
            let digest = checksum(&source)?;
            if let Some(pin) = pin {
                if digest != pin.checksum || package.version != pin.version || commit != pin.commit
                {
                    return Err(fail(format!(
                        "{name}: locked package content changed; use 'scr update' to accept new content"
                    )));
                }
                if !lock_matches(&package_manifest, &pin.dependencies) {
                    return Err(fail(format!("{name}: invalid transitive lock entries")));
                }
            }
            copy_tree(&source, &destination)?;
            if checksum(&destination)? != digest {
                return Err(fail(format!(
                    "{name}: source changed during installation; retry"
                )));
            }
            if !destination.join(&package.entry).is_file() {
                return Err(fail(format!(
                    "{name}: entry '{}' is missing",
                    package.entry
                )));
            }
            let children = self.dependencies(
                &source,
                &package_manifest,
                pin.map(|p| &p.dependencies),
                &destination.join(STORE),
                &old.join(STORE),
            )?;
            result.insert(
                name.clone(),
                Locked {
                    source: dep.clone(),
                    version: package.version.clone(),
                    commit,
                    checksum: digest,
                    dependencies: children,
                },
            );
            self.active.remove(&identity);
        }
        Ok(result)
    }
}
fn is_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn git(root: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    command.env("GIT_TERMINAL_PROMPT", "0");
    // Keep checkouts independent of machine line-ending defaults and prevent
    // hooks inherited from Git templates/configuration from running on install.
    let hooks = tempfile::tempdir()?;
    command
        .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf", "-c"])
        .arg(format!("core.hooksPath={}", hooks.path().display()));
    if let Some(root) = root {
        command.arg("-C").arg(root);
    }
    let output = command.args(args).output().map_err(|e| {
        fail(format!(
            "cannot run Git (install Git to use Git packages): {e}"
        ))
    })?;
    if !output.status.success() {
        return Err(fail(format!(
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)?)
}

// Deterministic content identity, independent of timestamps, file modes and host
// path separators. Links and special files are rejected rather than followed.
fn files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn visit(
        root: &Path,
        at: &Path,
        out: &mut Vec<(String, PathBuf)>,
        bytes: &mut u64,
    ) -> Result<()> {
        for entry in fs::read_dir(at)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_str().is_none_or(|s| s.contains('\\')) {
                return Err(fail("package filenames must be UTF-8 without backslashes"));
            }
            if matches!(
                name.to_str(),
                Some(".git" | ".scripta" | ".scripta-install.lock")
            ) {
                continue;
            }
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(fail(format!(
                    "package symlinks are unsupported: {}",
                    entry.path().display()
                )));
            }
            if kind.is_dir() {
                if entry.path().strip_prefix(root)?.components().count() > 64 {
                    return Err(fail("package directory depth exceeds 64 levels"));
                }
                visit(root, &entry.path(), out, bytes)?;
            } else if kind.is_file() {
                *bytes = bytes
                    .checked_add(entry.metadata()?.len())
                    .ok_or_else(|| fail("package size overflow"))?;
                if *bytes > 64 * 1024 * 1024 || out.len() >= 10_000 {
                    return Err(fail("package exceeds 64 MiB or 10000 files"));
                }
                let path = entry.path();
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .ok_or_else(|| fail("package filenames must be UTF-8"))?
                    .replace('\\', "/");
                out.push((relative, path));
            } else {
                return Err(fail("package contains a special file"));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    visit(root, root, &mut out, &mut 0)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}
fn checksum(root: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    for (name, path) in files(root)? {
        let bytes = fs::read(path)?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for (name, path) in files(source)? {
        let target = dest.join(name);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::copy(path, target)?;
    }
    Ok(())
}

/// Resolve a bare package name from the nearest manifest. Direct dependencies
/// are private to that manifest, preventing accidental access to a parent's deps.
pub fn resolve(base: &Path, name: &str) -> Result<Option<PathBuf>> {
    if !valid_name(name) {
        return Ok(None);
    }
    let root = match project(base) {
        Ok(root) => root,
        Err(_) => return Ok(None),
    };
    let manifest = read_manifest(&root)?;
    if !manifest.dependencies.contains_key(name) {
        return Ok(None);
    }
    let packages = installed_dependencies(&root, 0)?;
    let pin = &packages[name];
    let package_root = root.join(STORE).join(name);
    if checksum(&package_root)
        .map_err(|_| fail(format!("{name} is not installed; run 'scr install'")))?
        != pin.checksum
    {
        return Err(fail(format!(
            "{name}: installed checksum mismatch; run 'scr install'"
        )));
    }
    let package_manifest = read_manifest(&package_root)?;
    let package = package_manifest
        .package
        .ok_or_else(|| fail("installed package has no [package] section"))?;
    let entry = fs::canonicalize(package_root.join(package.entry))?;
    if !entry.starts_with(fs::canonicalize(&package_root)?) {
        return Err(fail("package entry escapes package directory"));
    }
    Ok(Some(entry))
}

// Walk the private installation layout back to the project's authoritative
// lockfile. Package-supplied lockfiles never override the root dependency graph.
fn installed_dependencies(root: &Path, depth: usize) -> Result<BTreeMap<String, Locked>> {
    if depth > 32 {
        return Err(fail("installed dependency depth exceeded"));
    }
    let packages = if let Some(store) = root
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "packages"))
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == ".scripta"))
    {
        let parent = store
            .parent()
            .ok_or_else(|| fail("invalid package layout"))?;
        let parent_dependencies = installed_dependencies(parent, depth + 1)?;
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| fail("invalid installed package name"))?;
        let pin = parent_dependencies
            .get(name)
            .ok_or_else(|| fail("package is absent from scripta.lock"))?;
        if checksum(root)? != pin.checksum {
            return Err(fail("installed package checksum mismatch"));
        }
        pin.dependencies.clone()
    } else {
        read_lock(root)?
            .ok_or_else(|| fail("scripta.lock is missing; run 'scr install'"))?
            .packages
    };
    if !lock_matches(&read_manifest(root)?, &packages) {
        return Err(fail("dependencies changed; run 'scr install'"));
    }
    Ok(packages)
}
