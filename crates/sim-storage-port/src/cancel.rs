// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Caller cancellation probes.

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
