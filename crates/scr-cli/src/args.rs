//! Command-line parsing without filesystem access or process termination.
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

pub const HELP: &str = "Scripta systems scripting language

Usage: scr version
       scr version --verbose
       scr help
       scr run <file> [arguments...]
       scr <file> [arguments...]
       scr check <file>
       scr fmt [--check] <file>
       scr playground [options]

Options: --help, -h, --version
Use scr run -- <file> for paths starting with a dash or matching command names.
Arguments after the script path belong to the script; -- ends script option queries.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Playground {
        arguments: Vec<OsString>,
    },
    Help,
    Version,
    VersionVerbose,
    Run {
        path: PathBuf,
        arguments: Vec<OsString>,
    },
    Check(PathBuf),
    Format {
        path: PathBuf,
        check: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);
impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for UsageError {}

/// Parse interpreter options only before the script path. Script arguments are
/// preserved verbatim, including native OS encoding and leading hyphens.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, UsageError> {
    let args: Vec<_> = args.into_iter().collect();
    let Some(first) = args.first() else {
        return Err(UsageError("expected a command or source file".into()));
    };
    if first == "playground" {
        return Ok(Command::Playground {
            arguments: args[1..].to_vec(),
        });
    }
    if matches!(first.to_str(), Some("version" | "--version"))
        && args.len() == 2
        && args[1] == "--verbose"
    {
        return Ok(Command::VersionVerbose);
    }
    if first == "help"
        || first == "--help"
        || first == "-h"
        || first == "version"
        || first == "--version"
    {
        if args.len() != 1 {
            return Err(UsageError("unexpected extra arguments".into()));
        }
        return Ok(if first == "version" || first == "--version" {
            Command::Version
        } else {
            Command::Help
        });
    }
    if first == "fmt" {
        let mut rest = &args[1..];
        let mut check = false;
        if rest.first().is_some_and(|a| a == "--check") {
            check = true;
            rest = &rest[1..];
        }
        let escaped = rest.first().is_some_and(|a| a == "--");
        if escaped {
            rest = &rest[1..];
        }
        if rest.len() == 2 && !escaped && rest[1] == "--check" {
            check = true;
            rest = &rest[..1];
        }
        if rest.len() != 1
            || rest[0].is_empty()
            || !escaped && rest[0].to_string_lossy().starts_with('-')
        {
            return Err(UsageError(
                "fmt expects [--check] one source file; use -- before a dash-prefixed filename"
                    .into(),
            ));
        }
        return Ok(Command::Format {
            path: rest[0].clone().into(),
            check,
        });
    }
    let checking = first == "check";
    let rest = if first == "run" || checking {
        &args[1..]
    } else {
        &args[..]
    };
    let escaped = rest.first().is_some_and(|arg| arg == "--");
    let rest = if escaped { &rest[1..] } else { rest };
    let Some(path) = rest.first() else {
        return Err(UsageError("expected a source file".into()));
    };
    if path.is_empty() {
        return Err(UsageError("source path must not be empty".into()));
    }
    if !escaped && path.to_string_lossy().starts_with('-') {
        return Err(UsageError(
            "use -- before a source path starting with '-'".into(),
        ));
    }
    if checking {
        if rest.len() != 1 {
            return Err(UsageError("check expects exactly one source file".into()));
        }
        Ok(Command::Check(path.into()))
    } else {
        Ok(Command::Run {
            path: path.into(),
            arguments: rest[1..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strings(args: &[&str]) -> Result<Command, UsageError> {
        parse(args.iter().map(|arg| OsString::from(*arg)))
    }

    #[test]
    fn commands_and_aliases() {
        for arg in ["help", "--help", "-h"] {
            assert_eq!(parse_strings(&[arg]).unwrap(), Command::Help);
        }
        for arg in ["version", "--version"] {
            assert_eq!(parse_strings(&[arg]).unwrap(), Command::Version);
        }
        for args in [vec!["hello.scr"], vec!["run", "hello.scr"]] {
            assert_eq!(
                parse_strings(&args).unwrap(),
                Command::Run {
                    path: "hello.scr".into(),
                    arguments: vec![]
                }
            );
        }
        assert_eq!(
            parse_strings(&["check", "hello.scr"]).unwrap(),
            Command::Check("hello.scr".into())
        );
    }

    #[test]
    fn rejects_invalid_arity_and_options() {
        for args in [
            vec![],
            vec!["run"],
            vec!["check"],
            vec!["check", "a", "b"],
            vec!["help", "a"],
            vec!["version", "a"],
            vec!["--unknown"],
            vec!["run", "--unknown"],
            vec!["check", "--unknown"],
            vec![""],
            vec!["run", ""],
        ] {
            assert!(parse_strings(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn explicit_paths_and_spaces() {
        for path in ["help", "version", "-script.scr", "my script.scr"] {
            assert_eq!(
                parse_strings(&["run", "--", path]).unwrap(),
                Command::Run {
                    path: path.into(),
                    arguments: vec![]
                }
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_unicode_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(b"script-\xff.scr".to_vec());
        assert_eq!(
            parse([path.clone()]).unwrap(),
            Command::Run {
                path: path.into(),
                arguments: vec![]
            }
        );
    }
}

#[cfg(test)]
mod script_argument_tests {
    use super::*;
    #[test]
    fn both_invocations_expose_identical_script_arguments() {
        let arguments = ["script.scr", "one", "--two=x", "--", "--literal"].map(OsString::from);
        let shorthand = parse(arguments.clone()).unwrap();
        assert_eq!(
            parse(std::iter::once(OsString::from("run")).chain(arguments)).unwrap(),
            shorthand
        );
        let Command::Run { arguments, .. } = shorthand else {
            panic!();
        };
        assert_eq!(
            arguments,
            ["one", "--two=x", "--", "--literal"].map(OsString::from)
        );
    }
}
