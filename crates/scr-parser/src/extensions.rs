use super::*;
use scr_ast::{ClassDeclaration, FieldDeclaration, InterfaceDeclaration};

impl Parser<'_> {
    pub(super) fn member_name(&mut self) -> Result<String, Diagnostic> {
        let word = match self.current().kind {
            TokenKind::True => "true",
            TokenKind::False => "false",
            TokenKind::Null => "null",
            TokenKind::Echo => "echo",
            TokenKind::If => "if",
            TokenKind::Elseif => "elseif",
            TokenKind::Else => "else",
            TokenKind::While => "while",
            TokenKind::For => "for",
            TokenKind::Foreach => "foreach",
            TokenKind::As => "as",
            TokenKind::Function => "function",
            TokenKind::Class => "class",
            TokenKind::Interface => "interface",
            TokenKind::Implements => "implements",
            TokenKind::Extends => "extends",
            TokenKind::New => "new",
            TokenKind::In => "in",
            TokenKind::Return => "return",
            TokenKind::Break => "break",
            TokenKind::Continue => "continue",
            TokenKind::Try => "try",
            TokenKind::Catch => "catch",
            TokenKind::Finally => "finally",
            TokenKind::Throw => "throw",
            TokenKind::Exit => "exit",
            TokenKind::Require => "require",
            TokenKind::RequireOnce => "require_once",
            _ => return self.name(),
        };
        self.pos += 1;
        Ok(word.into())
    }
    pub(super) fn class_declaration(&mut self) -> Result<ClassDeclaration, Diagnostic> {
        if self.depth != 1 || self.function_depth != 0 {
            return Err(Diagnostic::new(
                "class declarations must be at file top level",
                self.current().span,
            ));
        }
        self.pos += 1;
        let name = Binding {
            span: self.current().span,
            name: self.name()?,
        };
        if [
            "int", "float", "string", "bytes", "bool", "list", "map", "callable", "void",
        ]
        .contains(&name.name.as_str())
        {
            return Err(Diagnostic::new(
                "class name conflicts with a built-in type",
                name.span,
            ));
        }
        let parent = if self.take(TokenKind::Extends) {
            Some(self.type_annotation()?)
        } else {
            None
        };
        let mut interfaces = Vec::new();
        if self.take(TokenKind::Implements) {
            loop {
                interfaces.push(self.type_annotation()?);
                if !self.take(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::LeftBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        let mut names = std::collections::HashSet::new();
        while self.current().kind != TokenKind::RightBrace {
            if self.take(TokenKind::Function) {
                let method = self.function(true)?;
                if !names.insert(method.name.clone()) {
                    return Err(Diagnostic::new("duplicate class member", method.name_span));
                }
                methods.push(method);
            } else {
                let binding = self.binding()?;
                if !names.insert(binding.name.clone()) {
                    return Err(Diagnostic::new("duplicate class member", binding.span));
                }
                let annotation = if self.take(TokenKind::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                let default = if self.take(TokenKind::Assign) {
                    Some(self.expression(0)?)
                } else {
                    None
                };
                self.expect(TokenKind::Semicolon)?;
                fields.push(FieldDeclaration {
                    binding,
                    annotation,
                    default,
                });
            }
        }
        self.expect(TokenKind::RightBrace)?;
        Ok(ClassDeclaration {
            name,
            parent,
            interfaces,
            fields,
            methods,
        })
    }

    pub(super) fn interface_declaration(&mut self) -> Result<InterfaceDeclaration, Diagnostic> {
        if self.depth != 1 || self.function_depth != 0 {
            return Err(Diagnostic::new(
                "interface declarations must be at file top level",
                self.current().span,
            ));
        }
        self.pos += 1;
        let name = Binding {
            span: self.current().span,
            name: self.name()?,
        };
        if [
            "int", "float", "string", "bytes", "bool", "list", "map", "callable", "void",
        ]
        .contains(&name.name.as_str())
        {
            return Err(Diagnostic::new(
                "interface name conflicts with a built-in type",
                name.span,
            ));
        }
        self.expect(TokenKind::LeftBrace)?;
        let mut methods = Vec::new();
        let mut names = std::collections::HashSet::new();
        while self.current().kind != TokenKind::RightBrace {
            self.expect(TokenKind::Function)?;
            let saved = (self.expression_depth, self.expression_nodes);
            self.expression_depth = 0;
            self.expression_nodes = 0;
            let method = self.function_inner(true, true);
            (self.expression_depth, self.expression_nodes) = saved;
            let method = method?;
            if method.name == "__construct" || !names.insert(method.name.clone()) {
                return Err(Diagnostic::new(
                    "invalid or duplicate interface method",
                    method.name_span,
                ));
            }
            methods.push(method);
        }
        self.expect(TokenKind::RightBrace)?;
        Ok(InterfaceDeclaration { name, methods })
    }

    pub(super) fn new_expression(&mut self) -> Result<ExprKind, Diagnostic> {
        let first = self.name()?;
        let (namespace, name) = if self.take(TokenKind::StaticAccess) {
            (Some(first), self.name()?)
        } else {
            (None, first)
        };
        Ok(ExprKind::New {
            namespace,
            name,
            arguments: self.arguments()?,
        })
    }

    pub(super) fn collection_item(&mut self) -> Result<Expr, Diagnostic> {
        if self.take(TokenKind::Ellipsis) {
            let start = self.tokens[self.pos - 1].span.start;
            let value = self.expression(0)?;
            let span = Span::new(start, value.span.end);
            Ok(Expr {
                kind: ExprKind::Spread(Box::new(value)),
                span,
            })
        } else {
            self.expression(0)
        }
    }

    pub(super) fn comprehension(&mut self, value: Expr) -> Result<ExprKind, Diagnostic> {
        if matches!(value.kind, ExprKind::Spread(_)) {
            return Err(Diagnostic::new(
                "comprehension result cannot be a spread",
                value.span,
            ));
        }
        let binding = self.binding()?;
        self.expect(TokenKind::In)?;
        let iterable = Box::new(self.expression(0)?);
        let condition = if self.take(TokenKind::If) {
            Some(Box::new(self.expression(0)?))
        } else {
            None
        };
        self.expect(TokenKind::RightBracket)?;
        Ok(ExprKind::Comprehension {
            value: Box::new(value),
            binding,
            iterable,
            condition,
        })
    }

    pub(super) fn index_or_slice(&mut self, base: Expr) -> Result<ExprKind, Diagnostic> {
        if self.take(TokenKind::StaticAccess) {
            let step = if self.current().kind == TokenKind::RightBracket {
                None
            } else {
                Some(Box::new(self.slice_expression()?))
            };
            self.expect(TokenKind::RightBracket)?;
            return Ok(ExprKind::Slice {
                base: Box::new(base),
                start: None,
                stop: None,
                step,
            });
        }
        let start = if self.current().kind == TokenKind::Colon {
            None
        } else {
            Some(Box::new(self.slice_expression()?))
        };
        if self.take(TokenKind::StaticAccess) {
            let step = if self.current().kind == TokenKind::RightBracket {
                None
            } else {
                Some(Box::new(self.slice_expression()?))
            };
            self.expect(TokenKind::RightBracket)?;
            return Ok(ExprKind::Slice {
                base: Box::new(base),
                start,
                stop: None,
                step,
            });
        }
        if !self.take(TokenKind::Colon) {
            self.expect(TokenKind::RightBracket)?;
            return Ok(ExprKind::IndexAccess(Box::new(base), start.unwrap()));
        }
        let stop = if matches!(
            self.current().kind,
            TokenKind::Colon | TokenKind::RightBracket
        ) {
            None
        } else {
            Some(Box::new(self.slice_expression()?))
        };
        let step = if self.take(TokenKind::Colon) {
            if self.current().kind == TokenKind::RightBracket {
                None
            } else {
                Some(Box::new(self.slice_expression()?))
            }
        } else {
            None
        };
        self.expect(TokenKind::RightBracket)?;
        Ok(ExprKind::Slice {
            base: Box::new(base),
            start,
            stop,
            step,
        })
    }
    fn slice_expression(&mut self) -> Result<Expr, Diagnostic> {
        self.slice_bound_depth += 1;
        let result = self.expression(0);
        self.slice_bound_depth -= 1;
        result
    }
}
