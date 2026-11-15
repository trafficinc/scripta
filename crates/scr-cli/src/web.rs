#[path = "web_config.rs"]
mod settings;
use std::{
    ffi::OsString,
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn register(runtime: &mut scr_runtime::Runtime) -> scr_runtime::NativeResult<()> {
    runtime.set_module_resolver(|base, name| {
        crate::packages::resolve(base, name)
            .map_err(|e| scr_runtime::NativeError::new("PackageError", e.to_string()))
    });
    scr_stdlib::register(runtime)?;
    #[cfg(feature = "sqlite")]
    scr_ext_sqlite::register(runtime)?;
    Ok(())
}
pub(crate) fn worker(arguments: Vec<OsString>) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        if arguments.len() != 8 {
            return Err("invalid internal worker arguments".into());
        }
        let output = arguments[1]
            .to_str()
            .ok_or("invalid output limit")?
            .parse()?;
        let mut sessions = scr_stdlib::web::session::Config::new(arguments[2].clone().into());
        sessions.ttl = Duration::from_secs(
            arguments[3]
                .to_str()
                .ok_or("invalid session ttl")?
                .parse()?,
        );
        sessions.max_sessions = arguments[4]
            .to_str()
            .ok_or("invalid session capacity")?
            .parse()?;
        sessions.max_bytes = arguments[5]
            .to_str()
            .ok_or("invalid session byte limit")?
            .parse()?;
        sessions.secure = arguments[6]
            .to_str()
            .ok_or("invalid session secure flag")?
            .parse()?;
        scr_web::run_worker(
            arguments[0].clone().into(),
            output,
            crate::entry::limits()?,
            sessions,
            arguments[7].clone().into(),
            register,
        )
    })();
    if let Err(error) = result {
        eprintln!(
            "web worker: {}",
            scr_diagnostics::redact_text(&error.to_string(), &[])
        );
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
fn config(
    arguments: Vec<OsString>,
) -> Result<(scr_web::Config, settings::Mode), Box<dyn std::error::Error>> {
    let mut arguments = arguments.into_iter();
    let first = arguments.next().ok_or("serve requires one entry script")?;
    let path = if first == "--" {
        arguments.next().ok_or("missing entry script after --")?
    } else {
        if first.to_string_lossy().starts_with('-') {
            return Err(
                "serve expects an entry script first; use -- before a dash-prefixed path".into(),
            );
        }
        first
    };
    if path.is_empty() {
        return Err("entry script path is empty".into());
    }
    let path = std::fs::canonicalize(PathBuf::from(path))?;
    let source = crate::entry::read_source(&path, crate::entry::limits()?.max_source_bytes)?;
    if crate::pipeline::check(&source).is_err() {
        return Err(
            "entry script failed validation; run scripta check locally for detailed diagnostics"
                .into(),
        );
    }
    let mut config = scr_web::Config::new(path, source);
    let options = settings::Options::parse(arguments)?;
    let mode = options.mode;
    options.apply(&mut config)?;
    Ok((config, mode))
}

pub(crate) fn launch(arguments: Vec<OsString>) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let (config, mode) = config(arguments)?;
        match mode {
            settings::Mode::Check => {
                println!("Server configuration is valid");
                return Ok(());
            }
            settings::Mode::Print => {
                print!("{}", settings::inspect(&config)?);
                return Ok(());
            }
            settings::Mode::Serve => {}
        }
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed))?;
        scr_web::serve(config, stop)
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "web server: {}",
                scr_diagnostics::redact_text(&error.to_string(), &[])
            );
            ExitCode::from(1)
        }
    }
}
