// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use crate::{
    CommitAction, GuardedAdmission, JournalEntry, JournalError, JournalHead, JournalObject, Lease,
    StoredDatumRef,
};
use sim_kernel::{ContentId, Datum};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A consistent backend snapshot used for verification and replay.
#[derive(Clone, Debug, Default)]
pub struct StoredState {
    pub objects: BTreeMap<ContentId, Vec<u8>>,
    /// Verified semantic values keyed by their kernel Datum identity.
    pub datums: BTreeMap<ContentId, Datum>,
    pub entries: BTreeMap<u64, JournalEntry>,
    pub head: Option<JournalHead>,
}

/// One atomic admission request. Implementations publish immutable objects and
/// compare the head/fence in the same linearized operation.
///
/// Every field is `pub(crate)`: the only place inside this crate that
/// constructs one is [`Journal::verified_admission`](crate::Journal), which
/// runs `verify::verify_state`/`verify::verify_batch` first. A backend
/// receives an `Admission` only as a trait-method parameter and reads it
/// through the accessors below; it cannot build one from scratch. This closes
/// the direct-bypass path where a caller outside this crate could once
/// construct an unverified `Admission` (any fence, any entries, no gapless or
/// predecessor check) and hand it straight to [`JournalBackend::admit`] or
/// [`JournalBackend::admit_then`], skipping every journal-law check that
/// `Journal` exists to enforce.
pub struct Admission {
    pub(crate) fence: u64,
    pub(crate) expected: Option<JournalHead>,
    pub(crate) objects: Vec<JournalObject>,
    pub(crate) entries: Vec<JournalEntry>,
}

impl Admission {
    /// Returns the fence this admission was verified against.
    pub const fn fence(&self) -> u64 {
        self.fence
    }

    /// Returns the head this admission was verified to extend, if any.
    pub const fn expected(&self) -> Option<&JournalHead> {
        self.expected.as_ref()
    }

    /// Returns the immutable objects this admission publishes.
    pub fn objects(&self) -> &[JournalObject] {
        &self.objects
    }

    /// Returns the ordered entry batch this admission publishes.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }
}

/// Object-safe storage seam. A Table backend implements `admit` with canonical
/// `table/cas`; it must refuse writes unless CAS and durability are provable.
pub trait JournalBackend: Send + Sync {
    fn acquire_lease(&self) -> Result<Lease, JournalError>;
    fn read_state(&self) -> Result<StoredState, JournalError>;
    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError>;
    /// Admits a new batch and invokes a short continuation before releasing
    /// exclusion against every head/fence mutation. Exact redelivery does not
    /// invoke the continuation. Unsupported backends refuse before committing.
    fn admit_then(
        &self,
        _admission: Admission,
        _action: &mut dyn CommitAction,
    ) -> Result<GuardedAdmission, JournalError> {
        Err(JournalError::WriteRefused("guarded admission unsupported"))
    }
    /// Durably publishes one immutable semantic object without making it a
    /// journal retention root.
    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError>;
    /// Resolves one semantic object by meaning, returning an owned Datum.
    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError>;
    /// Rebuilds and verifies the derived semantic-to-storage correspondence.
    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError>;
}

impl<T: JournalBackend + ?Sized> JournalBackend for Arc<T> {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        (**self).acquire_lease()
    }
    fn read_state(&self) -> Result<StoredState, JournalError> {
        (**self).read_state()
    }
    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
        (**self).admit(admission)
    }
    fn admit_then(
        &self,
        admission: Admission,
        action: &mut dyn CommitAction,
    ) -> Result<GuardedAdmission, JournalError> {
        (**self).admit_then(admission, action)
    }
    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
        (**self).put_datum(object)
    }
    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
        (**self).get_datum(meaning)
    }
    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
        (**self).rebuild_datum_index()
    }
}
