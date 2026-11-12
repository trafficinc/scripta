//! Token-preserving formatter. Syntax is validated before any replacement is saved.
use scr_ast::{Block, Expr, ExprKind, FunctionDeclaration, Program, Statement, StatementKind};
use scr_diagnostics::{Diagnostic, Span};
use scr_lexer::TokenKind as K;
use std::{collections::HashSet, io::Write, path::Path};

#[derive(Default)]
struct Layout {
    starts: HashSet<usize>,
    bodies: Vec<Span>,
    expression_ends: HashSet<usize>,
}
impl Layout {
    fn block(&mut self, block: &Block) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }
    fn body(&mut self, statement: &Statement) {
        if let StatementKind::Block(block) = &statement.kind {
            self.block(block);
        } else {
            self.bodies.push(statement.span);
            self.statement(statement);
        }
    }
    fn statement(&mut self, statement: &Statement) {
        self.starts.insert(statement.span.start);
        statement.visit_expressions(|e| self.expression(e));
        match &statement.kind {
            StatementKind::Block(block) => self.block(block),
            StatementKind::Function(function) => self.function(function, false),
            StatementKind::If {
                branches,
                else_branch,
            } => {
                for (_, body) in branches {
                    self.body(body);
                }
                if let Some(body) = else_branch {
                    self.body(body);
                }
            }
            StatementKind::While { body, .. }
            | StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. } => self.body(body),
            StatementKind::Try {
                body,
                catches,
                finally,
            } => {
                self.block(body);
                for catch in catches {
                    self.block(&catch.body);
                }
                if let Some(body) = finally {
                    self.block(body);
                }
            }
            _ => {}
        }
    }
    fn function(&mut self, function: &FunctionDeclaration, expression: bool) {
        for parameter in &function.parameters {
            if let Some(default) = &parameter.default {
                self.expression(default);
            }
        }
        if expression {
            self.expression_ends.insert(function.body.span.end);
        }
        self.block(&function.body);
    }
    fn expression(&mut self, expression: &Expr) {
        if let ExprKind::Function(function) = &expression.kind {
            self.function(function, true);
        } else {
            expression.visit_children(|e| self.expression(e));
        }
    }
    fn extra(&self, offset: usize) -> usize {
        self.bodies
            .iter()
            .filter(|span| span.start <= offset && offset < span.end)
            .count()
    }
    fn new(program: &Program) -> Self {
        let mut layout = Self::default();
        for statement in &program.statements {
            layout.statement(statement);
        }
        layout
    }
}
#[derive(Default)]
struct Writer {
    text: String,
    pending: bool,
}
impl Writer {
    fn newline(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with('\n') {
            self.text.push('\n');
        }
        self.pending = false;
    }
    fn space(&mut self) {
        if !self.text.is_empty() && !self.text.ends_with(char::is_whitespace) {
            self.text.push(' ');
        }
    }
    fn raw(&mut self, text: &str, depth: usize) {
        if self.text.is_empty() || self.text.ends_with('\n') {
            self.text.push_str(&"    ".repeat(depth));
        }
        self.text.push_str(text);
    }
    fn gap(&mut self, source: &str, start: usize, end: usize, depth: usize) {
        let mut position = start;
        let mut newline = false;
        while position < end {
            let rest = &source[position..end];
            let first = rest.chars().next().unwrap();
            if first.is_whitespace() {
                newline |= first == '\r' || first == '\n';
                position += first.len_utf8();
                continue;
            }
            if first == '\u{feff}' {
                self.text.push(first);
                position += first.len_utf8();
                continue;
            }
            if rest.starts_with("<?scr") {
                self.raw("<?scr", 0);
                self.newline();
                position += 5;
                newline = false;
                continue;
            }
            if rest.starts_with("?>") {
                self.newline();
                self.raw("?>", 0);
                self.newline();
                position += 2;
                continue;
            }
            let block = rest.starts_with("/*");
            let length = if block {
                rest.find("*/").unwrap() + 2
            } else {
                rest.find(['\r', '\n']).unwrap_or(rest.len())
            };
            if newline {
                self.newline();
            } else if !self.text.ends_with('\u{feff}') {
                self.space();
            }
            self.raw(&rest[..length], depth);
            if !block {
                self.newline();
            }
            position += length;
            newline = false;
        }
    }
}
fn ends_expression(kind: &K) -> bool {
    matches!(
        kind,
        K::Variable(_)
            | K::Ident(_)
            | K::Int(_)
            | K::Float(_)
            | K::String(_)
            | K::True
            | K::False
            | K::Null
            | K::RightParen
            | K::RightBracket
    )
}
fn operator(kind: &K) -> bool {
    matches!(
        kind,
        K::Plus
            | K::Minus
            | K::Star
            | K::Slash
            | K::Percent
            | K::Dot
            | K::Assign
            | K::PlusAssign
            | K::MinusAssign
            | K::StarAssign
            | K::SlashAssign
            | K::DotAssign
            | K::StrictEqual
            | K::StrictNotEqual
            | K::Less
            | K::LessEqual
            | K::Greater
            | K::GreaterEqual
            | K::And
            | K::Or
            | K::FatArrow
    )
}
fn needs_space(previous: Option<&K>, current: &K, previous_unary: bool, unary: bool) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if matches!(
        current,
        K::RightParen
            | K::RightBracket
            | K::Semicolon
            | K::Comma
            | K::Colon
            | K::Arrow
            | K::StaticAccess
    ) {
        return false;
    }
    if matches!(
        previous,
        K::Arrow | K::StaticAccess | K::LeftParen | K::LeftBracket | K::Ellipsis
    ) {
        return false;
    }
    if matches!(current, K::LeftParen) {
        return matches!(
            previous,
            K::If | K::Elseif | K::While | K::For | K::Foreach | K::Catch | K::Function
        );
    }
    if matches!(current, K::LeftBracket) && ends_expression(previous) {
        return false;
    }
    if previous_unary {
        return matches!(
            (previous, current),
            (K::Plus | K::Minus, K::Plus | K::Minus)
        );
    }
    if unary {
        return operator(previous) || !matches!(previous, K::Not);
    }
    if operator(previous) || operator(current) {
        return true;
    }
    true
}
/// Return formatted source, or diagnostics; malformed input is never rewritten.
pub fn format(source: &str) -> Result<String, Vec<Diagnostic>> {
    let tokens = scr_lexer::lex(source).map_err(|error| vec![error.into()])?;
    let program = scr_parser::parse_with_diagnostics(&tokens)?;
    let layout = Layout::new(&program);
    let mut writer = Writer::default();
    let mut offset = 0;
    let mut braces = 0usize;
    let mut previous: Option<&K> = None;
    let mut previous_unary = false;
    for token in &tokens {
        writer.gap(
            source,
            offset,
            token.span.start,
            braces + layout.extra(token.span.start),
        );
        offset = token.span.end;
        if token.kind == K::Eof {
            break;
        }
        if token.kind == K::RightBrace {
            braces = braces.saturating_sub(1);
            writer.newline();
        }
        let join = matches!(token.kind, K::Else | K::Elseif | K::Catch | K::Finally)
            && matches!(previous, Some(K::RightBrace));
        if join && !writer.text.ends_with('\n') {
            writer.pending = false;
            writer.space();
        } else if writer.pending
            || layout.starts.contains(&token.span.start)
            || matches!(token.kind, K::Else | K::Elseif | K::Catch | K::Finally) && !join
        {
            writer.newline();
        }
        let unary = matches!(token.kind, K::Not)
            || matches!(token.kind, K::Plus | K::Minus) && !previous.is_some_and(ends_expression);
        if needs_space(previous, &token.kind, previous_unary, unary) {
            writer.space();
        }
        writer.raw(
            &source[token.span.start..token.span.end],
            braces + layout.extra(token.span.start),
        );
        match token.kind {
            K::LeftBrace => {
                braces += 1;
                writer.pending = true;
            }
            K::RightBrace => writer.pending = !layout.expression_ends.contains(&token.span.end),
            // Statement start hints handle semicolons without splitting for clauses.
            _ => {}
        }
        previous = Some(&token.kind);
        previous_unary = unary;
    }
    writer.newline();
    // Identical token kinds prove whitespace did not merge operators or change literals.
    let formatted_tokens = scr_lexer::lex(&writer.text).map_err(|error| {
        vec![
            Diagnostic::new(
                format!("formatter verification failed: {error}"),
                error.span,
            )
            .with_code("E0501"),
        ]
    })?;
    if !tokens
        .iter()
        .map(|t| &t.kind)
        .eq(formatted_tokens.iter().map(|t| &t.kind))
    {
        return Err(vec![
            Diagnostic::new("formatter would change script tokens", Span::default())
                .with_code("E0501"),
        ]);
    }
    Ok(writer.text)
}
/// Replace the canonical target atomically, preserving permissions and symlinks.
pub fn write_file(path: &Path, formatted: &str) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path)?;
    let metadata = std::fs::metadata(&target)?;
    let mut temporary = tempfile::NamedTempFile::new_in(target.parent().unwrap_or(Path::new(".")))?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.write_all(formatted.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(target).map_err(|error| error.error)?;
    Ok(())
}
