use super::*;

fn kinds(body: &str) -> Vec<TokenKind> {
    lex(&format!("<?scr {body}"))
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

fn fails(source: &str, kind: LexerErrorKind) -> LexerError {
    let error = lex(source).unwrap_err();
    assert_eq!(error.kind, kind, "{source:?}: {error}");
    assert!(error.span.start <= error.span.end);
    assert!(error.span.end <= source.len());
    assert!(source.is_char_boundary(error.span.start));
    assert!(source.is_char_boundary(error.span.end));
    assert_eq!(error.start, SourcePosition::at(source, error.span.start));
    assert_eq!(error.end, SourcePosition::at(source, error.span.end));
    error
}

#[test]
fn every_keyword_and_keyword_boundaries() {
    for (spelling, expected) in [
        ("true", TokenKind::True),
        ("false", TokenKind::False),
        ("null", TokenKind::Null),
        ("echo", TokenKind::Echo),
        ("if", TokenKind::If),
        ("elseif", TokenKind::Elseif),
        ("else", TokenKind::Else),
        ("while", TokenKind::While),
        ("for", TokenKind::For),
        ("foreach", TokenKind::Foreach),
        ("as", TokenKind::As),
        ("function", TokenKind::Function),
        ("return", TokenKind::Return),
        ("break", TokenKind::Break),
        ("continue", TokenKind::Continue),
        ("try", TokenKind::Try),
        ("catch", TokenKind::Catch),
        ("throw", TokenKind::Throw),
        ("exit", TokenKind::Exit),
    ] {
        assert_eq!(kinds(spelling), vec![expected, TokenKind::Eof]);
        for name in [
            format!("{spelling}_x"),
            format!("{spelling}1"),
            spelling.to_uppercase(),
        ] {
            assert_eq!(kinds(&name), vec![TokenKind::Ident(name), TokenKind::Eof]);
        }
        assert_eq!(
            kinds(&format!("${spelling}")),
            vec![TokenKind::Variable(spelling.into()), TokenKind::Eof]
        );
    }
}

#[test]
fn every_operator_and_punctuation() {
    for (spelling, expected) in [
        ("+", TokenKind::Plus),
        ("-", TokenKind::Minus),
        ("*", TokenKind::Star),
        ("/", TokenKind::Slash),
        ("%", TokenKind::Percent),
        (".", TokenKind::Dot),
        ("=", TokenKind::Assign),
        ("+=", TokenKind::PlusAssign),
        ("-=", TokenKind::MinusAssign),
        ("*=", TokenKind::StarAssign),
        ("/=", TokenKind::SlashAssign),
        (".=", TokenKind::DotAssign),
        ("==", TokenKind::Equal),
        ("!=", TokenKind::NotEqual),
        ("===", TokenKind::StrictEqual),
        ("!==", TokenKind::StrictNotEqual),
        ("<", TokenKind::Less),
        ("<=", TokenKind::LessEqual),
        (">", TokenKind::Greater),
        (">=", TokenKind::GreaterEqual),
        ("&&", TokenKind::And),
        ("||", TokenKind::Or),
        ("!", TokenKind::Not),
        (";", TokenKind::Semicolon),
        (",", TokenKind::Comma),
        (":", TokenKind::Colon),
        ("(", TokenKind::LeftParen),
        (")", TokenKind::RightParen),
        ("{", TokenKind::LeftBrace),
        ("}", TokenKind::RightBrace),
        ("[", TokenKind::LeftBracket),
        ("]", TokenKind::RightBracket),
        ("=>", TokenKind::FatArrow),
        ("->", TokenKind::Arrow),
        ("::", TokenKind::StaticAccess),
    ] {
        let source = format!("<?scr {spelling}");
        let tokens = lex(&source).unwrap();
        assert_eq!(tokens[0].kind, expected, "{spelling}");
        assert_eq!(tokens[0].span, Span::new(6, source.len()));
        assert_eq!(tokens[1].kind, TokenKind::Eof);
    }
}

#[test]
fn missing_tag_and_executable_preamble() {
    for source in [
        "",
        "echo 1;",
        "<?php echo 1;",
        "<?script",
        "text <?scr",
        "?>",
        "// comment\n<?scr",
    ] {
        fails(source, LexerErrorKind::MissingOpeningTag);
    }
}

#[test]
fn bom_shebang_and_opening_tag() {
    for preamble in [
        "",
        "\u{feff}",
        "#!/usr/bin/env scr\n",
        "\u{feff}#!/usr/bin/env scr\r\n",
        " \n\t",
    ] {
        let source = format!("{preamble}<?scr/* comment */echo 'ok';");
        let tokens = lex(&source).unwrap();
        assert_eq!(tokens[0].kind, TokenKind::Echo);
        assert_eq!(tokens[0].span.start, source.find("echo").unwrap());
        assert_eq!(
            tokens[0].start,
            SourcePosition::at(&source, tokens[0].span.start)
        );
    }
    fails("#!/usr/bin/env scr", LexerErrorKind::MissingOpeningTag);
    fails(
        " \n#!/usr/bin/env scr\n<?scr",
        LexerErrorKind::MissingOpeningTag,
    );
    fails("\u{feff}\u{feff}<?scr", LexerErrorKind::MissingOpeningTag);
}

#[test]
fn closing_tag_must_be_last() {
    for source in ["<?scr", "<?scr?>", "<?scr ?> \r\n\t", "<?scr // eof"] {
        let tokens = lex(source).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].kind, TokenKind::Eof);
        assert_eq!(tokens[0].span, Span::new(source.len(), source.len()));
    }
    for tail in [
        "echo 'x';",
        "<html>",
        "// comment",
        "# comment",
        "/*comment*/",
        "<?scr",
        "?>",
    ] {
        let source = format!("<?scr ?> \n{tail}");
        let error = fails(&source, LexerErrorKind::ContentAfterClosingTag);
        assert_eq!(error.start.line, 2);
        assert_eq!(error.start.column, 1);
    }
}

#[test]
fn comments_are_trivia_and_tags_inside_comments_are_inert() {
    assert_eq!(
        kinds("// ?>\r\n# ?>\r/* ?> <?scr */echo/*middle*/\"ok\";"),
        vec![
            TokenKind::Echo,
            TokenKind::String("ok".into()),
            TokenKind::Semicolon,
            TokenKind::Eof
        ]
    );
    assert_eq!(kinds("/**/ // last"), vec![TokenKind::Eof]);
    for source in [
        "<?scr /*",
        "<?scr /* hi\n世界",
        "<?scr /* outer /* inner */ /*",
    ] {
        fails(source, LexerErrorKind::UnterminatedComment);
    }
}

#[test]
fn names_and_variables() {
    assert_eq!(
        kinds("$name $_name2 foo Foo_2 $if"),
        vec![
            TokenKind::Variable("name".into()),
            TokenKind::Variable("_name2".into()),
            TokenKind::Ident("foo".into()),
            TokenKind::Ident("Foo_2".into()),
            TokenKind::Variable("if".into()),
            TokenKind::Eof
        ]
    );
    for source in ["<?scr $", "<?scr $2x", "<?scr $é"] {
        fails(source, LexerErrorKind::InvalidVariable);
    }
    fails("<?scr é", LexerErrorKind::InvalidCharacter);
}

#[test]
fn decimal_numbers_and_exponents_preserve_spelling() {
    for number in ["0", "123", "001", "9223372036854775808"] {
        assert_eq!(
            kinds(number),
            vec![TokenKind::Int(number.into()), TokenKind::Eof]
        );
    }
    for number in ["0.0", "1.25", "1e3", "1E-3", "0.25e+10", "001.20"] {
        assert_eq!(
            kinds(number),
            vec![TokenKind::Float(number.into()), TokenKind::Eof]
        );
    }
    assert_eq!(
        kinds("1.foo"),
        vec![
            TokenKind::Int("1".into()),
            TokenKind::Dot,
            TokenKind::Ident("foo".into()),
            TokenKind::Eof
        ]
    );
    // Signs belong to operators, not numeric literals.
    assert_eq!(
        kinds("-1"),
        vec![TokenKind::Minus, TokenKind::Int("1".into()), TokenKind::Eof]
    );
}

#[test]
fn malformed_numbers() {
    for number in [
        "1e", "1e+", "1E-", "12abc", "0x10", "0b11", "1_000", "1.2.3", "1e2.3", "2e3x",
    ] {
        fails(&format!("<?scr {number}"), LexerErrorKind::MalformedNumber);
    }
}

#[test]
fn strings_decode_escapes_and_preserve_unicode_and_source_spans() {
    let source = "<?scr \"hé世界 🦀\" 'literal $name'";
    let tokens = lex(source).unwrap();
    assert_eq!(tokens[0].kind, TokenKind::String("hé世界 🦀".into()));
    assert_eq!(
        &source[tokens[0].span.start..tokens[0].span.end],
        "\"hé世界 🦀\""
    );
    assert_eq!(tokens[1].kind, TokenKind::String("literal $name".into()));
    assert_eq!(
        kinds(r#""\n\r\t\0\\\"\'\$name""#),
        vec![
            TokenKind::String("\n\r\t\0\\\"'$name".into()),
            TokenKind::Eof
        ]
    );
    assert_eq!(
        kinds(r"'it\'s \\'"),
        vec![TokenKind::String("it's \\".into()), TokenKind::Eof]
    );
    assert_eq!(
        kinds("\"?> <?scr\""),
        vec![TokenKind::String("?> <?scr".into()), TokenKind::Eof]
    );
    assert_eq!(
        kinds("\"$name\""),
        vec![TokenKind::String("$name".into()), TokenKind::Eof]
    );
}

#[test]
fn invalid_and_unterminated_strings() {
    for body in ["\"open", "'open", "\"ends\\", "'ends\\", "\"multi\nline"] {
        fails(&format!("<?scr {body}"), LexerErrorKind::UnterminatedString);
    }
    for body in [r"'\n'", r"'\t'", r#"'\"'"#, r#""\q""#, r#""\u1234""#] {
        fails(&format!("<?scr {body}"), LexerErrorKind::InvalidEscape);
    }
}

#[test]
fn variadic_ellipsis_is_a_single_token_distinct_from_concatenation() {
    let source = "<?scr function f(...$values: int) {} $text = 'a' . 'b';";
    let tokens = lex(source).unwrap();
    let token = tokens
        .iter()
        .find(|t| t.kind == TokenKind::Ellipsis)
        .unwrap();
    assert_eq!(&source[token.span.start..token.span.end], "...");
    assert_eq!(
        tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Ellipsis)
            .count(),
        1
    );
    assert_eq!(
        tokens.iter().filter(|t| t.kind == TokenKind::Dot).count(),
        1
    );
}

#[test]
fn invalid_operators_and_characters() {
    for op in [
        "++", "--", "**", "%=", "<<", ">>", "??", "&=", "|=", "^=", "&", "|", "^", "~", "?",
    ] {
        fails(&format!("<?scr {op}"), LexerErrorKind::InvalidOperator);
    }
    for c in ["@", "`", "\0", "\u{feff}", "🦀"] {
        fails(&format!("<?scr {c}"), LexerErrorKind::InvalidCharacter);
    }
}

#[test]
fn positions_across_bom_shebang_unicode_crlf_cr_and_tabs() {
    let source = "\u{feff}#!/usr/bin/env scr\r\n<?scr\r\necho \"é🦀\";\r\t$x = 2;\n";
    let tokens = lex(source).unwrap();
    for token in &tokens {
        assert_eq!(token.start, SourcePosition::at(source, token.span.start));
        assert_eq!(token.end, SourcePosition::at(source, token.span.end));
    }
    assert_eq!((tokens[0].start.line, tokens[0].start.column), (3, 1));
    assert_eq!((tokens[1].start.line, tokens[1].start.column), (3, 6));
    assert_eq!((tokens[1].end.line, tokens[1].end.column), (3, 10));
    assert_eq!((tokens[3].start.line, tokens[3].start.column), (4, 2));
    assert_eq!(tokens.last().unwrap().start.line, 5);
    let error = fails("<?scr\r\n\"é\" @", LexerErrorKind::InvalidCharacter);
    assert_eq!((error.start.line, error.start.column), (2, 5));
    assert_eq!(error.span, Span::new(12, 13));
    let diagnostic: Diagnostic = error.into();
    let rendered = diagnostic.render("test.scr", "<?scr\r\n\"é\" @");
    assert!(rendered.contains("test.scr:2:5"));
    assert!(rendered.contains("error[E0101]: unexpected character"));
    assert!(rendered.contains("2 | \"*\" @"));
}

#[test]
fn source_positions_remain_valid_over_a_small_adversarial_corpus() {
    // Exercises arbitrary UTF-8 transitions and EOF after every common prefix.
    for prefix in ["", "<?scr", "\u{feff}<?scr", "#!scr\n<?scr"] {
        for fragment in [
            "", "$", "/*", "\"", "'", "\\", "é", "\r\n", "?>", "1e+", "!==", "\"🦀\"",
        ] {
            let source = format!("{prefix}{fragment}");
            match lex(&source) {
                Ok(tokens) => {
                    let mut end = 0;
                    for token in tokens {
                        assert!(token.span.start >= end);
                        assert!(token.span.end <= source.len());
                        assert_eq!(token.start, SourcePosition::at(&source, token.span.start));
                        assert_eq!(token.end, SourcePosition::at(&source, token.span.end));
                        end = token.span.end;
                    }
                }
                Err(error) => {
                    fails(&source, error.kind);
                }
            }
        }
    }
}
