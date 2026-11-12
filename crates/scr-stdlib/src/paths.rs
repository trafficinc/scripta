//! Lexical path helpers use the host platform's path conventions.
use scr_runtime::{
    NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

pub(crate) fn text(value: &OsStr) -> NativeResult<String> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| NativeError::new("EncodingError", "filesystem path is not valid UTF-8"))
}
pub(crate) fn string(value: &Value) -> NativeResult<&str> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err(NativeError::type_error("path must be a string")),
    }
}
pub(crate) fn checked_path(value: &str) -> NativeResult<&Path> {
    if value.contains('\0') {
        return Err(NativeError::new(
            "PathError",
            "path contains a NUL character",
        ));
    }
    let path = Path::new(value);
    // Windows drive-relative (C:foo) and rooted-but-drive-less (\foo) paths
    // depend on process state. Require ordinary relative or fully absolute paths.
    #[cfg(windows)]
    if !path.is_absolute()
        && (path.has_root() || matches!(path.components().next(), Some(Component::Prefix(_))))
    {
        return Err(NativeError::new(
            "PathError",
            "Windows path must be relative without a drive, or fully absolute",
        ));
    }
    Ok(path)
}
pub(crate) fn resolve(context: &RuntimeContext<'_>, value: &str) -> NativeResult<PathBuf> {
    let path = checked_path(value)?;
    if value.is_empty() {
        return Err(NativeError::new(
            "PathError",
            "filesystem path must not be empty",
        ));
    }
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    let base = if context.working_directory.is_absolute() {
        context.working_directory.clone()
    } else {
        std::env::current_dir()?.join(&*context.working_directory)
    };
    // Preserve .. for actual I/O: collapsing it would change symlink semantics.
    Ok(base.join(path))
}
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(result.components().next_back(), Some(Component::Normal(_))) {
                    result.pop();
                } else if !result.has_root() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    result
}
pub(crate) fn extension(path: &Path) -> NativeResult<String> {
    path.extension().map_or(Ok(String::new()), text)
}

pub(crate) fn register(runtime: &mut Runtime) -> NativeResult<()> {
    runtime.register(
        "Path::join",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::String).with_variadic(NativeType::String),
            |_, args| {
                let mut result = PathBuf::new();
                for arg in args {
                    result.push(checked_path(string(arg)?)?);
                }
                Ok(Value::String(text(result.as_os_str())?))
            },
        ),
    )?;
    for name in ["filename", "extension", "parent", "normalize", "absolute"] {
        runtime.register(
            format!("Path::{name}"),
            NativeFunction::new(
                NativeSignature::new(vec![NativeType::String], NativeType::String),
                move |context, args| {
                    let [value] = args else {
                        return Err(NativeError::type_error("path helper expects one string"));
                    };
                    let path = checked_path(string(value)?)?;
                    let result = match name {
                        "filename" => path.file_name().map_or(Ok(String::new()), text)?,
                        "extension" => extension(path)?,
                        "parent" => path
                            .parent()
                            .map_or(Ok(String::new()), |parent| text(parent.as_os_str()))?,
                        "normalize" => text(normalize(path).as_os_str())?,
                        "absolute" => {
                            text(normalize(&resolve(context, string(value)?)?).as_os_str())?
                        }
                        _ => return Err(NativeError::new("NativeError", "unknown path helper")),
                    };
                    Ok(Value::String(result))
                },
            ),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_components_raise_encoding_errors() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let invalid = OsString::from_vec(vec![b'x', 0xff]);
        assert_eq!(text(&invalid).unwrap_err().kind, "EncodingError");
    }

    #[cfg(windows)]
    #[test]
    fn unpaired_windows_surrogates_raise_encoding_errors() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let invalid = OsString::from_wide(&[0xd800]);
        assert_eq!(text(&invalid).unwrap_err().kind, "EncodingError");
    }
}
