//! Recursive descent for statements and delimiters, with Pratt expression parsing.
use scr_ast::{
    AssignmentOp, BinaryOp, Binding, Block, CatchClause, Expr, ExprKind, FunctionDeclaration,
    Literal, Parameter, Program, Statement, StatementKind, TypeAnnotation, UnaryOp,
};
use scr_diagnostics::{Diagnostic, Span};
use scr_lexer::{Token, TokenKind};
mod extensions;

/// Parse a program, returning the first diagnostic for compatibility.
/// Use `parse_with_diagnostics` to collect all safely recoverable errors.
pub fn parse(tokens: &[Token]) -> Result<Program, Diagnostic> {
    parse_with_diagnostics(tokens).map_err(|errors| errors.into_iter().next().unwrap())
}

/// Parse all statements, synchronizing after errors at statement/block boundaries.
/// An erroneous program is never returned as a successful, partial AST.
pub fn parse_with_diagnostics(tokens: &[Token]) -> Result<Program, Vec<Diagnostic>> {
    validate(tokens).map_err(|error| vec![error])?;
    let mut parser = Parser::new(tokens);
    let statements = parser.statements(false);
    if parser.errors.is_empty() {
        Ok(Program { statements })
    } else {
        Err(parser.errors)
    }
}

/// Parse exactly one expression from a lexer stream (without a semicolon).
pub fn parse_expression(tokens: &[Token]) -> Result<Expr, Diagnostic> {
    validate(tokens)?;
    let mut parser = Parser::new(tokens);
    let expression = parser.expression(0)?;
    parser.expect(TokenKind::Eof)?;
    if let Some(error) = parser.errors.into_iter().next() {
        Err(error)
    } else {
        Ok(expression)
    }
}

fn validate(tokens: &[Token]) -> Result<(), Diagnostic> {
    if tokens.last().map(|t| &t.kind) != Some(&TokenKind::Eof)
        || tokens[..tokens.len().saturating_sub(1)]
            .iter()
            .any(|t| t.kind == TokenKind::Eof)
    {
        return Err(Diagnostic::new("invalid token stream", Span::default()));
    }
    Ok(())
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    loop_depth: usize,
    function_depth: usize,
    errors: Vec<Diagnostic>,
    depth: usize,
    expression_depth: usize,
    expression_nodes: usize,
    slice_bound_depth: usize,
}

impl Parser<'_> {
    fn new(tokens: &[Token]) -> Parser<'_> {
        Parser {
            tokens,
            pos: 0,
            loop_depth: 0,
            function_depth: 0,
            errors: Vec::new(),
            depth: 0,
            expression_depth: 0,
            expression_nodes: 0,
            slice_bound_depth: 0,
        }
    }

    fn statements(&mut self, in_block: bool) -> Vec<Statement> {
        let mut statements = Vec::new();
        while self.current().kind != TokenKind::Eof {
            if self.current().kind == TokenKind::RightBrace {
                if in_block {
                    break;
                }
                self.errors.push(Diagnostic::new(
                    "unmatched '}' without an opening '{'",
                    self.current().span,
                ));
                self.pos += 1;
                continue;
            }
            let start = self.pos;
            match self.statement() {
                Ok(statement) => statements.push(statement),
                Err(error) => {
                    self.errors.push(error);
                    self.synchronize(start);
                }
            }
        }
        statements
    }

    /// Discard only the remainder of a diagnosed statement. Preserve enclosing
    /// braces and the start of a following statement, and always make progress.
    fn synchronize(&mut self, start: usize) {
        if self.pos > start && self.tokens[self.pos - 1].kind == TokenKind::Semicolon {
            return;
        }
        let mut braces = 0usize;
        for token in &self.tokens[start..self.pos] {
            match token.kind {
                TokenKind::LeftBrace => braces += 1,
                TokenKind::RightBrace => braces = braces.saturating_sub(1),
                _ => {}
            }
        }
        while self.current().kind != TokenKind::Eof {
            if braces == 0 {
                if self.current().kind == TokenKind::RightBrace {
                    return;
                }
                if self.take(TokenKind::Semicolon) {
                    return;
                }
                if self.pos > start && statement_start(&self.current().kind) {
                    return;
                }
            }
            match self.current().kind {
                TokenKind::LeftBrace => braces += 1,
                TokenKind::RightBrace => {
                    braces -= 1;
                    self.pos += 1;
                    if braces == 0 {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn statement(&mut self) -> Result<Statement, Diagnostic> {
        self.enter_depth()?;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }
    fn enter_depth(&mut self) -> Result<(), Diagnostic> {
        if self.depth >= 64 {
            return Err(
                Diagnostic::new("syntax nesting exceeds 64 levels", self.current().span)
                    .with_code("E0204")
                    .with_help("split deeply nested code into smaller functions or statements"),
            );
        }
        self.depth += 1;
        Ok(())
    }
    fn count_expression(&mut self) -> Result<(), Diagnostic> {
        self.expression_nodes += 1;
        if self.expression_nodes > 256 {
            return Err(Diagnostic::new(
                "expression exceeds 256 syntax nodes",
                self.current().span,
            )
            .with_code("E0204")
            .with_help("split the expression using intermediate variables"));
        }
        Ok(())
    }
    fn statement_inner(&mut self) -> Result<Statement, Diagnostic> {
        let start = self.current().span.start;
        let keyword_span = self.current().span;
        let kind = match self.current().kind {
            TokenKind::Class => StatementKind::Class(self.class_declaration()?),
            TokenKind::Interface => StatementKind::Interface(self.interface_declaration()?),
            TokenKind::LeftBrace => StatementKind::Block(self.block()?),
            TokenKind::If => {
                self.pos += 1;
                let mut branches = vec![(self.condition()?, Box::new(self.statement()?))];
                while self.take(TokenKind::Elseif) {
                    branches.push((self.condition()?, Box::new(self.statement()?)));
                }
                let else_branch = if self.take(TokenKind::Else) {
                    Some(Box::new(self.statement()?))
                } else {
                    None
                };
                StatementKind::If {
                    branches,
                    else_branch,
                }
            }
            TokenKind::While => {
                self.pos += 1;
                StatementKind::While {
                    condition: self.condition()?,
                    body: self.loop_body()?,
                }
            }
            TokenKind::For => {
                self.pos += 1;
                self.expect(TokenKind::LeftParen)?;
                let initializer = self.expression_list(TokenKind::Semicolon)?;
                self.expect(TokenKind::Semicolon)?;
                let condition = self.expression_list(TokenKind::Semicolon)?;
                self.expect(TokenKind::Semicolon)?;
                let increment = self.expression_list(TokenKind::RightParen)?;
                self.expect(TokenKind::RightParen)?;
                StatementKind::For {
                    initializer,
                    condition,
                    increment,
                    body: self.loop_body()?,
                }
            }
            TokenKind::Foreach => {
                self.pos += 1;
                self.expect(TokenKind::LeftParen)?;
                let iterable = self.expression(0)?;
                self.expect(TokenKind::As)?;
                let first = self.binding()?;
                let (key, value) = if self.take(TokenKind::FatArrow) {
                    (Some(first), self.binding()?)
                } else {
                    (None, first)
                };
                self.expect(TokenKind::RightParen)?;
                StatementKind::Foreach {
                    iterable,
                    key,
                    value,
                    body: self.loop_body()?,
                }
            }
            TokenKind::Break | TokenKind::Continue => {
                let is_break = self.take(TokenKind::Break);
                if !is_break {
                    self.pos += 1;
                }
                self.expect(TokenKind::Semicolon)?;
                if self.loop_depth == 0 {
                    return Err(Diagnostic::new(
                        if is_break {
                            "break is only valid inside a loop"
                        } else {
                            "continue is only valid inside a loop"
                        },
                        keyword_span,
                    ));
                }
                if is_break {
                    StatementKind::Break
                } else {
                    StatementKind::Continue
                }
            }
            TokenKind::Function
                if matches!(
                    self.tokens.get(self.pos + 1).map(|t| &t.kind),
                    Some(TokenKind::Ident(_))
                ) =>
            {
                self.pos += 1;
                StatementKind::Function(self.function(true)?)
            }
            TokenKind::Return => {
                self.pos += 1;
                let value = self.optional_expression()?;
                self.expect(TokenKind::Semicolon)?;
                if self.function_depth == 0 {
                    return Err(Diagnostic::new(
                        "return is only valid inside a function",
                        keyword_span,
                    ));
                }
                StatementKind::Return(value)
            }
            TokenKind::Try => {
                self.pos += 1;
                let body = self.block()?;
                let mut catches = Vec::new();
                while self.current().kind == TokenKind::Catch {
                    let start = self.current().span.start;
                    self.pos += 1;
                    self.expect(TokenKind::LeftParen)?;
                    let exception_type =
                        if matches!(self.current().kind, TokenKind::Ident(_) | TokenKind::Null) {
                            Some(self.type_annotation()?)
                        } else {
                            None
                        };
                    let binding = self.binding()?;
                    self.expect(TokenKind::RightParen)?;
                    let body = self.block()?;
                    let span = Span::new(start, body.span.end);
                    catches.push(CatchClause {
                        binding,
                        exception_type,
                        body,
                        span,
                    });
                }
                let finally = if self.take(TokenKind::Finally) {
                    Some(self.block()?)
                } else {
                    None
                };
                if catches.is_empty() && finally.is_none() {
                    return Err(Diagnostic::new(
                        "expected catch or finally after try block",
                        self.current().span,
                    ));
                }
                StatementKind::Try {
                    body,
                    catches,
                    finally,
                }
            }
            TokenKind::Echo | TokenKind::Throw => {
                let echo = self.take(TokenKind::Echo);
                if !echo {
                    self.pos += 1;
                }
                // echo(...) is the same statement with a grouped expression.
                let value = self.expression(0)?;
                self.expect(TokenKind::Semicolon)?;
                if echo {
                    StatementKind::Echo(value)
                } else {
                    StatementKind::Throw(value)
                }
            }
            TokenKind::Require | TokenKind::RequireOnce => {
                let once = self.current().kind == TokenKind::RequireOnce;
                self.pos += 1;
                let path = self.expression(0)?;
                let alias = if self.take(TokenKind::As) {
                    let span = self.current().span;
                    Some(Binding {
                        name: self.name()?,
                        span,
                    })
                } else {
                    None
                };
                self.expect(TokenKind::Semicolon)?;
                StatementKind::Require { path, once, alias }
            }
            TokenKind::Exit => {
                self.pos += 1;
                let value = if self.take(TokenKind::LeftParen) {
                    let value = if self.current().kind == TokenKind::RightParen {
                        None
                    } else {
                        Some(self.expression(0)?)
                    };
                    self.expect(TokenKind::RightParen)?;
                    value
                } else {
                    self.optional_expression()?
                };
                self.expect(TokenKind::Semicolon)?;
                StatementKind::Exit(value)
            }
            TokenKind::Else | TokenKind::Elseif | TokenKind::Catch | TokenKind::Finally => {
                return Err(Diagnostic::new(
                    "unexpected clause; attach else/elseif to if or catch/finally to try",
                    keyword_span,
                ));
            }
            _ => {
                let expression = self.expression(0)?;
                self.expect(TokenKind::Semicolon)?;
                StatementKind::Expression(expression)
            }
        };
        Ok(Statement {
            kind,
            span: Span::new(start, self.tokens[self.pos - 1].span.end),
        })
    }

    fn block(&mut self) -> Result<Block, Diagnostic> {
        let opening = self.current().span;
        self.expect(TokenKind::LeftBrace)?;
        let statements = self.statements(true);
        if self.current().kind == TokenKind::Eof {
            return Err(Diagnostic::new(
                "unclosed block; expected '}' to match this '{'",
                opening,
            ));
        }
        let end = self.current().span.end;
        self.expect(TokenKind::RightBrace)?;
        Ok(Block {
            statements,
            span: Span::new(opening.start, end),
        })
    }

    fn loop_body(&mut self) -> Result<Box<Statement>, Diagnostic> {
        self.loop_depth += 1;
        let body = self.statement().map(Box::new);
        self.loop_depth -= 1;
        body
    }

    fn condition(&mut self) -> Result<Expr, Diagnostic> {
        self.expect(TokenKind::LeftParen)?;
        let expression = self.expression(0)?;
        self.expect(TokenKind::RightParen)?;
        Ok(expression)
    }

    fn expression_list(&mut self, end: TokenKind) -> Result<Vec<Expr>, Diagnostic> {
        let mut expressions = Vec::new();
        if self.current().kind != end {
            loop {
                expressions.push(self.expression(0)?);
                if !self.take(TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(expressions)
    }

    fn optional_expression(&mut self) -> Result<Option<Expr>, Diagnostic> {
        if matches!(
            self.current().kind,
            TokenKind::Semicolon | TokenKind::RightBrace | TokenKind::Eof
        ) {
            Ok(None)
        } else {
            self.expression(0).map(Some)
        }
    }

    fn binding(&mut self) -> Result<Binding, Diagnostic> {
        let token = self.current();
        if let TokenKind::Variable(name) = &token.kind {
            if name == "this" {
                return Err(Diagnostic::new(
                    "$this is reserved for method receivers",
                    token.span,
                ));
            }
            let binding = Binding {
                name: name.clone(),
                span: token.span,
            };
            self.pos += 1;
            Ok(binding)
        } else {
            Err(Diagnostic::new(
                "expected a variable binding such as $value",
                token.span,
            ))
        }
    }

    fn type_annotation(&mut self) -> Result<TypeAnnotation, Diagnostic> {
        let span = self.current().span;
        let mut name = if self.take(TokenKind::Null) {
            "null".into()
        } else {
            self.name()?
        };
        if self.take(TokenKind::StaticAccess) {
            name.push_str("::");
            name.push_str(&self.name()?);
        }
        Ok(TypeAnnotation { name, span })
    }

    fn function(&mut self, named: bool) -> Result<FunctionDeclaration, Diagnostic> {
        // A function's default expressions and body have their own expression
        // budgets, even when its literal occurs inside an enclosing expression.
        let saved = (self.expression_depth, self.expression_nodes);
        self.expression_depth = 0;
        self.expression_nodes = 0;
        let result = self.function_inner(named, false);
        (self.expression_depth, self.expression_nodes) = saved;
        result
    }

    fn function_inner(
        &mut self,
        named: bool,
        signature: bool,
    ) -> Result<FunctionDeclaration, Diagnostic> {
        let name_span = if named {
            self.current().span
        } else {
            self.tokens[self.pos - 1].span
        };
        let name = if named {
            self.name()?
        } else {
            "<anonymous>".into()
        };
        self.expect(TokenKind::LeftParen)?;
        let mut parameters = Vec::new();
        let mut optional = false;
        if self.current().kind != TokenKind::RightParen {
            loop {
                let start = self.current().span.start;
                let variadic = self.take(TokenKind::Ellipsis);
                let binding = self.binding()?;
                let annotation = if self.take(TokenKind::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                let default = if self.take(TokenKind::Assign) {
                    if variadic {
                        return Err(Diagnostic::new(
                            "variadic parameters cannot have defaults",
                            binding.span,
                        ));
                    }
                    optional = true;
                    Some(self.expression(0)?)
                } else {
                    if optional && !variadic {
                        return Err(Diagnostic::new(
                            "required parameters must precede default parameters",
                            binding.span,
                        ));
                    }
                    None
                };
                let span = Span::new(start, self.tokens[self.pos - 1].span.end);
                parameters.push(Parameter {
                    binding,
                    annotation,
                    default,
                    variadic,
                    span,
                });
                if !self.take(TokenKind::Comma) {
                    break;
                }
                if variadic {
                    return Err(Diagnostic::new("variadic parameter must be last", span));
                }
            }
        }
        self.expect(TokenKind::RightParen)?;
        let return_type = if self.take(TokenKind::Colon) {
            Some(self.type_annotation()?)
        } else {
            None
        };
        if signature {
            let span = self.current().span;
            self.expect(TokenKind::Semicolon)?;
            return Ok(FunctionDeclaration {
                name,
                name_span,
                parameters,
                return_type,
                body: Block {
                    statements: Vec::new(),
                    span,
                },
            });
        }
        let saved_loops = self.loop_depth;
        self.loop_depth = 0;
        self.function_depth += 1;
        let body = self.block();
        self.function_depth -= 1;
        self.loop_depth = saved_loops;
        Ok(FunctionDeclaration {
            name,
            name_span,
            parameters,
            return_type,
            body: body?,
        })
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn take(&mut self, kind: TokenKind) -> bool {
        if self.current().kind == kind {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<(), Diagnostic> {
        if !self.take(kind.clone()) {
            return Err(Diagnostic::new(
                match kind {
                    TokenKind::Semicolon => {
                        "expected Semicolon (';') after statement or for clause".into()
                    }
                    TokenKind::LeftParen => {
                        "expected LeftParen ('(') before condition or parameters".into()
                    }
                    TokenKind::RightParen => {
                        "expected RightParen (')') to close parentheses".into()
                    }
                    TokenKind::LeftBrace => "expected LeftBrace ('{') to start a block".into(),
                    _ => format!("expected {kind:?}"),
                },
                self.current().span,
            ));
        }
        Ok(())
    }

    fn expression(&mut self, minimum: u8) -> Result<Expr, Diagnostic> {
        self.enter_depth()?;
        if self.expression_depth == 0 {
            self.expression_nodes = 0;
        }
        self.expression_depth += 1;
        let result = self
            .count_expression()
            .and_then(|_| self.expression_inner(minimum));
        self.expression_depth -= 1;
        self.depth -= 1;
        result
    }
    fn expression_inner(&mut self, minimum: u8) -> Result<Expr, Diagnostic> {
        let mut left = self.primary()?;
        loop {
            if self.slice_bound_depth > 0
                && self.current().kind == TokenKind::StaticAccess
                && !matches!(left.kind, ExprKind::Identifier(_))
            {
                break;
            }
            // All postfix forms bind tighter than prefix and infix operators.
            if minimum <= 10
                && matches!(
                    self.current().kind,
                    TokenKind::LeftParen
                        | TokenKind::LeftBracket
                        | TokenKind::Arrow
                        | TokenKind::StaticAccess
                )
            {
                self.count_expression()?;
                left = self.postfix(left)?;
                continue;
            }
            let Some((precedence, operator)) = infix(&self.current().kind) else {
                break;
            };
            if precedence < minimum {
                break;
            }
            self.count_expression()?;
            self.pos += 1;
            if matches!(operator, Infix::Assignment(_)) && !assignable(&left) {
                return Err(Diagnostic::new(
                    "invalid assignment target; expected variable, property, or index",
                    left.span,
                ));
            }
            let operator_kind = self.tokens[self.pos - 1].kind.clone();
            let right = self
                .expression(if matches!(operator, Infix::Assignment(_)) {
                    precedence
                } else {
                    precedence + 1
                })
                .map_err(|error| {
                    if error.message == "expected expression" {
                        error.with_help(format!(
                            "provide a value after '{}'",
                            operator_spelling(&operator_kind)
                        ))
                    } else {
                        error
                    }
                })?;
            let span = Span::new(left.span.start, right.span.end);
            let kind = match operator {
                Infix::Binary(op) => ExprKind::Binary(Box::new(left), op, Box::new(right)),
                Infix::Assignment(op) => ExprKind::Assignment(Box::new(left), op, Box::new(right)),
            };
            left = Expr { kind, span };
        }
        Ok(left)
    }

    fn interpolated_string(
        &mut self,
        parts: Vec<scr_lexer::StringPart>,
        span: Span,
    ) -> Result<ExprKind, Diagnostic> {
        let mut expressions = Vec::new();
        for part in parts {
            match part {
                scr_lexer::StringPart::Text(text) => {
                    self.count_expression()?;
                    expressions.push(Expr {
                        kind: ExprKind::Literal(Literal::String(text)),
                        span,
                    });
                }
                scr_lexer::StringPart::Expression(tokens) => {
                    validate(&tokens)?;
                    let mut parser = Parser::new(&tokens);
                    parser.depth = self.depth;
                    parser.expression_depth = self.expression_depth;
                    parser.expression_nodes = self.expression_nodes;
                    parser.loop_depth = self.loop_depth;
                    parser.function_depth = self.function_depth;
                    let expression = parser.expression(0)?;
                    parser.expect(TokenKind::Eof)?;
                    if let Some(error) = parser.errors.into_iter().next() {
                        return Err(error);
                    }
                    self.expression_nodes = parser.expression_nodes;
                    expressions.push(expression);
                }
            }
        }
        Ok(ExprKind::InterpolatedString(expressions))
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.current().clone();
        if !matches!(
            token.kind,
            TokenKind::String(_)
                | TokenKind::InterpolatedString(_)
                | TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::Variable(_)
                | TokenKind::Ident(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::LeftParen
                | TokenKind::LeftBracket
                | TokenKind::Function
                | TokenKind::New
        ) {
            return Err(Diagnostic::new("expected expression", token.span)
                .with_help("provide a value, variable, or expression at this position"));
        }
        self.pos += 1;
        let kind = match token.kind {
            TokenKind::String(s) => ExprKind::Literal(Literal::String(s)),
            TokenKind::InterpolatedString(parts) => self.interpolated_string(parts, token.span)?,
            TokenKind::Int(n) => {
                ExprKind::Literal(Literal::Int(n.parse().map_err(|_| {
                    Diagnostic::new("integer literal out of range", token.span)
                })?))
            }
            TokenKind::Float(n) => {
                let value: f64 = n
                    .parse()
                    .map_err(|_| Diagnostic::new("invalid floating-point literal", token.span))?;
                if !value.is_finite() {
                    return Err(Diagnostic::new(
                        "floating-point literal out of range",
                        token.span,
                    ));
                }
                ExprKind::Literal(Literal::Float(value))
            }
            TokenKind::Variable(name) => ExprKind::Variable(name),
            TokenKind::Ident(name) => ExprKind::Identifier(name),
            TokenKind::True => ExprKind::Literal(Literal::Bool(true)),
            TokenKind::False => ExprKind::Literal(Literal::Bool(false)),
            TokenKind::Null => ExprKind::Literal(Literal::Null),
            TokenKind::Plus | TokenKind::Minus | TokenKind::Not => {
                let op = match token.kind {
                    TokenKind::Plus => UnaryOp::Positive,
                    TokenKind::Minus => UnaryOp::Negative,
                    _ => UnaryOp::Not,
                };
                ExprKind::Unary(op, Box::new(self.expression(9)?))
            }
            TokenKind::LeftParen => {
                let expr = self.expression(0)?;
                self.expect(TokenKind::RightParen)?;
                ExprKind::Grouped(Box::new(expr))
            }
            TokenKind::LeftBracket => self.collection()?,
            TokenKind::Function => ExprKind::Function(Box::new(self.function(false)?)),
            TokenKind::New => self.new_expression()?,
            _ => return Err(Diagnostic::new("expected expression", token.span)),
        };
        Ok(Expr {
            kind,
            span: Span::new(token.span.start, self.tokens[self.pos - 1].span.end),
        })
    }

    fn postfix(&mut self, left: Expr) -> Result<Expr, Diagnostic> {
        let start = left.span.start;
        let kind = match self.current().kind {
            TokenKind::LeftParen => ExprKind::Call(Box::new(left), self.arguments()?),
            TokenKind::LeftBracket => {
                self.pos += 1;
                if self.take(TokenKind::RightBracket) {
                    if self.current().kind != TokenKind::Assign || !assignable(&left) {
                        return Err(Diagnostic::new(
                            "empty [] is only valid as a list append target with '='",
                            Span::new(start, self.tokens[self.pos - 1].span.end),
                        ));
                    }
                    ExprKind::AppendAccess(Box::new(left))
                } else {
                    self.index_or_slice(left)?
                }
            }
            TokenKind::Arrow => {
                self.pos += 1;
                ExprKind::PropertyAccess(Box::new(left), self.member_name()?)
            }
            TokenKind::StaticAccess => {
                if !matches!(left.kind, ExprKind::Identifier(_)) {
                    return Err(Diagnostic::new(
                        "expected namespace identifier before ::",
                        left.span,
                    ));
                }
                self.pos += 1;
                let name = self.name()?;
                ExprKind::StaticCall {
                    namespace: Box::new(left),
                    name,
                    arguments: self.arguments()?,
                }
            }
            _ => unreachable!("postfix checked by Pratt loop"),
        };
        Ok(Expr {
            kind,
            span: Span::new(start, self.tokens[self.pos - 1].span.end),
        })
    }

    fn name(&mut self) -> Result<String, Diagnostic> {
        if let TokenKind::Ident(name) = self.current().kind.clone() {
            self.pos += 1;
            Ok(name)
        } else {
            Err(Diagnostic::new("expected identifier", self.current().span))
        }
    }

    fn arguments(&mut self) -> Result<Vec<Expr>, Diagnostic> {
        self.expect(TokenKind::LeftParen)?;
        let mut arguments = Vec::new();
        if self.current().kind != TokenKind::RightParen {
            loop {
                arguments.push(self.collection_item()?);
                if !self.take(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightParen)?;
        Ok(arguments)
    }

    fn collection(&mut self) -> Result<ExprKind, Diagnostic> {
        if self.take(TokenKind::RightBracket) {
            return Ok(ExprKind::List(Vec::new()));
        }
        let first = self.collection_item()?;
        if self.take(TokenKind::For) {
            return self.comprehension(first);
        }
        let map = self.take(TokenKind::FatArrow);
        let mut items = Vec::new();
        let mut entries = Vec::new();
        if map {
            entries.push((first, self.expression(0)?));
        } else {
            items.push(first);
        }
        while self.take(TokenKind::Comma) {
            if self.current().kind == TokenKind::RightBracket {
                break;
            }
            let item = self.collection_item()?;
            if map {
                self.expect(TokenKind::FatArrow)?;
                entries.push((item, self.expression(0)?));
            } else {
                if self.current().kind == TokenKind::FatArrow {
                    return Err(Diagnostic::new(
                        "cannot mix list elements and map entries",
                        self.current().span,
                    ));
                }
                items.push(item);
            }
        }
        self.expect(TokenKind::RightBracket)?;
        Ok(if map {
            ExprKind::Map(entries)
        } else {
            ExprKind::List(items)
        })
    }
}

fn operator_spelling(kind: &TokenKind) -> &'static str {
    use TokenKind::*;
    match kind {
        Or => "||",
        And => "&&",
        Equal => "==",
        StrictEqual => "===",
        NotEqual => "!=",
        StrictNotEqual => "!==",
        Less => "<",
        LessEqual => "<=",
        Greater => ">",
        GreaterEqual => ">=",
        Dot => ".",
        Plus => "+",
        Minus => "-",
        Star => "*",
        Slash => "/",
        Percent => "%",
        Assign => "=",
        PlusAssign => "+=",
        MinusAssign => "-=",
        StarAssign => "*=",
        SlashAssign => "/=",
        DotAssign => ".=",
        _ => "operator",
    }
}

fn statement_start(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::Echo
            | TokenKind::If
            | TokenKind::While
            | TokenKind::For
            | TokenKind::Foreach
            | TokenKind::Break
            | TokenKind::Continue
            | TokenKind::Function
            | TokenKind::Class
            | TokenKind::Interface
            | TokenKind::Return
            | TokenKind::Try
            | TokenKind::Throw
            | TokenKind::Exit
            | TokenKind::Require
            | TokenKind::RequireOnce
            | TokenKind::Variable(_)
    )
}

fn assignable(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Variable(_) | ExprKind::PropertyAccess(..) => true,
        ExprKind::IndexAccess(base, _) | ExprKind::AppendAccess(base) => assignable(base),
        ExprKind::List(items) => items.iter().enumerate().all(|(i, item)| match &item.kind {
            ExprKind::Variable(name) => name != "this",
            ExprKind::Spread(value) if i + 1 == items.len() => {
                matches!(&value.kind, ExprKind::Variable(name) if name != "this")
            }
            _ => false,
        }),
        _ => false,
    }
}

enum Infix {
    Binary(BinaryOp),
    Assignment(AssignmentOp),
}

/// Precedence from low to high; assignment uses equal left/right binding power.
fn infix(token: &TokenKind) -> Option<(u8, Infix)> {
    use AssignmentOp as A;
    use BinaryOp as B;
    use Infix::{Assignment, Binary};
    Some(match token {
        TokenKind::Assign => (1, Assignment(A::Assign)),
        TokenKind::PlusAssign => (1, Assignment(A::Add)),
        TokenKind::MinusAssign => (1, Assignment(A::Subtract)),
        TokenKind::StarAssign => (1, Assignment(A::Multiply)),
        TokenKind::SlashAssign => (1, Assignment(A::Divide)),
        TokenKind::DotAssign => (1, Assignment(A::Concat)),
        TokenKind::Or => (2, Binary(B::Or)),
        TokenKind::And => (3, Binary(B::And)),
        TokenKind::Equal | TokenKind::StrictEqual => (4, Binary(B::Equal)),
        TokenKind::NotEqual | TokenKind::StrictNotEqual => (4, Binary(B::NotEqual)),
        TokenKind::Less => (5, Binary(B::Less)),
        TokenKind::LessEqual => (5, Binary(B::LessEqual)),
        TokenKind::Greater => (5, Binary(B::Greater)),
        TokenKind::GreaterEqual => (5, Binary(B::GreaterEqual)),
        TokenKind::In => (5, Binary(B::In)),
        TokenKind::Dot => (6, Binary(B::Concat)),
        TokenKind::Plus => (7, Binary(B::Add)),
        TokenKind::Minus => (7, Binary(B::Subtract)),
        TokenKind::Star => (8, Binary(B::Multiply)),
        TokenKind::Slash => (8, Binary(B::Divide)),
        TokenKind::Percent => (8, Binary(B::Remainder)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod statement_tests;
