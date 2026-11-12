//! Source locations and errors shared by all interpreter phases.
use std::fmt;

/// Half-open UTF-8 byte range in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// UTF-8 byte offset and one-based Unicode-scalar line/column coordinates.
/// Tabs count as one column, CRLF is one newline, and a leading BOM has no width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

impl SourcePosition {
    pub fn at(source: &str, offset: usize) -> Self {
        let mut offset = offset.min(source.len());
        while !source.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut position = Self {
            offset,
            line: 1,
            column: 1,
        };
        let mut previous = None;
        for (index, c) in source[..offset].char_indices() {
            match c {
                '\r' => {
                    position.line += 1;
                    position.column = 1;
                }
                '\n' => {
                    if previous != Some('\r') {
                        position.line += 1;
                    }
                    position.column = 1;
                }
                '\u{feff}' if index == 0 => {}
                _ => position.column += 1,
            }
            previous = Some(c);
        }
        position
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub span: Span,
    pub code: &'static str,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            code: "E0201",
            help: None,
        }
    }

    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = code;
        self
    }
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn render(&self, path: &str, source: &str) -> String {
        self.render_with_secrets(path, source, &[])
    }
    pub fn render_with_secrets(&self, path: &str, source: &str, secrets: &[String]) -> String {
        let path = redact_text(path, secrets);
        let safe_source = mask_source(source);
        let original_source = source;
        let source = safe_source.as_str();
        let position = SourcePosition::at(original_source, self.span.start);
        let line = source.split(['\r', '\n']).next().unwrap_or("");
        // split on logical newlines, retaining empty final lines and CRLF semantics.
        let mut lines = source.split_inclusive(['\r', '\n']).peekable();
        let mut current = 1;
        let mut excerpt = line;
        while let Some(part) = lines.next() {
            if part.ends_with('\r') && lines.peek() == Some(&"\n") {
                lines.next();
            }
            if current == position.line {
                excerpt = part.trim_end_matches(['\r', '\n']);
                break;
            }
            current += 1;
            excerpt = "";
        }
        if position.line == 1 {
            excerpt = excerpt.strip_prefix('\u{feff}').unwrap_or(excerpt);
        }
        // Render tabs as one space to agree with SourcePosition's column convention.
        let mut excerpt = excerpt.replace('\t', " ");
        for secret in secrets.iter().filter(|s| !s.is_empty()) {
            excerpt = excerpt.replace(secret, &"*".repeat(secret.chars().count()));
        }
        let available = excerpt.chars().count().saturating_sub(position.column - 1);
        let end = SourcePosition::at(original_source, self.span.end.max(self.span.start));
        let width = if end.line == position.line {
            end.column.saturating_sub(position.column)
        } else {
            available
        };
        let width = width.min(available).max(1);
        let gutter = position.line.to_string().len();
        let mut text = format!(
            "error[{}]: {}\n {:>gutter$}--> {path}:{}:{}\n {:>gutter$}|\n{} | {}\n {:>gutter$}| {}{}",
            self.code,
            redact_text(&self.message, secrets),
            "",
            position.line,
            position.column,
            "",
            position.line,
            excerpt,
            "",
            " ".repeat(position.column - 1),
            "^".repeat(width)
        );
        if let Some(help) = &self.help {
            text.push_str(&format!("\nhelp: {}", redact_text(help, secrets)));
        }
        text
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_filename_line_and_character_column() {
        let source = "<?scr\n\"é\" $name";
        let start = source.find('$').unwrap();
        let error = Diagnostic::new("test error", Span::new(start, source.len()));
        let rendered = error.render("example.scr", source);
        assert!(rendered.contains("error[E0201]: test error"));
        assert!(rendered.contains("example.scr:2:5"));
        assert!(rendered.contains("2 | \"*\" $name"));
        assert_eq!(error.to_string(), "test error");
    }

    #[test]
    fn rendering_handles_external_invalid_offsets() {
        let error = Diagnostic::new("test", Span::new(1, usize::MAX));
        assert!(error.render("test.scr", "é").contains("test.scr:1:1"));
        let error = Diagnostic::new("test", Span::new(usize::MAX, usize::MAX));
        assert!(error.render("test.scr", "é").contains("test.scr:1:2"));
    }
}

#[cfg(test)]
mod rich_tests {
    use super::*;
    #[test]
    fn excerpts_follow_logical_lines_and_clamp_reversed_spans() {
        for source in ["<?scr\né @", "<?scr\r\né @", "<?scr\ré @"] {
            let offset = source.find('@').unwrap();
            let text = Diagnostic::new("unexpected token", Span::new(offset, offset + 1))
                .with_code("E0101")
                .with_help("remove @")
                .render("unicode.scr", source);
            assert!(text.contains("unicode.scr:2:3"));
            assert!(text.contains("2 | é @"));
            assert!(text.ends_with("help: remove @"));
        }
        assert!(
            Diagnostic::new("bad", Span::new(99, 0))
                .render("empty.scr", "")
                .contains("empty.scr:1:1")
        );
    }
}

/// Conservative credential-name detection shared by host diagnostics.
pub fn sensitive_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|key| lower.contains(key))
}
pub fn redact_text(value: &str, secrets: &[String]) -> String {
    let lower = value.to_ascii_lowercase();
    let shaped = [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|key| {
        lower.match_indices(key).any(|(offset, _)| {
            let tail = &lower[offset + key.len()..];
            let stripped =
                tail.trim_start_matches(|c: char| c.is_whitespace() || c == '\'' || c == '"');
            stripped.starts_with(['=', ':'])
                || tail.starts_with(char::is_whitespace) && !stripped.is_empty()
        })
    });
    if shaped
        || lower.contains("bearer ")
        || lower.contains("-----begin")
        || lower.contains("://") && lower.contains('@')
    {
        return "[REDACTED]".into();
    }
    let mut result = value.to_owned();
    let mut secrets: Vec<_> = secrets.iter().filter(|value| !value.is_empty()).collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in secrets {
        result = result.replace(secret, "[REDACTED]");
    }
    result
}
fn mask_source(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut quote = None;
    let mut escaped = false;
    let mut line = false;
    let mut block = false;
    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' {
            output.push(c);
            line = false;
            escaped = false;
            continue;
        }
        if line {
            output.push('*');
            continue;
        }
        if block {
            if c == '*' && chars.peek() == Some(&'/') {
                output.push_str("*/");
                chars.next();
                block = false;
            } else {
                output.push('*');
            }
            continue;
        }
        if let Some(q) = quote {
            if escaped {
                output.push('*');
                escaped = false;
            } else if c == q {
                output.push(c);
                quote = None;
            } else {
                output.push('*');
                escaped = c == '\\';
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                output.push(c);
            }
            '#' => {
                line = true;
                output.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                output.push_str("//");
                chars.next();
                line = true;
            }
            '/' if chars.peek() == Some(&'*') => {
                output.push_str("/*");
                chars.next();
                block = true;
            }
            _ => output.push(c),
        }
    }
    output
}
