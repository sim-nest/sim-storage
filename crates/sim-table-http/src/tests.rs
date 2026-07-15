use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
};

use sim_codec_lisp::LispCodecLib;
use sim_kernel::{
    DefaultFactory, EagerPolicy, Expr, ObjectEncode, ObjectEncoding, Symbol, Table,
    read_construct_capability,
};

use crate::{
    HttpDir, HttpDirOptions, HttpWriteMethod, http_dir_class_symbol, table_http_capability,
};

struct CapturedRequest {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    sim_test_support::register_core_classes(&mut cx);
    let lisp_id = cx.registry_mut().fresh_codec_id();
    cx.load_lib(&LispCodecLib::new(lisp_id).unwrap()).unwrap();
    cx
}

fn grant(cx: &mut sim_kernel::Cx) {
    cx.grant(table_http_capability());
}

fn serve_once<F>(handler: F) -> (String, JoinHandle<()>)
where
    F: FnOnce(CapturedRequest, &mut TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        handler(request, &mut stream);
    });
    (format!("http://127.0.0.1:{port}/items"), handle)
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    let head_text = String::from_utf8(head).unwrap();
    let mut lines = head_text.split("\r\n");
    let request_line = lines.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| {
            if line.is_empty() {
                None
            } else {
                let (key, value) = line.split_once(':').unwrap();
                Some((key.to_owned(), value.trim().to_owned()))
            }
        })
        .collect::<Vec<_>>();
    let content_length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, value)| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).unwrap();
    }
    CapturedRequest {
        request_line,
        headers,
        body,
    }
}

fn write_response(stream: &mut TcpStream, status: &str, body: &[u8]) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
}

fn table(base_url: String, max_body_bytes: usize) -> HttpDir {
    HttpDir::new(
        HttpDirOptions::new(base_url)
            .with_timeout_ms(1_000)
            .with_max_body_bytes(max_body_bytes),
    )
    .unwrap()
}

#[test]
fn http_get_returns_decoded_body() {
    let (base_url, handle) = serve_once(|request, stream| {
        assert_eq!(request.request_line, "GET /items/alpha HTTP/1.1");
        assert!(
            request
                .headers
                .iter()
                .any(|(key, value)| key == "Host" && value.starts_with("127.0.0.1"))
        );
        write_response(stream, "200 OK", br#""hello""#);
    });
    let mut cx = cx();
    grant(&mut cx);
    let dir = table(base_url, 1024);

    let value = dir.get(&mut cx, Symbol::new("alpha")).unwrap();

    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("hello".to_owned())
    );
    handle.join().unwrap();
}

#[test]
fn http_set_sends_encoded_body_with_configured_method() {
    let (base_url, handle) = serve_once(|request, stream| {
        assert_eq!(request.request_line, "POST /items/alpha HTTP/1.1");
        assert_eq!(request.body, br#""posted""#);
        write_response(stream, "204 No Content", b"");
    });
    let mut cx = cx();
    grant(&mut cx);
    let dir = HttpDir::new(
        HttpDirOptions::new(base_url)
            .with_write_method(HttpWriteMethod::Post)
            .with_timeout_ms(1_000)
            .with_max_body_bytes(1024),
    )
    .unwrap();
    let value = cx.factory().string("posted".to_owned()).unwrap();

    dir.set(&mut cx, Symbol::new("alpha"), value).unwrap();

    handle.join().unwrap();
}

#[test]
fn http_denied_without_capability() {
    let mut cx = cx();
    let dir = table("http://127.0.0.1:1/items".to_owned(), 1024);

    let err = dir.get(&mut cx, Symbol::new("alpha")).unwrap_err();

    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability }
            if capability == table_http_capability()
    ));
}

#[test]
fn http_oversize_body_capped() {
    let (base_url, handle) = serve_once(|_request, stream| {
        write_response(stream, "200 OK", br#""too-large""#);
    });
    let mut cx = cx();
    grant(&mut cx);
    let dir = table(base_url, 4);

    let err = dir.get(&mut cx, Symbol::new("alpha")).unwrap_err();

    assert!(
        err.to_string()
            .contains("response exceeded max body bytes 4")
    );
    handle.join().unwrap();
}

#[test]
fn http_non_2xx_is_error_value() {
    let (base_url, handle) = serve_once(|_request, stream| {
        write_response(stream, "404 Not Found", b"missing");
    });
    let mut cx = cx();
    grant(&mut cx);
    let dir = table(base_url, 1024);

    let err = dir.get(&mut cx, Symbol::new("missing")).unwrap_err();

    assert!(err.to_string().contains("http 404"));
    assert!(err.to_string().contains("missing"));
    handle.join().unwrap();
}

#[test]
fn http_has_maps_404_to_false() {
    let (base_url, handle) = serve_once(|request, stream| {
        assert_eq!(request.request_line, "HEAD /items/missing HTTP/1.1");
        write_response(stream, "404 Not Found", b"ignored");
    });
    let mut cx = cx();
    grant(&mut cx);
    let dir = table(base_url, 1024);

    assert!(!dir.has(&mut cx, Symbol::new("missing")).unwrap());
    handle.join().unwrap();
}

#[test]
fn http_dir_encodes_constructor_configuration() {
    let mut cx = cx();
    cx.grant(read_construct_capability());
    let dir = HttpDir::new(
        HttpDirOptions::new("http://example.test/root/")
            .with_timeout_ms(250)
            .with_max_body_bytes(64),
    )
    .unwrap();

    let encoding = dir.object_encoding(&mut cx).unwrap();

    let ObjectEncoding::Constructor { class, args } = encoding else {
        panic!("expected constructor encoding");
    };
    assert_eq!(class, http_dir_class_symbol());
    assert_eq!(args[0], Expr::Symbol(Symbol::new("v0")));
    assert_eq!(args[1], Expr::String("http://example.test/root".to_owned()));
}
