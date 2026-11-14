//! Scripta lexical analysis. Tags and comments are consumed, never emitted.
//! Identifiers are ASCII; strings are UTF-8. Columns count Unicode scalars (tabs
//! count as one); LF, CRLF, and CR are line breaks. A leading BOM has no width.
use std::fmt;

use scr_diagnostics::Diagnostic;
pub use scr_diagnostics::{SourcePosition, Span};

/// Supported opening spellings, longest first so the canonical tag is not
/// consumed as the legacy prefix. Shared with token-preserving tooling.
pub const OPENING_TAGS: [&str; 2] = ["<?scripta", "<?scr"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Variable(String),
    Ident(String),
    Int(String),
    Float(String),
    String(String),
    True,
    False,
    Null,
    Echo,
    If,
    Elseif,
    Else,
    While,
    For,
    Foreach,
    As,
    Function,
    Return,
    Break,
    Continue,
    Try,
    Catch,
    Finally,
    Throw,
    Exit,
    Require,
    RequireOnce,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Dot,
    Ellipsis,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    DotAssign,
    /// Legacy operator: tokenized for a targeted parser diagnostic, never valid syntax.
    Equal,
    /// Legacy operator: tokenized for a targeted parser diagnostic, never valid syntax.
    NotEqual,
    StrictEqual,
    StrictNotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
    Not,
    Semicolon,
    Comma,
    Colon,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    FatArrow,
    Arrow,
    StaticAccess,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexerErrorKind {
    MissingOpeningTag,
    ContentAfterClosingTag,
    InvalidVariable,
    InvalidCharacter,
    InvalidOperator,
    MalformedNumber,
    InvalidEscape,
    UnterminatedString,
    UnterminatedComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexerError {
    pub kind: LexerErrorKind,
    pub message: String,
    pub span: Span,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

impl fmt::Display for LexerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LexerError {}

impl From<LexerError> for Diagnostic {
    fn from(error: LexerError) -> Self {
        let help = match error.kind {
            LexerErrorKind::MissingOpeningTag => {
                "start every script with <?scripta (legacy <?scr is also accepted)"
            }
            LexerErrorKind::ContentAfterClosingTag => {
                "move script content before ?> or omit the closing tag"
            }
            LexerErrorKind::InvalidOperator => {
                "use supported Scripta operators; equality uses === and !=="
            }
            LexerErrorKind::UnterminatedString => "close the string with its matching quote",
            LexerErrorKind::UnterminatedComment => "close the block comment with */",
            _ => "check the highlighted token and the supported Scripta syntax",
        };
        Diagnostic::new(error.message, error.span)
            .with_code("E0101")
            .with_help(help)
    }
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexerError> {
    Lexer::new(source).tokenize()
}

struct Lexer<'a> {
    source: &'a str,
    position: SourcePosition,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            position: SourcePosition {
                offset: 0,
                line: 1,
                column: 1,
            },
        }
    }

    fn rest(&self) -> &'a str {
        &self.source[self.position.offset..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        let offset = self.position.offset;
        self.position.offset += c.len_utf8();
        match c {
            '\r' => {
                self.position.line += 1;
                self.position.column = 1;
            }
            '\n' => {
                if offset == 0 || self.source.as_bytes()[offset - 1] != b'\r' {
                    self.position.line += 1;
                }
                self.position.column = 1;
            }
            '\u{feff}' if offset == 0 => {}
            _ => self.position.column += 1,
        }
        Some(c)
    }

    fn consume(&mut self, text: &str) {
        debug_assert!(self.rest().starts_with(text));
        for _ in text.chars() {
            self.advance();
        }
    }

    fn error(&self, kind: LexerErrorKind, start: SourcePosition, message: &str) -> LexerError {
        LexerError {
            kind,
            message: message.into(),
            span: Span::new(start.offset, self.position.offset),
            start,
            end: self.position,
        }
    }

    fn whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.advance();
        }
    }

    fn line_comment(&mut self) {
        while self.peek().is_some_and(|c| c != '\r' && c != '\n') {
            self.advance();
        }
    }

    fn preamble(&mut self) -> Result<(), LexerError> {
        if self.peek() == Some('\u{feff}') {
            self.advance();
        }
        if self.rest().starts_with("#!") {
            self.line_comment();
        }
        self.whitespace();
        let start = self.position;
        let Some(tag) = OPENING_TAGS
            .into_iter()
            .find(|tag| self.rest().starts_with(tag))
        else {
            return Err(self.error(
                LexerErrorKind::MissingOpeningTag,
                start,
                "expected opening tag <?scripta (legacy <?scr is also accepted)",
            ));
        };
        self.consume(tag);
        if self.peek().is_some_and(identifier_continue) {
            return Err(self.error(
                LexerErrorKind::MissingOpeningTag,
                start,
                "expected opening tag <?scripta (legacy <?scr is also accepted)",
            ));
        }
        Ok(())
    }

    fn trivia(&mut self) -> Result<(), LexerError> {
        loop {
            self.whitespace();
            if self.rest().starts_with("//") || self.rest().starts_with('#') {
                self.line_comment();
            } else if self.rest().starts_with("/*") {
                let start = self.position;
                self.consume("/*");
                while !self.rest().starts_with("*/") {
                    if self.advance().is_none() {
                        return Err(self.error(
                            LexerErrorKind::UnterminatedComment,
                            start,
                            "unterminated block comment",
                        ));
                    }
                }
                self.consume("*/");
            } else {
                return Ok(());
            }
        }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, LexerError> {
        self.preamble()?;
        let mut tokens = Vec::new();
        loop {
            self.trivia()?;
            if self.rest().starts_with("?>") {
                self.consume("?>");
                self.whitespace();
                if self.peek().is_some() {
                    let start = self.position;
                    self.advance();
                    return Err(self.error(
                        LexerErrorKind::ContentAfterClosingTag,
                        start,
                        "unexpected content after closing tag; only whitespace is allowed",
                    ));
                }
            }
            let start = self.position;
            let kind = match self.peek() {
                None => TokenKind::Eof,
                Some('$') => {
                    self.advance();
                    if !self.peek().is_some_and(identifier_start) {
                        return Err(self.error(
                            LexerErrorKind::InvalidVariable,
                            start,
                            "expected variable name after $",
                        ));
                    }
                    TokenKind::Variable(self.identifier())
                }
                Some(c) if identifier_start(c) => {
                    let word = self.identifier();
                    if matches!(word.as_str(), "and" | "or" | "xor") {
                        return Err(self.error(LexerErrorKind::InvalidOperator, start, "unsupported legacy PHP word operator; use && or ||, or an explicit boolean expression"));
                    }
                    keyword(word)
                }
                Some(c) if c.is_ascii_digit() => self.number(start)?,
                Some('"' | '\'') => self.string(start)?,
                Some(_) => self.operator(start)?,
            };
            let eof = kind == TokenKind::Eof;
            tokens.push(Token {
                kind,
                span: Span::new(start.offset, self.position.offset),
                start,
                end: self.position,
            });
            if eof {
                return Ok(tokens);
            }
        }
    }

    fn identifier(&mut self) -> String {
        let start = self.position.offset;
        while self.peek().is_some_and(identifier_continue) {
            self.advance();
        }
        self.source[start..self.position.offset].to_owned()
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
        }
    }

    fn number(&mut self, start: SourcePosition) -> Result<TokenKind, LexerError> {
        self.digits();
        let mut float = false;
        if self.rest().starts_with('.')
            && self
                .rest()
                .as_bytes()
                .get(1)
                .is_some_and(u8::is_ascii_digit)
        {
            float = true;
            self.advance();
            self.digits();
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            float = true;
            self.advance();
            if matches!(self.peek(), Some('+' | '-')) {
                self.advance();
            }
            let exponent = self.position.offset;
            self.digits();
            if exponent == self.position.offset {
                return Err(self.error(
                    LexerErrorKind::MalformedNumber,
                    start,
                    "malformed number: expected exponent digits",
                ));
            }
        }
        if self.peek().is_some_and(identifier_continue)
            || (float
                && self.rest().starts_with('.')
                && self
                    .rest()
                    .as_bytes()
                    .get(1)
                    .is_some_and(u8::is_ascii_digit))
        {
            while self
                .peek()
                .is_some_and(|c| identifier_continue(c) || c == '.')
            {
                self.advance();
            }
            return Err(self.error(
                LexerErrorKind::MalformedNumber,
                start,
                "malformed numeric literal",
            ));
        }
        let text = self.source[start.offset..self.position.offset].to_owned();
        Ok(if float {
            TokenKind::Float(text)
        } else {
            TokenKind::Int(text)
        })
    }

    fn string(&mut self, start: SourcePosition) -> Result<TokenKind, LexerError> {
        let quote = self.advance().expect("string starts at a quote");
        let mut value = String::new();
        while let Some(c) = self.advance() {
            if c == quote {
                return Ok(TokenKind::String(value));
            }
            if c != '\\' {
                value.push(c);
                continue;
            }
            let escape_start = SourcePosition {
                offset: self.position.offset - 1,
                line: self.position.line,
                column: self.position.column - 1,
            };
            let Some(escaped) = self.advance() else {
                break;
            };
            let decoded = match escaped {
                '\\' => '\\',
                '\'' => '\'',
                '"' if quote == '"' => '"',
                'n' if quote == '"' => '\n',
                'r' if quote == '"' => '\r',
                't' if quote == '"' => '\t',
                '0' if quote == '"' => '\0',
                '$' if quote == '"' => '$',
                _ => {
                    return Err(self.error(
                        LexerErrorKind::InvalidEscape,
                        escape_start,
                        "unsupported string escape",
                    ));
                }
            };
            value.push(decoded);
        }
        Err(self.error(
            LexerErrorKind::UnterminatedString,
            start,
            "unterminated string",
        ))
    }

    fn operator(&mut self, start: SourcePosition) -> Result<TokenKind, LexerError> {
        // Reject unsupported multi-character operators instead of silently splitting them.
        for spelling in [
            "<=>", "<>", "++", "--", "**", "%=", "<<", ">>", "??", "&=", "|=", "^=",
        ] {
            if self.rest().starts_with(spelling) {
                self.consume(spelling);
                return Err(self.error(
                    LexerErrorKind::InvalidOperator,
                    start,
                    "unsupported operator",
                ));
            }
        }
        for (spelling, kind) in [
            ("...", TokenKind::Ellipsis),
            ("===", TokenKind::StrictEqual),
            ("!==", TokenKind::StrictNotEqual),
            ("+=", TokenKind::PlusAssign),
            ("-=", TokenKind::MinusAssign),
            ("*=", TokenKind::StarAssign),
            ("/=", TokenKind::SlashAssign),
            (".=", TokenKind::DotAssign),
            ("==", TokenKind::Equal),
            ("!=", TokenKind::NotEqual),
            ("<=", TokenKind::LessEqual),
            (">=", TokenKind::GreaterEqual),
            ("&&", TokenKind::And),
            ("||", TokenKind::Or),
            ("=>", TokenKind::FatArrow),
            ("->", TokenKind::Arrow),
            ("::", TokenKind::StaticAccess),
        ] {
            if self.rest().starts_with(spelling) {
                self.consume(spelling);
                return Ok(kind);
            }
        }
        let c = self.advance().expect("operator starts before EOF");
        Ok(match c {
            '+' => TokenKind::Plus,
            '-' => TokenKind::Minus,
            '*' => TokenKind::Star,
            '/' => TokenKind::Slash,
            '%' => TokenKind::Percent,
            '.' => TokenKind::Dot,
            '=' => TokenKind::Assign,
            '<' => TokenKind::Less,
            '>' => TokenKind::Greater,
            '!' => TokenKind::Not,
            ';' => TokenKind::Semicolon,
            ',' => TokenKind::Comma,
            ':' => TokenKind::Colon,
            '(' => TokenKind::LeftParen,
            ')' => TokenKind::RightParen,
            '{' => TokenKind::LeftBrace,
            '}' => TokenKind::RightBrace,
            '[' => TokenKind::LeftBracket,
            ']' => TokenKind::RightBracket,
            '&' | '|' | '^' | '~' | '?' => {
                return Err(self.error(
                    LexerErrorKind::InvalidOperator,
                    start,
                    "unsupported operator",
                ));
            }
            _ => {
                return Err(self.error(
                    LexerErrorKind::InvalidCharacter,
                    start,
                    "unexpected character",
                ));
            }
        })
    }
}

fn identifier_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn identifier_continue(c: char) -> bool {
    identifier_start(c) || c.is_ascii_digit()
}

fn keyword(name: String) -> TokenKind {
    match name.as_str() {
        "true" => TokenKind::True,
        "false" => TokenKind::False,
        "null" => TokenKind::Null,
        "echo" => TokenKind::Echo,
        "if" => TokenKind::If,
        "elseif" => TokenKind::Elseif,
        "else" => TokenKind::Else,
        "while" => TokenKind::While,
        "for" => TokenKind::For,
        "foreach" => TokenKind::Foreach,
        "as" => TokenKind::As,
        "function" => TokenKind::Function,
        "return" => TokenKind::Return,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "try" => TokenKind::Try,
        "catch" => TokenKind::Catch,
        "finally" => TokenKind::Finally,
        "throw" => TokenKind::Throw,
        "exit" => TokenKind::Exit,
        "require" => TokenKind::Require,
        "require_once" => TokenKind::RequireOnce,
        _ => TokenKind::Ident(name),
    }
}

#[cfg(test)]
mod tests;
