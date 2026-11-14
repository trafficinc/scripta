//! Optional companion dispatch. The editor and server are distributed separately.
use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitCode},
};

pub fn launch(arguments: Vec<OsString>) -> ExitCode {
    let result = (|| -> std::io::Result<_> {
        let interpreter = env::current_exe()?;
        let name = format!("scr-playground{}", env::consts::EXE_SUFFIX);
        let adjacent = interpreter.parent().unwrap().join(&name);
        let executable = env::var_os("SCR_PLAYGROUND_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if adjacent.is_file() {
                    adjacent
                } else {
                    PathBuf::from(name)
                }
            });
        Command::new(executable)
            .args(arguments)
            .arg("--scr")
            .arg(interpreter)
            .status()
    })();
    match result {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(error) => {
            eprintln!(
                "Could not start the optional Scripta Playground: {error}\n\
                Install the separate playground download and put scr-playground beside scripta or on PATH.\n\
                You can also set SCR_PLAYGROUND_BIN to its full executable path.\n\
                The playground is not included in normal Scripta installations."
            );
            ExitCode::from(1)
        }
    }
}
