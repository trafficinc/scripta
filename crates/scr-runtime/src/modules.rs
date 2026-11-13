//! Local source loading with canonical identity and retained definition sources.
use super::*;
use scr_diagnostics::Diagnostic;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(crate) type ModulePolicy = dyn Fn(&Path) -> NativeResult<()>;
pub(crate) type ModuleResolver = dyn Fn(&Path, &str) -> NativeResult<Option<PathBuf>>;

impl Runtime {
    /// Resolve package names relative to the including file. Local files win.
    /// Called only after the filesystem.read permission check; the resolved path
    /// still passes through the host's module policy.
    pub fn set_module_resolver(
        &mut self,
        resolver: impl Fn(&Path, &str) -> NativeResult<Option<PathBuf>> + 'static,
    ) {
        self.module_resolver = Some(Rc::new(resolver));
    }
    /// Additional host path restriction. filesystem.read is always required.
    pub fn set_module_policy(&mut self, policy: impl Fn(&Path) -> NativeResult<()> + 'static) {
        self.module_policy = Some(Rc::new(policy));
    }
    /// Parse and execute with retained source text for module/function diagnostics.
    /// The entry filename comes from ContextOptions.script_path, if supplied.
    pub fn execute_source(
        &mut self,
        text: &str,
        output: &mut impl Write,
    ) -> std::result::Result<u8, RuntimeError> {
        if text.len() > self.options.max_source_bytes {
            return Err(*self.error("source byte limit exceeded", Span::default()));
        }
        let path = self
            .options
            .context
            .script_path
            .clone()
            .unwrap_or_else(|| self.options.context.working_directory.join("<script>"));
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        let previous = self.source.replace(Arc::new(SourceFile {
            path: path.clone(),
            text: text.into(),
        }));
        let result = (|| {
            let program = self.parse_source(text).map_err(|signal| match signal {
                Signal::NativeException(_, error) => *error,
                _ => unreachable!(),
            })?;
            self.execute(&program, output)
        })();
        self.source = previous;
        result
    }
    fn parse_source(&self, text: &str) -> Result<Program> {
        let tokens = scr_lexer::lex(text).map_err(Diagnostic::from);
        tokens
            .and_then(|tokens| scr_parser::parse(&tokens))
            .map_err(|error| {
                let signal = self.native_error(
                    NativeError::new("ModuleSyntaxError", error.message),
                    error.span,
                );
                match signal {
                    Signal::NativeException(native, mut runtime) => {
                        runtime.code = error.code;
                        runtime.help = error.help;
                        Signal::NativeException(native, runtime)
                    }
                    other => other,
                }
            })
    }
    pub(crate) fn require_module(
        &mut self,
        value: &str,
        once: bool,
        alias: Option<&scr_ast::Binding>,
        span: Span,
        output: &mut impl Write,
    ) -> Result<()> {
        let result = (|| -> NativeResult<PathBuf> {
            self.options
                .context
                .permissions
                .require("filesystem.read")?;
            if value.is_empty() || value.contains('\0') || value.contains("://") {
                return Err(NativeError::new(
                    "ModuleError",
                    "require expects a nonempty local file path",
                ));
            }
            let path = Path::new(value);
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                let base = self
                    .source
                    .as_ref()
                    .map(|s| s.path.as_path())
                    .or(self.options.context.script_path.as_deref())
                    .and_then(Path::parent)
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(&self.options.context.working_directory);
                let local = base.join(path);
                if local.is_file() {
                    local
                } else if let Some(resolver) = &self.module_resolver {
                    resolver(base, value)?.unwrap_or(local)
                } else {
                    local
                }
            };
            if let Some(policy) = &self.module_policy {
                policy(&path)?;
            }
            std::fs::canonicalize(&path).map_err(|error| {
                NativeError::new(
                    "ModuleNotFoundError",
                    format!("cannot resolve required file '{}': {error}", path.display()),
                )
            })
        })()
        .map_err(|error| self.native_error(error, span))?;
        if let Some(alias) = alias {
            if self.environments.scope(0).functions.keys().any(|name| {
                name.split_once("::")
                    .is_some_and(|(namespace, _)| namespace == alias.name)
            }) {
                return Err(self.native_error(
                    NativeError::new(
                        "ModuleAliasError",
                        format!("alias {} conflicts with a registered namespace", alias.name),
                    ),
                    alias.span,
                ));
            }
            if let Some(existing) = self.environments.find_alias(&alias.name) {
                if existing.path != result {
                    return Err(self.native_error(
                        NativeError::new(
                            "ModuleAliasError",
                            format!(
                                "alias {} is already bound to {}; cannot bind {}",
                                alias.name,
                                existing.path.display(),
                                result.display()
                            ),
                        ),
                        alias.span,
                    ));
                }
            }
        }
        if self.active_modules.contains(&result) {
            return Err(self.native_error(
                NativeError::new(
                    "CircularIncludeError",
                    format!("circular require: {}", result.display()),
                ),
                span,
            ));
        }
        let module_root = self.environments.module_root();
        if once {
            if let Some(alias) = alias {
                if let Some(&scope) = self.environments.module_cache.get(&result) {
                    self.environments
                        .scope_mut(self.environments.current)
                        .aliases
                        .insert(
                            alias.name.clone(),
                            environment::ModuleAlias {
                                path: result,
                                scope,
                            },
                        );
                    return Ok(());
                }
            } else if self
                .environments
                .scope(module_root)
                .loaded_modules
                .contains(&result)
            {
                return Ok(());
            }
        }
        if self.stack.len().saturating_sub(1)
            >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
        {
            return Err(self.native_error(
                NativeError::new("ModuleError", "maximum include depth exceeded"),
                span,
            ));
        }
        let text = (|| -> NativeResult<String> {
            let mut bytes = Vec::new();
            std::fs::File::open(&result)?
                .take((self.options.max_source_bytes as u64).saturating_add(1))
                .read_to_end(&mut bytes)?;
            if bytes.len() > self.options.max_source_bytes {
                return Err(NativeError::new(
                    "ModuleError",
                    "required file exceeds source byte limit",
                ));
            }
            String::from_utf8(bytes)
                .map_err(|_| NativeError::new("EncodingError", "required file is not UTF-8"))
        })()
        .map_err(|error| self.native_error(error, span))?;
        let isolated = if alias.is_some() {
            let previous = self.enter_scope(0, span)?;
            // Native functions use a dedicated fallback; no caller/global script
            // names or variable writes cross this lexical root.
            self.environments
                .scope_mut(self.environments.current)
                .parent = None;
            Some((previous, self.environments.current))
        } else {
            None
        };
        self.stack.push(StackFrame {
            function: if once { "require_once" } else { "require" }.into(),
            span,
            source: self.source.clone(),
        });
        let previous = self.source.replace(Arc::new(SourceFile {
            path: result.clone(),
            text,
        }));
        self.active_modules.push(result.clone());
        let execution = (|| {
            let program = self.parse_source(&self.source.as_ref().unwrap().text)?;
            // No extra block: declarations/assignments belong to the caller's scope.
            // Unlike the entry program, included declarations execute in source order.
            self.statements(&program.statements, output)
        })();
        self.active_modules.pop();
        self.source = previous;
        self.stack.pop();
        if let Some((previous, scope)) = isolated {
            if execution.is_ok() {
                // Even a module with no functions must remain a valid cached scope.
                self.environments.scope_mut(scope).captured = true;
                self.environments.module_cache.insert(result.clone(), scope);
            }
            self.environments.leave(previous);
            if execution.is_ok() {
                let alias = alias.expect("isolated load has an alias");
                self.environments
                    .scope_mut(self.environments.current)
                    .aliases
                    .insert(
                        alias.name.clone(),
                        environment::ModuleAlias {
                            path: result,
                            scope,
                        },
                    );
            }
        } else if execution.is_ok() {
            self.environments
                .scope_mut(module_root)
                .loaded_modules
                .insert(result);
        }
        execution
    }
}
