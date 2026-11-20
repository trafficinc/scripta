use std::{fmt, path::PathBuf, sync::Arc};

#[derive(Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: PathBuf,
    pub text: String,
}

use scr_diagnostics::{Diagnostic, SourcePosition, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub function: String,
    /// Call site, in the original source.
    pub span: Span,
    pub source: Option<Arc<SourceFile>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
    pub source: Option<Arc<SourceFile>>,
    pub code: &'static str,
    pub help: Option<String>,
    pub redacted_values: Arc<Vec<String>>,
    /// Outermost to innermost; rendering displays the innermost frame first.
    pub trace: Vec<StackFrame>,
}

impl RuntimeError {
    pub fn render(&self, path: &str, source: &str) -> String {
        let source_path = self
            .source
            .as_ref()
            .map(|s| s.path.to_string_lossy().into_owned());
        let error_path = source_path.as_deref().unwrap_or(path);
        let error_source = self.source.as_ref().map_or(source, |s| s.text.as_str());
        let mut diagnostic = Diagnostic::new(&self.message, self.span).with_code(self.code);
        diagnostic.help = self.help.clone();
        let mut text =
            diagnostic.render_with_secrets(error_path, error_source, &self.redacted_values);
        for frame in self.trace.iter().rev() {
            let frame_path = frame
                .source
                .as_ref()
                .map(|s| s.path.to_string_lossy().into_owned());
            let path = scr_diagnostics::redact_text(
                frame_path.as_deref().unwrap_or(path),
                &self.redacted_values,
            );
            let source = frame.source.as_ref().map_or(source, |s| s.text.as_str());
            let position = SourcePosition::at(source, frame.span.start);
            text.push_str(&format!(
                "\n  at {} ({path}:{}:{})",
                frame.function, position.line, position.column
            ));
        }
        text
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RuntimeError {}

impl From<Diagnostic> for RuntimeError {
    fn from(error: Diagnostic) -> Self {
        Self {
            message: error.message,
            span: error.span,
            trace: Vec::new(),
            source: None,
            code: error.code,
            help: error.help,
            redacted_values: Arc::default(),
        }
    }
}

impl From<RuntimeError> for Diagnostic {
    fn from(error: RuntimeError) -> Self {
        let mut diagnostic = Self::new(error.message, error.span).with_code(error.code);
        diagnostic.help = error.help;
        diagnostic
    }
}

impl fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceFile")
            .field(
                "path",
                &scr_diagnostics::redact_text(&self.path.to_string_lossy(), &[]),
            )
            .field("text_bytes", &self.text.len())
            .finish()
    }
}
impl fmt::Debug for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeError")
            .field("message", &self.message)
            .field("span", &self.span)
            .field("code", &self.code)
            .field("frames", &self.trace.len())
            .finish()
    }
}
