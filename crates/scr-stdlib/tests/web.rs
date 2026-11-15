use scr_runtime::{Runtime, RuntimeOptions, Value};
use scr_stdlib::web::{self, Request};
use std::{
    io::{self, Write},
    sync::{Arc, atomic::AtomicBool},
};
fn runtime(
    body: &[u8],
    content_type: &str,
    query: &str,
) -> (Runtime, std::rc::Rc<std::cell::RefCell<web::Response>>) {
    let mut runtime = Runtime::new();
    scr_stdlib::register(&mut runtime).unwrap();
    let state = web::register(
        &mut runtime,
        Request {
            method: "POST".into(),
            path: "/a%20b".into(),
            query_string: query.into(),
            headers: vec![
                ("content-type".into(), content_type.into()),
                ("x-test".into(), "first".into()),
                ("x-test".into(), "second".into()),
            ],
            body: body.to_vec(),
        },
        1024,
    )
    .unwrap();
    (runtime, state)
}
fn run(runtime: &mut Runtime, source: &str) -> Vec<u8> {
    let mut output = vec![];
    runtime
        .execute_source(&format!("<?scripta {source}"), &mut output)
        .unwrap();
    output
}
#[test]
fn request_data_preserves_duplicates_encoded_paths_and_strict_types() {
    let (mut runtime, _) = runtime(
        br#"{"n":1}"#,
        "application/json; charset=utf-8",
        "a=1&a=2&x=a+b&%31=v",
    );
    run(
        &mut runtime,
        r#"$r=Web::request(); $method=$r->method; $path=$r->path; $query=$r->query(); $header=$r->header("X-Test"); $headers=$r->headers("x-test"); $missing=$r->header("no"); $json=$r->json(); $n=$json["n"];"#,
    );
    assert_eq!(
        runtime.get_global("method"),
        Some(&Value::String("POST".into()))
    );
    assert_eq!(
        runtime.get_global("path"),
        Some(&Value::String("/a%20b".into()))
    );
    assert_eq!(
        runtime.get_global("header"),
        Some(&Value::String("first".into()))
    );
    assert_eq!(
        runtime.get_global("headers"),
        Some(&Value::List(vec![
            Value::String("first".into()),
            Value::String("second".into())
        ]))
    );
    assert_eq!(runtime.get_global("missing"), Some(&Value::Null));
    assert_eq!(runtime.get_global("n"), Some(&Value::Int(1)));
    assert_eq!(
        runtime.get_global("query"),
        Some(&Value::List(vec![
            Value::List(vec![Value::String("a".into()), Value::String("1".into())]),
            Value::List(vec![Value::String("a".into()), Value::String("2".into())]),
            Value::List(vec![Value::String("x".into()), Value::String("a b".into())]),
            Value::List(vec![Value::String("1".into()), Value::String("v".into())])
        ]))
    );
    assert!(
        runtime
            .execute_source("<?scripta $r->method=\"GET\";", &mut vec![])
            .unwrap_err()
            .message
            .contains("read-only")
    );
}
#[test]
fn bodies_reject_malformed_encoding_and_wrong_media_types_without_assignment() {
    for (body, kind, call) in [
        (
            b"%zz".as_slice(),
            "application/x-www-form-urlencoded",
            "form",
        ),
        (b"{", "application/json", "json"),
        (b"{}", "text/plain", "json"),
        (b"\xff", "text/plain", "text"),
    ] {
        let (mut runtime, _) = runtime(body, kind, "");
        run(&mut runtime, "$r=Web::request(); $value=42;");
        assert!(
            runtime
                .execute_source(&format!("<?scripta $value=$r->{call}();"), &mut vec![])
                .is_err()
        );
        assert_eq!(runtime.get_global("value"), Some(&Value::Int(42)));
    }
    let (mut runtime, _) = runtime(b"a=1&a=2", "application/x-www-form-urlencoded", "");
    run(&mut runtime, "$r=Web::request(); $pairs=$r->form();");
    let Some(Value::List(pairs)) = runtime.get_global("pairs") else {
        panic!()
    };
    assert_eq!(pairs.len(), 2);
    let (mut runtime, _) = self::runtime(b"\xff\x00", "application/octet-stream", "");
    assert_eq!(
        run(&mut runtime, "$r=Web::request(); Web::write($r->bytes());"),
        b"\xff\x00"
    );
}
#[test]
fn headers_redirects_and_failure_are_atomic() {
    let (mut runtime, state) = runtime(b"", "text/plain", "");
    run(
        &mut runtime,
        r#"Web::header("X-Test","old"); Web::header("x-test","new"); Web::addHeader("X-Test","extra"); Web::redirect("/done",303); echo "still running";"#,
    );
    assert_eq!(state.borrow().status, 303);
    assert_eq!(
        state.borrow().headers,
        vec![
            ("x-test".into(), "new".into()),
            ("x-test".into(), "extra".into()),
            ("location".into(), "/done".into())
        ]
    );
    for source in [
        r#"Web::header("x","bad\r\nInjected: yes");"#,
        r#"Web::header("Content-Length","1");"#,
        r#"Web::status(199);"#,
        r#"Web::redirect("/bad",200);"#,
        r#"Web::redirect("javascript:alert(1)");"#,
        r#"Web::redirect("//evil.test");"#,
        r#"Web::redirect("https://user:pass@example.com/");"#,
    ] {
        let before = state.borrow().headers.clone();
        assert!(
            runtime
                .execute_source(&format!("<?scripta {source}"), &mut vec![])
                .is_err(),
            "{source}"
        );
        assert_eq!(state.borrow().status, 303);
        assert_eq!(state.borrow().headers, before);
    }
    assert!(
        runtime
            .execute_source(
                &format!("<?scripta Web::header(\"big\",\"{}\");", "x".repeat(1024)),
                &mut vec![]
            )
            .is_err()
    );
    for code in [301, 302, 303, 307, 308] {
        run(
            &mut runtime,
            &format!("Web::redirect(\"https://example.com/\",{code});"),
        );
        assert_eq!(state.borrow().status, code);
    }
}
#[test]
fn web_functions_validate_signatures_and_context_is_opt_in() {
    let mut plain = Runtime::new();
    scr_stdlib::register(&mut plain).unwrap();
    assert!(
        plain
            .execute_source("<?scripta Web::request();", &mut vec![])
            .is_err()
    );
    let (mut runtime, _) = runtime(b"", "text/plain", "");
    for source in [
        "Web::request(1);",
        "Web::status(\"200\");",
        "Web::header(\"x\");",
        "Web::redirect(1);",
        "Web::write(1);",
        "$r=Web::request(); $r->header();",
    ] {
        assert!(
            runtime
                .execute_source(&format!("<?scripta {source}"), &mut vec![])
                .is_err(),
            "{source}"
        );
    }
}
#[test]
fn request_objects_obey_value_limits_and_cancellation() {
    let mut runtime = Runtime::with_options(RuntimeOptions {
        max_value_bytes: Some(100),
        ..Default::default()
    });
    web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            headers: vec![],
            body: vec![0; 101],
        },
        1024,
    )
    .unwrap();
    assert!(
        runtime
            .execute_source("<?scripta $r=Web::request();", &mut vec![])
            .unwrap_err()
            .message
            .contains("value byte limit")
    );
    assert!(runtime.get_global("r").is_none());
    let mut runtime = Runtime::with_options(RuntimeOptions {
        interrupt_flag: Some(Arc::new(AtomicBool::new(true))),
        ..Default::default()
    });
    web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            headers: vec![],
            body: vec![],
        },
        1024,
    )
    .unwrap();
    assert!(
        runtime
            .execute_source("<?scripta Web::status(404);", &mut vec![])
            .unwrap_err()
            .message
            .contains("interrupted")
    );
}
struct Failed;
impl Write for Failed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("full"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn binary_output_reports_writer_failure() {
    let (mut runtime, _) = runtime(b"", "text/plain", "");
    assert!(
        runtime
            .execute_source("<?scripta Web::write(\"x\");", &mut Failed)
            .unwrap_err()
            .message
            .contains("full")
    );
}

#[test]
fn response_header_exact_limits_reject_only_the_excess_without_mutation() {
    let mut runtime = Runtime::new();
    let state = web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            headers: vec![],
            body: vec![],
        },
        10,
    )
    .unwrap();
    run(&mut runtime, "Web::header(\"x\",\"12345\");");
    assert_eq!(state.borrow().headers, vec![("x".into(), "12345".into())]);
    assert!(
        runtime
            .execute_source("<?scripta Web::header(\"x\",\"123456\");", &mut vec![])
            .unwrap_err()
            .message
            .contains("header limit")
    );
    assert_eq!(state.borrow().headers, vec![("x".into(), "12345".into())]);
    let mut runtime = Runtime::new();
    let state = web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            headers: vec![],
            body: vec![],
        },
        32768,
    )
    .unwrap();
    for _ in 0..100 {
        run(&mut runtime, "Web::addHeader(\"x\",\"v\");");
    }
    assert!(
        runtime
            .execute_source("<?scripta Web::addHeader(\"x\",\"v\");", &mut vec![])
            .is_err()
    );
    assert_eq!(state.borrow().headers.len(), 100);
    run(&mut runtime, "Web::header(\"X\",\"replacement\");");
    assert_eq!(
        state.borrow().headers,
        vec![("x".into(), "replacement".into())]
    );
}

#[test]
fn cookies_decode_utf8_preserve_duplicates_and_keep_names_case_sensitive() {
    let mut runtime = Runtime::new();
    scr_stdlib::register(&mut runtime).unwrap();
    web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            body: vec![],
            headers: vec![
                (
                    "Cookie".into(),
                    "theme=hello%20world; theme=second; Name=%E4%B8%96%E7%95%8C; plus=a+b".into(),
                ),
                ("cookie".into(), "quoted=\"yes\"".into()),
            ],
        },
        32768,
    )
    .unwrap();
    run(
        &mut runtime,
        r#"$r=Web::request(); $theme=$r->cookie("theme");$name=$r->cookie("Name");$missing=$r->cookie("name");$plus=$r->cookie("plus");$all=$r->cookies();"#,
    );
    assert_eq!(
        runtime.get_global("theme"),
        Some(&Value::String("hello world".into()))
    );
    assert_eq!(
        runtime.get_global("name"),
        Some(&Value::String("世界".into()))
    );
    assert_eq!(runtime.get_global("missing"), Some(&Value::Null));
    assert_eq!(
        runtime.get_global("plus"),
        Some(&Value::String("a+b".into()))
    );
    let Some(Value::List(pairs)) = runtime.get_global("all") else {
        panic!()
    };
    assert_eq!(pairs.len(), 5);
    for cookie in ["no-equals", "bad name=x", "x=bad,value", "x=%zz", "x=%ff"] {
        let mut runtime = Runtime::new();
        web::register(
            &mut runtime,
            Request {
                method: "GET".into(),
                path: "/".into(),
                query_string: "".into(),
                body: vec![],
                headers: vec![("cookie".into(), cookie.into())],
            },
            32768,
        )
        .unwrap();
        assert!(
            runtime
                .execute_source(
                    "<?scripta $r=Web::request(); $value=42; $value=$r->cookies();",
                    &mut vec![]
                )
                .is_err()
        );
        assert_eq!(runtime.get_global("value"), Some(&Value::Int(42)));
    }
}
#[test]
fn cookie_helpers_encode_values_apply_attributes_and_delete_matching_scopes() {
    let (mut runtime, state) = runtime(b"", "text/plain", "");
    run(
        &mut runtime,
        r#"Web::cookie("greeting","hello 世界;=+\r\n");Web::cookie("theme","dark",["path"=>"/app","domain"=>".Example.com","secure"=>false,"httpOnly"=>false,"sameSite"=>"Strict","maxAge"=>60,"expires"=>0]);Web::deleteCookie("theme",["path"=>"/app","domain"=>"example.com","secure"=>false]);"#,
    );
    let headers = &state.borrow().headers;
    assert_eq!(headers.len(), 3);
    assert!(headers[0].1.starts_with(
        "greeting=hello%20%E4%B8%96%E7%95%8C%3B%3D%2B%0D%0A; Path=/; Secure; HttpOnly; SameSite=Lax"
    ));
    assert!(
        headers[1]
            .1
            .contains("Domain=example.com; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT")
    );
    assert!(!headers[1].1.contains("Secure"));
    assert!(!headers[1].1.contains("HttpOnly"));
    assert!(headers[2].1.contains(
        "theme=; Path=/app; Domain=example.com; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
    ));
}
#[test]
fn cookie_validation_rejects_injection_prefix_violations_and_byte_limit_plus_one() {
    let mut runtime = Runtime::new();
    let state = web::register(
        &mut runtime,
        Request {
            method: "GET".into(),
            path: "/".into(),
            query_string: "".into(),
            body: vec![],
            headers: vec![],
        },
        32768,
    )
    .unwrap();
    for source in [
        r#"Web::cookie("bad;name","x");"#,
        r#"Web::cookie("x","v",["path"=>"/; Evil=yes"]);"#,
        r#"Web::cookie("x","v",["domain"=>"example.com\r\nEvil: yes"]);"#,
        r#"Web::cookie("x","v",["secure"=>false,"sameSite"=>"None"]);"#,
        r#"Web::cookie("__Secure-x","v",["secure"=>false]);"#,
        r#"Web::cookie("__Host-x","v",["domain"=>"example.com"]);"#,
        r#"Web::cookie("__Http-x","v",["httpOnly"=>false]);"#,
        r#"Web::cookie("x","v",["maxAge"=>-1]);"#,
        r#"Web::cookie("x","v",["unknown"=>true]);"#,
        r#"Web::deleteCookie("x",["secure"=>1]);"#,
    ] {
        assert!(
            runtime
                .execute_source(&format!("<?scripta {source}"), &mut vec![])
                .is_err(),
            "{source}"
        );
        assert!(state.borrow().headers.is_empty());
    }
    run(&mut runtime, "Web::cookie(\"x\",\"\");");
    let overhead = state.borrow().headers[0].1.len();
    run(
        &mut runtime,
        &format!("Web::cookie(\"x\",\"{}\");", "a".repeat(4096 - overhead)),
    );
    assert_eq!(state.borrow().headers[1].1.len(), 4096);
    assert!(
        runtime
            .execute_source(
                &format!(
                    "<?scripta Web::cookie(\"x\",\"{}\");",
                    "a".repeat(4097 - overhead)
                ),
                &mut vec![]
            )
            .is_err()
    );
    assert_eq!(state.borrow().headers.len(), 2);
}

#[test]
fn decoded_cookie_pairs_use_exact_result_budgets() {
    let expected = 4 * std::mem::size_of::<Value>() + 2;
    for (limit, success) in [(expected, true), (expected - 1, false)] {
        let mut runtime = Runtime::with_options(RuntimeOptions {
            max_value_bytes: Some(limit),
            ..Default::default()
        });
        web::register(
            &mut runtime,
            Request {
                method: "GET".into(),
                path: "/".into(),
                query_string: "".into(),
                body: vec![],
                headers: vec![("cookie".into(), "a=%61".into())],
            },
            32768,
        )
        .unwrap();
        let result = runtime.execute_source(
            "<?scripta $r=Web::request();$pairs=$r->cookies();",
            &mut vec![],
        );
        assert_eq!(result.is_ok(), success, "{result:?}");
        if !success {
            assert!(runtime.get_global("pairs").is_none());
        }
    }
}
