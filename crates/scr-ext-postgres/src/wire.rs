//! Bounded synchronous PostgreSQL v3 transport. The worker is its sole owner.
use crate::{
    protocol::{Failure, Limits, failure},
    settings::Config,
    sql::Parameter,
};
use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use postgres_protocol::{
    IsNull,
    authentication::sasl::{ChannelBinding, SCRAM_SHA_256, ScramSha256},
    message::{
        backend::{self, Message},
        frontend,
    },
};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Column {
    pub name: String,
    pub oid: u32,
}
pub(crate) type Row = Vec<Option<Vec<u8>>>;
pub(crate) struct Response {
    pub columns: Vec<Column>,
    pub parameters: Vec<u32>,
    pub rows: Vec<Row>,
    pub affected: u64,
    pub bytes: usize,
}
pub(crate) struct Session {
    stream: rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    stop: Arc<AtomicBool>,
    packet_limit: usize,
    deadline: Instant,
    pub status: u8,
}
fn io_error(error: std::io::Error) -> Failure {
    let timeout = matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    );
    failure(
        if timeout { "DbTimeoutError" } else { "DbError" },
        if timeout {
            "PostgreSQL I/O deadline exceeded"
        } else {
            "PostgreSQL transport or protocol failed"
        },
        true,
    )
}
fn protocol_error() -> Failure {
    failure("DbError", "invalid PostgreSQL protocol response", true)
}
impl Session {
    pub fn open(
        config: Config,
        ca: Option<&[u8]>,
        packet_limit: usize,
        timeout: Duration,
        stop: Arc<AtomicBool>,
    ) -> Result<Self, Failure> {
        let connect_timeout = Duration::from_millis(config.attributes.connect_timeout_ms);
        let deadline = Instant::now()
            .checked_add(connect_timeout)
            .ok_or_else(protocol_error)?;
        let host = config.host.trim_start_matches('[').trim_end_matches(']');
        let addresses = (host, config.port).to_socket_addrs().map_err(io_error)?;
        let mut tcp = None;
        for address in addresses {
            if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                return Err(failure(
                    "DbTimeoutError",
                    "PostgreSQL connection deadline exceeded",
                    true,
                ));
            }
            if let Ok(stream) = TcpStream::connect_timeout(
                &address,
                deadline.saturating_duration_since(Instant::now()),
            ) {
                tcp = Some(stream);
                break;
            }
        }
        let mut tcp =
            tcp.ok_or_else(|| failure("DbError", "PostgreSQL TCP connection failed", true))?;
        tcp.set_read_timeout(Some(connect_timeout))
            .map_err(io_error)?;
        tcp.set_write_timeout(Some(connect_timeout))
            .map_err(io_error)?;
        let mut ssl = BytesMut::new();
        frontend::ssl_request(&mut ssl);
        tcp.write_all(&ssl).map_err(io_error)?;
        let mut reply = [0];
        tcp.read_exact(&mut reply).map_err(io_error)?;
        if reply[0] != b'S' {
            return Err(failure(
                "DbError",
                "PostgreSQL verified TLS is required",
                true,
            ));
        }
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if let Some(mut ca) = ca {
            let certificates = rustls_pemfile::certs(&mut ca)
                .collect::<Result<Vec<_>, _>>()
                .map_err(io_error)?;
            if certificates.is_empty() {
                return Err(failure(
                    "DbError",
                    "PostgreSQL CA file has no certificates",
                    true,
                ));
            }
            for cert in certificates {
                roots
                    .add(cert)
                    .map_err(|_| failure("DbError", "invalid PostgreSQL CA certificate", true))?;
            }
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| protocol_error())?
        .with_root_certificates(roots)
        .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|_| protocol_error())?;
        let tls =
            rustls::ClientConnection::new(Arc::new(tls), name).map_err(|_| protocol_error())?;
        let mut session = Self {
            stream: rustls::StreamOwned::new(tls, tcp),
            stop,
            packet_limit,
            deadline,
            status: b'I',
        };
        let mut startup = BytesMut::new();
        frontend::startup_message(
            [
                ("user", config.user.as_str()),
                ("database", config.database.as_str()),
                ("client_encoding", "UTF8"),
                ("application_name", "scripta"),
            ],
            &mut startup,
        )
        .map_err(io_error)?;
        session.send(&startup)?;
        let mut scram: Option<ScramSha256> = None;
        let mut authenticated = false;
        let mut verified = false;
        let mut continued = false;
        loop {
            match session.read_message()? {
                Message::AuthenticationSasl(body) if scram.is_none() && !authenticated => {
                    if !body
                        .mechanisms()
                        .any(|m| Ok(m == SCRAM_SHA_256))
                        .map_err(io_error)?
                    {
                        return Err(failure(
                            "DbUnsupportedError",
                            "PostgreSQL requires SCRAM-SHA-256 authentication",
                            true,
                        ));
                    }
                    let state =
                        ScramSha256::new(config.password.as_bytes(), ChannelBinding::unsupported());
                    let mut message = BytesMut::new();
                    frontend::sasl_initial_response(SCRAM_SHA_256, state.message(), &mut message)
                        .map_err(io_error)?;
                    session.send(&message)?;
                    scram = Some(state);
                }
                Message::AuthenticationSaslContinue(body)
                    if scram.is_some() && !verified && !continued =>
                {
                    // Bound server-controlled PBKDF work before calling the library.
                    let text = std::str::from_utf8(body.data()).map_err(|_| protocol_error())?;
                    let iterations = text
                        .split(',')
                        .find_map(|s| s.strip_prefix("i="))
                        .and_then(|s| s.parse::<u32>().ok())
                        .ok_or_else(protocol_error)?;
                    if iterations == 0 || iterations > 100_000 {
                        return Err(failure(
                            "DbLimitError",
                            "PostgreSQL SCRAM work limit exceeded",
                            true,
                        ));
                    }
                    continued = true;
                    let state = scram.as_mut().ok_or_else(protocol_error)?;
                    state.update(body.data()).map_err(io_error)?;
                    let mut message = BytesMut::new();
                    frontend::sasl_response(state.message(), &mut message).map_err(io_error)?;
                    session.send(&message)?;
                }
                Message::AuthenticationSaslFinal(body) if continued && !verified => {
                    scram
                        .as_mut()
                        .ok_or_else(protocol_error)?
                        .finish(body.data())
                        .map_err(io_error)?;
                    verified = true;
                }
                Message::AuthenticationOk if verified => authenticated = true,
                Message::ErrorResponse(body) => return Err(server_error(body)?),
                Message::ParameterStatus(_)
                | Message::BackendKeyData(_)
                | Message::NoticeResponse(_) => {}
                Message::ReadyForQuery(body) if authenticated => {
                    session.status = body.status();
                    break;
                }
                _ => {
                    return Err(failure(
                        "DbUnsupportedError",
                        "PostgreSQL authentication or startup response is unsupported",
                        true,
                    ));
                }
            }
        }
        session.simple("SET standard_conforming_strings=on; SET DateStyle='ISO, YMD'; SET TimeZone='UTC'; SET bytea_output='hex'; SET IntervalStyle='iso_8601'", None)?;
        session
            .stream
            .sock
            .set_read_timeout(Some(timeout))
            .map_err(io_error)?;
        session
            .stream
            .sock
            .set_write_timeout(Some(timeout))
            .map_err(io_error)?;
        Ok(session)
    }
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }
    fn checkpoint(&self) -> Result<(), Failure> {
        if self.stop.load(Ordering::Acquire) {
            return Err(failure(
                "Interrupted",
                "PostgreSQL operation cancelled",
                true,
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(failure(
                "DbTimeoutError",
                "PostgreSQL operation deadline exceeded",
                true,
            ));
        }
        Ok(())
    }
    fn send(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        self.checkpoint()?;
        self.stream
            .sock
            .set_write_timeout(Some(
                self.deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(1)),
            ))
            .map_err(io_error)?;
        self.stream.write_all(bytes).map_err(io_error)?;
        self.stream.flush().map_err(io_error)
    }
    fn read_exact(&mut self, mut bytes: &mut [u8]) -> Result<(), Failure> {
        while !bytes.is_empty() {
            self.checkpoint()?;
            self.stream
                .sock
                .set_read_timeout(Some(
                    self.deadline
                        .saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1)),
                ))
                .map_err(io_error)?;
            let count = self.stream.read(bytes).map_err(io_error)?;
            if count == 0 {
                return Err(failure("DbError", "PostgreSQL connection closed", true));
            }
            bytes = &mut bytes[count..];
        }
        Ok(())
    }
    fn read_message(&mut self) -> Result<Message, Failure> {
        let mut header = [0u8; 5];
        self.read_exact(&mut header)?;
        let size = i32::from_be_bytes(header[1..].try_into().map_err(|_| protocol_error())?);
        let size = usize::try_from(size).map_err(|_| protocol_error())?;
        if size < 4 {
            return Err(protocol_error());
        }
        if size > self.packet_limit {
            return Err(failure(
                "DbLimitError",
                "PostgreSQL protocol frame exceeds byte limit",
                true,
            ));
        }
        let mut buffer = BytesMut::with_capacity(size + 1);
        buffer.extend_from_slice(&header);
        buffer.resize(size + 1, 0);
        self.read_exact(&mut buffer[5..])?;
        backend::Message::parse(&mut buffer)
            .map_err(io_error)?
            .ok_or_else(protocol_error)
    }
    pub fn simple(&mut self, sql: &str, limits: Option<&Limits>) -> Result<Response, Failure> {
        let mut buffer = BytesMut::new();
        frontend::query(sql, &mut buffer).map_err(io_error)?;
        self.send(&buffer)?;
        self.collect(limits)
    }
    pub fn prepare(&mut self, name: &str, sql: &str) -> Result<Response, Failure> {
        let mut buffer = BytesMut::new();
        frontend::parse(name, sql, [], &mut buffer).map_err(io_error)?;
        frontend::describe(b'S', name, &mut buffer).map_err(io_error)?;
        frontend::sync(&mut buffer);
        self.send(&buffer)?;
        self.collect(None)
    }
    pub fn execute(
        &mut self,
        name: &str,
        params: &[Parameter],
        limits: &Limits,
    ) -> Result<Response, Failure> {
        self.deadline = self.deadline.min(limits.deadline);
        let mut buffer = BytesMut::new();
        frontend::bind(
            "",
            name,
            params.iter().map(|p| i16::from(p.binary)),
            params.iter(),
            |p, out| {
                if let Some(data) = &p.data {
                    out.extend_from_slice(data);
                    Ok(IsNull::No)
                } else {
                    Ok(IsNull::Yes)
                }
            },
            [0],
            &mut buffer,
        )
        .map_err(|_| protocol_error())?;
        frontend::describe(b'P', "", &mut buffer).map_err(io_error)?;
        frontend::execute("", 0, &mut buffer).map_err(io_error)?;
        frontend::sync(&mut buffer);
        self.send(&buffer)?;
        self.collect(Some(limits))
    }
    pub fn close_statement(&mut self, name: &str) -> Result<(), Failure> {
        let mut buffer = BytesMut::new();
        frontend::close(b'S', name, &mut buffer).map_err(io_error)?;
        frontend::sync(&mut buffer);
        self.send(&buffer)?;
        self.collect(None).map(|_| ())
    }
    fn collect(&mut self, limits: Option<&Limits>) -> Result<Response, Failure> {
        let mut out = Response {
            columns: Vec::new(),
            parameters: Vec::new(),
            rows: Vec::new(),
            affected: 0,
            bytes: 0,
        };
        let mut error = None;
        loop {
            self.checkpoint()?;
            match self.read_message()? {
                Message::ReadyForQuery(body) => {
                    if !matches!(body.status(), b'I' | b'T' | b'E') {
                        return Err(protocol_error());
                    }
                    self.status = body.status();
                    return error.map_or(Ok(out), Err);
                }
                Message::ErrorResponse(body) => {
                    error = Some(server_error(body)?);
                }
                Message::ParameterDescription(body) => {
                    out.parameters = body.parameters().collect().map_err(io_error)?;
                }
                Message::RowDescription(body) => {
                    let mut fields = body.fields();
                    while let Some(field) = fields.next().map_err(io_error)? {
                        self.checkpoint()?;
                        out.bytes = out
                            .bytes
                            .saturating_add(field.name().len())
                            .saturating_add(std::mem::size_of::<Column>());
                        if out.bytes > limits.map_or(self.packet_limit, |l| l.bytes) {
                            return Err(failure(
                                "DbLimitError",
                                "PostgreSQL column metadata exceeds limit",
                                true,
                            ));
                        }
                        if field.format() != 0 {
                            return Err(protocol_error());
                        }
                        out.columns.push(Column {
                            name: field.name().into(),
                            oid: field.type_oid(),
                        });
                    }
                }
                Message::DataRow(body) => {
                    let limits = limits.ok_or_else(|| {
                        failure(
                            "DbUnsupportedError",
                            "unexpected PostgreSQL result rows",
                            true,
                        )
                    })?;
                    if out.rows.len() >= limits.rows {
                        return Err(failure(
                            "DbLimitError",
                            "PostgreSQL row limit exceeded",
                            true,
                        ));
                    }
                    out.bytes = out.bytes.saturating_add(std::mem::size_of::<Row>());
                    let mut row = Vec::new();
                    let mut ranges = body.ranges();
                    let raw = body.buffer();
                    while let Some(range) = ranges.next().map_err(io_error)? {
                        self.checkpoint()?;
                        let value = range.map(|r| &raw[r]);
                        if value.is_some_and(|v| {
                            v.len() > limits.cell.saturating_mul(2).saturating_add(2)
                        }) {
                            return Err(failure(
                                "DbLimitError",
                                "PostgreSQL cell limit exceeded",
                                true,
                            ));
                        }
                        out.bytes = out
                            .bytes
                            .saturating_add(std::mem::size_of::<Option<Vec<u8>>>())
                            .saturating_add(value.map_or(0, <[u8]>::len));
                        if out.bytes > limits.bytes {
                            return Err(failure(
                                "DbLimitError",
                                "PostgreSQL result byte limit exceeded",
                                true,
                            ));
                        }
                        row.push(value.map(<[u8]>::to_vec));
                    }
                    if row.len() != out.columns.len() {
                        return Err(protocol_error());
                    }
                    out.rows.push(row);
                }
                Message::CommandComplete(body) => {
                    let tag = body.tag().map_err(io_error)?;
                    if matches!(
                        tag.split_whitespace().next(),
                        Some("INSERT" | "UPDATE" | "DELETE" | "SELECT")
                    ) {
                        out.affected = tag
                            .split_whitespace()
                            .last()
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(protocol_error)?;
                    }
                }
                Message::ParseComplete
                | Message::BindComplete
                | Message::CloseComplete
                | Message::NoData
                | Message::EmptyQueryResponse
                | Message::NoticeResponse(_) => {}
                Message::ParameterStatus(body) => {
                    let name = body.name().map_err(io_error)?;
                    let value = body.value().map_err(io_error)?;
                    if (name == "standard_conforming_strings" && value != "on")
                        || (name == "client_encoding" && value != "UTF8")
                    {
                        return Err(failure(
                            "DbUnsupportedError",
                            "PostgreSQL session changed required SQL or encoding settings",
                            true,
                        ));
                    }
                }
                _ => {
                    return Err(failure(
                        "DbUnsupportedError",
                        "PostgreSQL response form is unsupported",
                        true,
                    ));
                }
            }
        }
    }
}
fn server_error(body: backend::ErrorResponseBody) -> Result<Failure, Failure> {
    let mut fields = body.fields();
    let mut state = None;
    while let Some(field) = fields.next().map_err(io_error)? {
        if field.type_() == b'C' {
            let value = std::str::from_utf8(field.value_bytes()).map_err(|_| protocol_error())?;
            if value.len() == 5
                && value
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            {
                state = Some(value.to_owned());
            }
        }
    }
    let category = match state.as_deref().and_then(|s| s.get(..2)) {
        Some("23") => scr_db::ErrorCategory::Constraint,
        Some("28") => scr_db::ErrorCategory::Authentication,
        Some("08") => scr_db::ErrorCategory::Connection,
        _ => scr_db::ErrorCategory::Query,
    };
    let mut failed = failure(
        "DbError",
        &format!(
            "PostgreSQL operation failed (SQLSTATE {})",
            state.as_deref().unwrap_or("unknown")
        ),
        false,
    );
    failed.details = Some(scr_db::ErrorDetails {
        category,
        sql_state: state,
        driver_code: None,
    });
    Ok(failed)
}
