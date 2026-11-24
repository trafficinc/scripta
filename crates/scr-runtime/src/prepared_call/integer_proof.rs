//! Inductive proof for the already bounded, pure self-recursive plan shape.
use super::*;

// Bounded compilation and entry-shape validation precede this pass. Parameters
// are immutable, self-call arity is exact, and the base/recursive return pair
// covers every successful exit. Assume completed self calls return int,
// then require both returns and every recursive argument to satisfy that claim.
// With checked outer integer arguments, induction over completed calls justifies
// omitting successful inner annotations. Arithmetic can still fail normally.
pub(super) fn proves(declaration: &UserDeclaration) -> bool {
    let [first, last] = declaration.body.statements.as_slice() else {
        return false;
    };
    let StatementKind::If {
        branches,
        else_branch: None,
    } = &first.kind
    else {
        return false;
    };
    let [(_, branch)] = branches.as_slice() else {
        return false;
    };
    [returned(branch), returned(last)]
        .into_iter()
        .all(|expression| expression.is_some_and(|e| integer(e, declaration)))
}

fn integer(expression: &Expr, declaration: &UserDeclaration) -> bool {
    match &expression.kind {
        ExprKind::Literal(Literal::Int(_)) => true,
        ExprKind::Variable(name) => declaration
            .parameters
            .iter()
            .any(|p| p.binding.name == *name),
        ExprKind::Grouped(inner)
        | ExprKind::Unary(UnaryOp::Positive | UnaryOp::Negative, inner) => {
            integer(inner, declaration)
        }
        ExprKind::Binary(
            left,
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder,
            right,
        ) => integer(left, declaration) && integer(right, declaration),
        ExprKind::Call(callee, arguments) => {
            matches!(&callee.kind, ExprKind::Identifier(name) if name == &declaration.name)
                && arguments.len() == declaration.parameters.len()
                && arguments
                    .iter()
                    .all(|argument| integer(argument, declaration))
        }
        _ => false,
    }
}
