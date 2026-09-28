// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Verified admission and post-commit continuation policy.

use crate::{
    Admission, Journal, JournalBackend, JournalEntry, JournalError, JournalHead, JournalObject,
    Lease,
};
pub use sim_storage_port::CommitAction;

/// Whether this admission invoked its post-commit continuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionDisposition {
    /// A new batch committed and its continuation was invoked under exclusion.
    CommittedActionInvoked,
    /// The exact batch is committed; no continuation was invoked by this call.
    AlreadyCommittedActionNotInvoked,
}

/// A committed head and this call's continuation disposition.
///
/// This is not proof that a domain-specific external effect succeeded. The
/// action's caller owns that result and reconciliation of uncertain effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardedAdmission {
    /// Current committed head returned by the admission owner.
    pub head: JournalHead,
    /// Whether this call invoked the continuation.
    pub disposition: AdmissionDisposition,
}

impl<B: JournalBackend> Journal<B> {
    /// Verifies and admits a batch, invoking a short action under the same fence.
    ///
    /// Objects become durable before the head commit. The action runs after a
    /// new head commit while competing head/fence mutations remain excluded.
    /// Exact redelivery never invokes it again. Reads may be reentrant; the
    /// action must not mutate this journal or wait for payload completion.
    /// Any error after commit requires reconciliation, not external-effect replay.
    pub fn publish_then(
        &self,
        lease: &Lease,
        expected: Option<&JournalHead>,
        objects: Vec<JournalObject>,
        entries: Vec<JournalEntry>,
        action: &mut dyn CommitAction,
    ) -> Result<GuardedAdmission, JournalError> {
        let admission = self.verified_admission(lease, expected, objects, entries)?;
        self.backend.admit_then(admission, action)
    }

    pub(crate) fn verified_admission(
        &self,
        lease: &Lease,
        expected: Option<&JournalHead>,
        objects: Vec<JournalObject>,
        entries: Vec<JournalEntry>,
    ) -> Result<Admission, JournalError> {
        if entries.is_empty() {
            return Err(JournalError::EmptyBatch);
        }
        let before = self.backend.read_state()?;
        crate::verify::verify_state(&before)?;
        crate::verify::verify_batch(&before, expected, &objects, &entries)?;
        Ok(Admission {
            fence: lease.fence,
            expected: expected.cloned(),
            objects,
            entries,
        })
    }
}
