//! Bounded method inspection and emission; conditional declarations stay general.
use super::*;
use scr_ast::Statement;

struct Inspector<'a> {
    names: Vec<&'a str>,
    parameters: usize,
    locals: Vec<String>,
    fields: Vec<String>,
    nodes: usize,
    statements: usize,
    structured: bool,
}
impl<'a> Inspector<'a> {
    fn statement(&mut self, statement: &'a Statement, depth: usize, root: bool) -> Option<()> {
        self.statements += 1;
        if self.statements > STATEMENTS || depth > DEPTH {
            return None;
        }
        match &statement.kind {
            StatementKind::Expression(Expr {
                kind: ExprKind::Assignment(target, op, value),
                ..
            }) if *op != AssignmentOp::Concat => {
                if let ExprKind::Variable(name) = &target.kind {
                    let slot = self.names.iter().position(|n| *n == name);
                    if name == "this" || slot.is_some_and(|i| i < self.parameters) {
                        return None;
                    }
                    if slot.is_none()
                        && (!root || *op != AssignmentOp::Assign || self.names.len() == PARAMETERS)
                    {
                        return None;
                    }
                    inspect(value, &mut self.fields, &mut self.nodes, depth, &self.names)?;
                    if slot.is_none() {
                        self.names.push(name);
                        self.locals.push(name.clone());
                    }
                } else {
                    field(target, &mut self.fields)?;
                    inspect(value, &mut self.fields, &mut self.nodes, depth, &self.names)?;
                }
            }
            StatementKind::Return(value) => {
                if let Some(value) = value {
                    inspect(value, &mut self.fields, &mut self.nodes, depth, &self.names)?;
                }
            }
            StatementKind::Block(block) => {
                self.structured = true;
                for child in &block.statements {
                    self.statement(child, depth + 1, false)?;
                }
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                self.structured = true;
                for (condition, body) in branches {
                    inspect(
                        condition,
                        &mut self.fields,
                        &mut self.nodes,
                        depth,
                        &self.names,
                    )?;
                    self.statement(body, depth + 1, false)?;
                }
                if let Some(body) = else_branch {
                    self.statement(body, depth + 1, false)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

struct Emitter<'a, 'b> {
    names: &'a [&'b str],
    fields: &'a [&'b str],
    steps: Vec<Step>,
    flow: Vec<Flow>,
    structured: bool,
    instructions: usize,
}
impl Emitter<'_, '_> {
    fn expression(&mut self, expression: &Expr) -> Option<Expression> {
        let result = Expression::compile_inputs(expression, self.names, self.fields)?;
        self.instructions += result.instruction_count();
        (self.instructions <= NODES).then_some(result)
    }
    fn statement(&mut self, statement: &Statement) -> Option<()> {
        match &statement.kind {
            StatementKind::Expression(Expr {
                kind: ExprKind::Assignment(target, op, value),
                ..
            }) => {
                let value = self.expression(value)?;
                let step = match &target.kind {
                    ExprKind::Variable(name) => Step::Local {
                        slot: self.names.iter().position(|n| *n == name)?,
                        op: *op,
                        value,
                        span: statement.span,
                        target: target.span,
                    },
                    ExprKind::PropertyAccess(_, name) => Step::Assign {
                        field: self.fields.iter().position(|n| *n == name)?,
                        op: *op,
                        value,
                        span: statement.span,
                        target: target.span,
                    },
                    _ => unreachable!("inspected assignment target"),
                };
                self.action(step);
            }
            StatementKind::Return(value) => {
                let value = match value {
                    Some(value) => Some(self.expression(value)?),
                    None => None,
                };
                self.action(Step::Return {
                    value,
                    span: statement.span,
                });
            }
            StatementKind::Block(block) => {
                self.flow.push(Flow::Tick(statement.span));
                self.flow.push(Flow::Enter(block.span));
                for child in &block.statements {
                    self.statement(child)?;
                }
                self.flow.push(Flow::Leave);
            }
            StatementKind::If {
                branches,
                else_branch,
            } => {
                self.flow.push(Flow::Tick(statement.span));
                let mut ends = Vec::new();
                for (condition, body) in branches {
                    let value = self.expression(condition)?;
                    let guard = self.flow.len();
                    self.flow.push(Flow::Test {
                        value,
                        span: condition.span,
                        next: 0,
                    });
                    self.statement(body)?;
                    ends.push(self.flow.len());
                    self.flow.push(Flow::Jump(0));
                    let next = self.flow.len();
                    let Flow::Test {
                        next: destination, ..
                    } = &mut self.flow[guard]
                    else {
                        unreachable!()
                    };
                    *destination = next;
                }
                if let Some(body) = else_branch {
                    self.statement(body)?;
                }
                let end = self.flow.len();
                for jump in ends {
                    self.flow[jump] = Flow::Jump(end);
                }
            }
            _ => unreachable!("inspected method statement"),
        }
        (self.flow.len() <= STATEMENTS * 6).then_some(())
    }
    fn action(&mut self, step: Step) {
        if self.structured {
            self.flow.push(Flow::Action(self.steps.len()));
        }
        self.steps.push(step);
    }
}

pub(super) fn compile(declaration: &UserDeclaration) -> Option<Box<Plan>> {
    if declaration.parameters.len() > PARAMETERS
        || declaration
            .parameters
            .iter()
            .any(|p| p.default.is_some() || p.variadic)
    {
        return None;
    }
    let mut inspector = Inspector {
        names: declaration
            .parameters
            .iter()
            .map(|p| p.binding.name.as_str())
            .collect(),
        parameters: declaration.parameters.len(),
        locals: Vec::new(),
        fields: Vec::new(),
        nodes: 0,
        statements: 0,
        structured: false,
    };
    for statement in &declaration.body.statements {
        inspector.statement(statement, 0, true)?;
    }
    let fields: Vec<_> = inspector.fields.iter().map(String::as_str).collect();
    let mut emitter = Emitter {
        names: &inspector.names,
        fields: &fields,
        steps: Vec::new(),
        flow: Vec::new(),
        structured: inspector.structured,
        instructions: 0,
    };
    for statement in &declaration.body.statements {
        emitter.statement(statement)?;
    }
    Some(Box::new(Plan {
        steps: emitter.steps.into(),
        flow: inspector.structured.then(|| emitter.flow.into()),
        fields: inspector.fields.into(),
        locals: inspector.locals.into(),
        positions: RefCell::new(None),
    }))
}
