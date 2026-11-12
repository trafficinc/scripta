use proptest::prelude::*;
proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]
    #[test]
    fn arbitrary_unicode_never_panics(chars in proptest::collection::vec(any::<char>(), 0..256)) {
        let body: String = chars.into_iter().collect();
        for source in [body.clone(), format!("<?scr {body}")] {
            if let Ok(tokens) = scr_lexer::lex(&source) {
                let _ = scr_parser::parse_with_diagnostics(&tokens);
                let _ = scr_parser::parse_expression(&tokens);
            }
        }
    }
    #[test]
    fn token_like_noise_never_panics(parts in proptest::collection::vec(0usize..24, 0..128)) {
        let pieces = ["$x", "if", "else", "function", "return", "break", "(", ")", "[", "]", "{", "}", ";", ",", "+", "=", "===", "1", "'x'", "echo", "foreach", "=>", "::", "require"];
        let source = format!("<?scr {}", parts.iter().map(|i| pieces[*i]).collect::<Vec<_>>().join(" "));
        if let Ok(tokens) = scr_lexer::lex(&source) { let _ = scr_parser::parse_with_diagnostics(&tokens); }
    }
}
