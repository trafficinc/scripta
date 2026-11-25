//! Compiled extension composition shared by CLI, embedding and worker entry points.
use scr_runtime::{NativeResult, Runtime};
use std::rc::Rc;

/// Compiled connector API version and operations, without opening a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub capabilities: &'static [&'static str],
}

fn database_drivers() -> Vec<Rc<dyn scr_db::Driver>> {
    vec![
        #[cfg(feature = "mysql")]
        scr_ext_mysql::driver(),
        #[cfg(feature = "postgres")]
        scr_ext_postgres::driver(),
    ]
}

/// Return the same driver descriptors used by registration. No network I/O occurs.
pub fn compiled() -> Vec<ExtensionInfo> {
    let mut extensions = vec![
        #[cfg(feature = "sqlite")]
        ExtensionInfo {
            name: "sqlite",
            version: scr_ext_sqlite::VERSION,
            capabilities: scr_ext_sqlite::CAPABILITIES,
        },
    ];
    extensions.extend(database_drivers().iter().map(|driver| {
        let info = driver.info();
        ExtensionInfo {
            name: info.name,
            version: info.version,
            capabilities: info.capabilities,
        }
    }));
    extensions
}

pub(crate) fn names() -> String {
    let extensions = compiled();
    if extensions.is_empty() {
        "none".into()
    } else {
        extensions
            .iter()
            .map(|info| info.name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Register once after the host's standard library. On failure discard the runtime.
/// The shared Db API is present even with no drivers, providing clear availability
/// errors without loading a native driver or claiming that check connects to it.
/// Hosts needing custom destination/filesystem policies should compose descriptors
/// directly through scr_db rather than registering this default composition twice.
pub fn register(runtime: &mut Runtime) -> NativeResult<()> {
    #[cfg(feature = "sqlite")]
    scr_ext_sqlite::register(runtime)?;
    scr_db::register(runtime, database_drivers())
}
