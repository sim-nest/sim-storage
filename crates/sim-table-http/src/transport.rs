//! Bounded HTTP transport used by `HttpDir`.

use std::{
    io::{self, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

use sim_kernel::{Error, Result};
use sim_lib_net_core::{HeadOutcome, HttpBodyMode, UrlParts};

#[cfg(feature = "tls")]
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, StreamOwned,
    pki_types::{CertificateDer, ServerName},
};
#[cfg(feature = "tls")]
use std::sync::Arc;

pub(crate) struct HttpRequest {
    pub(crate) method: &'static str,
    pub(crate) url: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: Duration,
    pub(crate) max_body_bytes: usize,
}

pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) reason: Option<String>,
    pub(crate) body: Vec<u8>,
}

trait ReadWrite: Read + Write {}

impl<T: Read + Write> ReadWrite for T {}

pub(crate) fn send(request: HttpRequest) -> Result<HttpResponse> {
    let parts = sim_lib_net_core::parse_url(&request.url)
        .map_err(|err| Error::Eval(format!("table/http: {err}")))?;
    let stream = connect_tcp(&parts, request.timeout)?;
    let mut stream = connect_stream(&parts, stream)?;
    let body_len = if request.body.is_empty() {
        None
    } else {
        Some(request.body.len())
    };
    let head = build_http_request_head(
        request.method,
        &parts.path,
        &host_header(&parts),
        body_len,
        &request.headers,
    )?;

    stream
        .write_all(head.as_bytes())
        .map_err(|err| io_error("write request head", err))?;
    if !request.body.is_empty() {
        stream
            .write_all(&request.body)
            .map_err(|err| io_error("write request body", err))?;
    }
    stream
        .flush()
        .map_err(|err| io_error("flush request", err))?;

    read_response(
        &mut *stream,
        request.max_body_bytes,
        request.method != "HEAD",
    )
}

fn connect_tcp(parts: &UrlParts, timeout: Duration) -> Result<TcpStream> {
    let mut addrs = (parts.host.as_str(), parts.port)
        .to_socket_addrs()
        .map_err(|err| io_error("resolve host", err))?;
    let addr = addrs
        .next()
        .ok_or_else(|| Error::HostError(format!("table/http: no address for {}", parts.host)))?;
    let stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|err| io_error("connect", err))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| io_error("set read timeout", err))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| io_error("set write timeout", err))?;
    Ok(stream)
}

fn connect_stream(parts: &UrlParts, stream: TcpStream) -> Result<Box<dyn ReadWrite>> {
    match parts.scheme.as_str() {
        "http" => Ok(Box::new(stream)),
        "https" => connect_tls(parts, stream),
        other => Err(Error::Eval(format!(
            "table/http: unsupported url scheme {other}"
        ))),
    }
}

#[cfg(feature = "tls")]
fn connect_tls(parts: &UrlParts, stream: TcpStream) -> Result<Box<dyn ReadWrite>> {
    let config = tls_client_config()?;
    let server_name = ServerName::try_from(parts.host.clone()).map_err(|_| {
        Error::Eval(format!(
            "table/http: invalid tls server name {}",
            parts.host
        ))
    })?;
    let connection = ClientConnection::new(config, server_name)
        .map_err(|err| Error::HostError(format!("table/http: tls {err}")))?;
    Ok(Box::new(StreamOwned::new(connection, stream)))
}

#[cfg(not(feature = "tls"))]
fn connect_tls(_parts: &UrlParts, _stream: TcpStream) -> Result<Box<dyn ReadWrite>> {
    Err(Error::Eval(
        "table/http: https URLs require the tls feature".to_owned(),
    ))
}

#[cfg(feature = "tls")]
fn tls_client_config() -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let cert_result = rustls_native_certs::load_native_certs();
    for certificate in cert_result.certs {
        roots
            .add(certificate)
            .map_err(|err| Error::HostError(format!("table/http: tls root {err}")))?;
    }
    if roots.is_empty() {
        return Err(Error::HostError(
            "table/http: no tls root certificates available".to_owned(),
        ));
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

fn read_response(
    stream: &mut dyn Read,
    max_body_bytes: usize,
    expect_body: bool,
) -> Result<HttpResponse> {
    let mut reader = BufReader::new(stream);
    let head = match sim_lib_net_core::read_head_until_double_crlf(&mut reader, 64 * 1024) {
        Ok(HeadOutcome::Head(head)) => head,
        Ok(HeadOutcome::TooLarge) => {
            return Err(Error::HostError(
                "table/http: response headers exceed size limit".to_owned(),
            ));
        }
        Ok(HeadOutcome::Eof | HeadOutcome::Truncated(_)) => {
            return Err(Error::HostError(
                "table/http: truncated response head".to_owned(),
            ));
        }
        Err(err) => return Err(io_error("read response head", err)),
    };
    let head_text = std::str::from_utf8(&head)
        .map_err(|_| Error::HostError("table/http: response headers are not utf-8".to_owned()))?;
    let parsed = sim_lib_net_core::parse_http_head(head_text)
        .map_err(|err| Error::HostError(format!("table/http: {err}")))?;
    if !expect_body {
        return Ok(HttpResponse {
            status: parsed.status,
            reason: Some(parsed.reason),
            body: Vec::new(),
        });
    }
    let mode = sim_lib_net_core::body_mode(&parsed)
        .map_err(|err| Error::HostError(format!("table/http: {err}")))?;
    let body = match mode {
        HttpBodyMode::ContentLength(length) => {
            if length > max_body_bytes {
                return Err(body_limit_error(max_body_bytes));
            }
            read_content_length(&mut reader, length)?
        }
        HttpBodyMode::Chunked => read_chunked(&mut reader, max_body_bytes)?,
        HttpBodyMode::UntilEof | HttpBodyMode::Empty => {
            read_to_end_limited(&mut reader, max_body_bytes)?
        }
    };
    Ok(HttpResponse {
        status: parsed.status,
        reason: Some(parsed.reason),
        body,
    })
}

fn read_content_length(reader: &mut BufReader<&mut dyn Read>, length: usize) -> Result<Vec<u8>> {
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .map_err(|err| io_error("read response body", err))?;
    Ok(body)
}

fn read_to_end_limited(
    reader: &mut BufReader<&mut dyn Read>,
    max_body_bytes: usize,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|err| io_error("read response body", err))?;
        if read == 0 {
            return Ok(body);
        }
        if body.len().saturating_add(read) > max_body_bytes {
            return Err(body_limit_error(max_body_bytes));
        }
        body.extend_from_slice(&chunk[..read]);
    }
}

fn read_chunked(reader: &mut BufReader<&mut dyn Read>, max_body_bytes: usize) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    let mut chunk = [0; 8192];
    let encoded_cap = max_body_bytes.saturating_add(64 * 1024);
    loop {
        match sim_lib_net_core::decode_chunked(&encoded, max_body_bytes) {
            Ok(body) => return Ok(body),
            Err(sim_lib_net_core::NetError::TruncatedChunk) => {}
            Err(sim_lib_net_core::NetError::OversizeBody(_)) => {
                return Err(body_limit_error(max_body_bytes));
            }
            Err(err) => return Err(Error::HostError(format!("table/http: {err}"))),
        }
        let read = reader
            .read(&mut chunk)
            .map_err(|err| io_error("read chunked response body", err))?;
        if read == 0 {
            return Err(Error::HostError(
                "table/http: truncated chunked response body".to_owned(),
            ));
        }
        if encoded.len().saturating_add(read) > encoded_cap {
            return Err(body_limit_error(max_body_bytes));
        }
        encoded.extend_from_slice(&chunk[..read]);
    }
}

fn host_header(parts: &UrlParts) -> String {
    let default = match parts.scheme.as_str() {
        "http" => 80,
        "https" => 443,
        _ => 0,
    };
    if parts.port == default {
        parts.host.clone()
    } else {
        format!("{}:{}", parts.host, parts.port)
    }
}

fn build_http_request_head(
    method: &str,
    target: &str,
    host: &str,
    content_length: Option<usize>,
    headers: &[(String, String)],
) -> Result<String> {
    validate_token("method", method)?;
    validate_field_value("target", target)?;
    validate_field_value("host", host)?;

    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if let Some(length) = content_length {
        head.push_str(&format!("Content-Length: {length}\r\n"));
    }
    for (name, value) in headers {
        validate_token("header name", name)?;
        validate_field_value("header value", value)?;
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    Ok(head)
}

fn validate_token(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value
            .bytes()
            .any(|byte| byte <= b' ' || byte == b':' || byte >= 0x7f)
    {
        return Err(Error::Eval(format!("table/http: invalid {label}")));
    }
    Ok(())
}

fn validate_field_value(label: &str, value: &str) -> Result<()> {
    if value.contains('\r') || value.contains('\n') {
        return Err(Error::Eval(format!("table/http: invalid {label}")));
    }
    Ok(())
}

fn io_error(context: &str, err: io::Error) -> Error {
    Error::HostError(format!("table/http: {context}: {err}"))
}

fn body_limit_error(max_body_bytes: usize) -> Error {
    Error::Eval(format!(
        "table/http: response exceeded max body bytes {max_body_bytes}"
    ))
}
