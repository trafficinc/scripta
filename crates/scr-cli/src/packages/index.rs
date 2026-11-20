//! Explicit package discovery. Resolved sources, rather than catalog references,
//! are written to manifests, so execution and locked installs need no index.
use super::*;
use std::io::Read;

const INDEX_FILE: &str = "scripta-index.toml";
const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    format: u32,
    packages: BTreeMap<String, Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    #[serde(default)]
    description: String,
    source: Dependency,
}

pub(super) struct Index {
    packages: BTreeMap<String, Entry>,
}

pub(super) fn load(
    cwd: &Path,
    project: Option<(&Path, &Manifest)>,
    override_source: Option<&str>,
    offline: bool,
) -> Result<Index> {
    let environment = std::env::var_os("SCR_PACKAGE_INDEX")
        .map(|v| {
            v.into_string()
                .map_err(|_| fail("SCR_PACKAGE_INDEX must be UTF-8"))
        })
        .transpose()?;
    let (source, base) = if let Some(source) = override_source.or(environment.as_deref()) {
        (source, cwd)
    } else if let Some((root, manifest)) = project {
        let registry = manifest.registry.as_ref().ok_or_else(|| {
            fail("no package index configured; set [registry].index, SCR_PACKAGE_INDEX, or --index")
        })?;
        (registry.index.as_str(), root)
    } else {
        return Err(fail(
            "no package index configured; set SCR_PACKAGE_INDEX or --index",
        ));
    };
    if source.is_empty() || source.starts_with('-') || source.contains('\0') {
        return Err(fail("invalid package index source"));
    }
    let local = base.join(source);
    let checkout;
    let (file, remote) = if local.is_file() {
        (fs::canonicalize(local)?, false)
    } else if local.join(INDEX_FILE).is_file() {
        (fs::canonicalize(local.join(INDEX_FILE))?, false)
    } else {
        if offline {
            return Err(fail("offline lookup requires a local package index file"));
        }
        if !local.is_dir() && !source.contains("://") && !source.contains('@') {
            return Err(fail("package index file or repository not found"));
        }
        checkout = tempfile::tempdir()?;
        let repo = checkout.path().join("index");
        let url = if local.is_dir() {
            local_git_path(&local)?
        } else {
            source.into()
        };
        git(
            None,
            &[
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--no-checkout",
                "--",
                &url,
                repo.to_str()
                    .ok_or_else(|| fail("temporary path is not UTF-8"))?,
            ],
        )?;
        git(
            Some(&repo),
            &["checkout", "--quiet", "--detach", "HEAD", "--"],
        )?;
        (repo.join(INDEX_FILE), true)
    };
    if remote && fs::symlink_metadata(&file)?.file_type().is_symlink() {
        return Err(fail("Git-hosted package index must not be a symlink"));
    }
    let mut bytes = Vec::new();
    fs::File::open(&file)?
        .take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err(fail("package index exceeds 4 MiB"));
    }
    let mut catalog: Catalog = toml::from_str(std::str::from_utf8(&bytes)?)?;
    if catalog.format != 1 || catalog.packages.len() > 10_000 {
        return Err(fail(
            "unsupported package index format or more than 10000 entries",
        ));
    }
    for (name, entry) in &mut catalog.packages {
        if !valid_name(name) || entry.description.len() > 4096 {
            return Err(fail(
                "invalid package index name or description exceeds 4096 bytes",
            ));
        }
        let dep = &mut entry.source;
        match (&mut dep.path, &mut dep.git) {
            (Some(path), None) if !path.is_empty() && dep.rev.is_none() && !remote => {
                *path = file
                    .parent()
                    .unwrap()
                    .join(&*path)
                    .to_str()
                    .ok_or_else(|| fail("package paths must be UTF-8"))?
                    .into();
            }
            (None, Some(url)) if !url.is_empty() && !url.starts_with('-') => {
                if dep.version.is_some() && dep.rev.is_some() {
                    return Err(fail(
                        "index Git sources cannot specify both version and rev",
                    ));
                }
                let relative = file.parent().unwrap().join(&*url);
                if remote && !url.contains("://") && !url.contains('@') {
                    return Err(fail("Git-hosted indexes must use absolute Git URLs"));
                }
                if !remote && relative.is_dir() {
                    *url = local_git_path(&relative)?;
                } else if !remote && !url.contains("://") && !url.contains('@') {
                    return Err(fail("index Git source path not found"));
                }
                if dep.version.is_none() && dep.rev.is_none() {
                    dep.version = Some("*".into());
                }
            }
            _ => {
                return Err(fail(
                    "index sources need exactly one of path or git; Git-hosted indexes cannot contain local paths",
                ));
            }
        }
        if let Some(version) = &dep.version {
            VersionReq::parse(version)?;
        }
        if dep
            .rev
            .as_ref()
            .is_some_and(|r| r.is_empty() || r.starts_with('-') || r.contains('\0'))
        {
            return Err(fail("invalid index Git revision"));
        }
    }
    Ok(Index {
        packages: catalog.packages,
    })
}

impl Index {
    pub(super) fn dependency(
        &self,
        name: &str,
        version: Option<String>,
        rev: Option<String>,
    ) -> Result<Dependency> {
        if version.is_some() && rev.is_some() {
            return Err(fail("specify either --version or --rev"));
        }
        let mut dep = self
            .packages
            .get(name)
            .ok_or_else(|| fail(format!("package '{name}' not found in index")))?
            .source
            .clone();
        if let Some(version) = version {
            dep.version = Some(version);
            dep.rev = None;
        }
        if let Some(rev) = rev {
            if dep.git.is_none() {
                return Err(fail("--rev requires a Git package"));
            }
            dep.rev = Some(rev);
            dep.version = None;
        }
        Ok(dep)
    }
}

pub(super) fn search(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut query = None;
    let mut source = None;
    let mut offline = false;
    let mut options = args.iter().copied();
    while let Some(arg) = options.next() {
        match arg {
            "--index" if source.is_none() => {
                source = Some(
                    options
                        .next()
                        .ok_or_else(|| fail("--index needs a source"))?,
                );
            }
            "--offline" if !offline => offline = true,
            arg if !arg.starts_with('-') && query.is_none() => query = Some(arg),
            _ => {
                return Err(fail(
                    "usage: scripta search [query] [--index <source>] [--offline]",
                ));
            }
        }
    }
    let root = project(cwd).ok();
    let manifest = root.as_deref().map(read_manifest).transpose()?;
    let project = root.as_deref().zip(manifest.as_ref());
    let catalog = load(cwd, project, source, offline)?;
    let query = query.unwrap_or("").to_lowercase();
    let mut rows = Vec::new();
    for (name, entry) in &catalog.packages {
        if name.to_lowercase().contains(&query) || entry.description.to_lowercase().contains(&query)
        {
            let description: String = entry
                .description
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            rows.push(format!("{name}\t{description}"));
        }
    }
    Ok(if rows.is_empty() {
        "No matching packages".into()
    } else {
        rows.join("\n")
    })
}
