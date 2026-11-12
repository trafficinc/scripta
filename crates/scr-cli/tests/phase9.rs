use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "scripta-cli-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn script(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn both_invocation_forms_expose_only_script_arguments() {
    let temp = Temp::new();
    let file = temp.script("arguments.scr", r#"<?scr foreach (Cli::args() as $argument) { echo $argument . "|"; } echo str(Cli::has("flag")) . ":" . Cli::option("name");"#);
    for named in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_scr"));
        if named {
            command.arg("run");
        }
        let output = command
            .arg(&file)
            .args(["first", "--flag", "--name=世界", "--", "--ignored"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "first|--flag|--name=世界|--|--ignored|true:世界"
        );
        assert!(output.stderr.is_empty());
    }
}
#[test]
fn process_default_arguments_and_inherited_streams_work_through_cli() {
    let temp = Temp::new();
    let child = temp.script(
        "child.scr",
        "<?scr echo \"child-out\"; throw \"child-error\";",
    );
    let parent = temp.script("parent.scr", r#"<?scr $program = Cli::arg(0); $child = Cli::arg(1); $default = Process::run($program); echo str($default->exitCode) . "|"; $result = Process::run($program, [$child], ["stdout" => "inherit", "stderr" => "inherit"]); echo "|" . str($result->exitCode) . ":" . $result->stdout . $result->stderr;"#);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(parent)
        .arg(env!("CARGO_BIN_EXE_scr"))
        .arg(child)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"2|child-out|1:");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("child-error")
    );
}
#[test]
fn prompt_reads_stdin_and_environment_changes_reach_children_only() {
    let temp = Temp::new();
    let file = temp.script("prompt.scr", r#"<?scr $answer = Cli::prompt("Name: "); echo "Hello " . $answer; Env::set("SCR_LOCAL_ENV", "local"); echo ":" . Env::get("SCR_LOCAL_ENV");"#);
    let before = std::env::var_os("SCR_LOCAL_ENV");
    let mut child = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("世界\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Name: Hello 世界:local"
    );
    assert_eq!(std::env::var_os("SCR_LOCAL_ENV"), before);
}
#[test]
fn child_environment_uses_runtime_changes_and_command_arguments_are_literal() {
    let temp = Temp::new();
    let child = temp.script(
        "child.scr",
        r#"<?scr echo Env::get("SCR_RUNTIME_ENV") . ":" . Cli::arg(0);"#,
    );
    let parent = temp.script("parent.scr", r#"<?scr Env::set("SCR_RUNTIME_ENV", "inherited"); $result = Process::run(Cli::arg(0), [Cli::arg(1), "literal ; & $(no-shell)"]); echo $result->stdout;"#);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .args([
            "run",
            parent.to_str().unwrap(),
            env!("CARGO_BIN_EXE_scr"),
            child.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"inherited:literal ; & $(no-shell)");
}

#[cfg(unix)]
#[test]
fn cli_cancellation_exits_130_and_cleans_up_spawned_child() {
    let temp = Temp::new();
    let child = temp.script("sleep.scr", "<?scr sleep(10000);");
    let parent = temp.script("parent.scr", r#"<?scr $child = Process::spawn(Cli::arg(0), [Cli::arg(1)]); File::write(Cli::arg(2), str($child->pid)); while (true) { sleep(10); }"#);
    let pid_file = temp.0.join("pid");
    let mut process = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(parent)
        .arg(env!("CARGO_BIN_EXE_scr"))
        .arg(child)
        .arg(&pid_file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !pid_file.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    if !pid_file.exists() {
        let _ = process.kill();
        let _ = process.wait();
        panic!("child never started");
    }
    // Wait for the complete (small, non-atomic) file write, not just creation.
    let child_pid = loop {
        if let Ok(contents) = fs::read_to_string(&pid_file) {
            if let Ok(pid) = contents.parse::<u32>() {
                break pid;
            }
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &process.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while process.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if process.try_wait().unwrap().is_none() {
        let _ = process.kill();
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &child_pid.to_string()])
            .status();
        panic!("cancellation did not finish");
    }
    let output = process.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("interrupted")
    );
    let status = Command::new("/bin/kill")
        .args(["-0", &child_pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "spawned child is still alive");
}

#[test]
fn phase11_reuses_modules_from_an_unrelated_working_directory() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("lib")).unwrap();
    temp.script(
        "lib/helpers.scr",
        r#"<?scr function status($body) { $data = Json::decode($body); return $data["status"]; }"#,
    );
    let main = temp.script("reuse.scr", r#"<?scr require_once "lib/helpers.scr"; require_once "lib/../lib/helpers.scr"; echo status(Json::encode(["status" => "ready"]));"#);
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .current_dir(std::env::temp_dir())
        .arg(main)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ready");
}

#[test]
fn phase11_cli_errors_render_definition_and_call_files() {
    let temp = Temp::new();
    temp.script(
        "helper.scr",
        "<?scr\nfunction explode() {\n    Json::decode(\"invalid\");\n}\n",
    );
    let main = temp.script("main.scr", "<?scr\nrequire \"helper.scr\";\nexplode();\n");
    let output = Command::new(env!("CARGO_BIN_EXE_scr"))
        .arg(main)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("helper.scr:3:"), "{stderr}");
    assert!(stderr.contains("main.scr:3:"), "{stderr}");
    assert!(stderr.contains("JsonError"));
}
