//! Stage a complete UTF-8 file beside its destination, then publish once.
use super::{FilesystemPolicy, failure, metadata, nofollow_metadata};
use scr_runtime::{NativeError, NativeResult, RuntimeContext};
use std::{
    fs::{File, OpenOptions, Permissions},
    io::{self, Write},
    path::Path,
};
use tempfile::{PathPersistError, TempPath};

// Check both the path kind and actual write access, without creating/truncating
// anything. Atomic replacement must not bypass an existing file's write denial.
fn destination_permissions(path: &Path) -> NativeResult<Option<Permissions>> {
    match nofollow_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(failure("inspect atomic destination", path, error)),
        Ok(_) => {
            let info = metadata(path, false)?;
            if info.permissions().readonly() {
                return Err(NativeError::new(
                    "PermissionError",
                    format!("atomic destination is read-only: {}", path.display()),
                ));
            }
            let probe = OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| failure("open atomic destination for writing", path, error))?;
            let info = probe
                .metadata()
                .map_err(|error| failure("inspect atomic destination handle", path, error))?;
            if !info.is_file() {
                return Err(NativeError::new(
                    "FilesystemError",
                    "atomic destination is not a regular file",
                ));
            }
            let permissions = info.permissions();
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                // Ownership changes on replacement. Never transfer setuid/setgid
                // or sticky bits to the newly created inode.
                Permissions::from_mode(permissions.mode() & 0o777)
            };
            Ok(Some(permissions))
        }
    }
}

fn cleanup_error(mut error: NativeError, cleanup: io::Result<()>) -> NativeError {
    if let Err(cleanup) = cleanup {
        error
            .message
            .push_str(&format!("; temporary file cleanup failed: {cleanup}"));
    }
    error
}

pub(super) fn write(
    context: &RuntimeContext<'_>,
    path: &Path,
    contents: &str,
    policy: &FilesystemPolicy,
) -> NativeResult<()> {
    replace_with(
        context,
        path,
        policy,
        |context, file| {
            for chunk in contents.as_bytes().chunks(64 * 1024) {
                context.check_cancelled()?;
                file.write_all(chunk)
                    .map_err(|error| failure("write atomic staging file", path, error))?;
            }
            file.flush()
                .map_err(|error| failure("flush atomic staging file", path, error))?;
            Ok(())
        },
        |temporary, destination| temporary.persist(destination),
    )
}

// The two I/O seams permit deterministic partial-write and commit-failure tests
// against the actual staging/cleanup path, without platform-dependent disk faults.
fn replace_with(
    context: &RuntimeContext<'_>,
    path: &Path,
    policy: &FilesystemPolicy,
    stage: impl FnOnce(&RuntimeContext<'_>, &mut File) -> NativeResult<()>,
    publish: impl FnOnce(TempPath, &Path) -> Result<(), PathPersistError>,
) -> NativeResult<()> {
    policy.check(context, path, true)?;
    destination_permissions(path)?;
    let parent = path.parent().ok_or_else(|| {
        NativeError::new("PathError", "atomic destination needs a parent directory")
    })?;
    policy.check(context, parent, true)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".scripta-write-")
        .tempfile_in(parent)
        .map_err(|error| failure("create atomic staging file", path, error))?;
    let preparation = (|| {
        stage(context, temporary.as_file_mut())?;
        // Inspect again immediately before publication, including host policy,
        // path kind, read-only state, and any newly changed permission bits.
        policy.check(context, path, true)?;
        if let Some(permissions) = destination_permissions(path)? {
            temporary
                .as_file()
                .set_permissions(permissions)
                .map_err(|error| failure("set atomic file permissions", path, error))?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| failure("synchronize atomic staging file", path, error))?;
        context.check_cancelled()?;
        Ok(())
    })();
    if let Err(error) = preparation {
        return Err(cleanup_error(error, temporary.close()));
    }
    // Close the data handle before rename (important for Windows sharing rules).
    let temporary = temporary.into_temp_path();
    if let Err(error) = context.check_cancelled() {
        return Err(cleanup_error(error, temporary.close()));
    }
    // No fallible work or cancellation check follows the commit point. persist
    // uses same-filesystem rename/replacement, never a delete/copy fallback.
    publish(temporary, path).map_err(|error| {
        cleanup_error(
            failure("replace atomic destination", path, error.error),
            error.path.close(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;
    use std::{
        io::Cursor,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    fn with_context(test: impl FnOnce(&RuntimeContext<'_>, Arc<AtomicBool>)) {
        let mut options = ContextOptions::default();
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let mut error = Vec::new();
        let flag = Arc::new(AtomicBool::new(false));
        test(
            &RuntimeContext {
                working_directory: &mut options.working_directory,
                script_path: None,
                cli_arguments: &[],
                environment: &mut options.environment,
                input: &mut input,
                output: &mut output,
                error: &mut error,
                cancellation: Some(flag.clone()),
                permissions: &options.permissions,
                call_span: Default::default(),
                max_value_bytes: None,
            },
            flag,
        );
    }

    fn assert_clean(dir: &Path, destination: &Path, original: Option<&[u8]>) {
        if let Some(original) = original {
            assert_eq!(std::fs::read(destination).unwrap(), original);
        } else {
            assert!(!destination.exists());
        }
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            names.len(),
            usize::from(original.is_some()),
            "temporary files remain: {names:?}"
        );
    }

    #[test]
    fn partial_staging_failures_and_commit_failures_preserve_destination_and_clean_up() {
        for existing in [false, true] {
            for commit_failure in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let destination = dir.path().join("report.json");
                if existing {
                    std::fs::write(&destination, b"original").unwrap();
                }
                with_context(|context, _| {
                    let error = replace_with(
                        context,
                        &destination,
                        &FilesystemPolicy::default(),
                        |_, file| {
                            file.write_all(b"partial output")?;
                            if commit_failure {
                                Ok(())
                            } else {
                                Err(NativeError::new(
                                    "FilesystemError",
                                    "injected partial write failure",
                                ))
                            }
                        },
                        |path, _| {
                            assert!(commit_failure, "commit ran after staging failed");
                            Err(PathPersistError {
                                error: io::Error::other("injected commit failure"),
                                path,
                            })
                        },
                    )
                    .unwrap_err();
                    assert_eq!(error.kind, "FilesystemError");
                    assert!(error.message.contains(if commit_failure {
                        "commit failure"
                    } else {
                        "write failure"
                    }));
                });
                assert_clean(
                    dir.path(),
                    &destination,
                    existing.then_some(b"original".as_slice()),
                );
            }
        }
    }

    #[test]
    fn cancellation_after_staging_cleans_up_and_never_commits() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report");
        std::fs::write(&destination, b"original").unwrap();
        with_context(|context, flag| {
            let error = replace_with(
                context,
                &destination,
                &FilesystemPolicy::default(),
                |_, file| {
                    file.write_all(b"new")?;
                    flag.store(true, Ordering::Relaxed);
                    Ok(())
                },
                |_, _| panic!("must not commit after cancellation"),
            )
            .unwrap_err();
            assert_eq!(error.kind, "Interrupted");
        });
        assert_clean(dir.path(), &destination, Some(b"original"));
    }

    #[test]
    fn cancellation_arriving_after_successful_publication_does_not_report_failure() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report");
        std::fs::write(&destination, b"original").unwrap();
        with_context(|context, flag| {
            replace_with(
                context,
                &destination,
                &FilesystemPolicy::default(),
                |_, file| {
                    file.write_all(b"new")?;
                    Ok(())
                },
                |path, destination| {
                    path.persist(destination)?;
                    flag.store(true, Ordering::Relaxed);
                    Ok(())
                },
            )
            .unwrap();
        });
        assert_clean(dir.path(), &destination, Some(b"new"));
    }

    #[cfg(unix)]
    #[test]
    fn a_destination_changed_to_a_symlink_is_rejected_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report");
        let other = dir.path().join("other");
        std::fs::write(&other, "untouched").unwrap();
        with_context(|context, _| {
            let error = replace_with(
                context,
                &destination,
                &FilesystemPolicy::default(),
                |_, file| {
                    file.write_all(b"new")?;
                    std::os::unix::fs::symlink(&other, &destination)?;
                    Ok(())
                },
                |_, _| panic!("must not commit to a symlink"),
            )
            .unwrap_err();
            assert_eq!(error.kind, "FilesystemError");
        });
        assert_eq!(std::fs::read(&other).unwrap(), b"untouched");
        assert!(
            std::fs::symlink_metadata(destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_failure_during_staging_reports_both_errors_and_preserves_destination() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("report");
        std::fs::write(&destination, "original").unwrap();
        with_context(|context, _| {
            let error = replace_with(
                context,
                &destination,
                &FilesystemPolicy::default(),
                |_, file| {
                    file.write_all(b"partial")?;
                    // Simulate a concurrent path change that prevents removing the
                    // staging name. The open descriptor still refers to its file.
                    let staging = std::fs::read_dir(dir.path())?
                        .find_map(|entry| {
                            let path = entry.ok()?.path();
                            path.file_name()?
                                .to_str()?
                                .starts_with(".scripta-write-")
                                .then_some(path)
                        })
                        .unwrap();
                    std::fs::remove_file(&staging)?;
                    std::fs::create_dir(&staging)?;
                    Err(NativeError::new(
                        "FilesystemError",
                        "injected write failure",
                    ))
                },
                |_, _| panic!("must not commit after staging failure"),
            )
            .unwrap_err();
            assert_eq!(error.kind, "FilesystemError");
            assert!(error.message.contains("injected write failure"));
            assert!(error.message.contains("temporary file cleanup failed"));
        });
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
