//! Citizen descriptor for the HTTP directory class.

use sim_citizen_derive::Citizen;
use sim_kernel::Symbol;

/// Serialized configuration for an [`HttpDir`](crate::HttpDir).
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "table/HttpDir", version = 0)]
pub struct HttpDirDescriptor {
    /// Base URL whose children are addressed by table keys.
    pub base_url: String,
    /// Codec used to decode response bodies and encode request bodies.
    pub codec: Symbol,
    /// Write method used by `set`, either `PUT` or `POST`.
    pub write_method: String,
    /// Socket read/write timeout in milliseconds.
    pub timeout_ms: u64,
    /// Maximum response body size in bytes.
    pub max_body_bytes: usize,
}

impl Default for HttpDirDescriptor {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            codec: Symbol::qualified("codec", "lisp"),
            write_method: "PUT".to_owned(),
            timeout_ms: 5_000,
            max_body_bytes: 1024 * 1024,
        }
    }
}

/// The fully qualified class symbol (`table/HttpDir`) for the HTTP directory.
pub fn http_dir_class_symbol() -> Symbol {
    Symbol::qualified("table", "HttpDir")
}
