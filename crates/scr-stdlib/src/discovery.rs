//! Bounded, non-following traversal and portable relative-path glob matching.
use super::{FilesystemPolicy, failure, metadata};
use crate::paths;
use regex::{Regex, RegexBuilder};
use scr_runtime::{
    MapKey, NativeError, NativeFunction, NativeResult, NativeSignature, NativeType, Runtime,
    RuntimeContext, Value,
};
use std::{fs, mem::size_of, path::Path, rc::Rc};

const KEYS: [&str; 6] = [
    "name",
    "path",
    "relativePath",
    "isDirectory",
    "isFile",
    "isSymlink",
];
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;

fn resource(message: &str) -> NativeError {
    NativeError::new("ResourceLimitError", message)
}

fn glob(pattern: &str) -> NativeResult<Regex> {
    let invalid = || {
        NativeError::new(
            "GlobError",
            "expected a relative glob with *, ?, or whole-component ** (at most 4096 bytes); empty, dot, parent, bracket and brace components are unsupported",
        )
    };
    if pattern.is_empty() || pattern.len() > 4096 || pattern.contains(['\0', '[', ']', '{', '}']) {
        return Err(invalid());
    }
    let mut components: Vec<_> = pattern.split('/').collect();
    components.dedup_by(|a, b| *a == "**" && *b == "**");
    let mut expression = String::from("\\A");
    for (index, component) in components.iter().enumerate() {
        if matches!(*component, "" | "." | "..") {
            return Err(invalid());
        }
        if *component == "**" {
            expression.push_str(if index + 1 == components.len() {
                ".*"
            } else {
                "(?:[^/]+/)*"
            });
        } else {
            if component.contains("**") {
                return Err(invalid());
            }
            for ch in component.chars() {
                match ch {
                    '*' => expression.push_str("[^/]*"),
                    '?' => expression.push_str("[^/]"),
                    _ => expression.push_str(&regex::escape(&ch.to_string())),
                }
            }
            if index + 1 < components.len() {
                if index + 2 == components.len() && components[index + 1] == "**" {
                    expression.push_str("(?:/.*)?");
                    break;
                }
                expression.push('/');
            }
        }
    }
    expression.push_str("\\z");
    RegexBuilder::new(&expression)
        .dot_matches_new_line(true)
        .size_limit(1024 * 1024)
        .build()
        .map_err(|_| NativeError::new("GlobError", "glob exceeds compiled pattern limit"))
}

struct Options {
    depth: usize,
    entries: usize,
    hidden: bool,
}
impl Options {
    fn parse(value: Option<&Value>) -> NativeResult<Self> {
        let mut result = Self {
            depth: 64,
            entries: 10_000,
            hidden: true,
        };
        if let Some(Value::Map(entries)) = value {
            for (key, value) in entries {
                match (key, value) {
                    (MapKey::String(key), Value::Bool(value)) if key == "includeHidden" => {
                        result.hidden = *value
                    }
                    (MapKey::String(key), Value::Int(value))
                        if key == "maxDepth" || key == "maxEntries" =>
                    {
                        let maximum = if key == "maxDepth" { 256 } else { 1_000_000 };
                        if !(0..=maximum).contains(value) {
                            return Err(NativeError::new(
                                "RangeError",
                                format!("{key} must be between 0 and {maximum}"),
                            ));
                        }
                        if key == "maxDepth" {
                            result.depth = *value as usize;
                        } else {
                            result.entries = *value as usize;
                        }
                    }
                    _ => {
                        return Err(NativeError::type_error(
                            "discovery options: maxDepth/maxEntries must be ints; includeHidden must be bool; unknown keys are rejected",
                        ));
                    }
                }
            }
        }
        Ok(result)
    }
}

fn walk(
    context: &RuntimeContext<'_>,
    root: &Path,
    policy: &FilesystemPolicy,
    options: Options,
    pattern: Option<&Regex>,
) -> NativeResult {
    policy.check(context, root, false)?;
    metadata(root, true)?;
    let limit = context
        .max_value_bytes
        .unwrap_or(OUTPUT_LIMIT)
        .min(OUTPUT_LIMIT);
    let mut bytes = size_of::<Value>();
    if bytes > limit {
        return Err(resource("discovery result exceeds value byte limit"));
    }
    let mut rows = Vec::new();
    // Only one live directory iterator per depth; siblings are never queued.
    let mut stack = Vec::new();
    if options.depth > 0 {
        stack.push((
            fs::read_dir(root).map_err(|e| failure("walk directory", root, e))?,
            root.to_owned(),
            String::new(),
        ));
    }
    let mut visited = 0usize;
    while let Some((iterator, directory, relative)) = stack.last_mut() {
        context.check_cancelled()?;
        let Some(entry) = iterator.next() else {
            stack.pop();
            continue;
        };
        let entry = entry.map_err(|e| failure("walk directory entry", directory, e))?;
        visited += 1;
        if visited > options.entries {
            return Err(resource("directory walk exceeds maxEntries"));
        }
        let name = paths::text(&entry.file_name())?;
        if !options.hidden && name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Check the parent, not a symlink target, for snapshot observations.
        policy.check(context, directory, false)?;
        let info =
            fs::symlink_metadata(&path).map_err(|e| failure("inspect walk entry", &path, e))?;
        let relative = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        if pattern.is_none_or(|pattern| pattern.is_match(&relative)) {
            let path_text = paths::text(path.as_os_str())?;
            let fixed = size_of::<Value>()
                + KEYS
                    .iter()
                    .map(|k| size_of::<MapKey>() + k.len() + size_of::<Value>())
                    .sum::<usize>();
            bytes = bytes
                .checked_add(fixed)
                .and_then(|n| n.checked_add(name.len()))
                .and_then(|n| n.checked_add(path_text.len()))
                .and_then(|n| n.checked_add(relative.len()))
                .filter(|n| *n <= limit)
                .ok_or_else(|| resource("discovery result exceeds value byte limit"))?;
            rows.try_reserve(1)
                .map_err(|_| resource("discovery allocation failed"))?;
            let values = [
                Value::String(name),
                Value::String(path_text),
                Value::String(relative.clone()),
                Value::Bool(info.is_dir()),
                Value::Bool(info.is_file()),
                Value::Bool(info.file_type().is_symlink()),
            ];
            rows.push((
                relative.clone(),
                Value::Map(
                    KEYS.iter()
                        .zip(values)
                        .map(|(k, v)| (MapKey::String((*k).into()), v))
                        .collect(),
                ),
            ));
        }
        if info.is_dir() && !info.file_type().is_symlink() && stack.len() < options.depth {
            policy.check(context, &path, false)?;
            metadata(&path, true)?;
            stack.push((
                fs::read_dir(&path).map_err(|e| failure("walk directory", &path, e))?,
                path,
                relative,
            ));
        }
    }
    context.check_cancelled()?;
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut result = Vec::new();
    result
        .try_reserve_exact(rows.len())
        .map_err(|_| resource("discovery allocation failed"))?;
    for (_, value) in rows {
        context.check_cancelled()?;
        result.push(value);
    }
    Ok(Value::List(result))
}

pub(super) fn register(runtime: &mut Runtime, policy: Rc<FilesystemPolicy>) -> NativeResult<()> {
    use NativeType::{Bool, List, Map, String as Text};
    runtime.register(
        "Path::matchesGlob",
        NativeFunction::new(
            NativeSignature::new(vec![Text, Text], Bool),
            |context, args| {
                context.check_cancelled()?;
                let candidate = paths::string(&args[0])?;
                if candidate.contains('\0') {
                    return Err(NativeError::new("PathError", "glob candidate contains NUL"));
                }
                let pattern = glob(paths::string(&args[1])?)?;
                Ok(Value::Bool(pattern.is_match(candidate)))
            },
        ),
    )?;
    for filtered in [false, true] {
        let policy = policy.clone();
        let mut parameters = vec![Text];
        if filtered {
            parameters.push(Text);
        }
        runtime.register(
            if filtered { "Dir::glob" } else { "Dir::walk" },
            NativeFunction::new(
                NativeSignature::new(parameters, List).with_optional(vec![Map]),
                move |context, args| {
                    let root = paths::resolve(context, paths::string(&args[0])?)?;
                    let pattern = if filtered {
                        Some(glob(paths::string(&args[1])?)?)
                    } else {
                        None
                    };
                    let options = Options::parse(args.get(if filtered { 2 } else { 1 }))?;
                    walk(context, &root, &policy, options, pattern.as_ref())
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
    use std::sync::{Arc, atomic::AtomicBool};

    #[test]
    fn native_walk_checks_cancellation_before_io_and_empty_results() {
        let mut settings = ContextOptions::default();
        let mut input = std::io::empty();
        let mut output = Vec::new();
        let mut error = Vec::new();
        let context = RuntimeContext {
            working_directory: &mut settings.working_directory,
            script_path: None,
            cli_arguments: &[],
            environment: &mut settings.environment,
            input: &mut input,
            output: &mut output,
            error: &mut error,
            runtime_identity: None,
            cancellation: Some(Arc::new(AtomicBool::new(true))),
            permissions: &settings.permissions,
            call_span: Default::default(),
            max_value_bytes: None,
        };
        for depth in [0, 64] {
            let result = walk(
                &context,
                Path::new("missing"),
                &FilesystemPolicy::default(),
                Options {
                    depth,
                    entries: 0,
                    hidden: true,
                },
                None,
            );
            assert_eq!(result.unwrap_err().kind, "Interrupted");
        }
    }
}
