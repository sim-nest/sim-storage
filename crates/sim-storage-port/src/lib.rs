// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Portable contracts between SIM's Table/Dir policy and host storage.
//!
//! Paths are represented only as validated relative components. Native paths,
//! handles, synchronization, and error translation belong to platform adapters.

use std::{error::Error, fmt};

mod commit;
pub use commit::CommitAction;
mod observation;
pub use observation::HostDirObservation;

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
    /// A configured storage quota or caller-supplied enumeration bound would be
    /// exceeded.
    QuotaExceeded,
    /// The operation was cancelled before commit.
    Cancelled,
    /// A non-empty directory cannot be removed.
    NotEmpty,
    /// Another native I/O failure occurred.
    Native,
    /// The adapter cannot provide the requested contract or capability.
    Unsupported,
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

/// Outcome of a host-byte compare-exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCompareExchange {
    /// Whether the replacement was published.
    pub exchanged: bool,
    /// Bytes observed at the linearization point, or `None` when absent.
    pub observed: Option<Vec<u8>>,
}

/// Smallest observable host-directory surface needed by Table/Dir policy.
pub trait HostDirPort: Send + Sync {
    /// Human-readable non-native mount identity.
    fn label(&self) -> &str;
    /// Lists one directory in deterministic name order.
    fn list(&self, dir: &[String]) -> PortResult<Vec<HostEntry>>;
    /// Lists one directory in deterministic name order within an explicit
    /// member bound.
    ///
    /// Contract for implementors: `Ok` is the COMPLETE directory with
    /// `len() <= max_entries`. A directory with more than `max_entries`
    /// members fails with [`HostDirErrorKind::QuotaExceeded`]; it is never
    /// truncated into a successful result. Enumeration and allocation must stop
    /// at the bound, so an adapter must not materialize the whole directory
    /// (for example by calling [`Self::list`]) before checking it.
    ///
    /// A type implementing both this trait and [`crate::HostDirObservation`]
    /// shares the method name; call it with explicit trait syntax.
    ///
    /// The default refuses so callers needing a real bound fail closed
    /// instead of silently falling back to [`Self::list`]'s unbounded
    /// enumeration. A caller bounding recursive traversal (e.g. a depth/
    /// directory-count guard around repeated [`Self::list`] calls) MUST use
    /// this instead of [`Self::list`] once an adapter supports it: unlike
    /// [`Self::list`], which some native adapters implement by fully
    /// materializing and sorting an entire directory before returning,
    /// `list_bounded` lets the adapter itself stop enumeration at the
    /// storage boundary -- a local post-hoc count check on an already-fully-
    /// materialized `Vec` cannot bound the underlying enumeration cost.
    /// Mirrors [`crate::HostDirObservation::list_bounded`]'s exact same
    /// shape on the sibling read-only-view trait.
    fn list_bounded(&self, _dir: &[String], _max_entries: usize) -> PortResult<Vec<HostEntry>> {
        Err(HostDirError::new(
            HostDirErrorKind::Unsupported,
            "bounded directory listing unsupported",
        ))
    }
    /// Returns metadata for one relative entry, or `None` when absent.
    fn metadata(&self, path: &[String]) -> PortResult<Option<HostEntry>>;
    /// Reads one regular file.
    fn read(&self, path: &[String]) -> PortResult<Vec<u8>>;
    /// Atomically replaces one regular file and durably commits its parent.
    fn replace(&self, path: &[String], bytes: &[u8], cancel: &dyn Cancellation) -> PortResult<()>;
    /// Atomically replaces or deletes a leaf iff its bytes equal `expected`.
    fn compare_exchange(
        &self,
        _path: &[String],
        _expected: Option<&[u8]>,
        _replacement: Option<&[u8]>,
        _cancel: &dyn Cancellation,
    ) -> PortResult<HostCompareExchange> {
        Err(HostDirError::new(
            HostDirErrorKind::Unsupported,
            "host directory compare-exchange unsupported",
        ))
    }
    /// Whether this adapter implements the guarded compare-exchange contract.
    fn supports_guarded_compare_exchange(&self) -> bool {
        false
    }
    /// Commits a matching exchange, then invokes `action` before releasing
    /// exclusion against ordinary and guarded exchanges on the same leaf.
    ///
    /// Native adapters invoke the action only after replacement and containing
    /// directory durability. Modeled adapters supply ordering, not durability.
    /// A comparison miss or pre-commit error never invokes the action. Errors
    /// after commit can leave the action's outcome uncertain. Callers must not
    /// infer non-execution from an error or retry an uncertain external effect.
    /// Unsupported adapters refuse without modifying the leaf or invoking the
    /// action; composing ordinary compare-exchange with a later action is invalid.
    fn compare_exchange_then(
        &self,
        _path: &[String],
        _expected: Option<&[u8]>,
        _replacement: Option<&[u8]>,
        _cancel: &dyn Cancellation,
        _action: &mut dyn CommitAction,
    ) -> PortResult<HostCompareExchange> {
        Err(HostDirError::new(
            HostDirErrorKind::Unsupported,
            "guarded host directory compare-exchange unsupported",
        ))
    }
    /// Removes one regular file.
    fn remove_file(&self, path: &[String]) -> PortResult<()>;
    /// Creates one directory; existing directories are accepted.
    fn create_dir(&self, path: &[String]) -> PortResult<()>;
    /// Removes one directory recursively.
    fn remove_dir_all(&self, path: &[String]) -> PortResult<()>;
    /// Opens a child view sharing the same mount and quota.
    fn child(&self, name: &str) -> PortResult<std::sync::Arc<dyn HostDirPort>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adapter that implements only the required methods, so every defaulted
    /// method is the trait's own.
    struct Bare;

    fn refused<T>() -> PortResult<T> {
        Err(HostDirError::new(HostDirErrorKind::Unsupported, "bare"))
    }

    impl HostDirPort for Bare {
        fn label(&self) -> &str {
            "bare"
        }
        fn list(&self, _dir: &[String]) -> PortResult<Vec<HostEntry>> {
            Ok(Vec::new())
        }
        fn metadata(&self, _path: &[String]) -> PortResult<Option<HostEntry>> {
            refused()
        }
        fn read(&self, _path: &[String]) -> PortResult<Vec<u8>> {
            refused()
        }
        fn replace(
            &self,
            _path: &[String],
            _bytes: &[u8],
            _cancel: &dyn Cancellation,
        ) -> PortResult<()> {
            refused()
        }
        fn remove_file(&self, _path: &[String]) -> PortResult<()> {
            refused()
        }
        fn create_dir(&self, _path: &[String]) -> PortResult<()> {
            refused()
        }
        fn remove_dir_all(&self, _path: &[String]) -> PortResult<()> {
            refused()
        }
        fn child(&self, _name: &str) -> PortResult<std::sync::Arc<dyn HostDirPort>> {
            refused()
        }
    }

    #[test]
    fn default_list_bounded_fails_closed_instead_of_falling_back_to_list() {
        let port = Bare;
        // `list` succeeds on this adapter, so a silent fallback would return Ok.
        assert!(port.list(&[]).is_ok());
        let err = port.list_bounded(&[], 1).expect_err("default must refuse");
        assert_eq!(err.kind, HostDirErrorKind::Unsupported);
    }
}
