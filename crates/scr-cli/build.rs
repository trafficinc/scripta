use std::{env, process::Command};
fn main() {
    for key in ["TARGET", "PROFILE", "RUSTC", "SCR_BUILD_COMMIT"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let rustc = Command::new(env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .arg("--version")
        .output()
        .expect("Rust compiler is required at build time");
    assert!(
        rustc.status.success(),
        "could not read Rust compiler version"
    );
    for (name, value) in [
        ("SCR_TARGET", env::var("TARGET").unwrap()),
        ("SCR_PROFILE", env::var("PROFILE").unwrap()),
        (
            "SCR_COMPILER",
            String::from_utf8(rustc.stdout).unwrap().trim().to_owned(),
        ),
    ] {
        println!("cargo:rustc-env={name}={value}");
    }
    let commit = env::var("SCR_BUILD_COMMIT").unwrap_or_else(|_| "unknown".into());
    assert!(
        commit == "unknown"
            || (40..=64).contains(&commit.len()) && commit.bytes().all(|c| c.is_ascii_hexdigit()),
        "SCR_BUILD_COMMIT must be a hexadecimal revision"
    );
    println!("cargo:rustc-env=SCR_COMMIT={commit}");
}
