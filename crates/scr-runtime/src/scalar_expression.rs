//! Read-only scalar expressions using the compact compiler's bounded operands.
use super::*;

pub(crate) struct Expression {
    code: Box<[Instruction]>,
    spans: Box<[Span]>,
    constants: Box<[Scalar]>,
}

impl Expression {
    pub fn scalar(value: &Value) -> bool {
        Scalar::from_value(value).is_some()
    }

    pub fn compile(expression: &Expr, binding: &str) -> Option<Self> {
        Self::compile_inputs(expression, &[binding], &[])
    }

    pub fn compile_inputs(expression: &Expr, bindings: &[&str], fields: &[&str]) -> Option<Self> {
        let mut compiler = Compiler {
            names: bindings.to_vec(),
            fields: fields.to_vec(),
            code: Vec::new(),
            spans: Vec::new(),
            constants: Vec::new(),
            calls: Vec::new(),
            nodes: 0,
        };
        compiler.expression(expression, 0)?;
        // Reject every effect, including local writes and calls compiled by
        // the more general function compiler, before evaluating anything.
        if compiler.code.iter().any(|i| {
            !matches!(
                i.code,
                Code::Constant
                    | Code::Local
                    | Code::Unary(_)
                    | Code::Binary(_)
                    | Code::Boolean
                    | Code::Short(_)
            )
        }) {
            return None;
        }
        Some(Self {
            code: compiler.code.into(),
            spans: compiler.spans.into(),
            constants: compiler.constants.into(),
        })
    }

    pub fn instruction_count(&self) -> usize {
        self.code.len()
    }

    pub fn evaluate(&self, runtime: &Runtime, input: &Value) -> Result<Value> {
        self.evaluate_inputs(runtime, std::slice::from_ref(input))
    }

    pub fn evaluate_inputs(&self, runtime: &Runtime, inputs: &[Value]) -> Result<Value> {
        self.evaluate_with::<false>(runtime, |index| {
            Scalar::from_value(&inputs[index]).expect("guarded scalar inputs")
        })
        .map(Scalar::value)
    }

    pub fn evaluate_scalars(&self, runtime: &Runtime, inputs: &[Scalar]) -> Result<Scalar> {
        self.evaluate_with::<true>(runtime, |index| inputs[index])
    }

    #[inline(always)]
    fn evaluate_with<const DIRECT_SCALARS: bool>(
        &self,
        runtime: &Runtime,
        input: impl Fn(usize) -> Scalar,
    ) -> Result<Scalar> {
        // A compiled literal or input read has no operations, errors or ticks.
        // Return it directly instead of initializing the bounded operand stack.
        if let [Instruction { code, operand }] = self.code.as_ref() {
            match code {
                Code::Constant => return Ok(self.constants[*operand as usize]),
                Code::Local => return Ok(input(*operand as usize)),
                _ => {}
            }
        }
        let mut values = [Scalar::Null; STACK];
        let mut top = 0;
        let mut ip = 0;
        while ip < self.code.len() {
            let Instruction { code, operand } = self.code[ip];
            let span = self.spans[ip];
            ip += 1;
            match code {
                Code::Constant | Code::Local => {
                    values[top] = if matches!(code, Code::Constant) {
                        self.constants[operand as usize]
                    } else {
                        input(operand as usize)
                    };
                    top += 1;
                }
                Code::Unary(op) => {
                    values[top - 1] = if DIRECT_SCALARS {
                        runtime.operation(values[top - 1].unary(op), span)?
                    } else {
                        Scalar::from_value(
                            &runtime.operation(ops::unary(op, values[top - 1].value()), span)?,
                        )
                        .unwrap()
                    };
                }
                Code::Binary(op) => {
                    top -= 1;
                    values[top - 1] = if DIRECT_SCALARS {
                        runtime.operation(values[top - 1].binary(op, values[top]), span)?
                    } else {
                        Scalar::from_value(&runtime.operation(
                            ops::binary(op, values[top - 1].value(), values[top].value()),
                            span,
                        )?)
                        .unwrap()
                    };
                }
                Code::Boolean => {
                    values[top - 1] = Scalar::Bool(runtime.operation(
                        if DIRECT_SCALARS {
                            values[top - 1].boolean()
                        } else {
                            ops::boolean(values[top - 1].value())
                        },
                        span,
                    )?);
                }
                Code::Short(stop) => {
                    let value = runtime.operation(
                        if DIRECT_SCALARS {
                            values[top - 1].boolean()
                        } else {
                            ops::boolean(values[top - 1].value())
                        },
                        span,
                    )?;
                    if value == stop {
                        values[top - 1] = Scalar::Bool(value);
                        ip = operand as usize;
                    } else {
                        top -= 1;
                    }
                }
                _ => unreachable!("read-only expression instructions"),
            }
        }
        debug_assert_eq!(top, 1);
        Ok(values[0])
    }
}
