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
fn config(arguments: Vec<OsString>) -> Result<scr_web::Config, Box<dyn std::error::Error>> {
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
    if let Err(errors) = crate::pipeline::check(&source) {
        for error in errors {
            eprintln!("{}", error.render(&path.to_string_lossy(), &source));
        }
        return Err("entry script failed validation".into());
    }
    let mut config = scr_web::Config::new(path, source);
    let mut seen = std::collections::BTreeSet::new();
    while let Some(key) = arguments.next() {
        let key = key.to_str().ok_or("option must be UTF-8")?;
        if !seen.insert(key.to_owned()) {
            return Err(format!("duplicate serve option {key}").into());
        }
        let value = arguments
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        if key == "--upload-temp-dir" {
            config.uploads.directory = value.into();
            continue;
        }
        if key == "--session-dir" {
            config.sessions.directory = value.into();
            if config.sessions.directory.as_os_str().is_empty() {
                return Err("session directory must not be empty".into());
            }
            continue;
        }
        let value = value.to_str().ok_or("option value must be UTF-8")?;
        match key {
            "--upload-max-file-bytes" => config.uploads.max_file_bytes = value.parse()?,
            "--upload-max-files" => config.uploads.max_files = value.parse()?,
            "--upload-max-fields" => config.uploads.max_fields = value.parse()?,
            "--upload-max-field-bytes" => config.uploads.max_field_bytes = value.parse()?,
            "--upload-max-fields-bytes" => config.uploads.max_fields_bytes = value.parse()?,
            "--upload-max-header-bytes" => config.uploads.max_header_bytes = value.parse()?,
            "--upload-max-disk-bytes" => config.uploads.max_disk_bytes = value.parse()?,
            "--session-ttl-seconds" => config.sessions.ttl = Duration::from_secs(value.parse()?),
            "--session-max-records" => config.sessions.max_sessions = value.parse()?,
            "--session-max-bytes" => config.sessions.max_bytes = value.parse()?,
            "--session-cookie-secure" => config.sessions.secure = value.parse()?,
            "--host" => config.host = value.parse()?,
            "--port" => config.port = value.parse()?,
            "--workers" => config.workers = value.parse()?,
            "--queue" => config.queue = value.parse()?,
            "--max-connections" => config.max_connections = value.parse()?,
            "--max-body-bytes" => config.max_body_bytes = value.parse()?,
            "--max-response-bytes" => config.max_response_bytes = value.parse()?,
            "--timeout-ms" => config.request_timeout = Duration::from_millis(value.parse()?),
            _ => return Err(format!("unknown serve option {key}").into()),
        }
    }
    Ok(config)
}
pub(crate) fn launch(arguments: Vec<OsString>) -> ExitCode {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let config = config(arguments)?;
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
