use scr_ast::{Block, Expr, ExprKind, FunctionDeclaration, Program, Statement, StatementKind};
use scr_diagnostics::Diagnostic;
use std::collections::HashSet;

pub(crate) fn validate(program: &Program) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    scope(&program.statements, &mut errors);
    errors
}
fn scope(statements: &[Statement], errors: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for statement in statements {
        if let StatementKind::Function(function) = &statement.kind {
            if !names.insert(&function.name) {
                errors.push(
                    Diagnostic::new(
                        format!("duplicate function declaration {}", function.name),
                        function.name_span,
                    )
                    .with_code("E0301")
                    .with_help("rename or remove the duplicate function in this scope"),
                );
            }
        }
        nested(statement, errors);
    }
}
fn block(body: &Block, errors: &mut Vec<Diagnostic>) {
    scope(&body.statements, errors);
}
fn nested(statement: &Statement, errors: &mut Vec<Diagnostic>) {
    statement.visit_expressions(|e| expression(e, errors));
    match &statement.kind {
        StatementKind::Function(declaration) => function(declaration, errors),
        StatementKind::Block(body) => block(body, errors),
        StatementKind::If {
            branches,
            else_branch,
        } => {
            for (_, body) in branches {
                nested(body, errors);
            }
            if let Some(body) = else_branch {
                nested(body, errors);
            }
        }
        StatementKind::While { body, .. }
        | StatementKind::For { body, .. }
        | StatementKind::Foreach { body, .. } => nested(body, errors),
        StatementKind::Try { body, catches } => {
            block(body, errors);
            for catch in catches {
                block(&catch.body, errors);
            }
        }
        _ => {}
    }
}

fn function(function: &FunctionDeclaration, errors: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for parameter in &function.parameters {
        if !names.insert(&parameter.binding.name) {
            errors.push(
                Diagnostic::new(
                    format!("duplicate parameter ${}", parameter.binding.name),
                    parameter.binding.span,
                )
                .with_code("E0302")
                .with_help("give each parameter a distinct name"),
            );
        }
        if let Some(default) = &parameter.default {
            expression(default, errors);
        }
    }
    block(&function.body, errors);
}

fn expression(expr: &Expr, errors: &mut Vec<Diagnostic>) {
    if let ExprKind::Function(declaration) = &expr.kind {
        function(declaration, errors);
    } else {
        expr.visit_children(|e| expression(e, errors));
    }
}
