//! Syntax structures, independent of parsing and evaluation.
use scr_diagnostics::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Concat,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Positive,
    Negative,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentOp {
    Assign,
    Add,
    Subtract,
    Multiply,
    Divide,
    Concat,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    Variable(String),
    /// A callable/native name, distinct from a `$` variable.
    Identifier(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinaryOp, Box<Expr>),
    Assignment(Box<Expr>, AssignmentOp, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    StaticCall {
        namespace: Box<Expr>,
        name: String,
        arguments: Vec<Expr>,
    },
    PropertyAccess(Box<Expr>, String),
    IndexAccess(Box<Expr>, Box<Expr>),
    /// A write-only list append target, valid only with plain assignment.
    AppendAccess(Box<Expr>),
    List(Vec<Expr>),
    Map(Vec<(Expr, Expr)>),
    Grouped(Box<Expr>),
    Function(Box<FunctionDeclaration>),
}

/// A statement's span includes its semicolon or complete braced body.
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    Echo(Expr),
    Expression(Expr),
    Block(Block),
    If {
        branches: Vec<(Expr, Box<Statement>)>,
        else_branch: Option<Box<Statement>>,
    },
    While {
        condition: Expr,
        body: Box<Statement>,
    },
    For {
        initializer: Vec<Expr>,
        condition: Vec<Expr>,
        increment: Vec<Expr>,
        body: Box<Statement>,
    },
    Foreach {
        iterable: Expr,
        key: Option<Binding>,
        value: Binding,
        body: Box<Statement>,
    },
    Break,
    Continue,
    Function(FunctionDeclaration),
    Return(Option<Expr>),
    Try {
        body: Block,
        catches: Vec<CatchClause>,
        finally: Option<Block>,
    },
    Throw(Expr),
    Exit(Option<Expr>),
    Require {
        path: Expr,
        once: bool,
        alias: Option<Binding>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// Bound name; variable bindings omit the `$` prefix.
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeAnnotation {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    pub binding: Binding,
    pub annotation: Option<TypeAnnotation>,
    pub default: Option<Expr>,
    pub variadic: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FunctionDeclaration {
    pub name: String,
    pub name_span: Span,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CatchClause {
    pub binding: Binding,
    pub exception_type: Option<TypeAnnotation>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    pub statements: Vec<Statement>,
}

impl Expr {
    /// Visit direct expression children. Function signatures and bodies are
    /// separate scopes and are handled explicitly by callers.
    pub fn visit_children(&self, mut visit: impl FnMut(&Expr)) {
        match &self.kind {
            ExprKind::Unary(_, e)
            | ExprKind::Grouped(e)
            | ExprKind::AppendAccess(e)
            | ExprKind::PropertyAccess(e, _) => visit(e),
            ExprKind::Binary(a, _, b)
            | ExprKind::Assignment(a, _, b)
            | ExprKind::IndexAccess(a, b) => {
                visit(a);
                visit(b);
            }
            ExprKind::Call(callee, args) => {
                visit(callee);
                for arg in args {
                    visit(arg);
                }
            }
            ExprKind::StaticCall {
                namespace,
                arguments,
                ..
            } => {
                visit(namespace);
                for arg in arguments {
                    visit(arg);
                }
            }
            ExprKind::List(items) => {
                for item in items {
                    visit(item);
                }
            }
            ExprKind::Map(entries) => {
                for (key, value) in entries {
                    visit(key);
                    visit(value);
                }
            }
            _ => {}
        }
    }
}

impl Statement {
    /// Visit expressions directly owned by a statement, excluding nested
    /// statements and function signatures/bodies.
    pub fn visit_expressions(&self, mut visit: impl FnMut(&Expr)) {
        match &self.kind {
            StatementKind::Echo(e)
            | StatementKind::Expression(e)
            | StatementKind::Throw(e)
            | StatementKind::Require { path: e, .. } => visit(e),
            StatementKind::Return(Some(e)) | StatementKind::Exit(Some(e)) => visit(e),
            StatementKind::If { branches, .. } => {
                for (condition, _) in branches {
                    visit(condition);
                }
            }
            StatementKind::While { condition, .. } => visit(condition),
            StatementKind::For {
                initializer,
                condition,
                increment,
                ..
            } => {
                for e in initializer.iter().chain(condition).chain(increment) {
                    visit(e);
                }
            }
            StatementKind::Foreach { iterable, .. } => visit(iterable),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloned_tree_preserves_structure_and_independent_spans() {
        let variable = Expr {
            kind: ExprKind::Variable("x".into()),
            span: Span::new(1, 3),
        };
        let grouped = Expr {
            kind: ExprKind::Grouped(Box::new(variable.clone())),
            span: Span::new(0, 4),
        };
        assert_eq!(grouped.clone(), grouped);
        let ExprKind::Grouped(inner) = grouped.kind else {
            panic!("expected group")
        };
        assert_eq!(*inner, variable);
        assert_ne!(inner.span, grouped.span);
    }

    #[test]
    fn literals_and_names_remain_distinct() {
        assert_ne!(Literal::Int(1), Literal::Float(1.0));
        assert_ne!(Literal::Bool(false), Literal::Null);
        assert_ne!(
            ExprKind::Variable("str".into()),
            ExprKind::Identifier("str".into())
        );
    }
}
