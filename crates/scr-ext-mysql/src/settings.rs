use scr_runtime::{MapKey, NativeError, NativeResult, Value};
use std::{collections::BTreeMap, net::IpAddr, time::Duration};

/// An exact hostname/IP and TCP port that the host permits. Hostname grants trust
/// that hostname's DNS resolution; they are not an IP-subnet filter.
#[derive(Debug, Clone)]
pub struct Destination {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct MysqlOptions {
    pub destinations: Option<Vec<Destination>>,
    pub filesystem: scr_stdlib::FilesystemPolicy,
    pub max_workers: usize,
    pub max_connect_timeout_ms: u64,
    pub max_operation_timeout_ms: u64,
    pub max_rows: usize,
    pub max_result_bytes: usize,
    pub max_ca_bytes: usize,
    pub max_parameters: usize,
    pub max_cell_bytes: usize,
    pub max_parameter_bytes: usize,
}
impl Default for MysqlOptions {
    fn default() -> Self {
        Self {
            destinations: None,
            filesystem: scr_stdlib::FilesystemPolicy::default(),
            max_workers: 64,
            max_connect_timeout_ms: 5000,
            max_operation_timeout_ms: 30000,
            max_rows: 10000,
            max_result_bytes: 8 * 1024 * 1024,
            max_ca_bytes: 1024 * 1024,
            max_parameters: 1024,
            max_cell_bytes: 1024 * 1024,
            max_parameter_bytes: 8 * 1024 * 1024,
        }
    }
}
impl MysqlOptions {
    pub(crate) fn prepare(mut self) -> NativeResult<Self> {
        if self.max_workers == 0
            || self.max_connect_timeout_ms == 0
            || self.max_operation_timeout_ms == 0
            || self.max_rows == 0
            || self.max_result_bytes == 0
            || self.max_ca_bytes == 0
            || self.max_parameters == 0
            || self.max_parameters > u16::MAX as usize
            || self.max_cell_bytes == 0
            || self.max_parameter_bytes == 0
            || self.max_connect_timeout_ms > i64::MAX as u64
            || self.max_operation_timeout_ms > i64::MAX as u64
            || self.max_rows > i64::MAX as usize
            || self.max_result_bytes > i64::MAX as usize
        {
            return Err(invalid(
                "MySQL host limits must be positive representable integers",
            ));
        }
        self.filesystem = self.filesystem.prepare()?;
        if let Some(destinations) = &mut self.destinations {
            for destination in destinations {
                destination.host = host(&destination.host)?;
                if destination.port == 0 {
                    return Err(invalid("MySQL destination port must be positive"));
                }
            }
        }
        Ok(self)
    }
}

// No Debug: connection settings include credentials.
pub(crate) struct Config {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub attributes: Attributes,
}
#[derive(Debug, Clone)]
pub(crate) struct Attributes {
    pub connect_timeout_ms: u64,
    pub operation_timeout_ms: u64,
    pub ca_path: Option<String>,
    pub found_rows: bool,
    pub max_rows: usize,
    pub max_result_bytes: usize,
}

pub(crate) fn invalid(message: &str) -> NativeError {
    NativeError::type_error(message)
}
pub(crate) fn unsupported() -> NativeError {
    NativeError::new(
        "DbUnsupportedError",
        "MySQL attribute or value is not supported",
    )
}
fn host(value: &str) -> NativeResult<String> {
    let unwrapped = value
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(value);
    if let Ok(ip) = unwrapped.parse::<IpAddr>() {
        return Ok(match ip {
            IpAddr::V4(_) => ip.to_string(),
            IpAddr::V6(_) => format!("[{ip}]"),
        });
    }
    if value.len() > 253
        || value.is_empty()
        || !value.is_ascii()
        || value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err(invalid("invalid MySQL host"));
    }
    Ok(value.to_ascii_lowercase())
}
impl Config {
    pub fn parse(request: scr_db::ConnectRequest<'_>, policy: &MysqlOptions) -> NativeResult<Self> {
        if request.dsn.len() > 16384
            || request.user.is_empty()
            || request.user.len() > 4096
            || request.password.len() > 4096
            || request.dsn.contains('\0')
            || request.user.contains('\0')
            || request.password.contains('\0')
        {
            return Err(invalid("invalid MySQL DSN or credentials"));
        }
        let body = request
            .dsn
            .strip_prefix("mysql:")
            .ok_or_else(|| invalid("invalid MySQL DSN prefix"))?;
        let mut fields = BTreeMap::new();
        for field in body.split(';') {
            let (name, value) = field
                .split_once('=')
                .ok_or_else(|| invalid("invalid MySQL DSN field"))?;
            if !matches!(name, "host" | "port" | "dbname" | "charset")
                || value.is_empty()
                || fields.insert(name, value).is_some()
            {
                return Err(invalid("unknown, empty, or duplicate MySQL DSN field"));
            }
        }
        let host = host(
            fields
                .get("host")
                .ok_or_else(|| invalid("MySQL DSN requires host"))?,
        )?;
        let database = fields
            .get("dbname")
            .ok_or_else(|| invalid("MySQL DSN requires dbname"))?
            .to_string();
        let port = match fields.get("port") {
            Some(port) if port.bytes().all(|b| b.is_ascii_digit()) => port
                .parse::<u16>()
                .ok()
                .filter(|p| *p != 0)
                .ok_or_else(|| invalid("invalid MySQL port"))?,
            Some(_) => return Err(invalid("invalid MySQL port")),
            None => 3306,
        };
        if fields
            .get("charset")
            .is_some_and(|value| *value != "utf8mb4")
        {
            return Err(unsupported());
        }
        let mut attributes = Attributes {
            connect_timeout_ms: policy.max_connect_timeout_ms.min(5000),
            operation_timeout_ms: policy.max_operation_timeout_ms.min(30000),
            ca_path: None,
            found_rows: false,
            max_rows: policy.max_rows.min(10000),
            max_result_bytes: policy.max_result_bytes.min(8 * 1024 * 1024),
        };
        match request.options {
            Value::List(values) if values.is_empty() => {}
            Value::Map(values) => {
                for (name, value) in values {
                    let MapKey::String(name) = name else {
                        return Err(invalid("MySQL option names must be strings"));
                    };
                    attributes.set(name, value, policy, true)?;
                }
            }
            _ => return Err(invalid("MySQL options must be a map or empty list")),
        }
        if policy
            .destinations
            .as_ref()
            .is_some_and(|allowed| !allowed.iter().any(|d| d.host == host && d.port == port))
        {
            return Err(NativeError::new(
                "PermissionError",
                "MySQL destination denied by host policy",
            ));
        }
        Ok(Self {
            host,
            port,
            database,
            user: request.user.to_owned(),
            password: request.password.to_owned(),
            attributes,
        })
    }
}
impl Attributes {
    pub fn get(&self, name: &str) -> NativeResult {
        Ok(match name {
            "driverName" => Value::String("mysql".into()),
            "driverVersion" => Value::String(env!("CARGO_PKG_VERSION").into()),
            "errorMode" => Value::String("exception".into()),
            "fetchMode" => Value::String("assoc".into()),
            "emulatePrepares" => Value::Bool(false),
            "tlsMode" => Value::String("verifyIdentity".into()),
            "tlsCaFile" => self
                .ca_path
                .as_ref()
                .map_or(Value::Null, |path| Value::String(path.clone())),
            "connectTimeoutMs" => Value::Int(self.connect_timeout_ms as i64),
            "operationTimeoutMs" => Value::Int(self.operation_timeout_ms as i64),
            "mysqlFoundRows" => Value::Bool(self.found_rows),
            "maxRows" => Value::Int(self.max_rows as i64),
            "maxResultBytes" => Value::Int(self.max_result_bytes as i64),
            _ => return Err(unsupported()),
        })
    }
    pub fn set(
        &mut self,
        name: &str,
        value: &Value,
        policy: &MysqlOptions,
        connecting: bool,
    ) -> NativeResult<()> {
        if matches!(name, "driverName" | "driverVersion") {
            return Err(unsupported());
        }
        if !connecting
            && matches!(
                name,
                "connectTimeoutMs" | "tlsMode" | "tlsCaFile" | "mysqlFoundRows"
            )
        {
            return Err(NativeError::new(
                "DbStateError",
                "MySQL attribute is connection-only",
            ));
        }
        match name {
            "errorMode" | "fetchMode" | "tlsMode" => {
                let Value::String(text) = value else {
                    return Err(invalid("MySQL attribute requires a string"));
                };
                let expected = match name {
                    "errorMode" => "exception",
                    "fetchMode" => "assoc",
                    _ => "verifyIdentity",
                };
                if text != expected {
                    return Err(unsupported());
                }
            }
            "emulatePrepares" => match value {
                Value::Bool(false) => {}
                Value::Bool(true) => return Err(unsupported()),
                _ => return Err(invalid("emulatePrepares requires bool")),
            },
            "connectTimeoutMs" => {
                self.connect_timeout_ms = positive(value, policy.max_connect_timeout_ms)?
            }
            "operationTimeoutMs" => {
                self.operation_timeout_ms = positive(value, policy.max_operation_timeout_ms)?
            }
            "maxRows" => self.max_rows = positive(value, policy.max_rows as u64)? as usize,
            "maxResultBytes" => {
                self.max_result_bytes = positive(value, policy.max_result_bytes as u64)? as usize
            }
            "mysqlFoundRows" => match value {
                Value::Bool(v) => self.found_rows = *v,
                _ => return Err(invalid("mysqlFoundRows requires bool")),
            },
            "tlsCaFile" => {
                self.ca_path = match value {
                    Value::Null => None,
                    Value::String(s) if !s.is_empty() && !s.contains('\0') && s.len() <= 16384 => {
                        Some(s.clone())
                    }
                    _ => return Err(invalid("tlsCaFile requires a nonempty path string or null")),
                }
            }
            _ => return Err(unsupported()),
        }
        Ok(())
    }
    pub fn operation_timeout(&self) -> Duration {
        Duration::from_millis(self.operation_timeout_ms)
    }
}
fn positive(value: &Value, ceiling: u64) -> NativeResult<u64> {
    let Value::Int(value) = value else {
        return Err(invalid("MySQL limit requires int"));
    };
    if *value <= 0 {
        return Err(invalid("MySQL limit must be positive"));
    }
    if *value as u64 > ceiling {
        return Err(NativeError::new(
            "DbLimitError",
            "MySQL limit exceeds host ceiling",
        ));
    }
    Ok(*value as u64)
}
