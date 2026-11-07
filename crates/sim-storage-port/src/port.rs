// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Host-directory port contract.

use std::sync::Arc;

use crate::{
    Cancellation, CommitAction, HostCompareExchange, HostDirError, HostDirErrorKind, HostEntry,
    PortResult,
};

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
    /// truncated into a successful result. An adapter may observe one entry
    /// beyond the bound (even at `max_entries == 0`) to detect excess, but must
    /// retain at most `max_entries` entries and stop without enumerating the
    /// remainder. It must not materialize the whole directory (for example by
    /// calling [`Self::list`]) before checking it.
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
    /// Reads one regular file without ever returning or allocating more than
    /// `max_bytes`.
    ///
    /// Contract for implementors: a file longer than `max_bytes`, including one
    /// that grows during the read, fails with [`HostDirErrorKind::QuotaExceeded`];
    /// it is never truncated into a successful result, and the adapter must stop
    /// reading at `max_bytes + 1` rather than materializing the whole file. The
    /// default refuses, so a caller needing a real bound fails closed instead of
    /// falling back to [`Self::read`].
    fn read_bounded(&self, _path: &[String], _max_bytes: usize) -> PortResult<Vec<u8>> {
        Err(HostDirError::new(
            HostDirErrorKind::Unsupported,
            "bounded file read unsupported",
        ))
    }
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
    fn child(&self, name: &str) -> PortResult<Arc<dyn HostDirPort>>;
}

#[cfg(test)]
mod tests {
    use super::HostDirPort;
    use crate::{HostDirError, HostDirErrorKind, PortResult};

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
        fn list(&self, _dir: &[String]) -> PortResult<Vec<crate::HostEntry>> {
            Ok(Vec::new())
        }
        fn metadata(&self, _path: &[String]) -> PortResult<Option<crate::HostEntry>> {
            refused()
        }
        fn read(&self, _path: &[String]) -> PortResult<Vec<u8>> {
            refused()
        }
        fn replace(
            &self,
            _path: &[String],
            _bytes: &[u8],
            _cancel: &dyn crate::Cancellation,
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
        let err = port.read_bounded(&[], 1).expect_err("default must refuse");
        assert_eq!(err.kind, HostDirErrorKind::Unsupported);
    }
}
