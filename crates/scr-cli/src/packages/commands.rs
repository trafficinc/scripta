use super::*;

pub(super) fn remove(cwd: &Path, name: &str) -> Result<String> {
    if !valid_name(name) {
        return Err(fail("invalid dependency name"));
    }
    let root = project(cwd)?;
    let mut manifest = read_manifest(&root)?;
    if manifest.dependencies.remove(name).is_none() {
        return Err(fail(format!("{name} is not a direct dependency")));
    }
    let text = toml::to_string_pretty(&manifest)?;
    // Reuse verified snapshots for retained packages, including local sources
    // that have moved or changed. The existing installer stages the new graph
    // and rolls back the store and lock if metadata replacement fails.
    install(&root, &manifest, false, false, true, Some(text), true)?;
    Ok(format!("Removed {name}"))
}

fn locked_project(cwd: &Path) -> Result<(PathBuf, Manifest, BTreeMap<String, Locked>)> {
    let root = project(cwd)?;
    let manifest = read_manifest(&root)?;
    let lock = read_lock(&root)?;
    let packages = match lock {
        Some(lock) if lock_matches(&manifest, &lock.packages) => lock.packages,
        None if manifest.dependencies.is_empty() => BTreeMap::new(),
        _ => {
            return Err(fail(
                "scripta.lock is missing or out of date; run 'scripta install'",
            ));
        }
    };
    Ok((root, manifest, packages))
}

pub(super) fn list(cwd: &Path, args: &[&str]) -> Result<String> {
    let tree = match args {
        [] => false,
        ["--tree"] => true,
        _ => return Err(fail("usage: scripta list [--tree]")),
    };
    let (_, _, packages) = locked_project(cwd)?;
    if packages.is_empty() {
        return Ok("No dependencies".into());
    }
    fn rows(
        packages: &BTreeMap<String, Locked>,
        tree: bool,
        depth: usize,
        count: &mut usize,
        out: &mut Vec<String>,
    ) -> Result<()> {
        if depth > 32 {
            return Err(fail("dependency graph exceeds 32 levels"));
        }
        for (name, pin) in packages {
            *count += 1;
            if *count > 256 || !valid_name(name) {
                return Err(fail("invalid or oversized dependency graph"));
            }
            out.push(format!(
                "{}{name}\t{}\t{}",
                "  ".repeat(depth),
                pin.version,
                if pin.source.path.is_some() {
                    "path"
                } else {
                    "git"
                }
            ));
            if tree {
                rows(&pin.dependencies, true, depth + 1, count, out)?;
            }
        }
        Ok(())
    }
    let mut out = vec!["PACKAGE\tVERSION\tSOURCE".into()];
    rows(&packages, tree, 0, &mut 0, &mut out)?;
    Ok(out.join("\n"))
}

fn git_url(root: &Path, value: &str) -> Result<String> {
    if root.join(value).is_dir() {
        local_git_path(&root.join(value))
    } else {
        Ok(value.into())
    }
}

pub(super) fn outdated(cwd: &Path, args: &[&str]) -> Result<String> {
    let offline = match args {
        [] => false,
        ["--offline"] => true,
        _ => return Err(fail("usage: scripta outdated [--offline]")),
    };
    let (root, manifest, packages) = locked_project(cwd)?;
    let mut rows = Vec::new();
    for (name, dep) in &manifest.dependencies {
        let pin = &packages[name];
        if let Some(path) = &dep.path {
            let source = fs::canonicalize(root.join(path))?;
            let package = read_manifest(&source)?
                .package
                .ok_or_else(|| fail(format!("{name}: source needs a [package] section")))?;
            if package.name != *name {
                return Err(fail(format!("{name}: source package name changed")));
            }
            let version = Version::parse(&package.version)?;
            if pin.version != package.version || checksum(&source)? != pin.checksum {
                let wanted = if dep
                    .version
                    .as_ref()
                    .map(|v| VersionReq::parse(v))
                    .transpose()?
                    .is_none_or(|req| req.matches(&version))
                {
                    package.version.as_str()
                } else {
                    "outside constraint"
                };
                rows.push(format!(
                    "{name}\t{}\t{wanted}\t{}\tcontent changed",
                    pin.version, package.version
                ));
            }
        } else if let Some(url) = &dep.git {
            if dep.rev.as_deref().is_some_and(is_commit) {
                // An immutable commit is intentionally pinned, not outdated.
                continue;
            }
            if offline {
                rows.push(format!(
                    "{name}\t{}\tunknown\tunknown\tGit lookup skipped (offline)",
                    pin.version
                ));
                continue;
            }
            let url = git_url(&root, url)?;
            if let Some(rev) = &dep.rev {
                let refs = git(
                    None,
                    &[
                        "ls-remote",
                        "--",
                        &url,
                        rev,
                        &format!("refs/heads/{rev}"),
                        &format!("refs/tags/{rev}"),
                        &format!("refs/tags/{rev}^{{}}"),
                    ],
                )?;
                let entries: Vec<_> = refs
                    .lines()
                    .filter_map(|line| line.split_once('\t'))
                    .collect();
                // Match exact refs and peel annotated tags. Like rev-parse,
                // prefer a short tag name over a same-named branch.
                let latest = [
                    rev.to_owned(),
                    format!("refs/tags/{rev}"),
                    format!("refs/heads/{rev}"),
                ]
                .iter()
                .find_map(|candidate| {
                    entries
                        .iter()
                        .find(|(_, reference)| *reference == format!("{candidate}^{{}}"))
                        .or_else(|| entries.iter().find(|(_, reference)| reference == candidate))
                })
                .ok_or_else(|| fail(format!("{name}: Git revision not found")))?
                .0;
                if !is_commit(latest) {
                    return Err(fail("Git returned an invalid commit ID"));
                }
                if pin.commit.as_deref() != Some(latest) {
                    rows.push(format!(
                        "{name}\t{}\t{latest}\t{latest}\trevision changed",
                        pin.version
                    ));
                }
            } else {
                let refs = git(None, &["ls-remote", "--tags", "--refs", "--", &url])?;
                let mut versions: Vec<_> = refs
                    .lines()
                    .filter_map(|line| {
                        let (_, reference) = line.split_once('\t')?;
                        let tag = reference.strip_prefix("refs/tags/")?;
                        Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
                    })
                    .collect();
                versions.sort();
                let req = VersionReq::parse(dep.version.as_deref().unwrap())?;
                let stable = VersionReq::STAR;
                let wanted = versions.iter().rev().find(|v| req.matches(v));
                let latest = versions.iter().rev().find(|v| stable.matches(v));
                let current = Version::parse(&pin.version)?;
                if wanted.is_none()
                    || wanted.is_some_and(|v| *v > current)
                    || latest.is_some_and(|v| *v > current)
                {
                    rows.push(format!(
                        "{name}\t{}\t{}\t{}\t{}",
                        pin.version,
                        wanted
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "none".into()),
                        latest
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "none".into()),
                        if wanted.is_none() {
                            "no matching version"
                        } else {
                            "new version"
                        }
                    ));
                }
            }
        }
    }
    Ok(if rows.is_empty() {
        "All dependencies are up to date".into()
    } else {
        format!(
            "PACKAGE\tCURRENT\tWANTED\tLATEST\tSTATUS\n{}",
            rows.join("\n")
        )
    })
}
