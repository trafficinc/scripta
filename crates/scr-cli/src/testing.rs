//! Native script tests: deterministic discovery, isolated workers, bounded reports.
mod assertions;
mod discovery;
mod worker;

use process_wrap::std::{StdChildWrapper, StdCommandWrap, StdCommandWrapper};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) const HELP: &str = "Usage: scripta test [paths...] [--filter <substring>] [--list] [--fail-fast] [--format text|json] [--show-output]\nDiscover tests/**/*_test.scripta and invoke top-level test_* functions without arguments.\nEach case initializes its file in a fresh process/runtime. Configure [test] in scripta.toml.\nTests use installed packages; no dependencies are fetched. Exit codes: 0 success, 1 failed tests, 2 configuration/discovery error or no matches, 130 interrupted.";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TestConfig {
    pub directories: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}
impl Default for TestConfig {
    fn default() -> Self {
        Self {
            directories: vec!["tests".into()],
            timeout_ms: 10_000,
            max_output_bytes: 1024 * 1024,
        }
    }
}
impl TestConfig {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.directories.is_empty()
            || self.directories.len() > 64
            || self.directories.iter().any(|p| {
                p.is_empty()
                    || !Path::new(p)
                        .components()
                        .all(|c| matches!(c, Component::Normal(_)))
            })
        {
            return Err(
                "test.directories must contain 1..64 relative directories without '..'".into(),
            );
        }
        if !(1..=3_600_000).contains(&self.timeout_ms) || self.max_output_bytes > 64 * 1024 * 1024 {
            return Err(
                "test.timeout_ms must be 1..3600000; test.max_output_bytes must be 0..67108864"
                    .into(),
            );
        }
        Ok(())
    }
}

#[derive(Default)]
struct Options {
    paths: Vec<PathBuf>,
    filter: Option<String>,
    list: bool,
    fail_fast: bool,
    json: bool,
    show_output: bool,
}
impl Options {
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        let mut escaped = false;
        while let Some(arg) = args.next() {
            if escaped {
                options.paths.push(arg.into());
                continue;
            }
            match arg.to_str() {
                Some("--") => escaped = true,
                Some("--list") if !options.list => options.list = true,
                Some("--fail-fast") if !options.fail_fast => options.fail_fast = true,
                Some("--show-output") if !options.show_output => options.show_output = true,
                Some("--filter") if options.filter.is_none() => {
                    options.filter = Some(
                        args.next()
                            .and_then(|v| v.into_string().ok())
                            .ok_or("--filter needs a UTF-8 substring")?,
                    );
                }
                Some("--format") => {
                    options.json = match args.next().as_deref().and_then(|v| v.to_str()) {
                        Some("json") => true,
                        Some("text") => false,
                        _ => return Err("--format must be text or json".into()),
                    };
                }
                Some(value) if value.starts_with('-') => {
                    return Err(format!("unknown or repeated test option {value}"));
                }
                _ => options.paths.push(arg.into()),
            }
        }
        Ok(options)
    }
}

#[derive(Clone, Serialize)]
struct Case {
    #[serde(skip)]
    path: PathBuf,
    file: String,
    name: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerRequest {
    name: String,
    max_output_bytes: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkerResult {
    status: Status,
    diagnostic: Option<String>,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Status {
    Passed,
    Failed,
    Skipped,
}
impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Passed => "PASS",
            Self::Failed => "FAIL",
            Self::Skipped => "SKIP",
        }
    }
}
#[derive(Serialize)]
struct CaseResult {
    file: String,
    name: String,
    status: Status,
    duration_ms: u128,
    diagnostic: Option<String>,
    output: String,
}
#[derive(Default, Serialize)]
struct Summary {
    passed: usize,
    failed: usize,
    skipped: usize,
}

// Child cleanup happens even on parent errors. A test process is not a sandbox
// for its filesystem or external services; runtime capabilities remain in force.
struct WorkerChild(Box<dyn StdChildWrapper>);
impl WorkerChild {
    fn terminate(&mut self) {
        let _ = self.0.start_kill();
        if self.0.inner_mut().try_wait().ok().flatten().is_none() {
            let _ = self.0.inner_mut().kill();
            let _ = self.0.inner_mut().wait();
        }
    }
}
#[derive(Debug)]
struct SetupCleanup;
#[derive(Debug)]
struct SetupChild(Option<Box<dyn StdChildWrapper>>);
impl StdCommandWrapper for SetupCleanup {
    fn wrap_child(
        &mut self,
        child: Box<dyn StdChildWrapper>,
        _: &StdCommandWrap,
    ) -> std::io::Result<Box<dyn StdChildWrapper>> {
        Ok(Box::new(SetupChild(Some(child))))
    }
}
impl StdChildWrapper for SetupChild {
    fn inner(&self) -> &std::process::Child {
        self.0.as_ref().expect("owned child").inner()
    }
    fn inner_mut(&mut self) -> &mut std::process::Child {
        self.0.as_mut().expect("owned child").inner_mut()
    }
    fn into_inner(mut self: Box<Self>) -> std::process::Child {
        self.0.take().expect("owned child").into_inner()
    }
}
impl Drop for SetupChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            if child.inner_mut().try_wait().ok().flatten().is_none() {
                let _ = child.inner_mut().kill();
                let _ = child.inner_mut().wait();
            }
        }
    }
}
impl Drop for WorkerChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

fn redact(text: &str) -> String {
    let secrets: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| scr_diagnostics::sensitive_name(&key.to_string_lossy()))
        .filter_map(|(_, value)| value.into_string().ok())
        .collect();
    scr_diagnostics::redact_text(text, &secrets)
}
fn clipped(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    const MARKER: &str = "\n[truncated]";
    if limit < MARKER.len() {
        return MARKER[..limit].into();
    }
    let mut end = limit - MARKER.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARKER}", &text[..end])
}

fn run_case(
    case: &Case,
    root: &Path,
    config: &TestConfig,
    interrupted: &AtomicBool,
) -> Result<CaseResult, String> {
    let start = Instant::now();
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let request_path = directory.path().join("request.json");
    std::fs::create_dir(directory.path().join("tmp")).map_err(|e| e.to_string())?;
    std::fs::write(
        &request_path,
        serde_json::to_vec(&WorkerRequest {
            name: case.name.clone(),
            max_output_bytes: config.max_output_bytes,
        })
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let capture = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut reader = capture.try_clone().map_err(|e| e.to_string())?;
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .arg("__test-worker")
        .arg(&request_path)
        .arg(&case.path)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(capture.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(capture));
    let mut command = StdCommandWrap::from(command);
    command.wrap(SetupCleanup);
    #[cfg(unix)]
    command.wrap(process_wrap::std::ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(process_wrap::std::JobObject);
    let mut child = WorkerChild(
        command
            .spawn()
            .map_err(|e| format!("cannot start test worker: {e}"))?,
    );
    let mut forced = None;
    let exit = loop {
        if interrupted.load(Ordering::Relaxed) {
            forced = Some("test interrupted".into());
            break None;
        }
        if reader.metadata().map_err(|e| e.to_string())?.len() > config.max_output_bytes as u64 {
            forced = Some("test output limit exceeded".into());
            break None;
        }
        if let Some(exit) = child.0.inner_mut().try_wait().map_err(|e| e.to_string())? {
            break Some(exit);
        }
        if start.elapsed() >= Duration::from_millis(config.timeout_ms) {
            forced = Some(format!("test timed out after {} ms", config.timeout_ms));
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if exit.is_none() {
        child.terminate();
    }
    reader.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    reader
        .take(config.max_output_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > config.max_output_bytes {
        forced = Some("test output limit exceeded".into());
        bytes.truncate(config.max_output_bytes);
    }
    let result = if let Some(diagnostic) = forced {
        WorkerResult {
            status: Status::Failed,
            diagnostic: Some(diagnostic),
        }
    } else if !exit.is_some_and(|e| e.success()) {
        WorkerResult {
            status: Status::Failed,
            diagnostic: Some("test worker terminated abnormally".into()),
        }
    } else {
        let result = std::fs::File::open(directory.path().join("result.json"));
        match result {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(65_537)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 65_536 {
                    return Err("test worker report exceeded 64 KiB".into());
                }
                serde_json::from_slice(&bytes)
                    .map_err(|_| "invalid test worker report".to_owned())?
            }
            Err(_) => WorkerResult {
                status: Status::Failed,
                diagnostic: Some("test worker exited without completing the test".into()),
            },
        }
    };
    Ok(CaseResult {
        file: case.file.clone(),
        name: case.name.clone(),
        status: result.status,
        duration_ms: start.elapsed().as_millis(),
        diagnostic: result.diagnostic.map(|s| redact(&s)),
        output: redact(&String::from_utf8_lossy(&bytes)),
    })
}

pub(crate) fn launch(args: Vec<OsString>) -> ExitCode {
    // Preserve JSON framing even for option/discovery errors.
    let wants_json = args
        .windows(2)
        .any(|pair| pair[0] == "--format" && pair[1] == "json");
    match run(args) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let error = redact(&error);
            if wants_json {
                println!(
                    "{}",
                    serde_json::json!({"version":1,"tests":[],"error":error})
                );
            } else {
                eprintln!("error[E0701]: {error}");
            }
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<OsString>) -> Result<u8, String> {
    let options = Options::parse(args)?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let (root, config) = crate::packages::test_configuration(&cwd).map_err(|e| e.to_string())?;
    config.validate()?;
    let limits = crate::entry::limits()?;
    let cases = discovery::discover(&cwd, &root, &config, &options, limits.max_source_bytes)?;
    if cases.is_empty() {
        return Err("no matching tests found".into());
    }
    if options.list {
        if options.json {
            println!("{}", serde_json::json!({"version":1,"tests":cases}));
        } else {
            for case in &cases {
                println!("{}::{}", case.file, case.name);
            }
        }
        return Ok(0);
    }
    crate::packages::check_execution(&root).map_err(|e| e.to_string())?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = interrupted.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed)).map_err(|e| e.to_string())?;
    let mut results = Vec::new();
    let mut summary = Summary::default();
    let mut report_remaining = 64 * 1024 * 1024_usize;
    for case in cases {
        if interrupted.load(Ordering::Relaxed) {
            break;
        }
        let mut result = run_case(&case, &root, &config, &interrupted)?;
        if result.status != Status::Failed && !options.show_output {
            result.output.clear();
        }
        let output_limit = report_remaining.min(result.output.len());
        result.output = clipped(&result.output, output_limit);
        report_remaining = report_remaining.saturating_sub(result.output.len());
        if let Some(diagnostic) = &mut result.diagnostic {
            *diagnostic = clipped(diagnostic, report_remaining.min(diagnostic.len()));
            report_remaining = report_remaining.saturating_sub(diagnostic.len());
        }
        match result.status {
            Status::Passed => summary.passed += 1,
            Status::Failed => summary.failed += 1,
            Status::Skipped => summary.skipped += 1,
        }
        if !options.json {
            println!("{} {}::{}", result.status.label(), result.file, result.name);
            if let Some(message) = &result.diagnostic {
                println!("\n{message}\n");
            }
            if !result.output.is_empty() && (result.status == Status::Failed || options.show_output)
            {
                println!("{}", result.output);
            }
        }
        let failed = result.status == Status::Failed;
        results.push(result);
        if failed && options.fail_fast {
            break;
        }
    }
    let cancelled = interrupted.load(Ordering::Relaxed);
    if options.json {
        println!(
            "{}",
            serde_json::json!({"version":1,"tests":results,"summary":summary,"interrupted":cancelled})
        );
    } else {
        println!(
            "\n{} tests: {} passed, {} failed, {} skipped",
            results.len(),
            summary.passed,
            summary.failed,
            summary.skipped
        );
    }
    Ok(if cancelled {
        130
    } else if summary.failed > 0 {
        1
    } else {
        0
    })
}

pub(crate) fn worker(args: Vec<OsString>) -> ExitCode {
    worker::launch(args)
}
