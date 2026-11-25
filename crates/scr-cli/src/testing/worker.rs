use super::*;
use scr_runtime::{ContextOptions, ExecutionOutcome, Runtime, RuntimeOptions};
use std::{cell::RefCell, rc::Rc};

pub(super) fn launch(args: Vec<OsString>) -> ExitCode {
    let [request_path, source_path] = args.as_slice() else {
        return ExitCode::from(2);
    };
    let request_path = Path::new(request_path);
    let Some(directory) = request_path.parent() else {
        return ExitCode::from(2);
    };
    match run(request_path, Path::new(source_path), directory) {
        Ok(result) => {
            let bytes = serde_json::to_vec(&result);
            match bytes {
                Ok(bytes)
                    if bytes.len() <= 65_536
                        && std::fs::write(directory.join("result.json"), &bytes).is_ok() =>
                {
                    ExitCode::SUCCESS
                }
                _ => ExitCode::from(2),
            }
        }
        Err(error) => {
            eprintln!("{}", redact(&error));
            ExitCode::from(2)
        }
    }
}

fn run(request_path: &Path, path: &Path, directory: &Path) -> Result<WorkerResult, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(request_path)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("test request exceeds 64 KiB".into());
    }
    let request: WorkerRequest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if request.max_output_bytes > 64 * 1024 * 1024
        || request.name.len() > 4096
        || !request.name.starts_with("test_")
    {
        return Err("invalid test worker request".into());
    }
    let limits = crate::entry::limits()?;
    let source =
        crate::entry::read_source(path, limits.max_source_bytes).map_err(|e| e.to_string())?;
    let mut runtime = Runtime::with_options(RuntimeOptions {
        context: ContextOptions {
            script_path: Some(path.to_owned()),
            ..Default::default()
        },
        max_output_bytes: Some(request.max_output_bytes),
        ..limits
    });
    runtime.set_input_stream(std::io::empty());
    runtime.set_module_resolver(|base, name| {
        crate::packages::resolve(base, name)
            .map_err(|e| scr_runtime::NativeError::new("PackageError", e.to_string()))
    });
    scr_stdlib::register(&mut runtime).map_err(|e| e.to_string())?;
    crate::extensions::register(&mut runtime).map_err(|e| e.to_string())?;
    let state = Rc::new(RefCell::new(assertions::State::default()));
    assertions::register(&mut runtime, state.clone(), directory.join("tmp"))?;
    let mut output = std::io::stdout();
    let initialization = runtime.execute_source_outcome(&source, &mut output);
    let mut skip_signal = false;
    let outcome = match initialization {
        Err(error) => {
            skip_signal = error.message == "Interrupted: test skipped";
            Some(error.render(&path.to_string_lossy(), &source))
        }
        Ok(ExecutionOutcome::Exited(code)) => Some(format!(
            "test file exited during initialization with code {code}"
        )),
        Ok(ExecutionOutcome::Completed) => {
            let callee = runtime
                .get_callable(&request.name)
                .ok_or("discovered test function is missing")?;
            match runtime.invoke(callee, &[], &mut output) {
                Ok(_) => None,
                Err(error) => Some(if let Some(diagnostic) = error.diagnostic() {
                    skip_signal =
                        error.is_interrupted() && diagnostic.message == "Interrupted: test skipped";
                    diagnostic.render(&path.to_string_lossy(), &source)
                } else {
                    format!(
                        "test exited before completion (code {})",
                        error.exit_code().unwrap_or(0)
                    )
                }),
            }
        }
    };
    let state = state.borrow();
    if let Some(error) = &state.failure {
        return Ok(WorkerResult {
            status: Status::Failed,
            diagnostic: Some(clipped(
                &error.render(&path.to_string_lossy(), &source),
                24_000,
            )),
        });
    }
    if let Some(reason) = state.skipped.as_ref().filter(|_| skip_signal) {
        return Ok(WorkerResult {
            status: Status::Skipped,
            diagnostic: Some(reason.clone()),
        });
    }
    Ok(WorkerResult {
        status: if outcome.is_some() {
            Status::Failed
        } else {
            Status::Passed
        },
        diagnostic: outcome.map(|s| clipped(&s, 24_000)),
    })
}
