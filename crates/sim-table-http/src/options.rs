//! HTTP directory option types.

use sim_kernel::{Error, Result, Symbol};

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

/// Configuration for an [`HttpDir`](crate::HttpDir).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpDirOptions {
    /// Base URL whose children are addressed by table keys.
    pub base_url: String,
    /// Codec used to decode response bodies and encode request bodies.
    pub codec: Symbol,
    /// Write method used by [`Table::set`](sim_kernel::Table::set).
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

pub(crate) fn validate_options(options: &HttpDirOptions) -> Result<()> {
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

pub(crate) fn normalize_options(mut options: HttpDirOptions) -> HttpDirOptions {
    options.base_url = options.base_url.trim().trim_end_matches('/').to_owned();
    options
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
