use std::env;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;

use scr_cli::args::{self, Command};
use scr_cli::{formatter, pipeline};

fn main() -> ExitCode {
    let command = match args::parse(env::args_os().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error[E0001]: {error}\n\n{}", args::HELP);
            return ExitCode::from(2);
        }
    };
    let (path, check_only, format_only, arguments) = match command {
        Command::Package(arguments) => {
            return match env::current_dir()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                .and_then(|cwd| scr_cli::packages::execute(&arguments, &cwd))
            {
                Ok(message) => {
                    println!("{message}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!(
                        "error[E0601]: {}",
                        scr_diagnostics::redact_text(&error.to_string(), &[])
                    );
                    ExitCode::from(1)
                }
            };
        }
        Command::Help => {
            println!("{}", args::HELP);
            return ExitCode::SUCCESS;
        }
        Command::VersionVerbose => {
            println!(
                "scr {}\ntarget: {}\nprofile: {}\ncompiler: {}\ncommit: {}\nextensions: {}",
                env!("CARGO_PKG_VERSION"),
                env!("SCR_TARGET"),
                env!("SCR_PROFILE"),
                env!("SCR_COMPILER"),
                env!("SCR_COMMIT"),
                if cfg!(feature = "sqlite") {
                    "sqlite"
                } else {
                    "none"
                }
            );
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("scr {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::Run { path, arguments } => (path, false, None, arguments),
        Command::Check(path) => (path, true, None, Vec::new()),
        Command::Format { path, check } => (path, true, Some(check), Vec::new()),
    };
    let limits = match limits() {
        Ok(limits) => limits,
        Err(error) => {
            eprintln!("error[E0001]: {error}");
            return ExitCode::from(2);
        }
    };
    let source = match read_source(&path, limits.max_source_bytes) {
        Ok(source) => source,
        Err(error) => {
            eprintln!(
                "error[E0002]: {}: {}",
                scr_diagnostics::redact_text(&path.to_string_lossy(), &[]),
                scr_diagnostics::redact_text(&error.to_string(), &[])
            );
            return ExitCode::from(1);
        }
    };
    if let Some(check) = format_only {
        return match formatter::format(&source) {
            Err(errors) => {
                for error in errors {
                    eprintln!("{}", error.render(&path.to_string_lossy(), &source));
                }
                ExitCode::from(1)
            }
            Ok(formatted) if formatted == source => ExitCode::SUCCESS,
            Ok(_) if check => {
                eprintln!("error[E0502]: {} needs formatting", path.display());
                ExitCode::from(1)
            }
            Ok(formatted) => match formatter::write_file(&path, &formatted) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("error[E0002]: {}: {error}", path.display());
                    ExitCode::from(1)
                }
            },
        };
    }
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if !check_only {
        let flag = cancellation.clone();
        if let Err(error) =
            ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::Relaxed))
        {
            eprintln!("error[E0002]: could not install cancellation handler: {error}");
            return ExitCode::from(1);
        }
    }
    let options = scr_runtime::RuntimeOptions {
        interrupt_flag: Some(cancellation.clone()),
        context: scr_runtime::ContextOptions {
            script_path: Some(path.clone()),
            cli_arguments: arguments,
            ..Default::default()
        },
        ..limits
    };
    let path = path.to_string_lossy();
    let result = if check_only {
        pipeline::check(&source).map(|()| 0).map_err(|errors| {
            errors
                .into_iter()
                .map(|error| error.render(&path, &source))
                .collect::<Vec<_>>()
        })
    } else {
        pipeline::run_with_options(&source, &mut io::stdout().lock(), options)
            .map_err(|error| vec![error.render(&path, &source)])
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(errors) => {
            for error in errors {
                eprintln!("{error}");
            }
            ExitCode::from(if cancellation.load(std::sync::atomic::Ordering::Relaxed) {
                130
            } else {
                1
            })
        }
    }
}

fn limits() -> Result<scr_runtime::RuntimeOptions, String> {
    fn number(name: &str) -> Result<Option<usize>, String> {
        env::var_os(name)
            .map(|value| {
                value
                    .to_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(|| {
                        format!("{name} must be a nonnegative integer in platform range")
                    })
            })
            .transpose()
    }
    Ok(scr_runtime::RuntimeOptions {
        instruction_limit: number("SCR_MAX_STEPS")?.map(|n| n as u64),
        max_value_bytes: number("SCR_MAX_VALUE_BYTES")?,
        max_scopes: number("SCR_MAX_SCOPES")?,
        max_source_bytes: number("SCR_MAX_SOURCE_BYTES")?
            .unwrap_or(scr_runtime::DEFAULT_MAX_SOURCE_BYTES),
        ..Default::default()
    })
}
fn read_source(path: &std::path::Path, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("source byte limit exceeded"));
    }
    String::from_utf8(bytes).map_err(|_| io::Error::other("source is not valid UTF-8"))
}
