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
    transport::{HttpRequest, send},
};

/// The HTTP method used for table `set`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HttpWriteMethod {
    /// Write with `PUT`.
    #[default]
    Put,
    /// Write with `POST`.
    Post,
}

impl HttpWriteMethod {
    /// Returns the wire method token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Put => "PUT",
            Self::Post => "POST",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "PUT" => Ok(Self::Put),
            "POST" => Ok(Self::Post),
            other => Err(Error::Eval(format!(
                "table/http: unsupported write method {other}"
            ))),
        }
    }
}

/// Configuration for an [`HttpDir`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpDirOptions {
    /// Base URL whose children are addressed by table keys.
    pub base_url: String,
    /// Codec used to decode response bodies and encode request bodies.
    pub codec: Symbol,
    /// Write method used by [`Table::set`].
    pub write_method: HttpWriteMethod,
    /// Socket read/write timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum response body size in bytes.
    pub max_body_bytes: usize,
}

impl HttpDirOptions {
    /// Builds options for `base_url` with the Lisp codec, `PUT`, a five-second
    /// timeout, and a 1 MiB response body cap.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            codec: Symbol::qualified("codec", "lisp"),
            write_method: HttpWriteMethod::Put,
            timeout_ms: 5_000,
            max_body_bytes: 1024 * 1024,
        }
    }

    /// Returns options using `codec`.
    pub fn with_codec(mut self, codec: Symbol) -> Self {
        self.codec = codec;
        self
    }

    /// Returns options using `write_method` for `set`.
    pub fn with_write_method(mut self, write_method: HttpWriteMethod) -> Self {
        self.write_method = write_method;
        self
    }

    /// Returns options using `timeout_ms`.
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Returns options using `max_body_bytes`.
    pub fn with_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }
}

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

    fn child(&self, key: &Symbol) -> Result<Self> {
        Self::new(HttpDirOptions {
            base_url: self.url_for_key(key)?,
            codec: self.options.codec.clone(),
            write_method: self.options.write_method,
            timeout_ms: self.options.timeout_ms,
            max_body_bytes: self.options.max_body_bytes,
        })
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
        Ok(format!("{}/{segment}", self.options.base_url))
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
        cx.factory().opaque(Arc::new(self.child(&name)?))
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        require_table_http(cx)?;
        Ok(Some(cx.factory().opaque(Arc::new(self.child(&name)?))?))
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        self.del(cx, name)
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        require_table_http(cx)?;
        let _ = self.url_for_key(&name)?;
        Ok(true)
    }
}

/// Creates an HTTP directory value from `options`.
pub fn install_http_dir_lib(cx: &mut Cx, options: HttpDirOptions) -> Result<Value> {
    cx.factory().opaque(Arc::new(HttpDir::new(options)?))
}

fn validate_options(options: &HttpDirOptions) -> Result<()> {
    if options.timeout_ms == 0 {
        return Err(Error::Eval(
            "table/http: timeout_ms must be non-zero".to_owned(),
        ));
    }
    if options.base_url.trim().is_empty() {
        return Err(Error::Eval("table/http: base_url is empty".to_owned()));
    }
    let _ = sim_lib_net_core::parse_url(options.base_url.trim())
        .map_err(|err| Error::Eval(format!("table/http: {err}")))?;
    Ok(())
}

fn normalize_options(mut options: HttpDirOptions) -> HttpDirOptions {
    options.base_url = options.base_url.trim().trim_end_matches('/').to_owned();
    options
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

impl TryFrom<crate::HttpDirDescriptor> for HttpDirOptions {
    type Error = Error;

    fn try_from(value: crate::HttpDirDescriptor) -> Result<Self> {
        Ok(Self {
            base_url: value.base_url,
            codec: value.codec,
            write_method: HttpWriteMethod::from_str(&value.write_method)?,
            timeout_ms: value.timeout_ms,
            max_body_bytes: value.max_body_bytes,
        })
    }
}
