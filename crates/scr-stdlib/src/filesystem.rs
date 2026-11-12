//! Filesystem modules, implemented only through Rust's standard library.
use crate::paths;
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    rc::Rc,
    time::UNIX_EPOCH,
};

/// Optional host restrictions. None means unrestricted roots; Some([]) denies
/// every path. Configured roots must be existing absolute directories.
#[derive(Debug, Clone)]
pub struct FilesystemPolicy {
    pub read: bool,
    pub write: bool,
    pub roots: Option<Vec<PathBuf>>,
}
impl Default for FilesystemPolicy {
    fn default() -> Self {
        Self {
            read: true,
            write: true,
            roots: None,
        }
    }
}
impl FilesystemPolicy {
    pub(crate) fn prepare(mut self) -> NativeResult<Self> {
        if let Some(roots) = &mut self.roots {
            for root in roots {
                if !root.is_absolute() {
                    return Err(NativeError::type_error(
                        "filesystem policy roots must be absolute",
                    ));
                }
                *root = fs::canonicalize(&*root)
                    .map_err(|error| failure("resolve policy root", root, error))?;
                if !fs::metadata(&*root)
                    .map_err(|error| failure("inspect policy root", root, error))?
                    .is_dir()
                {
                    return Err(NativeError::type_error(
                        "filesystem policy root must be a directory",
                    ));
                }
            }
        }
        Ok(self)
    }
    pub(crate) fn check(
        &self,
        context: &RuntimeContext<'_>,
        path: &Path,
        writing: bool,
    ) -> NativeResult<()> {
        context.check_cancelled()?;
        context.permissions.require(if writing {
            "filesystem.write"
        } else {
            "filesystem.read"
        })?;
        self.check_path(path, writing)
    }
    pub(crate) fn check_path(&self, path: &Path, writing: bool) -> NativeResult<()> {
        if (writing && !self.write) || (!writing && !self.read) {
            return Err(NativeError::new(
                "PermissionError",
                "filesystem policy denies this operation",
            ));
        }
        if let Some(roots) = &self.roots {
            let resolved = canonical_candidate(path)?;
            if !roots.iter().any(|root| resolved.starts_with(root)) {
                return Err(NativeError::new(
                    "PermissionError",
                    format!(
                        "path is outside allowed filesystem roots: {}",
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

// Canonicalize existing ancestors before checking a path that may be created.
// Resolve symlinks before lexical normalization of a missing suffix.
fn canonical_candidate(path: &Path) -> NativeResult<PathBuf> {
    let mut ancestor = path.to_owned();
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(&ancestor) {
            Ok(mut canonical) => {
                for part in suffix.iter().rev() {
                    canonical.push(part);
                }
                return Ok(paths::normalize(&canonical));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let part = ancestor
                    .components()
                    .next_back()
                    .ok_or_else(|| failure("resolve path", path, error))?
                    .as_os_str()
                    .to_owned();
                suffix.push(part);
                if !ancestor.pop() {
                    return Err(NativeError::new(
                        "PathError",
                        "cannot resolve path ancestor",
                    ));
                }
            }
            Err(error) => return Err(failure("resolve path", path, error)),
        }
    }
}
fn failure(operation: &str, path: &Path, error: io::Error) -> NativeError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => "FileNotFoundError",
        io::ErrorKind::AlreadyExists => "AlreadyExistsError",
        io::ErrorKind::PermissionDenied => "PermissionError",
        _ => "FilesystemError",
    };
    NativeError::new(kind, format!("{operation} '{}': {error}", path.display()))
}
// lstat("link/") may follow a directory link on Unix. Inspect the final
// component without trailing separators/dots first, while preserving actual
// OS errors (for example regular-file/.) for non-link paths.
fn nofollow_metadata(path: &Path) -> io::Result<Metadata> {
    let component_path: PathBuf = path.components().collect();
    let info = fs::symlink_metadata(&component_path)?;
    if info.file_type().is_symlink() || component_path.as_os_str() == path.as_os_str() {
        Ok(info)
    } else {
        fs::symlink_metadata(path)
    }
}
fn metadata(path: &Path, directory: bool) -> NativeResult<Metadata> {
    let metadata = nofollow_metadata(path).map_err(|error| failure("inspect", path, error))?;
    if metadata.file_type().is_symlink()
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file()
        }
    {
        return Err(NativeError::new(
            "FilesystemError",
            format!(
                "expected {} without a final symlink: {}",
                if directory {
                    "directory"
                } else {
                    "regular file"
                },
                path.display()
            ),
        ));
    }
    Ok(metadata)
}
fn int_size(metadata: &Metadata) -> NativeResult<Value> {
    i64::try_from(metadata.len())
        .map(Value::Int)
        .map_err(|_| NativeError::new("RangeError", "file size exceeds i64 range"))
}
fn modified(metadata: &Metadata, path: &Path) -> NativeResult<Value> {
    let time = metadata
        .modified()
        .map_err(|error| failure("read modification time", path, error))?;
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).ok(),
        Err(error) => {
            let duration = error.duration();
            duration
                .as_secs()
                .checked_add(u64::from(duration.subsec_nanos() != 0))
                .and_then(|seconds| i64::try_from(seconds).ok())
                .and_then(i64::checked_neg)
        }
    }
    .ok_or_else(|| NativeError::new("RangeError", "modification time exceeds i64 range"))?;
    Ok(Value::Int(seconds))
}
fn open_read(path: &Path) -> NativeResult<File> {
    metadata(path, false)?;
    let file = File::open(path).map_err(|error| failure("open for reading", path, error))?;
    if !file
        .metadata()
        .map_err(|error| failure("inspect open file", path, error))?
        .is_file()
    {
        return Err(NativeError::new(
            "FilesystemError",
            "opened path is not a regular file",
        ));
    }
    Ok(file)
}
fn open_write(path: &Path, append: bool) -> NativeResult<File> {
    match OpenOptions::new()
        .write(true)
        .append(append)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            metadata(path, false)?;
            let file = OpenOptions::new()
                .write(true)
                .append(append)
                .open(path)
                .map_err(|error| failure("open for writing", path, error))?;
            if !file
                .metadata()
                .map_err(|error| failure("inspect open file", path, error))?
                .is_file()
            {
                return Err(NativeError::new(
                    "FilesystemError",
                    "opened path is not a regular file",
                ));
            }
            if !append {
                file.set_len(0)
                    .map_err(|error| failure("truncate", path, error))?;
            }
            Ok(file)
        }
        Err(error) => Err(failure("create file", path, error)),
    }
}
fn read_text(context: &RuntimeContext<'_>, path: &Path) -> NativeResult<Value> {
    let mut file = open_read(path)?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        context.check_cancelled()?;
        let count = file
            .read(&mut buffer)
            .map_err(|error| failure("read", path, error))?;
        if count == 0 {
            break;
        }
        if context.max_value_bytes.is_some_and(|limit| {
            bytes
                .len()
                .checked_add(count)
                .and_then(|n| n.checked_add(std::mem::size_of::<Value>()))
                .is_none_or(|size| size > limit)
        }) {
            return Err(NativeError::new(
                "ResourceLimitError",
                "file read exceeds execution value byte limit",
            ));
        }
        bytes
            .try_reserve(count)
            .map_err(|_| NativeError::new("ResourceLimitError", "file read allocation failed"))?;
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).map(Value::String).map_err(|_| {
        NativeError::new(
            "EncodingError",
            format!("file is not valid UTF-8: {}", path.display()),
        )
    })
}
fn copy(context: &RuntimeContext<'_>, source: &Path, destination: &Path) -> NativeResult<()> {
    let mut input = open_read(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| failure("create copy destination", destination, error))?;
    let result = (|| {
        let mut buffer = [0; 64 * 1024];
        loop {
            context.check_cancelled()?;
            let count = input
                .read(&mut buffer)
                .map_err(|error| failure("read copy source", source, error))?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| failure("write copy destination", destination, error))?;
        }
        output
            .flush()
            .map_err(|error| failure("flush copy destination", destination, error))
    })();
    drop(output);
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(destination) {
            if cleanup.kind() != io::ErrorKind::NotFound {
                return Err(NativeError::new(
                    error.kind,
                    format!("{}; cleanup failed: {cleanup}", error.message),
                ));
            }
        }
        return Err(error);
    }
    Ok(())
}

#[derive(Debug)]
struct Entry {
    name: String,
    path: String,
    extension: String,
    size: Value,
    modified: Value,
    directory: bool,
}
impl NativeObject for Entry {
    fn type_name(&self) -> &str {
        "DirEntry"
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "name" => Ok(Value::String(self.name.clone())),
            "path" => Ok(Value::String(self.path.clone())),
            "extension" => Ok(Value::String(self.extension.clone())),
            "size" => Ok(self.size.clone()),
            "modified" => Ok(self.modified.clone()),
            "isDirectory" => Ok(Value::Bool(self.directory)),
            _ => Err(format!("unknown DirEntry property {name}")),
        }
    }
}
fn entries(context: &RuntimeContext<'_>, path: &Path, directories: bool) -> NativeResult<Value> {
    metadata(path, true)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| failure("list directory", path, error))? {
        context.check_cancelled()?;
        let entry = entry.map_err(|error| failure("read directory entry", path, error))?;
        let entry_path = entry.path();
        let info = fs::symlink_metadata(&entry_path)
            .map_err(|error| failure("inspect directory entry", &entry_path, error))?;
        if info.file_type().is_symlink()
            || if directories {
                !info.is_dir()
            } else {
                !info.is_file()
            }
        {
            continue;
        }
        entries.push(Entry {
            name: paths::text(&entry.file_name())?,
            path: paths::text(entry_path.as_os_str())?,
            extension: paths::extension(&entry_path)?,
            size: if directories {
                Value::Null
            } else {
                int_size(&info)?
            },
            modified: modified(&info, &entry_path)?,
            directory: directories,
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Value::List(
        entries
            .into_iter()
            .map(|entry| Value::NativeObject(Rc::new(entry)))
            .collect(),
    ))
}

#[derive(Clone, Copy)]
enum Operation {
    Exists,
    Read,
    Write,
    Append,
    Copy,
    Move,
    Delete,
    Size,
    Modified,
    Extension,
    DirExists,
    Create,
    CreateAll,
    Files,
    Directories,
    Remove,
    Current,
    Change,
}

/// Register only File and Dir, with an optional root/read/write policy.
pub fn register(runtime: &mut Runtime, policy: FilesystemPolicy) -> NativeResult<()> {
    let policy = Rc::new(policy.prepare()?);
    use NativeType::{Bool, Int, List, Null, String as Text};
    use Operation::*;
    for (name, operation, count, returns) in [
        ("File::exists", Exists, 1, Bool),
        ("File::read", Read, 1, Text),
        ("File::write", Write, 2, Null),
        ("File::append", Append, 2, Null),
        ("File::copy", Copy, 2, Null),
        ("File::move", Move, 2, Null),
        ("File::delete", Delete, 1, Null),
        ("File::size", Size, 1, Int),
        ("File::modified", Modified, 1, Int),
        ("File::extension", Extension, 1, Text),
        ("Dir::exists", DirExists, 1, Bool),
        ("Dir::create", Create, 1, Null),
        ("Dir::createAll", CreateAll, 1, Null),
        ("Dir::files", Files, 1, List),
        ("Dir::directories", Directories, 1, List),
        ("Dir::remove", Remove, 1, Null),
        ("Dir::current", Current, 0, Text),
        ("Dir::change", Change, 1, Null),
    ] {
        let policy = policy.clone();
        runtime.register(
            name,
            NativeFunction::new(
                NativeSignature::new(vec![Text; count], returns),
                move |context, args| perform(operation, context, args, &policy),
            ),
        )?;
    }
    Ok(())
}
fn perform(
    operation: Operation,
    context: &mut RuntimeContext<'_>,
    args: &[Value],
    policy: &FilesystemPolicy,
) -> NativeResult {
    use Operation::*;
    if matches!(operation, Current) {
        let path = paths::resolve(context, ".")?;
        policy.check(context, &path, false)?;
        let canonical = fs::canonicalize(&path)
            .map_err(|error| failure("resolve working directory", &path, error))?;
        return Ok(Value::String(paths::text(canonical.as_os_str())?));
    }
    let value = args
        .first()
        .ok_or_else(|| NativeError::type_error("filesystem operation requires a path"))?;
    let value = paths::string(value)?;
    if matches!(operation, Extension) {
        return Ok(Value::String(paths::extension(paths::checked_path(
            value,
        )?)?));
    }
    let path = paths::resolve(context, value)?;
    let writing = matches!(
        operation,
        Write | Append | Move | Delete | Create | CreateAll | Remove
    );
    policy.check(context, &path, writing)?;
    match operation {
        Exists | DirExists => {
            let exists = match nofollow_metadata(&path) {
                Ok(info) => {
                    if matches!(operation, Exists) {
                        info.is_file()
                    } else {
                        info.is_dir()
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(failure("inspect existence", &path, error)),
            };
            Ok(Value::Bool(exists))
        }
        Read => read_text(context, &path),
        Write | Append => {
            let contents = args
                .get(1)
                .ok_or_else(|| NativeError::type_error("file contents are required"))?;
            let mut file = open_write(&path, matches!(operation, Append))?;
            for chunk in paths::string(contents)?.as_bytes().chunks(64 * 1024) {
                context.check_cancelled()?;
                file.write_all(chunk)
                    .map_err(|error| failure("write file", &path, error))?;
            }
            file.flush()
                .map_err(|error| failure("flush file", &path, error))?;
            Ok(Value::Null)
        }
        Copy | Move => {
            let destination = paths::resolve(
                context,
                paths::string(
                    args.get(1)
                        .ok_or_else(|| NativeError::type_error("destination is required"))?,
                )?,
            )?;
            policy.check(context, &destination, true)?;
            metadata(&path, false)?;
            match fs::symlink_metadata(&destination) {
                Ok(_) => {
                    return Err(NativeError::new(
                        "AlreadyExistsError",
                        format!("destination already exists: {}", destination.display()),
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(failure("inspect destination", &destination, error)),
            }
            if matches!(operation, Copy) {
                copy(context, &path, &destination)?;
            } else {
                // Hard-link creation atomically refuses existing destinations.
                // Cross-device/unsupported hard links fall back to an exclusive copy.
                match fs::hard_link(&path, &destination) {
                    Ok(()) => {}
                    Err(error)
                        if error.kind() == io::ErrorKind::CrossesDevices
                            || error.kind() == io::ErrorKind::Unsupported =>
                    {
                        copy(context, &path, &destination)?;
                    }
                    Err(error) => {
                        return Err(failure("create move destination", &destination, error));
                    }
                }
                fs::remove_file(&path).map_err(|error| {
                    failure("remove move source (destination remains)", &path, error)
                })?;
            }
            Ok(Value::Null)
        }
        Delete => {
            metadata(&path, false)?;
            fs::remove_file(&path).map_err(|error| failure("delete file", &path, error))?;
            Ok(Value::Null)
        }
        Size => int_size(&metadata(&path, false)?),
        Modified => modified(&metadata(&path, false)?, &path),
        Create => {
            fs::create_dir(&path).map_err(|error| failure("create directory", &path, error))?;
            Ok(Value::Null)
        }
        CreateAll => {
            match nofollow_metadata(&path) {
                Ok(_) => {
                    metadata(&path, true)?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(failure("inspect directory", &path, error)),
            }
            fs::create_dir_all(&path)
                .map_err(|error| failure("create directory tree", &path, error))?;
            Ok(Value::Null)
        }
        Files | Directories => entries(context, &path, matches!(operation, Directories)),
        Remove => {
            metadata(&path, true)?;
            let canonical = fs::canonicalize(&path)
                .map_err(|error| failure("resolve removal target", &path, error))?;
            let current = fs::canonicalize(paths::resolve(context, ".")?).map_err(|error| {
                failure(
                    "resolve working directory",
                    context.working_directory,
                    error,
                )
            })?;
            if canonical.parent().is_none() || current.starts_with(&canonical) {
                return Err(NativeError::new(
                    "PermissionError",
                    "refusing to remove root, working directory, or its ancestor",
                ));
            }
            fs::remove_dir(&path)
                .map_err(|error| failure("remove empty directory", &path, error))?;
            Ok(Value::Null)
        }
        Change => {
            metadata(&path, true)?;
            let canonical = fs::canonicalize(&path)
                .map_err(|error| failure("change directory", &path, error))?;
            paths::text(canonical.as_os_str())?;
            *context.working_directory = canonical;
            Ok(Value::Null)
        }
        Extension | Current => Err(NativeError::new(
            "NativeError",
            "unexpected filesystem operation",
        )),
    }
}
