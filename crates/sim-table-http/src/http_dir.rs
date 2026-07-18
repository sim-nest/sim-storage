//! The [`HttpDir`] object: a capability-gated HTTP-backed table directory.

use std::{sync::Arc, time::Duration};

use sim_citizen::CitizenField;
use sim_codec::{Input, Output, decode_with_codec, encode_with_codec};
use sim_kernel::{
    Cx, EncodeOptions, Error, Expr, Object, ObjectEncode, ObjectEncoding, ReadPolicy, Result,
    Symbol, Value,
    id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
    table::{Dir, Table},
};

use crate::{
    capabilities::require_table_http,
    citizen::http_dir_class_symbol,
    options::{HttpDirOptions, normalize_options, validate_options},
    transport::{HttpRequest, send},
};

/// A table directory backed by direct HTTP resources.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpDir {
    options: HttpDirOptions,
}

impl HttpDir {
    /// Builds an HTTP directory from `options`.
    pub fn new(options: HttpDirOptions) -> Result<Self> {
        validate_options(&options)?;
        Ok(Self {
            options: normalize_options(options),
        })
    }

    /// Returns this directory's options.
    pub fn options(&self) -> &HttpDirOptions {
        &self.options
    }

    fn request(&self, method: &'static str, url: String, body: Vec<u8>) -> HttpRequest {
        HttpRequest {
            method,
            url,
            headers: Vec::new(),
            body,
            timeout: Duration::from_millis(self.options.timeout_ms),
            max_body_bytes: self.options.max_body_bytes,
        }
    }

    fn url_for_key(&self, key: &Symbol) -> Result<String> {
        let segment = key.name.as_ref();
        if !sim_table_core::is_legal_table_segment(segment) {
            return Err(Error::Eval(format!("table/http: illegal name {segment:?}")));
        }
        let encoded = encode_path_segment(segment);
        Ok(format!("{}/{}", self.options.base_url, encoded))
    }

    fn decode_body(&self, cx: &mut Cx, body: Vec<u8>) -> Result<Value> {
        let expr = decode_with_codec(
            cx,
            &self.options.codec,
            Input::Bytes(body),
            ReadPolicy::default(),
        )?;
        cx.factory().expr(expr)
    }

    fn encode_value(&self, cx: &mut Cx, value: Value) -> Result<Vec<u8>> {
        let expr = value.object().as_expr(cx)?;
        match encode_with_codec(cx, &self.options.codec, &expr, EncodeOptions::default())? {
            Output::Text(text) => Ok(text.into_bytes()),
            Output::Bytes(bytes) => Ok(bytes),
        }
    }
}

impl Object for HttpDir {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("table/http[{}]", self.options.base_url))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for HttpDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = http_dir_class_symbol();
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        let symbol = Symbol::qualified("core", "Table");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol)
    }

    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }

    fn truth(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(true)
    }

    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }

    fn as_dir(&self) -> Option<&dyn Dir> {
        Some(self)
    }

    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for HttpDir {
    fn object_encoding(&self, _cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: http_dir_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                self.options.base_url.encode_field(),
                self.options.codec.encode_field(),
                self.options.write_method.as_str().to_owned().encode_field(),
                self.options.timeout_ms.encode_field(),
                self.options.max_body_bytes.encode_field(),
            ],
        })
    }
}

impl sim_citizen::Citizen for HttpDir {
    fn citizen_symbol() -> Symbol {
        http_dir_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        5
    }

    fn citizen_fields() -> &'static [&'static str] {
        &[
            "base_url",
            "codec",
            "write_method",
            "timeout_ms",
            "max_body_bytes",
        ]
    }
}

impl Table for HttpDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "http")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        require_table_http(cx)?;
        let response = send(self.request("GET", self.url_for_key(&key)?, Vec::new()))?;
        ensure_success(response.status, response.reason.as_deref(), &response.body)?;
        self.decode_body(cx, response.body)
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        require_table_http(cx)?;
        let body = self.encode_value(cx, value)?;
        let response = send(self.request(
            self.options.write_method.as_str(),
            self.url_for_key(&key)?,
            body,
        ))?;
        ensure_success(response.status, response.reason.as_deref(), &response.body)?;
        Ok(())
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        require_table_http(cx)?;
        let response = send(self.request("HEAD", self.url_for_key(&key)?, Vec::new()))?;
        match response.status {
            status if (200..300).contains(&status) => Ok(true),
            404 => Ok(false),
            status => Err(status_error(
                status,
                response.reason.as_deref(),
                &response.body,
            )),
        }
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        require_table_http(cx)?;
        let response = send(self.request("DELETE", self.url_for_key(&key)?, Vec::new()))?;
        match response.status {
            status if (200..300).contains(&status) || status == 404 => cx.factory().nil(),
            status => Err(status_error(
                status,
                response.reason.as_deref(),
                &response.body,
            )),
        }
    }

    fn keys(&self, _cx: &mut Cx) -> Result<Vec<Symbol>> {
        Err(Error::Eval(
            "table/http: keys are not available without an index resource".to_owned(),
        ))
    }

    fn entries(&self, _cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        Err(Error::Eval(
            "table/http: entries are not available without an index resource".to_owned(),
        ))
    }

    fn len(&self, _cx: &mut Cx) -> Result<usize> {
        Err(Error::Eval(
            "table/http: len is not available without an index resource".to_owned(),
        ))
    }

    fn clear(&self, _cx: &mut Cx) -> Result<()> {
        Err(Error::Eval(
            "table/http: clear is not available without an index resource".to_owned(),
        ))
    }
}

impl Dir for HttpDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        require_table_http(cx)?;
        let _ = self.url_for_key(&name)?;
        Err(index_resource_error("mkdir"))
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        require_table_http(cx)?;
        let _ = self.url_for_key(&name)?;
        Err(index_resource_error("opendir"))
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        require_table_http(cx)?;
        let _ = self.url_for_key(&name)?;
        Err(index_resource_error("rmdir"))
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        require_table_http(cx)?;
        let _ = self.url_for_key(&name)?;
        Err(index_resource_error("is_dir"))
    }
}

/// Creates an HTTP directory value from `options`.
pub fn install_http_dir_lib(cx: &mut Cx, options: HttpDirOptions) -> Result<Value> {
    cx.factory().opaque(Arc::new(HttpDir::new(options)?))
}

fn encode_path_segment(segment: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(segment.len());
    for &byte in segment.as_bytes() {
        if is_unreserved_path_byte(byte) {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

fn is_unreserved_path_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~'
    )
}

fn index_resource_error(operation: &str) -> Error {
    Error::Eval(format!(
        "table/http: {operation} requires an index resource"
    ))
}

fn ensure_success(status: u16, reason: Option<&str>, body: &[u8]) -> Result<()> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(status_error(status, reason, body))
    }
}

fn status_error(status: u16, reason: Option<&str>, body: &[u8]) -> Error {
    let reason = reason.unwrap_or_default();
    let body = String::from_utf8_lossy(body);
    let detail = if body.is_empty() {
        reason.to_owned()
    } else if reason.is_empty() {
        body.into_owned()
    } else {
        format!("{reason}: {body}")
    };
    Error::HostError(format!("table/http: http {status}: {detail}"))
}
