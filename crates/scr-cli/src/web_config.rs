//! CLI/file settings share one schema; filesystem preparation stays in the host.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};
type Error = Box<dyn std::error::Error>;

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Settings {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    health_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trusted_proxies: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_hosts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workers: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    queue: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_connections: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_body_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_response_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    connection_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    header_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    drain_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_temp_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_file_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_fields: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_field_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_fields_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_header_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_max_disk_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_ttl_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_max_records: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_max_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_cookie_secure: Option<bool>,
}
impl Settings {
    fn apply(self, config: &mut scr_web::Config) -> Result<(), Error> {
        if let Some(value) = self.log_format {
            config.log_format = match value.as_str() {
                "text" => scr_web::LogFormat::Text,
                "json" => scr_web::LogFormat::Json,
                _ => return Err("log-format must be text or json".into()),
            };
        }
        if let Some(value) = self.health_address {
            config.health_address = if value == "off" {
                None
            } else {
                Some(
                    value
                        .parse()
                        .map_err(|_| "health-address must be a loopback IP:port or off")?,
                )
            };
        }
        if let Some(value) = self.trusted_proxies {
            config.trusted_proxies = value;
        }
        if let Some(value) = self.allowed_hosts {
            config.allowed_hosts = value;
        }
        if let Some(value) = self.host {
            config.host = value
                .parse()
                .map_err(|_| "host must be a literal IP address")?;
        }
        if let Some(value) = self.port {
            config.port = value;
        }
        if let Some(value) = self.workers {
            config.workers = value;
        }
        if let Some(value) = self.queue {
            config.queue = value;
        }
        if let Some(value) = self.max_connections {
            config.max_connections = value;
        }
        if let Some(value) = self.max_body_bytes {
            config.max_body_bytes = value;
        }
        if let Some(value) = self.max_response_bytes {
            config.max_response_bytes = value;
        }
        if let Some(value) = self.timeout_ms {
            config.request_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.connection_timeout_ms {
            config.connection_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.header_timeout_ms {
            config.header_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.drain_timeout_ms {
            config.drain_timeout = Duration::from_millis(value);
        }
        if self
            .upload_temp_dir
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
        {
            return Err("upload-temp-dir must not be empty".into());
        }
        if let Some(value) = self.upload_temp_dir {
            config.uploads.directory = value;
        }
        if let Some(value) = self.upload_max_file_bytes {
            config.uploads.max_file_bytes = value;
        }
        if let Some(value) = self.upload_max_files {
            config.uploads.max_files = value;
        }
        if let Some(value) = self.upload_max_fields {
            config.uploads.max_fields = value;
        }
        if let Some(value) = self.upload_max_field_bytes {
            config.uploads.max_field_bytes = value;
        }
        if let Some(value) = self.upload_max_fields_bytes {
            config.uploads.max_fields_bytes = value;
        }
        if let Some(value) = self.upload_max_header_bytes {
            config.uploads.max_header_bytes = value;
        }
        if let Some(value) = self.upload_max_disk_bytes {
            config.uploads.max_disk_bytes = value;
        }
        if self
            .session_dir
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
        {
            return Err("session-dir must not be empty".into());
        }
        if let Some(value) = self.session_dir {
            config.sessions.directory = value;
        }
        if let Some(value) = self.session_ttl_seconds {
            config.sessions.ttl = Duration::from_secs(value);
        }
        if let Some(value) = self.session_max_records {
            config.sessions.max_sessions = value;
        }
        if let Some(value) = self.session_max_bytes {
            config.sessions.max_bytes = value;
        }
        if let Some(value) = self.session_cookie_secure {
            config.sessions.secure = value;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Serve,
    Check,
    Print,
}
pub(super) struct Options {
    settings: Settings,
    file: Option<PathBuf>,
    pub mode: Mode,
}
impl Options {
    pub fn parse(arguments: impl Iterator<Item = OsString>) -> Result<Self, Error> {
        let mut args = arguments;
        let mut settings = Settings::default();
        let mut file = None;
        let mut mode = Mode::Serve;
        let mut seen = std::collections::BTreeSet::new();
        while let Some(key) = args.next() {
            let key = key.to_str().ok_or("option must be UTF-8")?;
            if !seen.insert(key.to_owned()) {
                return Err(format!("duplicate serve option {key}").into());
            }
            if matches!(key, "--check-config" | "--print-config") {
                if mode != Mode::Serve {
                    return Err("--check-config and --print-config are mutually exclusive".into());
                }
                mode = if key == "--check-config" {
                    Mode::Check
                } else {
                    Mode::Print
                };
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {key}"))?;
            if key == "--config" {
                if value.is_empty() {
                    return Err("config path must not be empty".into());
                }
                file = Some(PathBuf::from(value));
                continue;
            }
            // Parse errors identify the setting, never echo its supplied value.
            match key {
                "--log-format" => {
                    settings.log_format =
                        Some(value.to_str().ok_or("log-format must be UTF-8")?.into())
                }
                "--health-address" => {
                    settings.health_address =
                        Some(value.to_str().ok_or("health-address must be UTF-8")?.into())
                }
                "--trusted-proxies" => settings.trusted_proxies = Some(list(&value)?),
                "--allowed-hosts" => settings.allowed_hosts = Some(list(&value)?),
                "--host" => {
                    settings.host = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<String>().ok())
                            .ok_or("invalid value for --host")?,
                    )
                }
                "--port" => {
                    settings.port = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u16>().ok())
                            .ok_or("invalid value for --port")?,
                    )
                }
                "--workers" => {
                    settings.workers = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --workers")?,
                    )
                }
                "--queue" => {
                    settings.queue = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --queue")?,
                    )
                }
                "--max-connections" => {
                    settings.max_connections = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --max-connections")?,
                    )
                }
                "--max-body-bytes" => {
                    settings.max_body_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --max-body-bytes")?,
                    )
                }
                "--max-response-bytes" => {
                    settings.max_response_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --max-response-bytes")?,
                    )
                }
                "--timeout-ms" => {
                    settings.timeout_ms = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("invalid value for --timeout-ms")?,
                    )
                }
                "--connection-timeout-ms" => {
                    settings.connection_timeout_ms = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("invalid value for --connection-timeout-ms")?,
                    )
                }
                "--header-timeout-ms" => {
                    settings.header_timeout_ms = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("invalid value for --header-timeout-ms")?,
                    )
                }
                "--drain-timeout-ms" => {
                    settings.drain_timeout_ms = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("invalid value for --drain-timeout-ms")?,
                    )
                }
                "--upload-temp-dir" => settings.upload_temp_dir = Some(PathBuf::from(value)),
                "--upload-max-file-bytes" => {
                    settings.upload_max_file_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-file-bytes")?,
                    )
                }
                "--upload-max-files" => {
                    settings.upload_max_files = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-files")?,
                    )
                }
                "--upload-max-fields" => {
                    settings.upload_max_fields = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-fields")?,
                    )
                }
                "--upload-max-field-bytes" => {
                    settings.upload_max_field_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-field-bytes")?,
                    )
                }
                "--upload-max-fields-bytes" => {
                    settings.upload_max_fields_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-fields-bytes")?,
                    )
                }
                "--upload-max-header-bytes" => {
                    settings.upload_max_header_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-header-bytes")?,
                    )
                }
                "--upload-max-disk-bytes" => {
                    settings.upload_max_disk_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --upload-max-disk-bytes")?,
                    )
                }
                "--session-dir" => settings.session_dir = Some(PathBuf::from(value)),
                "--session-ttl-seconds" => {
                    settings.session_ttl_seconds = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("invalid value for --session-ttl-seconds")?,
                    )
                }
                "--session-max-records" => {
                    settings.session_max_records = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --session-max-records")?,
                    )
                }
                "--session-max-bytes" => {
                    settings.session_max_bytes = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .ok_or("invalid value for --session-max-bytes")?,
                    )
                }
                "--session-cookie-secure" => {
                    settings.session_cookie_secure = Some(
                        value
                            .to_str()
                            .and_then(|s| s.parse::<bool>().ok())
                            .ok_or("invalid value for --session-cookie-secure")?,
                    )
                }
                _ => return Err(format!("unknown serve option {key}").into()),
            }
        }
        Ok(Self {
            settings,
            file,
            mode,
        })
    }
    pub fn apply(self, config: &mut scr_web::Config) -> Result<(), Error> {
        if let Some(path) = self.file {
            let settings = read_settings(&path)?;
            settings.apply(config)?;
        }
        self.settings.apply(config)?;
        config.validate()?;
        Ok(())
    }
}
fn list(value: &std::ffi::OsStr) -> Result<Vec<String>, Error> {
    let value = value.to_str().ok_or("proxy/host list must be UTF-8")?;
    if value.is_empty() {
        return Ok(vec![]);
    }
    if value.len() > 128 * 261 {
        return Err("proxy/host list exceeds its byte limit".into());
    }
    let values: Vec<_> = value.split(',').map(|s| s.trim().to_owned()).collect();
    if values.len() > 128 || values.iter().any(|s| s.is_empty()) {
        return Err("proxy/host list requires 1..128 nonempty comma-separated entries".into());
    }
    Ok(values)
}
fn read_settings(path: &Path) -> Result<Settings, Error> {
    // Bound configuration input independently of script source limits.
    let file = std::fs::File::open(path).map_err(|_| "cannot open server configuration")?;
    if !file.metadata()?.is_file() {
        return Err("server configuration must be a regular file".into());
    }
    use std::io::Read;
    let mut data = String::new();
    file.take(65537)
        .read_to_string(&mut data)
        .map_err(|_| "cannot read UTF-8 server configuration")?;
    if data.len() > 65536 {
        return Err("server configuration exceeds 65536 bytes".into());
    }
    let table: toml::Table = toml::from_str(&data).map_err(|e: toml::de::Error| {
        let line = data.as_bytes()[..e.span().map_or(0, |s| s.start)]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
            + 1;
        format!("invalid TOML server configuration near line {line}")
    })?;
    // Validate types with safe diagnostics rather than printing TOML parser
    // excerpts, which can contain credentials accidentally placed in this file.
    for (key, value) in &table {
        let valid =
            match key.as_str() {
                "trusted-proxies" | "allowed-hosts" => value.as_array().is_some_and(|v| {
                    v.len() <= 128 && v.iter().all(|s| s.as_str().is_some_and(|s| !s.is_empty()))
                }),
                "host" | "log-format" | "health-address" => value.is_str(),
                "port" => value.as_integer().is_some_and(|n| n >= 0),
                "workers" => value.as_integer().is_some_and(|n| n >= 0),
                "queue" => value.as_integer().is_some_and(|n| n >= 0),
                "max-connections" => value.as_integer().is_some_and(|n| n >= 0),
                "max-body-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "max-response-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "timeout-ms" => value.as_integer().is_some_and(|n| n >= 0),
                "connection-timeout-ms" => value.as_integer().is_some_and(|n| n >= 0),
                "header-timeout-ms" => value.as_integer().is_some_and(|n| n >= 0),
                "drain-timeout-ms" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-temp-dir" => value.as_str().is_some_and(|s| !s.is_empty()),
                "upload-max-file-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-files" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-fields" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-field-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-fields-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-header-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "upload-max-disk-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "session-dir" => value.as_str().is_some_and(|s| !s.is_empty()),
                "session-ttl-seconds" => value.as_integer().is_some_and(|n| n >= 0),
                "session-max-records" => value.as_integer().is_some_and(|n| n >= 0),
                "session-max-bytes" => value.as_integer().is_some_and(|n| n >= 0),
                "session-cookie-secure" => value.is_bool(),
                _ => return Err(
                    "unknown server configuration key; see docs/web-servers.md for supported keys"
                        .into(),
                ),
            };
        if !valid {
            return Err(format!("invalid type or empty path for server setting {key}").into());
        }
    }
    let mut settings: Settings = table
        .try_into()
        .map_err(|_| "server configuration value exceeds its setting's integer range")?;
    // File paths are relative to the configuration file; CLI paths remain relative
    // to the working directory. Do not change the application's working directory.
    let base = path.parent().unwrap_or(Path::new("."));
    for path in [&mut settings.session_dir, &mut settings.upload_temp_dir]
        .into_iter()
        .flatten()
    {
        if path.is_relative() {
            *path = base.join(&*path);
        }
    }
    Ok(settings)
}
pub(super) fn inspect(config: &scr_web::Config) -> Result<String, Error> {
    let settings = Settings {
        host: Some(config.host.to_string()),
        log_format: Some(
            if config.log_format == scr_web::LogFormat::Json {
                "json"
            } else {
                "text"
            }
            .into(),
        ),
        health_address: Some(
            config
                .health_address
                .map_or("off".into(), |a| a.to_string()),
        ),
        trusted_proxies: Some(config.trusted_proxies.clone()),
        allowed_hosts: Some(config.allowed_hosts.clone()),
        port: Some(config.port),
        workers: Some(config.workers),
        queue: Some(config.queue),
        max_connections: Some(config.max_connections),
        max_body_bytes: Some(config.max_body_bytes),
        max_response_bytes: Some(config.max_response_bytes),
        timeout_ms: Some(config.request_timeout.as_millis() as u64),
        connection_timeout_ms: Some(config.connection_timeout.as_millis() as u64),
        header_timeout_ms: Some(config.header_timeout.as_millis() as u64),
        drain_timeout_ms: Some(config.drain_timeout.as_millis() as u64),
        upload_temp_dir: Some(PathBuf::from(
            if config.uploads.directory.as_os_str().is_empty() {
                "<temporary>"
            } else {
                "<configured>"
            },
        )),
        upload_max_file_bytes: Some(config.uploads.max_file_bytes),
        upload_max_files: Some(config.uploads.max_files),
        upload_max_fields: Some(config.uploads.max_fields),
        upload_max_field_bytes: Some(config.uploads.max_field_bytes),
        upload_max_fields_bytes: Some(config.uploads.max_fields_bytes),
        upload_max_header_bytes: Some(config.uploads.max_header_bytes),
        upload_max_disk_bytes: Some(config.uploads.max_disk_bytes),
        session_dir: Some(PathBuf::from(
            if config.sessions.directory.as_os_str().is_empty() {
                "<temporary>"
            } else {
                "<configured>"
            },
        )),
        session_ttl_seconds: Some(config.sessions.ttl.as_secs()),
        session_max_records: Some(config.sessions.max_sessions),
        session_max_bytes: Some(config.sessions.max_bytes),
        session_cookie_secure: Some(config.sessions.secure),
    };
    let mut table = toml::Table::try_from(settings)?;
    let limits = crate::entry::limits()?;
    let runtime = toml::Table::from_iter([
        (
            "max-steps".into(),
            toml::Value::String(limits.instruction_limit.unwrap_or(1_000_000).to_string()),
        ),
        (
            "max-value-bytes".into(),
            toml::Value::String(
                limits
                    .max_value_bytes
                    .unwrap_or(16 * 1024 * 1024)
                    .to_string(),
            ),
        ),
        (
            "max-scopes".into(),
            toml::Value::String(limits.max_scopes.unwrap_or(4096).to_string()),
        ),
        (
            "max-source-bytes".into(),
            toml::Value::String(limits.max_source_bytes.to_string()),
        ),
    ]);
    table.insert("runtime".into(), toml::Value::Table(runtime));
    Ok(toml::to_string_pretty(&table)?)
}
