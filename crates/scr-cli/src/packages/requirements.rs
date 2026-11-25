//! Native requirements use the host's registered connector metadata, never Git or SQL.
use super::*;
use crate::extensions::ExtensionInfo;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Requirement {
    version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<String>,
}
fn extension_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}
fn capability(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.as_bytes()[0].is_ascii_alphabetic()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        })
}
pub(super) fn validate(manifest: &Manifest) -> Result<()> {
    if manifest.extensions.len() > 64 {
        return Err(fail("manifest exceeds 64 extension requirements"));
    }
    for (name, requirement) in &manifest.extensions {
        if !extension_name(name) {
            return Err(fail(
                "invalid extension name; use up to 64 lowercase ASCII letters, digits, '_' or '-', starting with a letter",
            ));
        }
        if requirement.version.len() > 256 {
            return Err(fail(format!(
                "extension '{name}': version constraint exceeds 256 bytes"
            )));
        }
        if requirement.version.trim().is_empty() {
            return Err(fail(format!(
                "extension '{name}': invalid API version constraint"
            )));
        }
        VersionReq::parse(&requirement.version).map_err(|_| {
            fail(format!(
                "extension '{name}': invalid API version constraint"
            ))
        })?;
        if requirement.capabilities.len() > 64 {
            return Err(fail(format!(
                "extension '{name}': more than 64 required capabilities"
            )));
        }
        let mut names = HashSet::new();
        for item in &requirement.capabilities {
            if !capability(item) || !names.insert(item) {
                return Err(fail(format!(
                    "extension '{name}': invalid or duplicate capability; use dotted ASCII identifiers, at most 64 bytes"
                )));
            }
        }
    }
    Ok(())
}
pub(super) fn check(manifest: &Manifest, extensions: &[ExtensionInfo]) -> Result<()> {
    if manifest.extensions.is_empty() {
        return Ok(());
    }
    let context = manifest
        .package
        .as_ref()
        .map(|p| {
            let version = if p.version.len() > 128 {
                format!("{}...", p.version.chars().take(128).collect::<String>())
            } else {
                p.version.clone()
            };
            format!("package '{}@{}'", p.name, version)
        })
        .unwrap_or_else(|| "root project".into());
    for (name, requirement) in &manifest.extensions {
        let wanted = VersionReq::parse(&requirement.version)?;
        let Some(extension) = extensions.iter().find(|item| item.name == name) else {
            let hint = if name == "mysql" {
                "; CLI source builds can enable MySQL with --features mysql"
            } else if name == "pgsql" {
                "; CLI source builds can enable PostgreSQL with --features postgres"
            } else {
                ""
            };
            return Err(fail(format!(
                "{context}: required extension '{name}' (API {wanted}) is unavailable; inspect 'scripta version --verbose' and use a compatible executable or register it in the embedding host{hint}. Installing script packages does not install native extensions"
            )));
        };
        let actual = Version::parse(extension.version)
            .map_err(|_| fail(format!("extension '{name}' reports an invalid API version")))?;
        if !wanted.matches(&actual) {
            return Err(fail(format!(
                "{context}: extension '{name}' API {actual} does not satisfy {wanted}; use a compatible executable/host (inspect 'scripta version --verbose'); package installation cannot replace the interpreter"
            )));
        }
        for required in &requirement.capabilities {
            if !extension.capabilities.contains(&required.as_str()) {
                return Err(fail(format!(
                    "{context}: extension '{name}' API {actual} lacks required capability '{required}'; inspect 'scripta version --verbose' and use a compatible executable/host"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const MYSQL: ExtensionInfo = ExtensionInfo {
        name: "mysql",
        version: "1.2.3",
        capabilities: &["prepare.native", "transactions"],
    };
    fn parse(text: &str) -> Result<Manifest> {
        let manifest: Manifest = toml::from_str(text)?;
        validate(&manifest)?;
        Ok(manifest)
    }
    #[test]
    fn semver_capabilities_and_actionable_failures_use_host_metadata() {
        for newline in ["\n", "\r\n"] {
            let manifest=parse(&format!("[extensions]{newline}mysql = {{ version = '^1.2', capabilities = ['prepare.native', 'transactions'] }}{newline}")).unwrap();
            check(&manifest, &[MYSQL]).unwrap();
            let error = check(&manifest, &[]).unwrap_err().to_string();
            assert!(
                error.contains("mysql")
                    && error.contains("--features mysql")
                    && error.contains("version --verbose")
            );
            let error = check(
                &manifest,
                &[ExtensionInfo {
                    version: "2.0.0",
                    ..MYSQL
                }],
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("API 2.0.0 does not satisfy ^1.2"));
            let error = check(
                &manifest,
                &[ExtensionInfo {
                    capabilities: &[],
                    ..MYSQL
                }],
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("lacks required capability 'prepare.native'"));
        }
        let stable = parse("[extensions]\nmysql={version='^1.2'}").unwrap();
        assert!(
            check(
                &stable,
                &[ExtensionInfo {
                    version: "1.3.0-beta.2",
                    ..MYSQL
                }]
            )
            .is_err()
        );
        let beta = parse("[extensions]\nmysql={version='>=1.3.0-beta.1, <1.3.0'}").unwrap();
        check(
            &beta,
            &[ExtensionInfo {
                version: "1.3.0-beta.2",
                ..MYSQL
            }],
        )
        .unwrap();
        assert!(
            check(
                &stable,
                &[ExtensionInfo {
                    version: "invalid",
                    ..MYSQL
                }]
            )
            .unwrap_err()
            .to_string()
            .contains("reports an invalid API version")
        );
    }
    #[test]
    fn strict_schema_names_and_exact_plus_one_limits_are_enforced() {
        for text in [
            "[extensions]\nmysql='^1'",
            "[extensions]\nmysql={version=1}",
            "[extensions]\nmysql={capabilities=[]}",
            "[extensions]\nmysql={version='1',serverVersion='8'}",
            "[extensions]\nMySQL={version='1'}",
            "[extensions]\nmysql={version='wrong'}",
            "[extensions]\nmysql={version='1',capabilities=['transactions','transactions']}",
            "[extensions]\nmysql={version='1',capabilities=['a..b']}",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        for (count, succeeds) in [(64, true), (65, false)] {
            let extensions = (0..count)
                .map(|n| format!("ext{n}={{version='1'}}\n"))
                .collect::<String>();
            assert_eq!(
                parse(&format!("[extensions]\n{extensions}")).is_ok(),
                succeeds
            );
            let caps = (0..count)
                .map(|n| format!("'c{n}'"))
                .collect::<Vec<_>>()
                .join(",");
            assert_eq!(
                parse(&format!(
                    "[extensions]\nmysql={{version='1',capabilities=[{caps}]}}"
                ))
                .is_ok(),
                succeeds
            );
            let name = "a".repeat(count);
            assert_eq!(
                parse(&format!("[extensions]\n{name}={{version='1'}}")).is_ok(),
                succeeds
            );
            assert_eq!(
                parse(&format!(
                    "[extensions]\nmysql={{version='1',capabilities=['{name}']}}"
                ))
                .is_ok(),
                succeeds
            );
        }
        for (size, succeeds) in [(256, true), (257, false)] {
            let constraint = format!("=1.0.0-{}", "a".repeat(size - 7));
            assert_eq!(
                parse(&format!("[extensions]\nmysql={{version='{constraint}'}}")).is_ok(),
                succeeds
            );
        }
    }
}
