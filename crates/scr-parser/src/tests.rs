use super::*;

fn expression(source: &str) -> Expr {
    parse_expression(&scr_lexer::lex(&format!("<?scr {source}")).unwrap()).unwrap()
}

fn binary(expr: &Expr, expected: BinaryOp) -> (&Expr, &Expr) {
    let ExprKind::Binary(left, op, right) = &expr.kind else {
        panic!("expected binary {expected:?}, got {expr:?}")
    };
    assert_eq!(*op, expected);
    (left, right)
}

#[test]
fn literals_variables_and_identifiers() {
    for (source, expected) in [
        ("null", ExprKind::Literal(Literal::Null)),
        ("true", ExprKind::Literal(Literal::Bool(true))),
        ("false", ExprKind::Literal(Literal::Bool(false))),
        ("42", ExprKind::Literal(Literal::Int(42))),
        ("1.25", ExprKind::Literal(Literal::Float(1.25))),
        ("1e2", ExprKind::Literal(Literal::Float(100.0))),
        (r#""é\n""#, ExprKind::Literal(Literal::String("é\n".into()))),
        ("$count", ExprKind::Variable("count".into())),
        ("str", ExprKind::Identifier("str".into())),
    ] {
        let expr = expression(source);
        assert_eq!(expr.kind, expected, "{source}");
        assert_eq!(expr.span, Span::new(6, 6 + source.len()));
    }
}

#[test]
fn arithmetic_concat_and_group_precedence() {
    let expr = expression(r#""Count: " . 1 + 2 * 3 - 4 / 2 % 2"#);
    let (_, arithmetic) = binary(&expr, BinaryOp::Concat);
    let (sum, remainder) = binary(arithmetic, BinaryOp::Subtract);
    let (_, product) = binary(sum, BinaryOp::Add);
    binary(product, BinaryOp::Multiply);
    let (quotient, _) = binary(remainder, BinaryOp::Remainder);
    binary(quotient, BinaryOp::Divide);

    let expr = expression("($count + 5) * 2");
    let (group, _) = binary(&expr, BinaryOp::Multiply);
    let ExprKind::Grouped(inner) = &group.kind else {
        panic!("expected group")
    };
    binary(inner, BinaryOp::Add);
    assert_eq!(group.span, Span::new(6, 18));
    assert_eq!(inner.span, Span::new(7, 17));
}

#[test]
fn every_binary_operator_is_left_associative() {
    for (spelling, op) in [
        ("||", BinaryOp::Or),
        ("&&", BinaryOp::And),
        ("===", BinaryOp::Equal),
        ("!==", BinaryOp::NotEqual),
        ("<", BinaryOp::Less),
        ("<=", BinaryOp::LessEqual),
        (">", BinaryOp::Greater),
        (">=", BinaryOp::GreaterEqual),
        (".", BinaryOp::Concat),
        ("+", BinaryOp::Add),
        ("-", BinaryOp::Subtract),
        ("*", BinaryOp::Multiply),
        ("/", BinaryOp::Divide),
        ("%", BinaryOp::Remainder),
    ] {
        let expr = expression(&format!("$a {spelling} $b {spelling} $c"));
        let (left, right) = binary(&expr, op);
        binary(left, op);
        assert_eq!(right.kind, ExprKind::Variable("c".into()));
    }
}

#[test]
fn precedence_levels_are_ordered() {
    let levels = [
        ("||", BinaryOp::Or),
        ("&&", BinaryOp::And),
        ("===", BinaryOp::Equal),
        ("<", BinaryOp::Less),
        (".", BinaryOp::Concat),
        ("+", BinaryOp::Add),
        ("*", BinaryOp::Multiply),
    ];
    for pair in levels.windows(2) {
        let [(low, low_op), (high, high_op)] = pair else {
            unreachable!()
        };
        let expr = expression(&format!("$a {low} $b {high} $c"));
        let (_, right) = binary(&expr, *low_op);
        binary(right, *high_op);
        let expr = expression(&format!("$a {high} $b {low} $c"));
        let (left, _) = binary(&expr, *low_op);
        binary(left, *high_op);
    }
}

#[test]
fn prefix_operators_bind_below_postfix_and_above_binary() {
    for (spelling, expected) in [
        ("+", UnaryOp::Positive),
        ("-", UnaryOp::Negative),
        ("!", UnaryOp::Not),
    ] {
        let expr = expression(&format!("{spelling}$f()[0]->value * 2"));
        let (left, _) = binary(&expr, BinaryOp::Multiply);
        let ExprKind::Unary(op, operand) = &left.kind else {
            panic!("expected unary")
        };
        assert_eq!(*op, expected);
        assert!(matches!(operand.kind, ExprKind::PropertyAccess(..)));
    }
    let expr = expression("!- +$x");
    let ExprKind::Unary(UnaryOp::Not, inner) = expr.kind else {
        panic!()
    };
    let ExprKind::Unary(UnaryOp::Negative, inner) = inner.kind else {
        panic!()
    };
    assert!(matches!(inner.kind, ExprKind::Unary(UnaryOp::Positive, _)));
}

#[test]
fn assignments_are_expressions_and_associate_right() {
    for (spelling, expected) in [
        ("=", AssignmentOp::Assign),
        ("+=", AssignmentOp::Add),
        ("-=", AssignmentOp::Subtract),
        ("*=", AssignmentOp::Multiply),
        ("/=", AssignmentOp::Divide),
        (".=", AssignmentOp::Concat),
    ] {
        let expr = expression(&format!("$a {spelling} $b = 1 || 2"));
        let ExprKind::Assignment(left, op, right) = expr.kind else {
            panic!()
        };
        assert_eq!(left.kind, ExprKind::Variable("a".into()));
        assert_eq!(op, expected);
        let ExprKind::Assignment(_, AssignmentOp::Assign, value) = right.kind else {
            panic!()
        };
        binary(&value, BinaryOp::Or);
    }
    for source in ["$a[0] = 1", "$a->b = 1", "$a->b[0] = 1", "f($a = 2)"] {
        expression(source);
    }
}

#[test]
fn calls_native_calls_and_postfix_chains() {
    let expr = expression(r#"Process::run("git", ["status"])[0]->name($x)(2)"#);
    let ExprKind::Call(callee, args) = expr.kind else {
        panic!()
    };
    assert_eq!(args.len(), 1);
    let ExprKind::Call(callee, args) = callee.kind else {
        panic!()
    };
    assert_eq!(args[0].kind, ExprKind::Variable("x".into()));
    let ExprKind::PropertyAccess(base, name) = callee.kind else {
        panic!()
    };
    assert_eq!(name, "name");
    let ExprKind::IndexAccess(base, index) = base.kind else {
        panic!()
    };
    assert_eq!(index.kind, ExprKind::Literal(Literal::Int(0)));
    let ExprKind::StaticCall {
        namespace,
        name,
        arguments,
    } = base.kind
    else {
        panic!()
    };
    assert_eq!(namespace.kind, ExprKind::Identifier("Process".into()));
    assert_eq!(namespace.span, Span::new(6, 13));
    assert_eq!(name, "run");
    assert_eq!(arguments.len(), 2);
    assert!(matches!(arguments[1].kind, ExprKind::List(_)));
    let expr = expression("str()");
    let ExprKind::Call(callee, args) = expr.kind else {
        panic!()
    };
    assert_eq!(callee.kind, ExprKind::Identifier("str".into()));
    assert_eq!(callee.span, Span::new(6, 9));
    assert!(args.is_empty());
}

#[test]
fn lists_maps_and_nesting() {
    assert_eq!(expression("[]").kind, ExprKind::List(vec![]));
    let expr = expression(r#"["host" => "localhost", "port" => 8080,]"#);
    let ExprKind::Map(entries) = expr.kind else {
        panic!()
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].0.kind,
        ExprKind::Literal(Literal::String("host".into()))
    );
    assert_eq!(entries[1].1.kind, ExprKind::Literal(Literal::Int(8080)));
    let expr = expression(r#"[1, ["key" => [true, null]],]"#);
    let ExprKind::List(items) = expr.kind else {
        panic!()
    };
    assert_eq!(items.len(), 2);
    assert!(matches!(items[1].kind, ExprKind::Map(_)));
}

#[test]
fn append_targets_are_write_only_and_keep_their_spans() {
    for source in [
        "$items[] = 1",
        "$groups[0][] = $other[] = 2",
        "$object->items[] = 3",
    ] {
        let expr = expression(source);
        let ExprKind::Assignment(target, AssignmentOp::Assign, _) = expr.kind else {
            panic!("expected assignment: {source}")
        };
        assert!(matches!(target.kind, ExprKind::AppendAccess(_)));
        assert_eq!(
            &source[..target.span.end - 6],
            source.split(" =").next().unwrap()
        );
    }
    for source in [
        "$items[]",
        "$items[] += 1",
        "$items[] .= 'x'",
        "$items[][] = 1",
        "$items[][0] = 1",
        "$items[]->name = 1",
        "f($items[])",
        "[1][] = 2",
        "make()[] = 2",
        "($items[]) = 1",
    ] {
        let tokens = scr_lexer::lex(&format!("<?scr {source}")).unwrap();
        let error = parse_expression(&tokens).unwrap_err();
        assert!(
            error.message.contains("list append target"),
            "{source}: {error:?}"
        );
    }
}

#[test]
fn function_literals_defaults_and_variadics_have_complete_spans() {
    let source = r#"function ($value: int = 2, ...$rest: int): int { return $value; }"#;
    let expr = expression(source);
    assert_eq!(expr.span, Span::new(6, source.len() + 6));
    let ExprKind::Function(function) = expr.kind else {
        panic!()
    };
    assert_eq!(function.name, "<anonymous>");
    assert_eq!(
        &source[function.name_span.start - 6..function.name_span.end - 6],
        "function"
    );
    assert!(function.parameters[0].default.is_some());
    assert!(!function.parameters[0].variadic);
    assert!(function.parameters[1].variadic);
    assert!(function.parameters[1].default.is_none());
    let span = function.parameters[0].span;
    assert_eq!(&source[span.start - 6..span.end - 6], "$value: int = 2");
    let span = function.parameters[1].span;
    assert_eq!(&source[span.start - 6..span.end - 6], "...$rest: int");
    for source in [
        "(function ($n = 2) { return $n; })()",
        "function ($f = function () { return 1; }) { return $f(); }",
        "[function () {}, function () {}]",
    ] {
        expression(source);
    }
}

#[test]
fn malformed_signatures_and_anonymous_control_flow_are_rejected() {
    for source in [
        "function ($a = 1, $b) {}",
        "function (...$a, $b) {}",
        "function (...$a = []) {}",
        "function (...$a, ...$b) {}",
        "function (...$a,) {}",
        "function ($a = ) {}",
        "function (...$a: ) {}",
        "function named() {}",
        "function () { break; }",
        "function () { continue; }",
        "function () { echo ; }",
        "function () { function f() { break; } }",
        "f(...$items)",
    ] {
        let tokens = scr_lexer::lex(&format!("<?scr {source}")).unwrap();
        assert!(parse_expression(&tokens).is_err(), "{source}");
    }
    let tokens = scr_lexer::lex("<?scr while (true) { $f = function () { break; }; }").unwrap();
    assert!(parse(&tokens).is_err());
}

#[test]
fn anonymous_function_bodies_have_independent_expression_budgets() {
    let source = format!("<?scr $f = function () {{ {} }};", "$x = 1;".repeat(300));
    parse(&scr_lexer::lex(&source).unwrap()).unwrap();
    let source = format!(
        "<?scr $f = {}0{};",
        "function ($x = ".repeat(80),
        ") {}".repeat(80)
    );
    let error = parse(&scr_lexer::lex(&source).unwrap()).unwrap_err();
    assert_eq!(error.code, "E0204");
}

#[test]
fn user_examples_parse_without_evaluation() {
    let source = r#"<?scr
$count = 10;
$name = "Scripta";
$total = ($count + 5) * 2;
$text = "Count: " . str($count);
$items = [1, 2, 3];
$config = ["host" => "localhost", "port" => 8080];
$value = $config["host"];
$result = Process::run("git", ["status"]);
$name = $file->name;
"#;
    let program = parse(&scr_lexer::lex(source).unwrap()).unwrap();
    assert_eq!(program.statements.len(), 9);
    for statement in program.statements {
        let StatementKind::Expression(expr) = statement.kind else {
            panic!()
        };
        assert!(matches!(expr.kind, ExprKind::Assignment(..)));
        assert!(source[expr.span.start..expr.span.end].starts_with('$'));
        assert_eq!(&source[expr.span.end..expr.span.end + 1], ";");
    }
}

#[test]
fn nested_spans_preserve_original_utf8_offsets_and_delimiters() {
    let source = "\u{feff}<?scr /* é */ ($x[\"é\"])->name;";
    let program = parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let StatementKind::Expression(expr) = &program.statements[0].kind else {
        panic!()
    };
    assert_eq!(&source[expr.span.start..expr.span.end], "($x[\"é\"])->name");
    let ExprKind::PropertyAccess(group, _) = &expr.kind else {
        panic!()
    };
    assert_eq!(&source[group.span.start..group.span.end], "($x[\"é\"])");
    let ExprKind::Grouped(index) = &group.kind else {
        panic!()
    };
    assert_eq!(&source[index.span.start..index.span.end], "$x[\"é\"]");
    let ExprKind::IndexAccess(variable, key) = &index.kind else {
        panic!()
    };
    assert_eq!(&source[variable.span.start..variable.span.end], "$x");
    assert_eq!(&source[key.span.start..key.span.end], "\"é\"");
}

#[test]
fn syntax_errors_have_useful_spans_and_messages() {
    for (source, message, offending) in [
        ("$a == $b", "use === or !==", "=="),
        ("$a != $b", "use === or !==", "!="),
        ("1 = 2", "invalid assignment target", "1"),
        ("f() = 2", "invalid assignment target", "f()"),
        ("[1][0] = 2", "invalid assignment target", "[1][0]"),
        ("($a) = 2", "invalid assignment target", "($a)"),
        ("$a + $b = 2", "invalid assignment target", "$a + $b"),
        ("$a->1", "expected identifier", "1"),
        ("$a::run()", "namespace identifier", "$a"),
        ("Process::run", "expected LeftParen", ""),
        ("$a[]", "list append target", "$a[]"),
        ("f(1,)", "expected expression", ")"),
        ("[1, 2 => 3]", "cannot mix", "=>"),
        ("[1 => 2, 3]", "expected FatArrow", "]"),
        ("[1 => ]", "expected expression", "]"),
        ("[1", "expected RightBracket", ""),
        ("(1", "expected RightParen", ""),
        ("1 +", "expected expression", ""),
        (
            "9223372036854775808",
            "integer literal out of range",
            "9223372036854775808",
        ),
        ("1e999", "floating-point literal out of range", "1e999"),
        ("1 2", "expected Eof", "2"),
    ] {
        let source = format!("<?scr {source}");
        let error = parse_expression(&scr_lexer::lex(&source).unwrap()).unwrap_err();
        assert!(error.message.contains(message), "{source}: {error}");
        assert_eq!(
            &source[error.span.start..error.span.end],
            offending,
            "{source}"
        );
    }
}

#[test]
fn incomplete_and_malformed_streams_are_errors() {
    assert!(parse(&[]).is_err());
    assert!(parse_expression(&[]).is_err());
    let eof = scr_lexer::lex("<?scr").unwrap();
    assert!(parse(&eof).unwrap().statements.is_empty());
    assert!(parse_expression(&eof).is_err());
    assert!(parse(&[eof[0].clone(), eof[0].clone()]).is_err());
    let mut tokens = scr_lexer::lex("<?scr 1").unwrap();
    tokens.pop();
    assert!(parse(&tokens).is_err());
    for source in [
        "<?scr $a =",
        "<?scr f(",
        "<?scr [",
        "<?scr $a->",
        "<?scr Process::",
        "<?scr 1",
    ] {
        assert!(parse(&scr_lexer::lex(source).unwrap()).is_err(), "{source}");
    }
}
