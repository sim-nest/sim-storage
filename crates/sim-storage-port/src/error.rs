// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sanitized host-storage failures.

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
