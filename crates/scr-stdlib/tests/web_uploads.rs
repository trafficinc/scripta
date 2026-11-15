use scr_runtime::{NativeFunction, NativeSignature, NativeType, Runtime, RuntimeOptions, Value};
use scr_stdlib::{
    FilesystemPolicy,
    web::{
        self, Request,
        uploads::{File, Multipart, RequestResources, Stage},
    },
};
use std::{cell::RefCell, path::Path, rc::Rc};
struct Fixture {
    runtime: Runtime,
    guard: Option<RequestResources>,
    directory: tempfile::TempDir,
    stages: Rc<RefCell<Vec<tempfile::TempPath>>>,
}
impl Fixture {
    fn new(policy: FilesystemPolicy, options: RuntimeOptions) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("body");
        std::fs::write(&raw, b"raw\x00\xff").unwrap();
        let file = directory.path().join("file");
        std::fs::write(&file, b"\x00\xffabc").unwrap();
        let mut runtime = Runtime::with_options(options);
        scr_stdlib::register(&mut runtime).unwrap();
        let stages = Rc::new(RefCell::new(vec![]));
        let owned = stages.clone();
        let stage: Stage = Rc::new(move |_, parent: &Path| {
            let path = tempfile::Builder::new()
                .prefix(".scripta-upload-")
                .tempfile_in(parent)?
                .into_temp_path();
            let result = path.to_path_buf();
            owned.borrow_mut().push(path);
            Ok(result)
        });
        let (_, guard) = web::register_multipart(
            &mut runtime,
            Request {
                method: "POST".into(),
                path: "/".into(),
                query_string: String::new(),
                headers: vec![],
                body: vec![],
                request_id: None,
                connection: Default::default(),
            },
            Multipart {
                is_multipart: true,
                body: raw,
                body_size: 5,
                fields: vec![("a".into(), "1".into()), ("a".into(), "é".into())],
                files: vec![File {
                    field_name: "f".into(),
                    filename: "../../untrusted.bin".into(),
                    media_type: Some("image/png".into()),
                    size: 5,
                    path: file,
                }],
            },
            policy,
            stage,
            1024,
        )
        .unwrap();
        Self {
            runtime,
            guard: Some(guard),
            directory,
            stages,
        }
    }
    fn run(&mut self, source: &str) -> Result<Vec<u8>, scr_runtime::RuntimeError> {
        let mut out = vec![];
        self.runtime
            .execute_source(&format!("<?scripta {source}"), &mut out)
            .map(|_| out)
    }
    fn init(&mut self) {
        self.run("$r=Web::request(); $files=$r->uploads(); $f=$files[0];")
            .unwrap();
    }
    fn destination(&self) -> String {
        serde_json::to_string(self.directory.path().join("saved").to_str().unwrap()).unwrap()
    }
}
#[test]
fn metadata_duplicates_raw_bytes_move_and_shared_consumption() {
    let mut f = Fixture::new(FilesystemPolicy::default(), RuntimeOptions::default());
    f.init();
    assert_eq!(
        f.run("echo Json::encode($r->form());").unwrap(),
        "[[\"a\",\"1\"],[\"a\",\"é\"]]".as_bytes()
    );
    f.run("$raw=$r->bytes();$name=$f->filename;$type=$f->mediaType;$size=$f->size;$alias=$r->uploads();").unwrap();
    assert_eq!(
        f.runtime.get_global("raw"),
        Some(&Value::Bytes(b"raw\x00\xff".to_vec().into()))
    );
    assert!(
        f.run("$r->text();")
            .unwrap_err()
            .message
            .contains("EncodingError")
    );
    assert_eq!(
        f.runtime.get_global("name"),
        Some(&Value::String("../../untrusted.bin".into()))
    );
    f.run(&format!(
        "$f->moveTo({});$moved=$alias[0]->moved;",
        f.destination()
    ))
    .unwrap();
    assert_eq!(
        std::fs::read(f.directory.path().join("saved")).unwrap(),
        b"\x00\xffabc"
    );
    assert_eq!(f.runtime.get_global("moved"), Some(&Value::Bool(true)));
    assert!(
        f.run(&format!("$f->moveTo({});", f.destination()))
            .unwrap_err()
            .message
            .contains("already been moved")
    );
    assert!(f.stages.borrow().iter().all(|p| !p.exists()));
}
#[test]
fn failure_preserves_destination_and_handle_and_rejects_expiry_and_foreign_runtime() {
    let mut f = Fixture::new(FilesystemPolicy::default(), RuntimeOptions::default());
    f.init();
    std::fs::write(f.directory.path().join("saved"), b"original").unwrap();
    assert!(f.run(&format!("$f->moveTo({});", f.destination())).is_err());
    assert_eq!(
        std::fs::read(f.directory.path().join("saved")).unwrap(),
        b"original"
    );
    f.run("$moved=$f->moved;").unwrap();
    assert_eq!(f.runtime.get_global("moved"), Some(&Value::Bool(false)));
    f.run("$move=$f->moveTo;").unwrap();
    let function = f.runtime.get_global("move").unwrap().clone();
    let mut other = Runtime::new();
    other
        .register(
            "Foreign::move",
            NativeFunction::new(
                NativeSignature::new(vec![], NativeType::Any),
                move |_, _| Ok(function.clone()),
            ),
        )
        .unwrap();
    assert!(
        other
            .execute_source("<?scripta $fn=Foreign::move();$fn(\"other\");", &mut vec![])
            .unwrap_err()
            .message
            .contains("another or expired")
    );
    f.guard.take();
    assert!(
        f.run("$r->uploads();")
            .unwrap_err()
            .message
            .contains("expired")
    );
    assert!(
        f.run("$move(\"other\");")
            .unwrap_err()
            .message
            .contains("expired")
    );
}
#[test]
fn write_permissions_roots_and_symlinks_are_enforced() {
    for policy in [
        FilesystemPolicy {
            write: false,
            ..Default::default()
        },
        FilesystemPolicy {
            roots: Some(vec![]),
            ..Default::default()
        },
    ] {
        let mut f = Fixture::new(policy, RuntimeOptions::default());
        f.init();
        assert!(
            f.run(&format!("$f->moveTo({});", f.destination()))
                .unwrap_err()
                .message
                .contains("PermissionError")
        );
        assert!(f.stages.borrow().is_empty());
    }
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.write");
    let mut f = Fixture::new(FilesystemPolicy::default(), options);
    f.init();
    assert!(f.run(&format!("$f->moveTo({});", f.destination())).is_err());
    #[cfg(unix)]
    {
        let mut f = Fixture::new(FilesystemPolicy::default(), RuntimeOptions::default());
        f.init();
        std::os::unix::fs::symlink(
            f.directory.path().join("file"),
            f.directory.path().join("saved"),
        )
        .unwrap();
        assert!(f.run(&format!("$f->moveTo({});", f.destination())).is_err());
        assert!(f.stages.borrow().is_empty());
    }
}

#[test]
fn changed_source_fails_without_publication_and_allows_retry() {
    let mut f = Fixture::new(FilesystemPolicy::default(), RuntimeOptions::default());
    f.init();
    std::fs::write(f.directory.path().join("file"), b"x").unwrap();
    assert!(
        f.run(&format!("$f->moveTo({});", f.destination()))
            .unwrap_err()
            .message
            .contains("size changed")
    );
    assert!(!f.directory.path().join("saved").exists());
    assert!(f.stages.borrow().iter().all(|p| !p.exists()));
    std::fs::write(f.directory.path().join("file"), b"\0\xffabc").unwrap();
    f.run(&format!("$f->moveTo({});", f.destination())).unwrap();
}
#[test]
fn body_streams_share_handle_quota_preserve_raw_bytes_and_expire_with_request() {
    let mut f = Fixture::new(FilesystemPolicy::default(), RuntimeOptions::default());
    f.run("$r=Web::request();$input=$r->bodyStream();$size=$r->bodySize;$data=Stream::read($input,5);").unwrap();
    assert_eq!(
        f.runtime.get_global("data"),
        Some(&Value::Bytes(b"raw\0\xff".to_vec().into()))
    );
    assert_eq!(f.runtime.get_global("size"), Some(&Value::Int(5)));
    f.guard.take();
    f.run("$closed=$input->closed;").unwrap();
    assert_eq!(f.runtime.get_global("closed"), Some(&Value::Bool(true)));
    assert!(
        f.run("Stream::read($input,1);")
            .unwrap_err()
            .message
            .contains("expired")
    );
}
#[test]
fn uploaded_contents_can_be_inspected_without_paths_or_ambient_read_permission() {
    let mut options = RuntimeOptions::default();
    options
        .context
        .permissions
        .allowed
        .remove("filesystem.read");
    let mut f = Fixture::new(
        FilesystemPolicy {
            read: false,
            ..Default::default()
        },
        options,
    );
    f.init();
    f.run("$input=$f->stream();$prefix=Stream::read($input,2);Stream::close($input);")
        .unwrap();
    assert_eq!(
        f.runtime.get_global("prefix"),
        Some(&Value::Bytes(b"\0\xff".to_vec().into()))
    );
    f.run(&format!("$f->moveTo({});", f.destination())).unwrap();
    assert!(
        f.run("$f->stream();")
            .unwrap_err()
            .message
            .contains("already been moved")
    );
}
