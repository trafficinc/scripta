//! Execution-scoped temporary paths and independently opened advisory locks.
use super::{FilesystemPolicy, failure, metadata};
use crate::paths;
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    cell::RefCell,
    fs::{self, File, OpenOptions},
    io,
    mem::size_of,
    path::Path,
    rc::{Rc, Weak},
    thread,
    time::{Duration, Instant},
};

const MAX_HANDLES: usize = 64;
const HANDLE_BYTES: usize = size_of::<Resource>() + size_of::<State>();
fn resource_error(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}
fn temp_error(message: &str) -> NativeError {
    NativeError::new("TempError", message)
}
fn payload(context: &RuntimeContext<'_>, bytes: usize) -> NativeResult<()> {
    if context.max_value_bytes.is_some_and(|limit| {
        size_of::<Value>()
            .checked_add(bytes)
            .is_none_or(|n| n > limit)
    }) {
        return Err(resource_error(
            "filesystem resource exceeds execution value byte limit",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    File,
    Directory,
    Lock,
}
#[derive(Debug)]
enum State {
    Temporary(same_file::Handle),
    Lock(File),
}
#[derive(Debug)]
struct Resource {
    path: String,
    kind: Kind,
    mode: &'static str,
    owner: Rc<()>,
    policy: Rc<FilesystemPolicy>,
    state: RefCell<Option<State>>,
}
impl NativeObject for Resource {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        match self.kind {
            Kind::File => "TempFile",
            Kind::Directory => "TempDirectory",
            Kind::Lock => "FileLock",
        }
    }
    fn payload_bytes(&self) -> usize {
        HANDLE_BYTES + self.path.len()
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "path" => Ok(Value::String(self.path.clone())),
            "closed" => Ok(Value::Bool(self.state.borrow().is_none())),
            "mode" if self.kind == Kind::Lock => Ok(Value::String(self.mode.into())),
            _ => Err(format!("unknown {} property {name}", self.type_name())),
        }
    }
}
impl Resource {
    fn close(&self) -> NativeResult<()> {
        let mut state = self.state.borrow_mut();
        let Some(active) = state.as_ref() else {
            return Ok(());
        };
        match active {
            State::Lock(file) => {
                fs2::FileExt::unlock(file)
                    .map_err(|e| failure("unlock file", Path::new(&self.path), e))?;
            }
            State::Temporary(identity) => {
                let path = Path::new(&self.path);
                // Cleanup does not require ambient capabilities, but never follows a
                // replaced final link or escapes the registration's root policy.
                self.policy.check_path(path, true)?;
                match fs::symlink_metadata(path) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        state.take();
                        return Ok(());
                    }
                    Err(e) => return Err(failure("inspect temporary cleanup root", path, e)),
                    Ok(info) => {
                        if info.file_type().is_symlink()
                            || (self.kind == Kind::Directory && !info.is_dir())
                            || (self.kind == Kind::File && !info.is_file())
                        {
                            return Err(temp_error(
                                "temporary path was replaced; refusing cleanup",
                            ));
                        }
                    }
                }
                let current = same_file::Handle::from_path(path)
                    .map_err(|e| failure("identify temporary cleanup root", path, e))?;
                if *identity != current {
                    return Err(temp_error(
                        "temporary path identity changed; refusing cleanup",
                    ));
                }
                drop(current);
                if self.kind == Kind::Directory {
                    // std removal does not follow links below the verified root.
                    fs::remove_dir_all(path)
                        .map_err(|e| failure("remove temporary directory", path, e))?;
                } else {
                    fs::remove_file(path).map_err(|e| failure("remove temporary file", path, e))?;
                }
            }
        }
        state.take();
        Ok(())
    }
}
impl Drop for Resource {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
struct Registration {
    owner: Rc<()>,
    runtime: Weak<()>,
    policy: Rc<FilesystemPolicy>,
    handles: RefCell<Vec<Rc<Resource>>>,
}
impl Registration {
    fn check_runtime(&self, context: &RuntimeContext<'_>) -> NativeResult<()> {
        if !context
            .runtime_identity
            .as_ref()
            .is_some_and(|identity| self.runtime.ptr_eq(&Rc::downgrade(identity)))
        {
            return Err(NativeError::type_error(
                "filesystem resource registration belongs to another runtime",
            ));
        }
        Ok(())
    }
    fn reserve(&self) -> NativeResult<()> {
        let mut handles = self.handles.borrow_mut();
        handles.retain(|h| h.state.borrow().is_some());
        if handles.len() >= MAX_HANDLES {
            return Err(resource_error(
                "open temporary/lock resource limit exceeded (64)",
            ));
        }
        handles
            .try_reserve(1)
            .map_err(|_| resource_error("filesystem resource registry allocation failed"))?;
        Ok(())
    }
    fn publish(&self, path: String, kind: Kind, mode: &'static str, state: State) -> Value {
        let handle = Rc::new(Resource {
            path,
            kind,
            mode,
            owner: self.owner.clone(),
            policy: self.policy.clone(),
            state: RefCell::new(Some(state)),
        });
        self.handles.borrow_mut().push(handle.clone());
        Value::NativeObject(handle)
    }
    fn handle<'a>(&self, value: &'a Value, lock: bool) -> NativeResult<&'a Resource> {
        let Value::NativeObject(object) = value else {
            return Err(NativeError::type_error("expected filesystem resource"));
        };
        let handle = object
            .as_any()
            .and_then(|o| o.downcast_ref::<Resource>())
            .ok_or_else(|| {
                NativeError::type_error("expected a real temporary resource or FileLock")
            })?;
        if !Rc::ptr_eq(&handle.owner, &self.owner) || (handle.kind == Kind::Lock) != lock {
            return Err(NativeError::type_error(
                "filesystem resource has a foreign registration or wrong kind",
            ));
        }
        Ok(handle)
    }
    fn cleanup(&self) -> NativeResult<()> {
        let mut handles = self.handles.borrow_mut();
        let mut first = None;
        // Unlock first, then remove children before parents. Retain failed resources
        // so explicit close or a later execution can retry cleanup.
        for locks in [true, false] {
            for handle in handles.iter().rev() {
                if (handle.kind == Kind::Lock) == locks {
                    if let Err(error) = handle.close() {
                        if first.is_none() {
                            first = Some(error);
                        }
                    }
                }
            }
        }
        handles.retain(|handle| handle.state.borrow().is_some());
        first.map_or(Ok(()), Err)
    }
    fn temporary(
        &self,
        context: &RuntimeContext<'_>,
        parent: &str,
        directory: bool,
    ) -> NativeResult<Value> {
        let parent = paths::resolve(context, parent)?;
        self.policy.check(context, &parent, true)?;
        metadata(&parent, true)?;
        let parent = fs::canonicalize(&parent)
            .map_err(|e| failure("resolve temporary parent", &parent, e))?;
        self.policy.check(context, &parent, true)?;
        self.reserve()?;
        payload(context, HANDLE_BYTES)?;
        let mut builder = tempfile::Builder::new();
        builder.prefix("scripta-").rand_bytes(16);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(fs::Permissions::from_mode(if directory {
                0o700
            } else {
                0o600
            }));
        }
        if directory {
            let temporary = builder
                .tempdir_in(&parent)
                .map_err(|e| failure("create temporary directory", &parent, e))?;
            let text = paths::text(temporary.path().as_os_str())?;
            payload(context, HANDLE_BYTES + text.len())?;
            let identity = same_file::Handle::from_path(temporary.path())
                .map_err(|e| failure("identify temporary directory", temporary.path(), e))?;
            context.check_cancelled()?;
            let _ = temporary.keep();
            Ok(self.publish(text, Kind::Directory, "", State::Temporary(identity)))
        } else {
            let temporary = builder
                .tempfile_in(&parent)
                .map_err(|e| failure("create temporary file", &parent, e))?;
            let text = paths::text(temporary.path().as_os_str())?;
            payload(context, HANDLE_BYTES + text.len())?;
            let identity = same_file::Handle::from_file(
                temporary
                    .as_file()
                    .try_clone()
                    .map_err(|e| failure("retain temporary file", temporary.path(), e))?,
            )
            .map_err(|e| failure("identify temporary file", temporary.path(), e))?;
            context.check_cancelled()?;
            let (_file, path) = temporary.into_parts();
            path.keep()
                .map_err(|e| failure("retain temporary path", &parent, e.error))?;
            Ok(self.publish(text, Kind::File, "", State::Temporary(identity)))
        }
    }
    fn lock(
        &self,
        context: &RuntimeContext<'_>,
        path: &str,
        mode: &str,
        timeout: i64,
    ) -> NativeResult<Value> {
        let mode = match mode {
            "shared" => "shared",
            "exclusive" => "exclusive",
            _ => {
                return Err(NativeError::type_error(
                    "lock mode must be 'shared' or 'exclusive'",
                ));
            }
        };
        if !(0..=30_000).contains(&timeout) {
            return Err(NativeError::new(
                "RangeError",
                "lock timeoutMs must be between 0 and 30000",
            ));
        }
        let path = paths::resolve(context, path)?;
        self.policy.check(context, &path, false)?;
        if mode == "exclusive" {
            self.policy.check(context, &path, true)?;
        }
        metadata(&path, false)?;
        let text = paths::text(path.as_os_str())?;
        payload(context, HANDLE_BYTES + text.len())?;
        self.reserve()?;
        let file = OpenOptions::new()
            .read(true)
            .write(mode == "exclusive")
            .open(&path)
            .map_err(|e| failure("open lock file", &path, e))?;
        if !file
            .metadata()
            .map_err(|e| failure("inspect lock file", &path, e))?
            .is_file()
        {
            return Err(NativeError::new(
                "FilesystemError",
                "lock requires a regular file",
            ));
        }
        if !acquire_lock(
            &file,
            &path,
            mode,
            Duration::from_millis(timeout as u64),
            || context.check_cancelled(),
        )? {
            return Ok(Value::Null);
        }
        context.check_cancelled()?;
        Ok(self.publish(text, Kind::Lock, mode, State::Lock(file)))
    }
}
// The owned caller drops its descriptor on any failure, including cancellation
// immediately after a successful OS acquisition.
fn acquire_lock(
    file: &File,
    path: &Path,
    mode: &str,
    timeout: Duration,
    mut checkpoint: impl FnMut() -> NativeResult<()>,
) -> NativeResult<bool> {
    let deadline = Instant::now() + timeout;
    let mut attempted = false;
    let timed_out = || NativeError::new("TimeoutError", "file lock acquisition timed out");
    loop {
        checkpoint()?;
        // A late scheduler wakeup must not acquire a newly free lock after the
        // deadline. The initial nonblocking attempt is always allowed.
        if attempted && Instant::now() >= deadline {
            return Err(timed_out());
        }
        attempted = true;
        let result = if mode == "shared" {
            fs2::FileExt::try_lock_shared(file)
        } else {
            fs2::FileExt::try_lock_exclusive(file)
        };
        match result {
            Ok(()) => return Ok(true),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if timeout.is_zero() {
                    return Ok(false);
                }
                let now = Instant::now();
                if now >= deadline {
                    return Err(timed_out());
                }
                thread::sleep((deadline - now).min(Duration::from_millis(10)));
            }
            Err(error) => return Err(failure("acquire file lock", path, error)),
        }
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub(super) fn register(runtime: &mut Runtime, policy: Rc<FilesystemPolicy>) -> NativeResult<()> {
    use NativeType::{Int, Null, Object, OneOf, String as Text};
    let registration = Rc::new(Registration {
        owner: Rc::new(()),
        runtime: Rc::downgrade(&runtime.native_identity()),
        policy,
        handles: RefCell::new(Vec::new()),
    });
    let cleanup = registration.clone();
    runtime.add_execution_cleanup(move || cleanup.cleanup());
    for directory in [false, true] {
        let registration = registration.clone();
        runtime.register(
            if directory {
                "Temp::directory"
            } else {
                "Temp::file"
            },
            NativeFunction::new(
                NativeSignature::new(
                    vec![Text],
                    Object(if directory {
                        "TempDirectory"
                    } else {
                        "TempFile"
                    }),
                ),
                move |context, args| {
                    context.check_cancelled()?;
                    registration.check_runtime(context)?;
                    registration.temporary(context, paths::string(&args[0])?, directory)
                },
            ),
        )?;
    }
    let locks = registration.clone();
    runtime.register(
        "File::lock",
        NativeFunction::new(
            NativeSignature::new(vec![Text, Text], OneOf(vec![Object("FileLock"), Null]))
                .with_optional(vec![Int]),
            move |context, args| {
                context.check_cancelled()?;
                locks.check_runtime(context)?;
                let timeout = match args.get(2) {
                    Some(Value::Int(n)) => *n,
                    None => 0,
                    _ => unreachable!(),
                };
                locks.lock(
                    context,
                    paths::string(&args[0])?,
                    paths::string(&args[1])?,
                    timeout,
                )
            },
        ),
    )?;
    for lock in [false, true] {
        let registration = registration.clone();
        runtime.register(
            if lock { "File::unlock" } else { "Temp::close" },
            NativeFunction::new(
                NativeSignature::new(
                    vec![if lock {
                        Object("FileLock")
                    } else {
                        OneOf(vec![Object("TempFile"), Object("TempDirectory")])
                    }],
                    Null,
                ),
                move |context, args| {
                    context.check_cancelled()?;
                    registration.check_runtime(context)?;
                    payload(context, 0)?;
                    registration.handle(&args[0], lock)?.close()?;
                    Ok(Value::Null)
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scr_runtime::ContextOptions;

    fn with_context(test: impl FnOnce(&mut RuntimeContext<'_>)) {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        test(&mut RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: Some(Rc::new(())),
            cancellation: None,
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        });
    }
    fn registration(context: &RuntimeContext<'_>) -> Registration {
        Registration {
            owner: Rc::new(()),
            runtime: Rc::downgrade(context.runtime_identity.as_ref().unwrap()),
            policy: Rc::new(FilesystemPolicy::default()),
            handles: RefCell::new(Vec::new()),
        }
    }
    #[test]
    fn exact_temp_handle_budgets_cleanup_failed_creation_and_include_path_payload() {
        with_context(|context| {
            let directory = tempfile::tempdir().unwrap();
            let parent = fs::canonicalize(directory.path()).unwrap();
            let parent = parent.to_str().unwrap();
            // All generated names contain the eight-byte prefix and 16 random characters.
            let expected = size_of::<Value>() + HANDLE_BYTES + parent.len() + 1 + 8 + 16;
            for is_directory in [false, true] {
                for (limit, success) in [(expected, true), (expected - 1, false)] {
                    let registration = registration(context);
                    context.max_value_bytes = Some(limit);
                    let result = registration.temporary(context, parent, is_directory);
                    if success {
                        let value = result.unwrap();
                        let handle = registration.handle(&value, false).unwrap();
                        assert_eq!(handle.payload_bytes() + size_of::<Value>(), expected);
                        registration.cleanup().unwrap();
                        assert!(handle.state.borrow().is_none());
                    } else {
                        assert_eq!(result.unwrap_err().kind, "ResourceLimitError");
                        assert!(registration.handles.borrow().is_empty());
                    }
                    assert_eq!(fs::read_dir(parent).unwrap().count(), 0);
                }
            }
        });
    }
    #[test]
    fn a_lock_released_after_the_wait_deadline_is_not_acquired() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        fs::write(&path, "safe").unwrap();
        let holder = File::options().read(true).write(true).open(&path).unwrap();
        let waiter = File::options().read(true).write(true).open(&path).unwrap();
        fs2::FileExt::lock_exclusive(&holder).unwrap();
        let mut checks = 0;
        let result = acquire_lock(
            &waiter,
            &path,
            "exclusive",
            Duration::from_millis(100),
            || {
                checks += 1;
                if checks == 2 {
                    thread::sleep(Duration::from_millis(110));
                    fs2::FileExt::unlock(&holder).unwrap();
                }
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err().kind, "TimeoutError");
        assert_eq!(checks, 2);
        assert!(acquire_lock(&waiter, &path, "exclusive", Duration::ZERO, || Ok(())).unwrap());
    }
    #[test]
    fn resource_registration_rejects_foreign_runtime_and_survives_teardown_aliases() {
        with_context(|context| {
            let directory = tempfile::tempdir().unwrap();
            let registration = registration(context);
            registration.check_runtime(context).unwrap();
            let value = registration
                .temporary(context, directory.path().to_str().unwrap(), true)
                .unwrap();
            let path = registration.handle(&value, false).unwrap().path.clone();
            context.runtime_identity = Some(Rc::new(()));
            assert_eq!(
                registration.check_runtime(context).unwrap_err().kind,
                "TypeError"
            );
            context.runtime_identity = None;
            assert_eq!(
                registration.check_runtime(context).unwrap_err().kind,
                "TypeError"
            );
            drop(registration);
            assert!(!Path::new(&path).exists());
            let Value::NativeObject(handle) = value else {
                unreachable!()
            };
            assert_eq!(handle.get_property("closed").unwrap(), Value::Bool(true));
        });
    }
}
