use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scripta"));
    command.current_dir(root).env_remove("SCR_PACKAGE_INDEX");
    for key in [
        "SCR_MAX_STEPS",
        "SCR_MAX_VALUE_BYTES",
        "SCR_MAX_SCOPES",
        "SCR_MAX_SOURCE_BYTES",
    ] {
        command.env_remove(key);
    }
    command
}
fn run(root: &Path, args: &[&str], code: i32) -> Output {
    let output = command(root).args(args).output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(code),
        "{args:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn json(root: &Path, args: &[&str], code: i32) -> Value {
    let mut command_args = vec!["test", "--format", "json"];
    command_args.extend_from_slice(args);
    serde_json::from_slice(&run(root, &command_args, code).stdout).unwrap()
}
fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

#[test]
fn discovery_is_sorted_filtered_deduplicated_and_does_not_execute_initializers() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/b_test.scripta",
        "File::write('executed', 'bad'); function test_z() {} function test_a() {} function helper() { function test_nested() {} }",
    );
    write(
        dir.path(),
        "tests/a_test.scripta",
        "function test_other() {}",
    );
    write(
        dir.path(),
        "tests/fixtures/broken_test.scripta",
        "invalid ! source",
    );
    let report = json(dir.path(), &["--list"], 0);
    let cases = report["tests"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    assert_eq!(cases[0]["file"], "tests/a_test.scripta");
    assert_eq!(cases[1]["name"], "test_a");
    assert!(!dir.path().join("executed").exists());
    let report = json(
        dir.path(),
        &[
            "--list",
            "--filter",
            "test_a",
            "tests/b_test.scripta",
            "tests",
        ],
        0,
    );
    assert_eq!(report["tests"].as_array().unwrap().len(), 1);
    assert!(!dir.path().join("executed").exists());
}

#[test]
fn assertions_callbacks_unicode_and_automatic_temp_cleanup_work_without_external_tools() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/native_test.scripta",
        r#"
function test_assertions() {
    Assert::same([1, "λ", ["x" => true]], [1, "λ", ["x" => true]]);
    Assert::notSame(1, 1.0);
    Assert::isTrue(true);
    Assert::isFalse(false);
    Assert::contains("🌍", "Hello 🌍");
    Assert::contains(1, ["1", 1]);
    Assert::throws(function () { return 1 + "x"; }, "TypeError");
    Assert::throws(function () { return 1 / 0; }, "ArithmeticError");
    Assert::throws(function () { throw "expected"; }, "string");
}
function test_temp() {
    $dir = Test::tempDir();
    File::write("tmp-path.txt", $dir);
    File::write(Path::join($dir, "data.txt"), "data");
    Assert::same("data", File::read(Path::join($dir, "data.txt")));
}
"#,
    );
    let output = command(dir.path())
        .env("PATH", dir.path())
        .args(["test", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["passed"], 2);
    let temp = fs::read_to_string(dir.path().join("tmp-path.txt")).unwrap();
    assert!(!Path::new(&temp).exists());
}

#[test]
fn failures_stay_sticky_skips_are_distinct_and_fail_fast_stops_later_cases() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/failure_test.scripta",
        r#"
function test_a_caught() { try { Assert::same(1, "1", "strict values"); } catch (AssertionError $e) {} }
function test_b_wrong() { Assert::throws(function () { throw "wrong"; }, "TypeError"); }
function test_c_skipped() { Test::skip("not available"); Assert::fail("unreachable"); }
function test_d_pass() { Assert::isTrue(true); }
function test_e_nested_assertion() { Assert::throws(function () { Assert::fail("inner failure"); }, "AssertionError"); }
"#,
    );
    let report = json(dir.path(), &[], 1);
    assert_eq!(
        report["summary"],
        serde_json::json!({"passed":1,"failed":3,"skipped":1})
    );
    let message = report["tests"][0]["diagnostic"].as_str().unwrap();
    assert!(
        message.contains("Expected: Int(1)")
            && message.contains("Actual:")
            && message.contains("failure_test.scripta:2"),
        "{message}"
    );
    assert_eq!(report["tests"][2]["diagnostic"], "not available");
    let report = json(dir.path(), &["--fail-fast"], 1);
    assert_eq!(report["tests"].as_array().unwrap().len(), 1);
}

#[test]
fn explicit_exits_never_pass_during_initialization_functions_or_expected_exceptions() {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "tests/init_test.scripta",
            "exit 0; function test_never() {}",
        ),
        (
            "tests/case_test.scripta",
            "function test_exit() { exit 0; } function test_throws_exit() { Assert::throws(function () { exit 0; }, 'Error'); }",
        ),
    ] {
        write(dir.path(), path, source);
    }
    let report = json(dir.path(), &[], 1);
    assert_eq!(report["summary"]["failed"], 3);
    assert_eq!(report["summary"]["passed"], 0);
}

#[test]
fn output_is_bounded_at_exact_limits_and_hidden_for_passing_tests() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "scripta.toml", "[test]\nmax_output_bytes = 3\n");
    write(
        dir.path(),
        "tests/output_test.scripta",
        "function test_exact() { echo 'ABC'; } function test_over() { echo 'ABCD'; } function test_report() { echo 'ABC'; Assert::fail('failure'); }",
    );
    let report = json(dir.path(), &[], 1);
    assert_eq!(report["summary"]["passed"], 1);
    assert_eq!(report["summary"]["failed"], 2);
    assert_eq!(report["tests"][0]["output"], "");
    assert_eq!(report["tests"][2]["output"], "ABC");
    let report = json(dir.path(), &["--filter", "test_exact", "--show-output"], 0);
    assert_eq!(report["tests"][0]["output"], "ABC");
    write(dir.path(), "scripta.toml", "[test]\nmax_output_bytes = 0\n");
    write(
        dir.path(),
        "tests/output_test.scripta",
        "function test_empty() {} function test_byte() { echo 'x'; }",
    );
    assert_eq!(
        json(dir.path(), &[], 1)["summary"],
        serde_json::json!({"passed":1,"failed":1,"skipped":0})
    );
}

#[test]
fn timeouts_cleanup_temp_dirs_and_do_not_stop_the_next_test() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "scripta.toml", "[test]\ntimeout_ms = 500\n");
    write(
        dir.path(),
        "tests/timeout_test.scripta",
        "function test_a_timeout() { File::write('timeout-path.txt', Test::tempDir()); while (true) {} } function test_b_pass() { Assert::isTrue(true); }",
    );
    let report = json(dir.path(), &[], 1);
    assert_eq!(
        report["summary"],
        serde_json::json!({"passed":1,"failed":1,"skipped":0})
    );
    assert!(
        report["tests"][0]["diagnostic"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    assert!(!Path::new(&fs::read_to_string(dir.path().join("timeout-path.txt")).unwrap()).exists());
}

#[test]
fn fatal_limits_are_not_swallowed_by_assert_throws() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/limits_test.scripta",
        "function test_steps() { Assert::throws(function () { while (true) {} }, 'Error'); }",
    );
    let output = command(dir.path())
        .env("SCR_MAX_STEPS", "20")
        .args(["test", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["tests"][0]["diagnostic"]
            .as_str()
            .unwrap()
            .contains("instruction limit")
    );
}

#[test]
fn discovery_errors_empty_matches_and_invalid_configuration_return_two_without_execution() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        json(dir.path(), &[], 2)["error"]
            .as_str()
            .unwrap()
            .contains("no matching")
    );
    write(
        dir.path(),
        "tests/a_test.scripta",
        "File::write('executed', 'bad'); function test_a() {}",
    );
    write(dir.path(), "tests/z_test.scripta", "function test_bad(");
    assert!(json(dir.path(), &[], 2)["error"].is_string());
    assert!(!dir.path().join("executed").exists());
    write(
        dir.path(),
        "tests/z_test.scripta",
        "function test_bad($arg=1) {}",
    );
    assert!(
        json(dir.path(), &[], 2)["error"]
            .as_str()
            .unwrap()
            .contains("no arguments")
    );
    write(dir.path(), "scripta.toml", "[test]\ntimeout_ms = 0\n");
    assert!(
        json(dir.path(), &[], 2)["error"]
            .as_str()
            .unwrap()
            .contains("timeout_ms")
    );
    write(dir.path(), "scripta.toml", "[test]\nunknown = true\n");
    assert!(
        json(dir.path(), &[], 2)["error"]
            .as_str()
            .unwrap()
            .contains("unknown field")
    );
    run(dir.path(), &["test", "--help"], 0);
    run(dir.path(), &["help", "test"], 0);
}

#[test]
fn configuration_survives_package_commands_and_workers_isolate_module_and_environment_state() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "scripta.toml",
        "[test]\ndirectories = ['spec']\ntimeout_ms = 5000\nmax_output_bytes = 1024\n",
    );
    write(
        dir.path(),
        "lib/scripta.toml",
        "[package]\nname='counter'\nversion='1.0.0'\nentry='main.scripta'\n",
    );
    write(
        dir.path(),
        "lib/main.scripta",
        "$n=0; function next() { $n+=1; return $n; }",
    );
    write(
        dir.path(),
        "spec/module_test.scripta",
        "require 'counter' as Counter; function test_a() { Assert::same(1, Counter::next()); Env::set('SCR_CASE_LOCAL', 'changed'); } function test_b() { Assert::same(1, Counter::next()); Assert::isFalse(Env::has('SCR_CASE_LOCAL')); }",
    );
    run(
        dir.path(),
        &["add", "counter", "--path", "lib", "--offline"],
        0,
    );
    let lock = fs::read(dir.path().join("scripta.lock")).unwrap();
    let output = command(dir.path())
        .env_remove("SCR_CASE_LOCAL")
        .args(["test", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(fs::read(dir.path().join("scripta.lock")).unwrap(), lock);
    run(dir.path(), &["remove", "counter"], 0);
    let manifest: toml::Value =
        toml::from_str(&fs::read_to_string(dir.path().join("scripta.toml")).unwrap()).unwrap();
    assert_eq!(manifest["test"]["directories"][0].as_str(), Some("spec"));
}

#[test]
fn reports_redact_environment_secrets_and_normal_runs_have_no_test_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/secret_test.scripta",
        "function test_secret() { echo Env::get('SCR_TEST_SECRET'); Assert::same('expected', Env::get('SCR_TEST_SECRET')); }",
    );
    let output = command(dir.path())
        .env("SCR_TEST_SECRET", "unique-hidden-value")
        .args(["test", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("unique-hidden-value"), "{text}");
    serde_json::from_str::<Value>(&text).unwrap();
    write(dir.path(), "normal.scripta", "Assert::isTrue(true);");
    let output = run(dir.path(), &["run", "normal.scripta"], 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("undefined function Assert::isTrue"));
}

#[test]
fn explicit_files_allow_legacy_extensions_and_symlink_discovery_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "standalone.scr",
        "<?scr function test_legacy() { Assert::isTrue(true); }",
    );
    assert_eq!(
        json(dir.path(), &["standalone.scr"], 0)["summary"]["passed"],
        1
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            dir.path().join("standalone.scr"),
            dir.path().join("link.scr"),
        )
        .unwrap();
        assert!(
            json(dir.path(), &["link.scr"], 2)["error"]
                .as_str()
                .unwrap()
                .contains("symlink")
        );
    }
}

#[cfg(unix)]
#[test]
fn interruption_returns_130_and_cleans_parent_owned_temp_directory() {
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/interrupt_test.scripta",
        "function test_wait() { File::write('interrupt-path.txt', Test::tempDir()); while (true) {} }",
    );
    let mut child = command(dir.path())
        .args(["test", "--format", "json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let start = Instant::now();
    while !dir.path().join("interrupt-path.txt").exists() {
        if start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            panic!("worker did not initialize");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let temp_path = fs::read_to_string(dir.path().join("interrupt-path.txt")).unwrap();
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(130),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(!Path::new(&temp_path).exists());
}

#[test]
fn discovery_case_limits_accept_exact_limit_and_reject_limit_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut source = String::new();
    for i in 0..4096 {
        source.push_str(&format!("function test_case_{i}() {{}}\n"));
    }
    write(dir.path(), "tests/bounds_test.scripta", &source);
    assert_eq!(
        json(dir.path(), &["--list"], 0)["tests"]
            .as_array()
            .unwrap()
            .len(),
        4096
    );
    source.push_str("function test_extra() {}\n");
    write(dir.path(), "tests/bounds_test.scripta", &source);
    assert!(
        json(dir.path(), &["--list"], 2)["error"]
            .as_str()
            .unwrap()
            .contains("4096 cases")
    );
    let name = format!("test_{}", "a".repeat(4091));
    write(
        dir.path(),
        "tests/bounds_test.scripta",
        &format!("function {name}() {{}}"),
    );
    assert_eq!(json(dir.path(), &[], 0)["summary"]["passed"], 1);
    write(
        dir.path(),
        "tests/bounds_test.scripta",
        &format!("function {name}a() {{}}"),
    );
    assert!(
        json(dir.path(), &["--list"], 2)["error"]
            .as_str()
            .unwrap()
            .contains("4096 bytes")
    );
}

#[test]
fn configuration_boundaries_and_resource_exceptions_are_not_false_successes() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/config_test.scripta",
        "function test_empty() {}",
    );
    write(
        dir.path(),
        "scripta.toml",
        "[test]\ntimeout_ms=1\nmax_output_bytes=67108864\n",
    );
    json(dir.path(), &["--list"], 0);
    for config in [
        "timeout_ms=3600001",
        "max_output_bytes=67108865",
        "directories=[]",
    ] {
        write(dir.path(), "scripta.toml", &format!("[test]\n{config}\n"));
        json(dir.path(), &["--list"], 2);
    }
    write(dir.path(), "scripta.toml", "[test]\n");
    write(
        dir.path(),
        "tests/config_test.scripta",
        "function test_resource() { Assert::throws(function () { List::fill(10000, 'x'); }, 'ResourceLimitError'); }",
    );
    let output = command(dir.path())
        .env("SCR_MAX_VALUE_BYTES", "1024")
        .args(["test", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["tests"][0]["diagnostic"]
            .as_str()
            .unwrap()
            .contains("ResourceLimitError")
    );
}

#[test]
fn workers_register_only_extensions_available_in_the_executable() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tests/sqlite_test.scripta",
        "function test_sqlite() { $db = Sqlite::open(':memory:'); try { Assert::same(42, Sqlite::query($db, 'SELECT 42 AS answer')[0]['answer']); } finally { Sqlite::close($db); } }",
    );
    let report = json(
        dir.path(),
        &[],
        if cfg!(feature = "sqlite") { 0 } else { 1 },
    );
    if cfg!(feature = "sqlite") {
        assert_eq!(report["summary"]["passed"], 1);
    } else {
        assert!(
            report["tests"][0]["diagnostic"]
                .as_str()
                .unwrap()
                .contains("undefined function Sqlite::open")
        );
    }
}

#[test]
fn database_extensions_are_registered_in_script_test_workers() {
    let directory = tempfile::tempdir().unwrap();
    let kind = if cfg!(feature = "mysql") {
        "TypeError"
    } else {
        "DbUnsupportedError"
    };
    write(
        directory.path(),
        "tests/database_test.scripta",
        &format!(
            "function test_database() {{ Assert::throws(function () {{ Db::connect('mysql:host=;dbname=unused','fixture','secret'); }}, '{kind}'); }}"
        ),
    );
    let report = json(directory.path(), &[], 0);
    assert_eq!(report["summary"]["passed"], 1, "{report}");
}

#[test]
#[cfg(feature = "mysql")]
#[ignore = "requires disposable verified-TLS MySQL fixture"]
fn mysql_live_connections_work_in_script_test_workers() {
    assert!(
        std::env::var_os("SCR_MYSQL_FIXTURE_CA").is_some(),
        "run mysql_fixture.py"
    );
    let directory = tempfile::tempdir().unwrap();
    let source = include_str!("../../../tests/fixtures/mysql-cli.scripta");
    write(
        directory.path(),
        "tests/mysql_test.scripta",
        &format!("function test_mysql() {{ {source} }}"),
    );
    let report = json(directory.path(), &[], 0);
    assert_eq!(report["summary"]["passed"], 1, "{report}");
}
