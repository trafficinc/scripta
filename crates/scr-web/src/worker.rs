use crate::uploads::{MultipartMeta, WirePath};
use scr_runtime::{NativeResult, Runtime, RuntimeOptions, SourceFile};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    sync::Arc,
};

pub(crate) const CHUNK_LIMIT: usize = 64 * 1024;
pub(crate) const HEADER_LIMIT: usize = 32 * 1024;
pub(crate) const META_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const BODY_CEILING: usize = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub(crate) struct RequestMeta {
    pub method: String,
    pub path: String,
    pub query: String,
    pub headers: Vec<(String, String)>,
    pub multipart: Option<MultipartMeta>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct ResponseMeta {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum ParentMessage {
    Request { metadata: RequestMeta },
    StageReply { path: Option<WirePath> },
    FlushAck,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum ChildMessage {
    Response { metadata: ResponseMeta },
    StageRequest { index: usize, parent: WirePath },
    Start { metadata: ResponseMeta },
    Chunk,
    Flush,
    Done { success: bool },
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
    total: usize,
    exceeded: bool,
    started: bool,
    protocol: Rc<RefCell<io::Stdout>>,
    acknowledgements: Rc<std::sync::mpsc::Receiver<()>>,
    response: Rc<RefCell<scr_stdlib::web::Response>>,
    session: scr_stdlib::web::session::Session,
}
impl Output {
    fn message(&self, message: &ChildMessage) -> io::Result<()> {
        write_frame(
            &mut *self.protocol.borrow_mut(),
            &serde_json::to_vec(message)?,
        )
    }
    fn chunks(&mut self) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        for chunk in bytes.chunks(CHUNK_LIMIT) {
            self.message(&ChildMessage::Chunk)?;
            write_frame(&mut *self.protocol.borrow_mut(), chunk)?;
        }
        Ok(())
    }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.exceeded || bytes.len() > self.limit.saturating_sub(self.total) {
            self.exceeded = true;
            return Err(io::Error::other("web response body limit exceeded"));
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(io::Error::other)?;
        self.bytes.extend_from_slice(bytes);
        self.total += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if !self.started {
            self.session.seal().map_err(io::Error::other)?;
            let metadata = {
                let mut response = self.response.borrow_mut();
                response.commit().map_err(io::Error::other)?;
                ResponseMeta {
                    status: response.status,
                    headers: response.headers.clone(),
                }
            };
            self.started = true;
            self.message(&ChildMessage::Start { metadata })?;
        }
        self.chunks()?;
        self.message(&ChildMessage::Flush)?;
        self.acknowledgements.recv().map_err(io::Error::other)
    }
}
/// Internal persistent-worker entry. Protocol uses framed metadata and raw bodies;
/// script stdout is captured separately and never becomes protocol metadata.
pub fn run_worker(
    path: PathBuf,
    max_output: usize,
    mut options: RuntimeOptions,
    mut session_config: scr_stdlib::web::session::Config,
    upload_root: PathBuf,
    register: impl Fn(&mut Runtime) -> NativeResult<()>,
) -> Result<(), Box<dyn std::error::Error>> {
    if max_output == 0 || max_output > BODY_CEILING {
        return Err("invalid worker output limit".into());
    }
    session_config.prepare()?;
    options.max_output_bytes = Some(max_output);
    options.max_output_buffer_bytes = options.max_output_buffer_bytes.min(max_output);
    let mut input = io::stdin().lock();
    let protocol = Rc::new(RefCell::new(io::stdout()));
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
    options
        .context
        .permissions
        .allowed
        .insert("web.session".into());
    // Interactive input and child spawning are unsuitable defaults for request workers.
    // Other permissions retain the existing host's filesystem/network policy.
    for permission in [
        "network.bind",
        "network.listen",
        "cli.read",
        "cli.prompt",
        "process.execute",
        "process.inspect",
        "environment.write",
    ] {
        options.context.permissions.allowed.remove(permission);
    }
    write_frame(&mut *protocol.borrow_mut(), b"ready")?;
    drop(input);
    // A separate pipe reader observes parent death even while script/native code
    // is blocked. Worker processes must not outlive their supervisor.
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let (stage_send, stage_receive) = std::sync::mpsc::sync_channel(1);
    let stage_receive = Rc::new(stage_receive);
    let (flush_send, flush_receive) = std::sync::mpsc::sync_channel(1);
    let flush_receive = Rc::new(flush_receive);
    std::thread::spawn(move || {
        let result = (|| -> io::Result<()> {
            let mut input = io::stdin().lock();
            while let Some(metadata) = read_frame(&mut input, META_LIMIT)? {
                match serde_json::from_slice::<ParentMessage>(&metadata)? {
                    ParentMessage::Request { metadata } => {
                        let body = read_frame(&mut input, BODY_CEILING)?
                            .ok_or_else(|| io::Error::other("missing request body"))?;
                        if send.send((metadata, body)).is_err() {
                            return Ok(());
                        }
                    }
                    ParentMessage::FlushAck => {
                        if flush_send.send(()).is_err() {
                            return Ok(());
                        }
                    }
                    ParentMessage::StageReply { path } => {
                        if stage_send.send(path).is_err() {
                            return Ok(());
                        }
                    }
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
        let mut runtime = Runtime::with_options(options.clone());
        runtime.set_input_stream(io::empty());
        // Script error output belongs to the server log, never its response body.
        runtime.set_error_stream(io::stderr());
        register(&mut runtime)?;
        let headers = metadata.headers;
        let request = scr_stdlib::web::Request {
            method: metadata.method,
            path: metadata.path,
            query_string: metadata.query,
            headers: headers.clone(),
            body,
        };
        let (state, uploads) = if let Some(metadata) = metadata.multipart {
            if !metadata.spool.starts_with("request-")
                || metadata.spool.len() > 128
                || !metadata
                    .spool
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err("invalid upload spool".into());
            }
            let directory = upload_root.join(metadata.spool);
            let mut files = vec![];
            for file in metadata.uploads {
                if !file.file.starts_with("file-")
                    || !file.file[5..].bytes().all(|b| b.is_ascii_digit())
                {
                    return Err("invalid upload file".into());
                }
                files.push(scr_stdlib::web::uploads::File {
                    field_name: file.field_name,
                    filename: file.filename,
                    media_type: file.media_type,
                    size: file.size,
                    path: directory.join(file.file),
                });
            }
            let data = scr_stdlib::web::uploads::Multipart {
                is_multipart: metadata.parsed,
                body: directory.join("body"),
                body_size: metadata.raw_size,
                fields: metadata.fields,
                files,
            };
            let protocol = protocol.clone();
            let receive = stage_receive.clone();
            let stage = scr_stdlib::web::uploads::Stage::from(Rc::new(
                move |index, parent: &std::path::Path| {
                    let message = serde_json::to_vec(&ChildMessage::StageRequest {
                        index,
                        parent: WirePath::new(parent),
                    })
                    .map_err(|_| {
                        scr_runtime::NativeError::new(
                            "UploadError",
                            "cannot request upload staging",
                        )
                    })?;
                    write_frame(&mut *protocol.borrow_mut(), &message)?;
                    receive
                        .recv()
                        .map_err(|_| {
                            scr_runtime::NativeError::new(
                                "UploadError",
                                "upload supervisor disconnected",
                            )
                        })?
                        .ok_or_else(|| {
                            scr_runtime::NativeError::new(
                                "UploadError",
                                "cannot allocate upload staging file",
                            )
                        })?
                        .path()
                        .map_err(Into::into)
                },
            )
                as Rc<dyn Fn(usize, &std::path::Path) -> scr_runtime::NativeResult<PathBuf>>);
            let (state, guard) = scr_stdlib::web::register_multipart(
                &mut runtime,
                request,
                data,
                scr_stdlib::FilesystemPolicy::default(),
                stage,
                HEADER_LIMIT,
            )?;
            (state, Some(guard))
        } else {
            (
                scr_stdlib::web::register(&mut runtime, request, HEADER_LIMIT)?,
                None,
            )
        };
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
            total: 0,
            started: false,
            protocol: protocol.clone(),
            acknowledgements: flush_receive.clone(),
            response: state.clone(),
            session: session.clone(),
        };
        let result = runtime.execute_program_with_source(&program, source.clone(), &mut output);
        let committed = if matches!(&result, Ok(0)) && !output.exceeded {
            session.finish()
        } else {
            Ok(())
        };
        let success = matches!((&result, &committed), (Ok(0), Ok(()))) && !output.exceeded;
        if let Err(error) = committed {
            eprintln!("web session commit failed: {error}");
        }
        if let Err(error) = result {
            eprintln!(
                "{}",
                error.render(&source.path.to_string_lossy(), &source.text)
            );
        } else if !success {
            eprintln!("web request failed: nonzero exit or response output limit");
        }
        let transmitted = output.started;
        let final_output = if transmitted && success {
            output.chunks()
        } else {
            Ok(())
        };
        let (metadata, body) = if success {
            let response = state.borrow();
            (
                ResponseMeta {
                    status: response.status,
                    headers: response.headers.clone(),
                },
                std::mem::take(&mut output.bytes),
            )
        } else {
            (
                ResponseMeta {
                    status: 500,
                    headers: vec![("content-type".into(), "text/plain; charset=utf-8".into())],
                },
                b"Internal Server Error\n".to_vec(),
            )
        };
        drop(output);
        drop(session);
        drop(runtime);
        drop(uploads);
        drop(state);
        final_output?;
        if transmitted {
            write_frame(
                &mut *protocol.borrow_mut(),
                &serde_json::to_vec(&ChildMessage::Done { success })?,
            )?;
        } else {
            write_frame(
                &mut *protocol.borrow_mut(),
                &serde_json::to_vec(&ChildMessage::Response { metadata })?,
            )?;
            write_frame(&mut *protocol.borrow_mut(), &body)?;
        }
        // Runtime and native resources are dropped before accepting the next request.
    }
    Ok(())
}
