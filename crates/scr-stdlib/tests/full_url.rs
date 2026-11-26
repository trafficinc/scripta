use scr_runtime::{NativeObject, Runtime, RuntimeOptions, Value};
use std::rc::Rc;

fn runtime() -> Runtime {
    let mut runtime = Runtime::new();
    scr_stdlib::register(&mut runtime).unwrap();
    runtime
}
fn run(runtime: &mut Runtime, source: &str) {
    runtime.execute_source(source, &mut Vec::new()).unwrap();
}
fn error(runtime: &mut Runtime, source: &str, kind: &str) {
    let error = runtime.execute_source(source, &mut Vec::new()).unwrap_err();
    assert!(error.message.starts_with(&format!("{kind}:")), "{error}");
}
fn parse(runtime: &mut Runtime, input: &str) {
    runtime
        .set_global("input", Value::String(input.into()))
        .unwrap();
    run(
        runtime,
        "$url=Url::parse($input); $canonical=Url::serialize($url);",
    );
}
fn string(text: &str) -> Value {
    Value::String(text.into())
}
fn property(runtime: &Runtime, name: &str) -> Value {
    let Some(Value::NativeObject(object)) = runtime.get_global("url") else {
        panic!("expected Url");
    };
    assert_eq!(object.type_name(), "Url");
    object.get_property(name).unwrap()
}
#[test]
fn special_urls_normalize_idna_credentials_ports_and_dot_paths() {
    let mut rt = runtime();
    parse(
        &mut rt,
        "HTTPS://UsEr:p%40ss@BÜCHER.example:443/a/../caf%C3%A9/%2F?q=a+b&q=%26#part",
    );
    assert_eq!(
        rt.get_global("canonical"),
        Some(&string(
            "https://UsEr:p%40ss@xn--bcher-kva.example/caf%C3%A9/%2F?q=a+b&q=%26#part"
        ))
    );
    for (name, value) in [
        ("scheme", string("https")),
        ("username", string("UsEr")),
        ("password", string("p%40ss")),
        ("host", string("xn--bcher-kva.example")),
        ("port", Value::Null),
        ("effectivePort", Value::Int(443)),
        ("path", string("/caf%C3%A9/%2F")),
        ("query", string("q=a+b&q=%26")),
        ("fragment", string("part")),
        ("origin", string("https://xn--bcher-kva.example")),
        ("canBeBase", Value::Bool(true)),
    ] {
        assert_eq!(property(&rt, name), value, "{name}");
    }
    parse(&mut rt, "https://usér:páss@example.test/π?x=é#é");
    assert_eq!(property(&rt, "username"), string("us%C3%A9r"));
    assert_eq!(property(&rt, "password"), string("p%C3%A1ss"));
    assert_eq!(property(&rt, "path"), string("/%CF%80"));
    assert_eq!(property(&rt, "query"), string("x=%C3%A9"));
    assert_eq!(property(&rt, "fragment"), string("%C3%A9"));
}
#[test]
fn ipv6_ipv4_schemes_default_ports_and_file_urls_are_platform_independent() {
    let mut rt = runtime();
    for (input, canonical, host, port, default) in [
        (
            "https://[2001:0db8::1]:443/a",
            "https://[2001:db8::1]/a",
            Some("[2001:db8::1]"),
            None,
            Some(443),
        ),
        (
            "http://127.1:80",
            "http://127.0.0.1/",
            Some("127.0.0.1"),
            None,
            Some(80),
        ),
        (
            "ftp://example.test:21",
            "ftp://example.test/",
            Some("example.test"),
            None,
            Some(21),
        ),
        (
            "ws://example.test:80",
            "ws://example.test/",
            Some("example.test"),
            None,
            Some(80),
        ),
        (
            "wss://example.test:443",
            "wss://example.test/",
            Some("example.test"),
            None,
            Some(443),
        ),
        (
            "https://example.test:0/",
            "https://example.test:0/",
            Some("example.test"),
            Some(0),
            Some(0),
        ),
        (
            "custom://Host:123/path",
            "custom://Host:123/path",
            Some("Host"),
            Some(123),
            Some(123),
        ),
        ("file://localhost/tmp/a", "file:///tmp/a", None, None, None),
        ("file:///C:/work/a", "file:///C:/work/a", None, None, None),
    ] {
        parse(&mut rt, input);
        assert_eq!(
            rt.get_global("canonical"),
            Some(&string(canonical)),
            "{input}"
        );
        assert_eq!(property(&rt, "host"), host.map_or(Value::Null, string));
        assert_eq!(property(&rt, "port"), port.map_or(Value::Null, Value::Int));
        assert_eq!(
            property(&rt, "effectivePort"),
            default.map_or(Value::Null, Value::Int)
        );
    }
    for (input, path, origin) in [
        ("mailto:user@example.test", "user@example.test", None),
        ("data:text/plain,caf%C3%A9", "text/plain,caf%C3%A9", None),
        ("urn:isbn:978", "isbn:978", None),
        ("javascript:alert(1)", "alert(1)", None),
        (
            "blob:https://example.test/id",
            "https://example.test/id",
            Some("https://example.test"),
        ),
    ] {
        parse(&mut rt, input);
        assert_eq!(property(&rt, "path"), string(path));
        assert_eq!(property(&rt, "host"), Value::Null);
        assert_eq!(property(&rt, "canBeBase"), Value::Bool(false));
        assert_eq!(property(&rt, "origin"), origin.map_or(Value::Null, string));
    }
}
#[test]
fn empty_components_encoded_delimiters_and_duplicate_queries_remain_explicit() {
    let mut rt = runtime();
    for (suffix, query, fragment) in [
        ("", None, None),
        ("?", Some(""), None),
        ("#", None, Some("")),
        ("?#", Some(""), Some("")),
    ] {
        parse(&mut rt, &format!("https://example.test/{suffix}"));
        assert_eq!(property(&rt, "query"), query.map_or(Value::Null, string));
        assert_eq!(
            property(&rt, "fragment"),
            fragment.map_or(Value::Null, string)
        );
    }
    parse(
        &mut rt,
        "https://example.test/a/%2e%2E/%2F/%FF/%00?tag=a+z&&tag=%2F&flag&empty=&encoded=%2526#%3F",
    );
    assert_eq!(property(&rt, "path"), string("/%2F/%FF/%00"));
    assert_eq!(
        property(&rt, "query"),
        string("tag=a+z&&tag=%2F&flag&empty=&encoded=%2526")
    );
    assert_eq!(property(&rt, "fragment"), string("%3F"));
    run(
        &mut rt,
        "$pairs=Url::parseQuery($url->query); $form=Form::decode($url->query); $round=Url::serialize(Url::parse(Url::serialize($url)));",
    );
    let pair = |a: &str, b: &str| Value::list(vec![string(a), string(b)]);
    assert_eq!(
        rt.get_global("pairs"),
        Some(&Value::list(vec![
            pair("tag", "a+z"),
            pair("tag", "/"),
            pair("flag", ""),
            pair("empty", ""),
            pair("encoded", "%26")
        ]))
    );
    let Some(Value::List(form)) = rt.get_global("form") else {
        panic!()
    };
    assert_eq!(form[0], pair("tag", "a z"));
    assert_eq!(rt.get_global("round"), rt.get_global("canonical"));
}
#[test]
fn relative_resolution_preserves_the_base_and_handles_query_fragment_and_authority() {
    let mut rt = runtime();
    parse(&mut rt, "http://a/b/c/d;p?q#original");
    run(&mut rt, "$base=$url;");
    for (reference, expected) in [
        ("g", "http://a/b/c/g"),
        ("./g", "http://a/b/c/g"),
        ("../g", "http://a/b/g"),
        ("../../g", "http://a/g"),
        ("/g", "http://a/g"),
        ("//other/x", "http://other/x"),
        ("?y", "http://a/b/c/d;p?y"),
        ("#s", "http://a/b/c/d;p?q#s"),
        ("", "http://a/b/c/d;p?q"),
        ("https://EXAMPLE.test:443/new", "https://example.test/new"),
        ("g?y#s", "http://a/b/c/g?y#s"),
        ("g%2Fh", "http://a/b/c/g%2Fh"),
        ("%2e%2e/g", "http://a/b/g"),
    ] {
        rt.set_global("reference", string(reference)).unwrap();
        run(
            &mut rt,
            "$resolved=Url::serialize(Url::resolve($base,$reference)); $unchanged=Url::serialize($base);",
        );
        assert_eq!(
            rt.get_global("resolved"),
            Some(&string(expected)),
            "{reference}"
        );
        assert_eq!(
            rt.get_global("unchanged"),
            Some(&string("http://a/b/c/d;p?q#original"))
        );
    }
    parse(&mut rt, "mailto:user@example.test");
    run(
        &mut rt,
        "$resolved=Url::serialize(Url::resolve($url,'#part'));",
    );
    assert_eq!(
        rt.get_global("resolved"),
        Some(&string("mailto:user@example.test#part"))
    );
    run(&mut rt, "$destination=42;");
    error(
        &mut rt,
        "$destination=Url::resolve($url,'../path');",
        "UrlError",
    );
    assert_eq!(rt.get_global("destination"), Some(&Value::Int(42)));
    run(&mut rt, "$unchanged=Url::serialize($url);");
    assert_eq!(
        rt.get_global("unchanged"),
        Some(&string("mailto:user@example.test"))
    );
}
#[test]
fn validation_failures_are_sanitized_and_preserve_assignments() {
    let mut rt = runtime();
    for input in [
        "",
        "relative/path",
        "//example.test/",
        "https://",
        "http://[bad]/",
        "http://example.test:65536/",
        "https://exa mple/",
        "https://example.test/%",
        "https://example.test/%0",
        "https://example.test/%GG",
        "https://example.test/\n",
        "https://example.test/\t",
        " https://example.test/",
        "https://example.test/\0",
        r"https:\\example.test\path",
    ] {
        rt.set_global("input", string(input)).unwrap();
        run(&mut rt, "$destination=42; $valid=Url::isValid($input);");
        assert_eq!(
            rt.get_global("valid"),
            Some(&Value::Bool(false)),
            "{input:?}"
        );
        error(&mut rt, "$destination=Url::parse($input);", "UrlError");
        assert_eq!(rt.get_global("destination"), Some(&Value::Int(42)));
    }
    rt.set_global("input", string("https://username:very-secret@[invalid]/"))
        .unwrap();
    let failure = rt
        .execute_source("Url::parse($input);", &mut Vec::new())
        .unwrap_err();
    assert!(!failure.message.contains("very-secret"));
    assert!(!failure.message.contains("username"));
    parse(
        &mut rt,
        "https://username:very-secret@example.test/?token=sensitive",
    );
    let debug = format!("{:?}", rt.get_global("url").unwrap());
    for private in ["username", "very-secret", "sensitive", "example.test"] {
        assert!(!debug.contains(private), "{debug}");
    }
    error(&mut rt, "$url->host='other';", "PropertyError");
    for source in [
        "Url::parse(1);",
        "Url::serialize('https://example.test/');",
        "Url::resolve('https://example.test/','a');",
        "Url::resolve($url,1);",
        "Url::isValid();",
    ] {
        error(&mut rt, source, "TypeError");
    }
    #[derive(Debug)]
    struct Fake;
    impl NativeObject for Fake {
        fn type_name(&self) -> &str {
            "Url"
        }
        fn get_property(&self, _: &str) -> Result<Value, String> {
            Err("fake".into())
        }
    }
    rt.set_global("fake", Value::NativeObject(Rc::new(Fake)))
        .unwrap();
    error(&mut rt, "Url::serialize($fake);", "TypeError");
    error(&mut rt, "Url::resolve($fake,'a');", "TypeError");
}
#[test]
fn parsing_is_pure_shareable_and_does_not_change_http_policy() {
    let mut options = RuntimeOptions::default();
    options.context.permissions.allowed.clear();
    let mut denied = Runtime::with_options(options);
    scr_stdlib::register(&mut denied).unwrap();
    parse(&mut denied, "http://user:secret@127.0.0.1:1/");
    run(&mut denied, "$valid=Url::isValid(Url::serialize($url));");
    assert_eq!(denied.get_global("valid"), Some(&Value::Bool(true)));
    error(
        &mut denied,
        "Http::get(Url::serialize($url));",
        "PermissionError",
    );
    let value = denied.get_global("url").unwrap().clone();
    drop(denied);
    let mut rt = runtime();
    rt.set_global("shared", value).unwrap();
    run(&mut rt, "$canonical=Url::serialize($shared);");
    assert_eq!(
        rt.get_global("canonical"),
        Some(&string("http://user:secret@127.0.0.1:1/"))
    );
    error(&mut rt, "Http::get(Url::serialize($shared));", "TypeError");
    error(
        &mut rt,
        "Http::get(Url::serialize(Url::parse('file:///tmp/a')));",
        "TypeError",
    );
}
#[test]
fn hard_input_and_canonical_limits_have_exact_boundaries() {
    let mut rt = runtime();
    let exact = format!("data:{}", "x".repeat(65536 - 5));
    parse(&mut rt, &exact);
    assert_eq!(rt.get_global("canonical"), Some(&string(&exact)));
    rt.set_global("large", string(&(exact + "x"))).unwrap();
    for source in [
        "Url::parse($large);",
        "Url::isValid($large);",
        "Url::resolve($url,$large);",
    ] {
        error(&mut rt, source, "ResourceLimitError");
    }
    let prefix = "https://example.test/";
    let count = (65536 - prefix.len()) / 6;
    let padding = "x".repeat(65536 - prefix.len() - count * 6);
    let expanding = format!("{prefix}{}{padding}", "é".repeat(count));
    assert!(expanding.len() < 65536);
    parse(&mut rt, &expanding);
    let Some(Value::String(canonical)) = rt.get_global("canonical") else {
        panic!()
    };
    assert_eq!(canonical.len(), 65536);
    rt.set_global("expanding", string(&(expanding + "x")))
        .unwrap();
    for source in [
        "Url::parse($expanding);",
        "Url::isValid($expanding);",
        "Url::resolve($url,$expanding);",
    ] {
        error(&mut rt, source, "ResourceLimitError");
    }
}
