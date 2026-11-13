//! Central syntax-validation and source-to-execution interfaces.
use std::io::Write;

use scr_diagnostics::Diagnostic;

/// Validate lexical structure and all statements without executing the program.
pub fn check(source: &str) -> Result<(), Vec<Diagnostic>> {
    let tokens = scr_lexer::lex(source).map_err(|error| vec![error.into()])?;
    let program = scr_parser::parse_with_diagnostics(&tokens)?;
    let errors = crate::validation::validate(&program);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Check then execute synchronously in a fresh environment with the standard library.
pub fn run(source: &str, output: &mut impl Write) -> Result<u8, scr_runtime::RuntimeError> {
    run_with_options(source, output, scr_runtime::RuntimeOptions::default())
}

/// Execute with explicit host metadata, permissions, and cancellation settings.
pub fn run_with_options(
    source: &str,
    output: &mut impl Write,
    options: scr_runtime::RuntimeOptions,
) -> Result<u8, scr_runtime::RuntimeError> {
    let mut runtime = scr_runtime::Runtime::with_options(options);
    runtime.set_module_resolver(|base, name| {
        crate::packages::resolve(base, name)
            .map_err(|error| scr_runtime::NativeError::new("PackageError", error.to_string()))
    });
    scr_stdlib::register(&mut runtime)
        .map_err(|error| Diagnostic::new(error.to_string(), Default::default()))?;
    #[cfg(feature = "sqlite")]
    scr_ext_sqlite::register(&mut runtime)
        .map_err(|error| Diagnostic::new(error.to_string(), Default::default()))?;
    runtime.execute_source(source, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_does_not_execute() {
        assert!(check("<?scr echo $undefined;").is_ok());
        assert!(run("<?scr echo $undefined;", &mut Vec::new()).is_err());
    }

    #[test]
    fn check_validates_syntax_without_execution() {
        assert!(check("<?scr function foo($x: int) { if ($x === 1) return; }").is_ok());
        assert!(check("<?scr echo").is_err());
        assert!(check("<?scr $x == $y;").is_err());
        assert!(check("<?scr /* unfinished").is_err());
    }

    #[test]
    fn syntax_failure_prevents_all_execution() {
        let mut output = Vec::new();
        assert!(run("<?scr echo \"first\"; echo", &mut output).is_err());
        assert!(output.is_empty());
    }
}
