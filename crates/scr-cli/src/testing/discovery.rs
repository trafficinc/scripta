use super::*;
use scr_ast::StatementKind;
use std::collections::BTreeSet;

pub(super) fn discover(
    cwd: &Path,
    root: &Path,
    config: &TestConfig,
    options: &Options,
    source_limit: usize,
) -> Result<Vec<Case>, String> {
    fn visit(
        path: &Path,
        explicit: bool,
        depth: usize,
        files: &mut BTreeSet<PathBuf>,
    ) -> Result<(), String> {
        if depth > 64 {
            return Err("test discovery exceeds 64 directory levels".into());
        }
        let metadata =
            std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if metadata.file_type().is_symlink() {
            if explicit {
                return Err(format!(
                    "test discovery does not follow symlinks: {}",
                    path.display()
                ));
            }
            return Ok(());
        }
        if metadata.is_file() {
            if explicit
                || path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with("_test.scripta"))
            {
                files.insert(std::fs::canonicalize(path).map_err(|e| e.to_string())?);
                if files.len() > 4096 {
                    return Err("test discovery exceeds 4096 files".into());
                }
            }
        } else if metadata.is_dir() {
            for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if matches!(
                    entry.file_name().to_str(),
                    Some("fixtures" | ".git" | ".scripta" | "target")
                ) {
                    continue;
                }
                visit(&entry.path(), false, depth + 1, files)?;
            }
        } else {
            return Err(format!(
                "test path is not a regular file or directory: {}",
                path.display()
            ));
        }
        Ok(())
    }
    let mut files = BTreeSet::new();
    if options.paths.is_empty() {
        for directory in &config.directories {
            let path = root.join(directory);
            match std::fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
                Ok(_) => visit(&path, true, 0, &mut files)?,
            }
        }
    } else {
        for path in &options.paths {
            visit(&cwd.join(path), true, 0, &mut files)?;
        }
    }
    let mut cases = Vec::new();
    let mut source_bytes = 0_usize;
    let mut name_bytes = 0_usize;
    for path in files {
        let source = crate::entry::read_source(&path, source_limit)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        source_bytes = source_bytes.saturating_add(source.len());
        if source_bytes > 64 * 1024 * 1024 {
            return Err("test discovery exceeds 64 MiB of source".into());
        }
        let tokens = scr_lexer::lex(&source).map_err(|e| {
            scr_diagnostics::Diagnostic::from(e).render(&path.to_string_lossy(), &source)
        })?;
        let program = scr_parser::parse_with_diagnostics(&tokens).map_err(|errors| {
            errors
                .iter()
                .map(|e| e.render(&path.to_string_lossy(), &source))
                .collect::<Vec<_>>()
                .join("\n")
        })?;
        let errors = crate::validation::validate(&program);
        if !errors.is_empty() {
            return Err(errors
                .iter()
                .map(|e| e.render(&path.to_string_lossy(), &source))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let file = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for statement in program.statements {
            if let StatementKind::Function(function) = statement.kind {
                if !function.name.starts_with("test_") {
                    continue;
                }
                if function.name.len() > 4096 {
                    return Err(format!("{file}: test function name exceeds 4096 bytes"));
                }
                if !function.parameters.is_empty() {
                    return Err(format!(
                        "{file}::{}: test functions must take no arguments",
                        function.name
                    ));
                }
                name_bytes = name_bytes
                    .saturating_add(file.len())
                    .saturating_add(function.name.len());
                if name_bytes > 8 * 1024 * 1024 {
                    return Err("test discovery names exceed 8 MiB".into());
                }
                let id = format!("{file}::{}", function.name);
                if options
                    .filter
                    .as_ref()
                    .is_none_or(|filter| id.contains(filter))
                {
                    cases.push(Case {
                        path: path.clone(),
                        file: file.clone(),
                        name: function.name,
                    });
                    if cases.len() > 4096 {
                        return Err("test discovery exceeds 4096 cases".into());
                    }
                }
            }
        }
    }
    cases.sort_by(|a, b| (&a.file, &a.name).cmp(&(&b.file, &b.name)));
    Ok(cases)
}
