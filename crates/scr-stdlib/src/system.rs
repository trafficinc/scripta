//! Host metrics: no subprocesses, no process-memory/system-memory ambiguity.
use crate::{filesystem::FilesystemPolicy, paths};
use scr_runtime::{
    NativeError, NativeFunction, NativeObject, NativeResult, NativeSignature, NativeType, Runtime,
    Value,
};
use std::{
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Usage {
    kind: &'static str,
    total: i64,
    used: i64,
    available: i64,
    percent: f64,
}
impl NativeObject for Usage {
    fn type_name(&self) -> &str {
        self.kind
    }
    fn get_property(&self, name: &str) -> Result<Value, String> {
        match name {
            "totalBytes" => Ok(Value::Int(self.total)),
            "usedBytes" => Ok(Value::Int(self.used)),
            "availableBytes" => Ok(Value::Int(self.available)),
            "percentUsed" => Ok(Value::Float(self.percent)),
            _ => Err(format!("unknown {} property {name}", self.kind)),
        }
    }
}
fn unavailable(metric: &str) -> NativeError {
    NativeError::new(
        "UnsupportedError",
        format!("{metric} is unavailable on this host"),
    )
}
fn supported(metric: &str) -> NativeResult<()> {
    if sysinfo::IS_SUPPORTED_SYSTEM {
        Ok(())
    } else {
        Err(unavailable(metric))
    }
}
fn integer(value: u64) -> NativeResult<i64> {
    i64::try_from(value)
        .map_err(|_| NativeError::new("RangeError", "system metric exceeds i64 range"))
}
fn usage(kind: &'static str, total: u64, free: u64, available: u64) -> NativeResult {
    if total == 0 {
        return Err(unavailable(kind));
    }
    if free > total || available > free {
        return Err(NativeError::new(
            "SystemError",
            "inconsistent system counters",
        ));
    }
    Ok(Value::NativeObject(Rc::new(Usage {
        kind,
        total: integer(total)?,
        used: integer(total - free)?,
        available: integer(available)?,
        percent: (total - free) as f64 / total as f64 * 100.0,
    })))
}
pub(crate) fn register(runtime: &mut Runtime, policy: FilesystemPolicy) -> NativeResult<()> {
    let policy = Rc::new(policy.prepare()?);
    for (name, value) in [
        ("platform", std::env::consts::OS),
        ("architecture", std::env::consts::ARCH),
    ] {
        runtime.register(
            format!("System::{name}"),
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::String),
                move |_, _| Ok(Value::String(value.into())),
            ),
        )?;
    }
    for name in [
        "hostname",
        "homeDirectory",
        "tempDirectory",
        "uptime",
        "memoryUsage",
    ] {
        let returns = match name {
            "uptime" => NativeType::Int,
            "memoryUsage" => NativeType::Object("MemoryUsage"),
            _ => NativeType::String,
        };
        runtime.register(
            format!("System::{name}"),
            NativeFunction::new(NativeSignature::new(vec![], returns), move |context, _| {
                context.permissions.require("system.inspect")?;
                match name {
                    "hostname" => {
                        supported(name)?;
                        sysinfo::System::host_name()
                            .filter(|value| !value.is_empty())
                            .map(Value::String)
                            .ok_or_else(|| unavailable(name))
                    }
                    "homeDirectory" => {
                        #[allow(deprecated)]
                        let home = std::env::home_dir().ok_or_else(|| unavailable(name))?;
                        Ok(Value::String(paths::text(home.as_os_str())?))
                    }
                    "tempDirectory" => Ok(Value::String(paths::text(
                        std::env::temp_dir().as_os_str(),
                    )?)),
                    "uptime" => {
                        supported(name)?;
                        let seconds = sysinfo::System::uptime();
                        if seconds == 0 {
                            return Err(unavailable(name));
                        }
                        integer(seconds).map(Value::Int)
                    }
                    "memoryUsage" => {
                        supported(name)?;
                        let mut system = sysinfo::System::new();
                        system.refresh_memory();
                        let available = system.available_memory();
                        // Used = total - available, including reclaimable caches in available.
                        usage("MemoryUsage", system.total_memory(), available, available)
                    }
                    _ => unreachable!(),
                }
            }),
        )?;
    }
    runtime.register(
        "System::cpuUsage",
        NativeFunction::new(
            NativeSignature::new(vec![], NativeType::Float).with_optional(vec![NativeType::Int]),
            |context, args| {
                context.permissions.require("system.inspect")?;
                context.permissions.require("sleep")?;
                supported("CPU usage")?;
                let minimum = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_millis(200));
                let duration = match args {
                    [] => minimum,
                    [Value::Int(ms)] if *ms >= 0 => Duration::from_millis(*ms as u64),
                    _ => {
                        return Err(NativeError::type_error(
                            "CPU sample duration must be nonnegative milliseconds",
                        ));
                    }
                };
                if duration < minimum || duration > Duration::from_secs(60) {
                    return Err(NativeError::new(
                        "RangeError",
                        format!(
                            "CPU sample duration must be {}..60000 milliseconds",
                            minimum.as_millis()
                        ),
                    ));
                }
                let mut system = sysinfo::System::new();
                system.refresh_cpu_usage();
                if system.cpus().is_empty() {
                    return Err(unavailable("CPU usage"));
                }
                let start = Instant::now();
                while start.elapsed() < duration {
                    context.check_cancelled()?;
                    std::thread::sleep(
                        duration
                            .saturating_sub(start.elapsed())
                            .min(Duration::from_millis(10)),
                    );
                }
                system.refresh_cpu_usage();
                let percent = f64::from(system.global_cpu_usage());
                if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
                    return Err(NativeError::new("SystemError", "invalid CPU counters"));
                }
                Ok(Value::Float(percent))
            },
        ),
    )?;
    runtime.register(
        "System::diskUsage",
        NativeFunction::new(
            NativeSignature::new(vec![NativeType::String], NativeType::Object("DiskUsage")),
            move |context, args| {
                context.permissions.require("system.inspect")?;
                let [Value::String(path)] = args else {
                    unreachable!()
                };
                let path = paths::resolve(context, path)?;
                policy.check(context, &path, false)?;
                #[cfg(any(unix, windows))]
                {
                    // Windows statvfs resolves the volume root and can succeed
                    // for a nonexistent child path. Enforce the API's existing
                    // file/directory contract before querying capacity.
                    std::fs::metadata(&path)?;
                    let stats = fs2::statvfs(&path)?;
                    usage(
                        "DiskUsage",
                        stats.total_space(),
                        stats.free_space(),
                        stats.available_space(),
                    )
                }
                #[cfg(not(any(unix, windows)))]
                Err(unavailable("disk usage"))
            },
        ),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_missing_inconsistent_and_overflowing_counters() {
        assert_eq!(
            usage("DiskUsage", 0, 0, 0).unwrap_err().kind,
            "UnsupportedError"
        );
        assert_eq!(
            usage("DiskUsage", 10, 11, 0).unwrap_err().kind,
            "SystemError"
        );
        assert_eq!(
            usage("DiskUsage", 10, 5, 6).unwrap_err().kind,
            "SystemError"
        );
        assert_eq!(
            usage("DiskUsage", u64::MAX, 1, 1).unwrap_err().kind,
            "RangeError"
        );
        let Value::NativeObject(value) = usage("DiskUsage", 100, 40, 30).unwrap() else {
            panic!()
        };
        assert_eq!(value.get_property("usedBytes").unwrap(), Value::Int(60));
        assert_eq!(
            value.get_property("percentUsed").unwrap(),
            Value::Float(60.0)
        );
        assert!(value.set_property("totalBytes", Value::Int(1)).is_err());
    }
}
