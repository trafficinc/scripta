use scr_runtime::{Runtime, RuntimeOptions};
use scr_stdlib::web::{self, session};
use std::{
    fs,
    io::Write,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn host_runtime() -> Runtime {
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .insert("web.session".into());
    Runtime::with_options(options)
}
struct Store {
    directory: tempfile::TempDir,
    config: session::Config,
}
#[derive(Debug)]
struct Reply {
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}
impl Reply {
    fn id(&self) -> String {
        self.headers
            .iter()
            .find(|(k, v)| k == "set-cookie" && v.starts_with("SCRIPTA_SESSION="))
            .unwrap()
            .1
            .split(';')
            .next()
            .unwrap()
            .strip_prefix("SCRIPTA_SESSION=")
            .unwrap()
            .into()
    }
}
impl Store {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut config = session::Config::new(directory.path().into());
        config.prepare().unwrap();
        Self { directory, config }
    }
    fn execute(&self, cookie: Option<&str>, source: &str) -> Result<Reply, String> {
        execute(self.config.clone(), cookie, source)
    }
}
fn execute(config: session::Config, cookie: Option<&str>, source: &str) -> Result<Reply, String> {
    let mut runtime = host_runtime();
    scr_stdlib::register(&mut runtime).unwrap();
    let headers = cookie
        .map(|v| vec![("cookie".into(), format!("SCRIPTA_SESSION={v}"))])
        .unwrap_or_default();
    let response = web::register(
        &mut runtime,
        web::Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            body: vec![],
            request_id: None,
            connection: Default::default(),
            headers: headers.clone(),
        },
        32768,
    )
    .unwrap();
    let session = session::register(&mut runtime, config, &headers, response.clone()).unwrap();
    let mut body = vec![];
    let code = runtime
        .execute_source(&format!("<?scripta {source}"), &mut body)
        .map_err(|e| e.message)?;
    if code != 0 {
        return Err("nonzero exit".into());
    }
    session.finish().map_err(|e| e.to_string())?;
    let headers = response.borrow().headers.clone();
    Ok(Reply { body, headers })
}
#[test]
fn sessions_roundtrip_strict_values_and_typed_map_keys_with_private_cookie_defaults() {
    let store = Store::new();
    let first=store.execute(None,r#"Session::start(); Session::start(); Session::set("value",[1=>"integer","1"=>"string","bytes"=>Bytes::encodeUtf8("世界"),"float"=>1.0,"nested"=>[true,null,2]]); Session::set("null",null);echo str(Session::has("null"));echo str(Session::has("missing"));echo str(Session::get("missing",42));"#).unwrap();
    assert_eq!(first.body, b"truefalse42");
    let id = first.id();
    assert_eq!(id.len(), 64);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    let cookie = &first
        .headers
        .iter()
        .find(|(k, _)| k == "set-cookie")
        .unwrap()
        .1;
    assert!(cookie.contains("; Secure; HttpOnly; SameSite=Lax"));
    assert!(cookie.contains("Max-Age=1800"));
    assert!(
        first
            .headers
            .contains(&("cache-control".into(), "no-store".into()))
    );
    let second=store.execute(Some(&id),r#"Session::start(); $expected=[1=>"integer","1"=>"string","bytes"=>Bytes::encodeUtf8("世界"),"float"=>1.0,"nested"=>[true,null,2]];echo str(Session::get("value")===$expected);echo str(Session::get("null")===null);echo str(Session::remove("null"));echo str(Session::remove("null"));"#).unwrap();
    assert_eq!(second.body, b"truetruetruefalse");
    assert_eq!(second.id(), id);
    let next = store
        .execute(
            Some(&id),
            "Session::start();echo str(Session::has(\"null\"));",
        )
        .unwrap();
    assert_eq!(next.body, b"false");
}
#[test]
fn failed_requests_rollback_mutations_and_close_is_an_explicit_early_commit() {
    let store = Store::new();
    let id = store
        .execute(None, "Session::start();Session::set(\"n\",1);")
        .unwrap()
        .id();
    assert!(
        store
            .execute(
                Some(&id),
                "Session::start();Session::set(\"n\",2);echo $undefined;"
            )
            .is_err()
    );
    assert!(
        store
            .execute(Some(&id), "Session::start();Session::set(\"n\",3);exit 1;")
            .is_err()
    );
    assert_eq!(
        store
            .execute(Some(&id), "Session::start();echo str(Session::get(\"n\"));")
            .unwrap()
            .body,
        b"1"
    );
    assert!(
        store
            .execute(
                Some(&id),
                "Session::start();Session::set(\"n\",4);Session::close();echo $undefined;"
            )
            .is_err()
    );
    assert_eq!(
        store
            .execute(Some(&id), "Session::start();echo str(Session::get(\"n\"));")
            .unwrap()
            .body,
        b"4"
    );
    assert!(
        store
            .execute(
                Some(&id),
                "Session::start();Session::close();Session::set(\"n\",5);"
            )
            .is_err()
    );
}
#[test]
fn regeneration_and_destroy_revoke_old_ids_and_work_at_store_capacity() {
    let mut store = Store::new();
    store.config.max_sessions = 1;
    let first = store
        .execute(None, "Session::start();Session::set(\"user\",\"Ada\");")
        .unwrap()
        .id();
    assert!(
        store
            .execute(None, "Session::start();")
            .unwrap_err()
            .contains("capacity")
    );
    let rotated = store
        .execute(
            Some(&first),
            "Session::start();Session::regenerate();echo Session::get(\"user\");",
        )
        .unwrap();
    let second = rotated.id();
    assert_ne!(first, second);
    assert_eq!(rotated.body, b"Ada");
    // The old ID can no longer resume this record, and cannot bypass the quota.
    assert!(store.execute(Some(&first), "Session::start();").is_err());
    let destroyed = store
        .execute(Some(&second), "Session::start();Session::destroy();")
        .unwrap();
    assert!(
        destroyed
            .headers
            .iter()
            .any(|(k, v)| k == "set-cookie" && v.contains("Max-Age=0"))
    );
    assert_eq!(destroyed.id(), "");
    let replacement = store
        .execute(
            Some(&second),
            "Session::start();echo str(Session::has(\"user\"));",
        )
        .unwrap();
    assert_ne!(replacement.id(), second);
    assert_eq!(replacement.body, b"false");
}
#[test]
fn attacker_chosen_unknown_expired_and_ambiguous_ids_are_not_adopted() {
    let store = Store::new();
    for chosen in [
        "../outside".to_owned(),
        "malformed".to_owned(),
        "a".repeat(64),
    ] {
        let reply = store.execute(Some(&chosen), "Session::start();").unwrap();
        assert_ne!(reply.id(), chosen);
    }
    let first = store
        .execute(None, "Session::start();Session::set(\"old\",true);")
        .unwrap()
        .id();
    let path = store.directory.path().join(format!("{first}.data"));
    let mut bytes = fs::read(&path).unwrap();
    bytes[..8].copy_from_slice(&0u64.to_be_bytes());
    fs::write(path, bytes).unwrap();
    let reply = store
        .execute(
            Some(&first),
            "Session::start();echo str(Session::has(\"old\"));",
        )
        .unwrap();
    assert_ne!(reply.id(), first);
    assert_eq!(reply.body, b"false");
    assert!(
        store
            .execute(
                Some(&format!("{first}; SCRIPTA_SESSION={first}")),
                "Session::start();"
            )
            .unwrap_err()
            .contains("duplicate")
    );
}
#[test]
fn unsupported_values_bounds_and_failed_assignment_preserve_session_data() {
    let mut store = Store::new();
    store.config.max_bytes = 512;
    let first = store
        .execute(None, "Session::start();Session::set(\"x\",42);")
        .unwrap()
        .id();
    let reply=store.execute(Some(&first),r#"Session::start();$result=42;try {$result=Session::set("x",function(){return 1;});}catch(SessionError $e){} echo str($result);echo str(Session::get("x"));"#).unwrap();
    assert_eq!(reply.body, b"4242");
    assert!(
        store
            .execute(
                Some(&first),
                &format!(
                    "Session::start();Session::set(\"x\",\"{}\");",
                    "x".repeat(600)
                )
            )
            .unwrap_err()
            .contains("byte limit")
    );
    assert_eq!(
        store
            .execute(
                Some(&first),
                "Session::start();echo str(Session::get(\"x\"));"
            )
            .unwrap()
            .body,
        b"42"
    );
    for source in [
        "Session::get(\"x\");",
        "Session::start(1);",
        "Session::set(1,2);",
        "Session::start();Session::regenerate();Session::regenerate();",
        "Session::start();Session::destroy();Session::start();",
    ] {
        assert!(store.execute(Some(&first), source).is_err(), "{source}");
    }
}
#[test]
fn locks_are_cancellable_and_released_when_request_state_drops() {
    let store = Store::new();
    let id = store.execute(None, "Session::start();").unwrap().id();
    let headers = vec![("cookie".into(), format!("SCRIPTA_SESSION={id}"))];
    let mut first = host_runtime();
    let response = web::register(
        &mut first,
        web::Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            body: vec![],
            request_id: None,
            connection: Default::default(),
            headers: headers.clone(),
        },
        32768,
    )
    .unwrap();
    let lease = session::register(&mut first, store.config.clone(), &headers, response).unwrap();
    first
        .execute_source("<?scripta Session::start();", &mut vec![])
        .unwrap();
    let config = store.config.clone();
    let id2 = id.clone();
    let flag = Arc::new(AtomicBool::new(false));
    let interrupt = flag.clone();
    let waiting = std::thread::spawn(move || {
        let mut options = RuntimeOptions {
            interrupt_flag: Some(interrupt),
            ..Default::default()
        };
        options
            .context
            .permissions
            .allowed
            .insert("web.session".into());
        let mut runtime = Runtime::with_options(options);
        let headers = vec![("cookie".into(), format!("SCRIPTA_SESSION={id2}"))];
        let response = web::register(
            &mut runtime,
            web::Request {
                method: "GET".into(),
                path: "/".into(),
                query_string: "".into(),
                body: vec![],
                request_id: None,
                connection: Default::default(),
                headers: headers.clone(),
            },
            32768,
        )
        .unwrap();
        let _session = session::register(&mut runtime, config, &headers, response).unwrap();
        runtime
            .execute_source("<?scripta Session::start();", &mut vec![])
            .unwrap_err()
            .message
    });
    std::thread::sleep(Duration::from_millis(30));
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(waiting.join().unwrap().contains("interrupted"));
    drop(lease);
    drop(first);
    assert!(store.execute(Some(&id), "Session::start();").is_ok());
}
#[test]
fn store_rejects_symlinks_and_cleans_abandoned_atomic_staging_files() {
    let store = Store::new();
    let first = store.execute(None, "Session::start();").unwrap().id();
    let stage = store.directory.path().join(".session-tmp-abandoned");
    let mut stage_file = fs::File::create(&stage).unwrap();
    stage_file.write_all(b"incomplete").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o600)).unwrap();
    }
    drop(stage_file);
    store.execute(None, "Session::start();").unwrap();
    assert!(!stage.exists());
    // Cleanup must preserve the already committed session on every platform.
    assert_eq!(
        store
            .execute(Some(&first), "Session::start();")
            .unwrap()
            .id(),
        first
    );
    #[cfg(unix)]
    {
        let external = tempfile::NamedTempFile::new().unwrap();
        let path = store.directory.path().join(format!("{first}.data"));
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(external.path(), &path).unwrap();
        assert!(
            store
                .execute(Some(&first), "Session::start();")
                .unwrap_err()
                .contains("regular files")
        );
        assert!(fs::read(external.path()).unwrap().is_empty());
    }
}

#[test]
fn session_logical_and_encoded_limits_preserve_records_at_the_boundary() {
    let mut store = Store::new();
    store.config.max_bytes = 2 * std::mem::size_of::<scr_runtime::Value>()
        + std::mem::size_of::<scr_runtime::MapKey>()
        + 1;
    let id = store
        .execute(None, "Session::start();Session::set(\"k\",null);")
        .unwrap()
        .id();
    assert!(
        store
            .execute(Some(&id), "Session::start();Session::set(\"k\",\"x\");")
            .unwrap_err()
            .contains("byte limit")
    );
    assert_eq!(
        store
            .execute(
                Some(&id),
                "Session::start();echo str(Session::get(\"k\")===null);"
            )
            .unwrap()
            .body,
        b"true"
    );
    store.config.max_bytes = 512;
    assert!(
        store
            .execute(
                Some(&id),
                &format!(
                    "Session::start();Session::set(\"k\",Bytes::encodeUtf8(\"{}\"));",
                    "a".repeat(200)
                )
            )
            .unwrap_err()
            .contains("encoded byte limit")
    );
}
#[test]
fn supported_nesting_roundtrips_and_excess_nesting_fails_before_mutation() {
    let store = Store::new();
    let first=store.execute(None,"Session::start();$v=null;for($i=0;$i<31;$i=$i+1){$v=[\"x\"=>$v];}Session::set(\"deep\",$v);").unwrap().id();
    assert_eq!(store.execute(Some(&first),"Session::start();$v=null;for($i=0;$i<31;$i=$i+1){$v=[\"x\"=>$v];}echo str(Session::get(\"deep\")===$v);").unwrap().body,b"true");
    assert!(store.execute(Some(&first),"Session::start();$v=null;for($i=0;$i<33;$i=$i+1){$v=[\"x\"=>$v];}Session::set(\"deep\",$v);").unwrap_err().contains("nesting"));
}
#[cfg(unix)]
#[test]
fn existing_public_session_directories_and_hard_links_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = session::Config::new(directory.path().into());
    assert!(config.prepare().unwrap_err().message.contains("private"));
    let store = Store::new();
    let id = store.execute(None, "Session::start();").unwrap().id();
    let original = store.directory.path().join(format!("{id}.data"));
    let outside = tempfile::tempdir().unwrap();
    fs::hard_link(&original, outside.path().join("linked")).unwrap();
    assert!(
        store
            .execute(Some(&id), "Session::start();")
            .unwrap_err()
            .contains("unlinked elsewhere")
    );
}

#[test]
fn session_capability_is_required_even_when_a_host_registers_the_module() {
    let store = Store::new();
    let mut runtime = Runtime::new();
    let response = web::register(
        &mut runtime,
        web::Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            body: vec![],
            request_id: None,
            connection: Default::default(),
            headers: vec![],
        },
        32768,
    )
    .unwrap();
    let _session = session::register(&mut runtime, store.config.clone(), &[], response).unwrap();
    let error = runtime
        .execute_source("<?scripta Session::start();", &mut vec![])
        .unwrap_err();
    assert!(error.message.contains("PermissionError"));
    assert!(error.message.contains("web.session"));
    assert_eq!(fs::read_dir(store.directory.path()).unwrap().count(), 0);
}

#[test]
fn readonly_session_records_fail_without_replacement_or_data_loss() {
    let store = Store::new();
    let id = store
        .execute(
            None,
            "Session::start();Session::set(\"secret\",\"preserve\");",
        )
        .unwrap()
        .id();
    let path = store.directory.path().join(format!("{id}.data"));
    let before = fs::read(&path).unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut readonly = permissions.clone();
    readonly.set_readonly(true);
    fs::set_permissions(&path, readonly).unwrap();
    let result = store.execute(
        Some(&id),
        "Session::start();Session::set(\"secret\",\"changed\");",
    );
    assert!(result.unwrap_err().contains("writable"));
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::set_permissions(path, permissions).unwrap();
}
