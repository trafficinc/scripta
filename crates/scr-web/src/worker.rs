use scr_runtime::{NativeResult, Runtime, RuntimeOptions, SourceFile};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    sync::Arc,
};

pub(crate) const HEADER_LIMIT: usize = 32 * 1024;
pub(crate) const META_LIMIT: usize = 256 * 1024;
pub(crate) const BODY_CEILING: usize = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub(crate) struct RequestMeta {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct ResponseMeta {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}
pub(crate) fn read_frame(input: &mut impl Read, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut size = [0; 4];
    if input.read(&mut size[..1])? == 0 {
        return Ok(None);
    }
    input.read_exact(&mut size[1..])?;
    let size = u32::from_be_bytes(size) as usize;
    if size > limit {
        return Err(io::Error::other("worker frame exceeds limit"));
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}
fn write_frame(output: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    let size = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    output.write_all(&size.to_be_bytes())?;
    output.write_all(bytes)?;
    output.flush()
}
struct Output {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.exceeded || bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("web response body limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Internal persistent-worker entry. Protocol uses framed metadata and raw bodies;
/// script stdout is captured separately and never becomes protocol metadata.
pub fn run_worker(
    path: PathBuf,
    max_output: usize,
    mut options: RuntimeOptions,
    mut session_config: scr_stdlib::web::session::Config,
    register: impl Fn(&mut Runtime) -> NativeResult<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    if max_output == 0 || max_output > BODY_CEILING {
        return Err("invalid worker output limit".into());
    }
    session_config.prepare()?;
    let mut input = io::stdin().lock();
    let mut protocol = io::stdout().lock();
    let source =
        read_frame(&mut input, options.max_source_bytes)?.ok_or("missing worker source")?;
    let text = String::from_utf8(source)?;
    let tokens = scr_lexer::lex(&text).map_err(|e| e.to_string())?;
    let program = scr_parser::parse(&tokens).map_err(|e| e.to_string())?;
    let source = Arc::new(SourceFile {
        path: path.clone(),
        text,
    });
    options.context.script_path = Some(path);
    options.instruction_limit = Some(options.instruction_limit.unwrap_or(1_000_000));
    options.max_value_bytes = Some(options.max_value_bytes.unwrap_or(16 * 1024 * 1024));
    options.max_scopes = Some(options.max_scopes.unwrap_or(4096));
    // Interactive input and child spawning are unsuitable defaults for request workers.
    // Other permissions retain the existing host's filesystem/network policy.
    for permission in [
        "cli.read",
        "cli.prompt",
        "process.execute",
        "process.inspect",
        "environment.write",
    ] {
        options.context.permissions.allowed.remove(permission);
    }
    write_frame(&mut protocol, b"ready")?;
    drop(input);
    // A separate pipe reader observes parent death even while script/native code
    // is blocked. Worker processes must not outlive their supervisor.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = (|| -> io::Result<()> {
            let mut input = io::stdin().lock();
            while let Some(metadata) = read_frame(&mut input, META_LIMIT)? {
                let body = read_frame(&mut input, BODY_CEILING)?
                    .ok_or_else(|| io::Error::other("missing request body"))?;
                if send.send((metadata, body)).is_err() {
                    return Ok(());
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("web worker input failed: {error}");
        }
        std::process::exit(0);
    });
    while let Ok((metadata, body)) = receive.recv() {
        let metadata: RequestMeta = serde_json::from_slice(&metadata)?;
        let mut runtime = Runtime::with_options(options.clone());
        runtime.set_input_stream(io::empty());
        // Script error output belongs to the server log, never its response body.
        runtime.set_error_stream(io::stderr());
        register(&mut runtime)?;
        let headers = metadata.headers;
        let state = scr_stdlib::web::register(
            &mut runtime,
            scr_stdlib::web::Request {
                method: metadata.method,
                path: metadata.path,
                query_string: metadata.query,
                headers: headers.clone(),
                body,
            },
            HEADER_LIMIT,
        )?;
        let session = scr_stdlib::web::session::register(
            &mut runtime,
            session_config.clone(),
            &headers,
            state.clone(),
        )?;
        let mut output = Output {
            bytes: vec![],
            limit: max_output,
            exceeded: false,
        };
        let result = runtime.execute_program_with_source(&program, source.clone(), &mut output);
        let committed = if matches!(&result, Ok(0)) && !output.exceeded {
            session.finish()
        } else {
            Ok(())
        };
        let (metadata, body) = match (result, committed) {
            (Ok(0), Ok(())) if !output.exceeded => {
                let response = state.borrow();
                (
                    ResponseMeta {
                        status: response.status,
                        headers: response.headers.clone(),
                    },
                    output.bytes,
                )
            }
            other => {
                if let Err(error) = other.1 {
                    eprintln!("web session commit failed: {error}");
                }
                if let Err(error) = other.0 {
                    eprintln!(
                        "{}",
                        error.render(&source.path.to_string_lossy(), &source.text)
                    );
                } else {
                    eprintln!("web request failed: nonzero exit or response output limit");
                }
                (
                    ResponseMeta {
                        status: 500,
                        headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
                    },
                    b"Internal Server Error\n".to_vec(),
                )
            }
        };
        write_frame(&mut protocol, &serde_json::to_vec(&metadata)?)?;
        write_frame(&mut protocol, &body)?;
        // Runtime and native resources are dropped before accepting the next request.
    }
    Ok(())
}
