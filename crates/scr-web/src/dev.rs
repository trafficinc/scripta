//! Opt-in bounded source polling and immutable per-generation include trees.
use crate::{observe::Observer, server::Config};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 1024 * 1024;
type Tree = BTreeMap<PathBuf, Vec<u8>>;
#[derive(Debug)]
pub(crate) struct Snapshot {
    directory: tempfile::TempDir,
    pub entry: PathBuf,
    pub source: String,
}
impl Snapshot {
    pub fn root(&self) -> &Path {
        self.directory.path()
    }
}
fn source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("scripta" | "scta" | "scr" | "inc")
    )
}
fn excluded(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default();
    name.to_string_lossy().starts_with('.')
        || matches!(name.to_str(), Some("target" | "node_modules"))
}
pub(crate) fn validate(config: &Config) -> io::Result<()> {
    if config.watch_debounce < Duration::from_millis(50)
        || config.watch_debounce > Duration::from_secs(5)
    {
        return Err(io::Error::other("watch-debounce-ms must be 50..5000"));
    }
    if config.watch && (!source(&config.path) || excluded(&config.path)) {
        return Err(io::Error::other(
            "watch entry must be a visible .scripta, .scta, .scr, or .inc file",
        ));
    }
    Ok(())
}
fn scan(entry: &Path, stop: &AtomicBool) -> io::Result<Tree> {
    let root = entry
        .parent()
        .ok_or_else(|| io::Error::other("watch entry has no parent"))?;
    let mut pending = vec![(root.to_path_buf(), 0)];
    let mut entries = 0usize;
    let mut bytes = 0usize;
    let mut paths = 0usize;
    let mut tree = Tree::new();
    while let Some((directory, depth)) = pending.pop() {
        if depth > 32 {
            return Err(io::Error::other("watch directory depth exceeds 32"));
        }
        for item in std::fs::read_dir(directory)? {
            if stop.load(Ordering::Acquire) {
                return Err(io::Error::other("watch stopped"));
            }
            let item = item?;
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(io::Error::other("watch tree exceeds 4096 entries"));
            }
            let path = item.path();
            if excluded(&path) {
                continue;
            }
            let relative = path.strip_prefix(root).map_err(io::Error::other)?;
            paths = paths.saturating_add(relative.as_os_str().len());
            if paths > MAX_PATH_BYTES {
                return Err(io::Error::other("watch paths exceed 1 MiB"));
            }
            let kind = item.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            if !kind.is_file() || !source(&path) {
                continue;
            }
            let limit = MAX_BYTES - bytes;
            let mut content = vec![];
            let file = std::fs::File::open(&path)?;
            if file.metadata()?.len() > limit as u64 {
                return Err(io::Error::other("watch sources exceed 16 MiB"));
            }
            file.take(limit as u64 + 1).read_to_end(&mut content)?;
            if content.len() > limit {
                return Err(io::Error::other("watch sources exceed 16 MiB"));
            }
            bytes += content.len();
            tree.insert(relative.to_owned(), content);
        }
    }
    if !tree.contains_key(Path::new(entry.file_name().unwrap_or_default())) {
        return Err(io::Error::other("watch entry is missing or excluded"));
    }
    Ok(tree)
}
fn build(entry: &Path, tree: &Tree) -> io::Result<Arc<Snapshot>> {
    let directory = tempfile::Builder::new()
        .prefix(".scripta-generation-")
        .tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    for (relative, bytes) in tree {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| io::Error::other("watch source is not UTF-8"))?;
        let tokens = scr_lexer::lex(text).map_err(|e| {
            io::Error::other(format!(
                "watch syntax error in {} at byte {}; run scripta check on that file",
                relative.display(),
                e.span.start
            ))
        })?;
        scr_parser::parse(&tokens).map_err(|e| {
            io::Error::other(format!(
                "watch syntax error in {} at byte {} ({}); run scripta check on that file",
                relative.display(),
                e.span.start,
                e.code
            ))
        })?;
        let target = directory.path().join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, bytes)?;
    }
    let name = entry
        .file_name()
        .ok_or_else(|| io::Error::other("missing watch entry filename"))?;
    let source =
        String::from_utf8(tree.get(Path::new(name)).unwrap().clone()).map_err(io::Error::other)?;
    let entry = directory.path().join(name);
    Ok(Arc::new(Snapshot {
        directory,
        entry,
        source,
    }))
}
pub(crate) struct Watcher {
    pub changes: watch::Receiver<Option<Arc<Snapshot>>>,
    stop: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.task.abort();
    }
}
pub(crate) async fn start(
    config: &Config,
    observer: Arc<Observer>,
) -> io::Result<(Arc<Snapshot>, Watcher)> {
    let entry = config.path.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let initial = scan(&entry, &stop)?;
    let snapshot = build(&entry, &initial)?;
    let (send, changes) = watch::channel(None);
    let flag = stop.clone();
    let debounce = config.watch_debounce;
    let task = tokio::spawn(async move {
        let mut observed = initial;
        let mut changed = None;
        let mut bad_scan = false;
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let path = entry.clone();
            let flag = flag.clone();
            let result = tokio::task::spawn_blocking(move || scan(&path, &flag)).await;
            let tree = match result {
                Ok(Ok(tree)) => {
                    bad_scan = false;
                    tree
                }
                _ => {
                    if !bad_scan {
                        observer.event(
                            "reload_failed",
                            None,
                            json!({"reason":"watch_scan"}),
                            "watch scan failed; check root permissions and documented limits",
                        );
                        bad_scan = true;
                    }
                    continue;
                }
            };
            if tree != observed {
                observed = tree;
                changed = Some(Instant::now());
                continue;
            }
            if changed.is_some_and(|when| when.elapsed() >= debounce) {
                changed = None;
                match build(&entry, &observed) {
                    Ok(snapshot) => {
                        send.send_replace(Some(snapshot));
                    }
                    Err(error) => observer.event(
                        "reload_failed",
                        None,
                        json!({"reason":"source_validation","diagnostic":error.to_string().chars().take(2048).collect::<String>()}),
                        "reload validation failed; retaining current generation",
                    ),
                }
            }
        }
    });
    Ok((
        snapshot,
        Watcher {
            changes,
            stop,
            task,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scan_bounds_cancellation_exclusions_and_snapshot_validation() {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("app.scripta");
        std::fs::write(&entry, "<?scripta echo \"ok\";").unwrap();
        for name in [".hidden", "target", "node_modules"] {
            std::fs::create_dir(directory.path().join(name)).unwrap();
            std::fs::write(
                directory.path().join(name).join("broken.scripta"),
                "<?scripta echo ;",
            )
            .unwrap();
        }
        std::fs::write(directory.path().join("data.txt"), [0xff]).unwrap();
        let stop = AtomicBool::new(false);
        let tree = scan(&entry, &stop).unwrap();
        assert_eq!(tree.len(), 1);
        let snapshot = build(&entry, &tree).unwrap();
        assert!(snapshot.entry.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(snapshot.root())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let root = snapshot.root().to_owned();
        drop(snapshot);
        assert!(!root.exists());
        stop.store(true, Ordering::Release);
        assert!(scan(&entry, &stop).is_err());
        stop.store(false, Ordering::Release);
        let file = std::fs::File::create(directory.path().join("large.inc")).unwrap();
        let initial_bytes = tree.values().map(Vec::len).sum::<usize>();
        file.set_len((MAX_BYTES - initial_bytes) as u64).unwrap();
        assert!(scan(&entry, &stop).is_ok());
        file.set_len((MAX_BYTES - initial_bytes + 1) as u64)
            .unwrap();
        assert!(
            scan(&entry, &stop)
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
        let mut tree = tree;
        tree.insert("bad.inc".into(), b"<?scripta echo ;".to_vec());
        assert!(
            build(&entry, &tree)
                .unwrap_err()
                .to_string()
                .contains("bad.inc")
        );
    }
    #[test]
    fn entry_count_limit_and_depth_are_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("app.scripta");
        std::fs::write(&entry, "<?scripta").unwrap();
        for index in 0..MAX_ENTRIES - 1 {
            std::fs::write(directory.path().join(format!("data-{index}")), b"").unwrap();
        }
        let stop = AtomicBool::new(false);
        assert!(scan(&entry, &stop).is_ok());
        std::fs::write(directory.path().join("extra"), b"").unwrap();
        assert!(
            scan(&entry, &stop)
                .unwrap_err()
                .to_string()
                .contains("4096")
        );
        let deep = tempfile::tempdir().unwrap();
        let entry = deep.path().join("app.scripta");
        std::fs::write(&entry, "<?scripta").unwrap();
        let mut path = deep.path().to_owned();
        for _ in 0..33 {
            path.push("d");
            std::fs::create_dir(&path).unwrap();
        }
        assert!(
            scan(&entry, &stop)
                .unwrap_err()
                .to_string()
                .contains("depth")
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_files_and_directories_are_not_snapshotted() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let entry = directory.path().join("app.scripta");
        std::fs::write(&entry, "<?scripta").unwrap();
        std::fs::write(outside.path().join("secret.inc"), "<?scripta").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.inc"),
            directory.path().join("link.inc"),
        )
        .unwrap();
        std::os::unix::fs::symlink(outside.path(), directory.path().join("linked")).unwrap();
        let tree = scan(&entry, &AtomicBool::new(false)).unwrap();
        assert_eq!(tree.len(), 1);
    }
}
