// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Portable directory entries.

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
