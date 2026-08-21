#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Portable contracts between SIM's Table/Dir policy and host storage.
//!
//! Paths are represented only as validated relative components. Native paths,
//! handles, synchronization, and error translation belong to platform adapters.

use std::{error::Error, fmt};

/// Result returned by a host-directory adapter.
pub type PortResult<T> = Result<T, HostDirError>;

/// Stable operation-independent storage error categories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostDirErrorKind {
    /// The requested entry does not exist.
    NotFound,
    /// The entry already exists.
    AlreadyExists,
    /// The operation would cross the mounted root.
    Escape,
    /// An entry has an unsupported native kind, such as a device or socket.
    SpecialFile,
    /// Stored bytes or a native name are malformed.
    Malformed,
    /// The configured storage quota would be exceeded.
    QuotaExceeded,
    /// The operation was cancelled before commit.
    Cancelled,
    /// A non-empty directory cannot be removed.
    NotEmpty,
    /// Another native I/O failure occurred.
    Native,
}

/// A sanitized host-storage failure with no native handle or path exposure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostDirError {
    /// Stable category used by portable policy and conformance tests.
    pub kind: HostDirErrorKind,
    /// Adapter-owned diagnostic text.
    pub message: String,
}

impl HostDirError {
    /// Constructs a port error.
    pub fn new(kind: HostDirErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for HostDirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for HostDirError {}

/// Entry kind visible to portable Table/Dir policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostEntryKind {
    /// A regular byte leaf.
    File,
    /// A directory containing more entries.
    Directory,
}

/// One directory entry returned in deterministic name order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostEntry {
    /// Single validated child name.
    pub name: String,
    /// Portable entry kind.
    pub kind: HostEntryKind,
    /// Byte length for files and zero for directories.
    pub len: u64,
}

/// Cancellation probe supplied by the caller of a potentially blocking write.
pub trait Cancellation: Send + Sync {
    /// Returns true when the operation must stop before committing.
    fn is_cancelled(&self) -> bool;
}

/// A cancellation probe that never cancels.
#[derive(Debug, Default)]
pub struct NeverCancel;

impl Cancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Smallest observable host-directory surface needed by Table/Dir policy.
pub trait HostDirPort: Send + Sync {
    /// Human-readable non-native mount identity.
    fn label(&self) -> &str;
    /// Lists one directory in deterministic name order.
    fn list(&self, dir: &[String]) -> PortResult<Vec<HostEntry>>;
    /// Returns metadata for one relative entry, or `None` when absent.
    fn metadata(&self, path: &[String]) -> PortResult<Option<HostEntry>>;
    /// Reads one regular file.
    fn read(&self, path: &[String]) -> PortResult<Vec<u8>>;
    /// Atomically replaces one regular file and durably commits its parent.
    fn replace(&self, path: &[String], bytes: &[u8], cancel: &dyn Cancellation) -> PortResult<()>;
    /// Removes one regular file.
    fn remove_file(&self, path: &[String]) -> PortResult<()>;
    /// Creates one directory; existing directories are accepted.
    fn create_dir(&self, path: &[String]) -> PortResult<()>;
    /// Removes one directory recursively.
    fn remove_dir_all(&self, path: &[String]) -> PortResult<()>;
    /// Opens a child view sharing the same mount and quota.
    fn child(&self, name: &str) -> PortResult<std::sync::Arc<dyn HostDirPort>>;
}
