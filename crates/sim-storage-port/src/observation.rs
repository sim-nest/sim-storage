// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Bounded read-only observations through a retained directory object.

use crate::{HostEntry, PortResult};

/// A read-only view pinned to a selected directory, not a pathname locator.
///
/// The resource owner retains exclusion and establishes writer quiescence around
/// all calls. This view neither grants that authority nor establishes a snapshot.
/// Relative components cannot traverse links or cross subordinate mounts. Errors
/// and unsupported native objects are not evidence of absence or empty storage.
pub trait HostDirObservation: Send + Sync {
    /// Lists one directory in deterministic order within an explicit member bound.
    ///
    /// The default refuses because metadata/read access alone does not prove a
    /// complete tree. Native implementations must retain the same directory
    /// object across enumeration and later reads.
    fn list_bounded(&self, _path: &[String], _max_entries: usize) -> PortResult<Vec<HostEntry>> {
        Err(crate::HostDirError::new(
            crate::HostDirErrorKind::Unsupported,
            "bounded directory observation is unavailable",
        ))
    }

    /// Observes a regular file or directory, refusing links and special objects.
    /// Returns `None` only for an absent entry beneath the retained root.
    fn metadata(&self, path: &[String]) -> PortResult<Option<HostEntry>>;

    /// Reads a regular file without allocating or returning more than `max_bytes`.
    /// Refuses oversized, changing or special files instead of truncating success.
    fn read_bounded(&self, path: &[String], max_bytes: usize) -> PortResult<Vec<u8>>;

    /// Checks directory emptiness without collecting its complete entry list.
    /// The empty component list denotes the retained root itself.
    fn is_empty(&self, path: &[String]) -> PortResult<bool>;
}
