use super::*;

fn program(source: &str) -> Program {
    parse_with_diagnostics(&scr_lexer::lex(&format!("<?scr {source}")).unwrap()).unwrap()
}

fn errors(source: &str) -> Vec<Diagnostic> {
    parse_with_diagnostics(&scr_lexer::lex(&format!("<?scr {source}")).unwrap()).unwrap_err()
}

#[test]
fn complete_example_and_optional_closing_tag() {
    let source = include_str!("../../../tests/fixtures/statements.scr");
    let parsed = parse_with_diagnostics(&scr_lexer::lex(source).unwrap()).unwrap();
    assert_eq!(parsed.statements.len(), 5);
    assert!(matches!(parsed.statements[0].kind, StatementKind::Echo(_)));
    let StatementKind::Function(function) = &parsed.statements[1].kind else {
        panic!()
    };
    assert_eq!(function.name, "add");
    assert_eq!(function.parameters.len(), 2);
    assert_eq!(function.parameters[0].binding.name, "a");
    assert_eq!(function.parameters[1].binding.name, "b");
    assert_eq!(
        function.parameters[0].annotation.as_ref().unwrap().name,
        "int"
    );
    assert_eq!(function.return_type.as_ref().unwrap().name, "int");
    assert!(matches!(
        function.body.statements[0].kind,
        StatementKind::Return(Some(_))
    ));
    assert!(matches!(
        parsed.statements[2].kind,
        StatementKind::Expression(_)
    ));
    let StatementKind::If {
        branches,
        else_branch,
    } = &parsed.statements[3].kind
    else {
        panic!()
    };
    assert_eq!(branches.len(), 1);
    assert!(else_branch.is_some());
    assert!(matches!(
        parsed.statements[4].kind,
        StatementKind::Foreach { .. }
    ));
    let without_tag = source.replace("?>", "");
    assert_eq!(
        parsed,
        parse(&scr_lexer::lex(&without_tag).unwrap()).unwrap()
    );
    assert!(program("?>").statements.is_empty());
    assert!(program("").statements.is_empty());
}

#[test]
fn echo_is_one_statement_kind_with_one_expression() {
    let parsed = program(r#"echo "hi"; echo("hi"); echo ("a") . "b";"#);
    assert_eq!(parsed.statements.len(), 3);
    let StatementKind::Echo(plain) = &parsed.statements[0].kind else {
        panic!()
    };
    let StatementKind::Echo(grouped) = &parsed.statements[1].kind else {
        panic!()
    };
    let ExprKind::Grouped(inner) = &grouped.kind else {
        panic!()
    };
    assert_eq!(plain.kind, inner.kind);
    assert!(matches!(parsed.statements[2].kind, StatementKind::Echo(_)));
    for source in [
        "echo;",
        "echo();",
        "echo(1, 2);",
        "echo 1, 2;",
        "$x = echo(1);",
    ] {
        errors(source);
    }
}

#[test]
fn if_elseif_else_and_nearest_if_binding() {
    let parsed = program("if ($a) {} elseif ($b) {} elseif ($c) {} else {}");
    let StatementKind::If {
        branches,
        else_branch,
    } = &parsed.statements[0].kind
    else {
        panic!()
    };
    assert_eq!(branches.len(), 3);
    assert!(else_branch.is_some());
    let parsed = program("if ($a) if ($b) echo 1; else echo 2;");
    let StatementKind::If {
        branches,
        else_branch,
    } = &parsed.statements[0].kind
    else {
        panic!()
    };
    assert!(else_branch.is_none());
    let StatementKind::If { else_branch, .. } = &branches[0].1.kind else {
        panic!()
    };
    assert!(else_branch.is_some());
    program("if ($a) {} else if ($b) {}");
}

#[test]
fn while_and_for_preserve_headers_and_bodies() {
    let parsed = program(
        "while ($x < 3) { $x += 1; continue; break; } for ($i = 0, $j = 2; $i < 3; $i += 1, $j -= 1) echo $i;",
    );
    let StatementKind::While { condition, body } = &parsed.statements[0].kind else {
        panic!()
    };
    assert!(matches!(
        condition.kind,
        ExprKind::Binary(_, BinaryOp::Less, _)
    ));
    let StatementKind::Block(block) = &body.kind else {
        panic!()
    };
    assert!(matches!(block.statements[1].kind, StatementKind::Continue));
    assert!(matches!(block.statements[2].kind, StatementKind::Break));
    let StatementKind::For {
        initializer,
        condition,
        increment,
        body,
    } = &parsed.statements[1].kind
    else {
        panic!()
    };
    assert_eq!(initializer.len(), 2);
    assert_eq!(condition.len(), 1);
    assert_eq!(increment.len(), 2);
    assert!(matches!(body.kind, StatementKind::Echo(_)));
    let parsed = program("for (;;) { break; } for (; $x = 1, $y < 2;) continue;");
    let StatementKind::For {
        initializer,
        condition,
        increment,
        ..
    } = &parsed.statements[0].kind
    else {
        panic!()
    };
    assert!(initializer.is_empty() && condition.is_empty() && increment.is_empty());
    let StatementKind::For { condition, .. } = &parsed.statements[1].kind else {
        panic!()
    };
    assert_eq!(condition.len(), 2);
}

#[test]
fn foreach_value_and_key_value_bindings() {
    let parsed =
        program("foreach ([1, 2] as $v) continue; foreach ($map as $key => $value) { break; }");
    let StatementKind::Foreach {
        iterable,
        key,
        value,
        ..
    } = &parsed.statements[0].kind
    else {
        panic!()
    };
    assert!(matches!(iterable.kind, ExprKind::List(_)));
    assert!(key.is_none());
    assert_eq!(value.name, "v");
    let StatementKind::Foreach { key, value, .. } = &parsed.statements[1].kind else {
        panic!()
    };
    assert_eq!(key.as_ref().unwrap().name, "key");
    assert_eq!(value.name, "value");
    for source in [
        "foreach ($xs $v) {}",
        "foreach ($xs as value) {}",
        "foreach ($xs as $k => 1) {}",
        "foreach ($xs as $v[0]) {}",
    ] {
        errors(source);
    }
}

#[test]
fn functions_optional_types_and_return_forms() {
    let parsed = program(
        "function f($a, $b: string): bool { if ($a) return true; return false; } function g() { return; } function h($x: Custom): void {}",
    );
    let StatementKind::Function(f) = &parsed.statements[0].kind else {
        panic!()
    };
    assert!(f.parameters[0].annotation.is_none());
    assert_eq!(f.parameters[1].annotation.as_ref().unwrap().name, "string");
    assert_eq!(f.return_type.as_ref().unwrap().name, "bool");
    let StatementKind::Function(g) = &parsed.statements[1].kind else {
        panic!()
    };
    assert!(g.parameters.is_empty());
    assert!(g.return_type.is_none());
    assert!(matches!(
        g.body.statements[0].kind,
        StatementKind::Return(None)
    ));
    let StatementKind::Function(h) = &parsed.statements[2].kind else {
        panic!()
    };
    assert_eq!(h.parameters[0].annotation.as_ref().unwrap().name, "Custom");
    program("function f(): null { return null; }");
    for source in [
        "function () {}",
        "function f(a) {}",
        "function f($a:) {}",
        "function f($a, ) {}",
        "function f(): {}",
        "function f();",
    ] {
        errors(source);
    }
}

#[test]
fn loop_and_function_contexts_are_lexical_and_restore_after_errors() {
    for (source, keyword) in [
        ("break;", "break"),
        ("continue;", "continue"),
        ("return;", "return"),
    ] {
        let diagnostics = errors(source);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("only valid inside"));
        assert_eq!(diagnostics[0].span, Span::new(6, 6 + keyword.len()));
    }
    let diagnostics =
        errors("while (true) { function f() { break; continue; return; } break; } return;");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
    assert!(diagnostics[0].message.contains("break"));
    assert!(diagnostics[1].message.contains("continue"));
    assert!(diagnostics[2].message.contains("return"));
    program(
        "function f() { while (true) { if (true) { return; continue; break; } } function g() { return; } return; }",
    );
    let diagnostics = errors("function f() echo 1; return; while (true) echo ; break; continue;");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("return is only"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("break is only"))
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("continue is only"))
    );
}

#[test]
fn try_catches_throw_and_exit() {
    let parsed = program(
        "try { throw $error; } catch (ProcessError $e) { echo str($e); } catch ($other) { throw $other; } exit; exit(); exit 1; exit(2);",
    );
    let StatementKind::Try { body, catches, .. } = &parsed.statements[0].kind else {
        panic!()
    };
    assert!(matches!(body.statements[0].kind, StatementKind::Throw(_)));
    assert_eq!(catches.len(), 2);
    assert_eq!(
        catches[0].exception_type.as_ref().unwrap().name,
        "ProcessError"
    );
    assert_eq!(catches[0].binding.name, "e");
    assert!(catches[1].exception_type.is_none());
    assert_eq!(catches[1].binding.name, "other");
    assert!(matches!(
        parsed.statements[1].kind,
        StatementKind::Exit(None)
    ));
    assert!(matches!(
        parsed.statements[2].kind,
        StatementKind::Exit(None)
    ));
    assert!(matches!(
        parsed.statements[3].kind,
        StatementKind::Exit(Some(_))
    ));
    assert!(matches!(
        parsed.statements[4].kind,
        StatementKind::Exit(Some(_))
    ));
    for source in [
        "try {}",
        "try echo 1; catch ($e) {}",
        "try {} catch () {}",
        "try {} catch (Error) {}",
        "try {} catch ($e) echo 1;",
        "throw;",
        "exit(1, 2);",
    ] {
        errors(source);
    }
}

#[test]
fn semicolons_required_and_blocks_do_not_consume_them() {
    for source in [
        "$x = 1",
        "echo 1",
        "echo(1)",
        "while (true) { break }",
        "while (true) { continue }",
        "function f() { return }",
        "throw $e",
        "exit",
        "exit()",
    ] {
        let diagnostics = errors(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("expected Semicolon")),
            "{source}: {diagnostics:?}"
        );
    }
    program(
        "{} if (true) {} elseif (false) {} else {} while (false) {} for (;;) {} foreach ([] as $v) {} function f() {} try {} catch ($e) {}",
    );
    // An extra semicolon is not an implicit empty statement.
    errors("{};");
    errors("if (true) {}; else {}");
    errors("for ($i = 0 $i < 2; $i += 1) {}");
    errors("for ($i = 0; $i < 2 $i += 1) {}");
}

#[test]
fn unmatched_delimiters_and_orphan_clauses_have_source_spans() {
    for (source, message, offending) in [
        ("}", "unmatched '}'", "}"),
        ("{ echo 1;", "unclosed block", "{"),
        ("while (true) {", "unclosed block", "{"),
        ("if true) {}", "expected LeftParen", "true"),
        ("if (true {}", "expected RightParen", "{"),
        ("echo(1;", "expected RightParen", ";"),
        ("function f($x {}", "expected RightParen", "{"),
        ("else {}", "unexpected clause", "else"),
        ("elseif (true) {}", "unexpected clause", "elseif"),
        ("catch ($e) {}", "unexpected clause", "catch"),
    ] {
        let diagnostics = errors(source);
        let first = &diagnostics[0];
        assert!(first.message.contains(message), "{source}: {diagnostics:?}");
        let full = format!("<?scr {source}");
        assert_eq!(&full[first.span.start..first.span.end], offending);
    }
}

#[test]
fn every_statement_annotation_and_block_preserves_utf8_spans() {
    let source = "\u{feff}<?scr /* é */ function f($x: int): string { return \"é\"; }";
    let parsed = parse(&scr_lexer::lex(source).unwrap()).unwrap();
    let statement = &parsed.statements[0];
    assert_eq!(
        &source[statement.span.start..statement.span.end],
        "function f($x: int): string { return \"é\"; }"
    );
    let StatementKind::Function(function) = &statement.kind else {
        panic!()
    };
    assert_eq!(
        &source[function.name_span.start..function.name_span.end],
        "f"
    );
    let param = &function.parameters[0];
    assert_eq!(&source[param.span.start..param.span.end], "$x: int");
    assert_eq!(
        &source[param.binding.span.start..param.binding.span.end],
        "$x"
    );
    let annotation = param.annotation.as_ref().unwrap();
    assert_eq!(&source[annotation.span.start..annotation.span.end], "int");
    let annotation = function.return_type.as_ref().unwrap();
    assert_eq!(
        &source[annotation.span.start..annotation.span.end],
        "string"
    );
    assert_eq!(
        &source[function.body.span.start..function.body.span.end],
        "{ return \"é\"; }"
    );
    let statement = &function.body.statements[0];
    assert_eq!(
        &source[statement.span.start..statement.span.end],
        "return \"é\";"
    );
    let parsed = program("try {} catch (Error $e) {}");
    let StatementKind::Try { catches, .. } = &parsed.statements[0].kind else {
        panic!()
    };
    let source = "<?scr try {} catch (Error $e) {}";
    assert_eq!(
        &source[catches[0].span.start..catches[0].span.end],
        "catch (Error $e) {}"
    );
}

#[test]
fn collects_independent_errors_including_legacy_equality() {
    let source = "<?scr echo ; $a == 1; $b != 2; break; continue; return; echo \"valid\";";
    let tokens = scr_lexer::lex(source).unwrap();
    let diagnostics = parse_with_diagnostics(&tokens).unwrap_err();
    assert_eq!(diagnostics.len(), 6, "{diagnostics:?}");
    assert_eq!(parse(&tokens).unwrap_err(), diagnostics[0]);
    for (diagnostic, text) in diagnostics
        .iter()
        .zip([";", "==", "!=", "break", "continue", "return"])
    {
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], text);
    }
    let mut parser = Parser::new(&tokens);
    let statements = parser.statements(false);
    assert_eq!(statements.len(), 1);
    assert!(matches!(statements[0].kind, StatementKind::Echo(_)));
}

#[test]
fn recovery_preserves_block_boundaries_and_following_statements() {
    let source = "<?scr while (true) { echo ; break; echo 1 echo 2; continue; } break; echo 3;";
    let tokens = scr_lexer::lex(source).unwrap();
    let mut parser = Parser::new(&tokens);
    let statements = parser.statements(false);
    assert_eq!(parser.errors.len(), 3, "{:?}", parser.errors);
    assert_eq!(statements.len(), 2);
    let StatementKind::While { body, .. } = &statements[0].kind else {
        panic!()
    };
    let StatementKind::Block(block) = &body.kind else {
        panic!()
    };
    assert_eq!(block.statements.len(), 3);
    assert!(matches!(block.statements[0].kind, StatementKind::Break));
    assert!(matches!(block.statements[1].kind, StatementKind::Echo(_)));
    assert!(matches!(block.statements[2].kind, StatementKind::Continue));
    // Missing semicolon before a brace must not swallow that brace or its else.
    let diagnostics = errors("if (true) { echo 1 } else { echo ; } echo ;");
    assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
}

#[test]
fn invalid_tokens_are_never_accepted_and_truncated_input_never_panics() {
    for source in [
        ";",
        ":",
        ")",
        "as;",
        "function",
        "while () {}",
        "echo 1; nonsense junk;",
        "echo 1; }",
        "echo 1; ?> junk",
    ] {
        match scr_lexer::lex(&format!("<?scr {source}")) {
            Ok(tokens) => assert!(parse_with_diagnostics(&tokens).is_err(), "{source}"),
            Err(_) => assert!(source.contains("?>")),
        }
    }
    let source = include_str!("../../../tests/fixtures/statements.scr");
    let tokens = scr_lexer::lex(source).unwrap();
    // Exercise every truncation boundary in a real nested program.
    for length in 0..tokens.len() {
        let mut prefix = tokens[..length].to_vec();
        let mut eof = tokens.last().unwrap().clone();
        let end = prefix.last().map_or(0, |token| token.span.end);
        eof.span = Span::new(end, end);
        prefix.push(eof);
        let _ = parse_with_diagnostics(&prefix);
    }
    assert!(parse_with_diagnostics(&[]).is_err());
    assert!(parse_with_diagnostics(&[tokens[0].clone()]).is_err());
    let eof = tokens.last().unwrap().clone();
    assert!(parse_with_diagnostics(&[eof.clone(), eof]).is_err());
}

#[test]
fn require_statements_parse_expressions_once_flags_and_semicolons() {
    let tokens =
        scr_lexer::lex(r#"<?scr require "lib/" . $file; require_once("config.scr");"#).unwrap();
    let program = crate::parse(&tokens).unwrap();
    assert!(matches!(
        program.statements[0].kind,
        StatementKind::Require { once: false, .. }
    ));
    assert!(matches!(
        program.statements[1].kind,
        StatementKind::Require { once: true, .. }
    ));
    for source in [
        "<?scr require;",
        "<?scr require_once",
        "<?scr require \"x.scr\"",
    ] {
        let tokens = scr_lexer::lex(source).unwrap();
        assert!(crate::parse(&tokens).is_err());
    }
}

#[test]
fn finally_parses_after_optional_catches_and_rejects_orphan_clauses() {
    for source in [
        "try {} finally {}",
        "try {} catch ($e) {} finally {}",
        "try {} catch (Error $e) {} catch ($e) {} finally {}",
    ] {
        let parsed = program(source);
        let StatementKind::Try { finally, .. } = &parsed.statements[0].kind else {
            panic!()
        };
        assert!(finally.is_some());
    }
    for body in [
        "try {}",
        "finally {}",
        "try {} finally echo 'x';",
        "try {} finally {} catch ($e) {}",
        "try {} finally {} finally {}",
        "try {} catch ($e) {} finally",
    ] {
        let source = format!("<?scr {body}");
        assert!(
            crate::parse(&scr_lexer::lex(&source).unwrap()).is_err(),
            "{body}"
        );
    }
}
