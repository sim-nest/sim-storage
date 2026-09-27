// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Borrowed continuation for an adapter-owned commit boundary.

/// A short action invoked after commit, while competing commits remain excluded.
///
/// The action runs synchronously on the admitting thread. It must not acquire
/// resources, wait for payload work, or mutate the same admission domain. Reads
/// are permitted. Its domain-specific result belongs to the caller; returning
/// from this method is not an acknowledgement of any external effect.
///
/// A native effect must be resolved locally under this boundary. Queueing an
/// asynchronous request and returning does not extend the exclusion to that
/// request. Failure or caller loss after commit requires reconciliation, not
/// unconditional repetition of the action.
pub trait CommitAction {
    /// Acts once after this invocation's new commit, before releasing exclusion.
    fn after_commit(&mut self);
}

impl<F: FnMut()> CommitAction for F {
    fn after_commit(&mut self) {
        self();
    }
}
