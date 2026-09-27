// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Opt-in conformance cuts over the original native journal publication.

use super::{Admission, Arc, Failpoint, HostDirJournalBackend, JournalError};

pub(super) type FailHook = Arc<dyn Fn(Failpoint) -> bool + Send + Sync>;
pub(super) type AdmissionFailHook = Arc<dyn Fn(Failpoint, &Admission) -> bool + Send + Sync>;

impl HostDirJournalBackend {
    /// Installs a deterministic crash hook for conformance models.
    ///
    /// Test-only. This crate's own conformance test suite is the only
    /// intended caller: chaos-testing a real deployment belongs to a
    /// purpose-built fault-injection layer outside the journal, not a debug
    /// hook left reachable in core. `#[cfg(test)]` keeps this out of every
    /// non-test build entirely, so no production caller can ever install one
    /// -- this is stronger than a runtime check or an off-by-default feature,
    /// neither of which stop the hook from being reachable in a release
    /// binary at all.
    #[cfg(test)]
    pub fn with_failpoint_hook(
        mut self,
        hook: impl Fn(Failpoint) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.fail = Some(Arc::new(hook));
        self
    }

    /// Installs an additional conformance cut over the actual borrowed admission.
    ///
    /// Called at the existing publication boundaries: before/after object
    /// publication, after durability receipts, before/after the head CAS, and
    /// before acknowledgement. Returning true produces the existing
    /// `JournalError::InjectedCrash` for that boundary; false preserves the
    /// original path. The callback can match exact entry kinds, objects, expected
    /// head and fence instead of counting unrelated publications. It cannot
    /// replace or mutate the admission. One-shot selection belongs to the
    /// callback; the backend does not silently disarm a persistent hook.
    ///
    /// The existing `with_failpoint_hook` runs first. If it refuses, this callback
    /// is not called at that boundary. Neither lease/migration work nor committed
    /// redelivery confirmation invokes this admission hook. In particular, a
    /// missed target cannot be inferred to have fired from successful redelivery.
    ///
    /// Callbacks must be bounded, nonblocking and must not reenter journal/port
    /// operations. Guarded `AfterCas` runs while the original CAS exclusion is
    /// held, BEFORE its native action; refusing there commits DATA but suppresses
    /// that action. `BeforeAcknowledgement` can run AFTER the action and must
    /// never imply that the action did not run or may be repeated. This is test
    /// instrumentation, not native effect, durability or qualification evidence.
    ///
    /// Test-only, for the same reason as [`with_failpoint_hook`](Self::with_failpoint_hook):
    /// `#[cfg(test)]` removes it from every non-test build, so an `AfterCas`
    /// refusal that commits data while suppressing the continuation can never
    /// happen outside this crate's own conformance tests.
    #[cfg(test)]
    pub fn with_admission_failpoint_hook(
        mut self,
        hook: impl Fn(Failpoint, &Admission) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.admission_fail = Some(Arc::new(hook));
        self
    }

    pub(super) fn trip(&self, point: Failpoint) -> Result<(), JournalError> {
        if self.fail.as_ref().is_some_and(|hook| hook(point)) {
            Err(JournalError::InjectedCrash(point.label()))
        } else {
            Ok(())
        }
    }

    pub(super) fn trip_admission(
        &self,
        point: Failpoint,
        admission: &Admission,
    ) -> Result<(), JournalError> {
        self.trip(point)?;
        if self
            .admission_fail
            .as_ref()
            .is_some_and(|hook| hook(point, admission))
        {
            Err(JournalError::InjectedCrash(point.label()))
        } else {
            Ok(())
        }
    }
}
