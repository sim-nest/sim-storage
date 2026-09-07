use std::collections::BTreeMap;

use sim_kernel::{ContentId, Datum};

use crate::{JournalEntry, JournalHead, StoredState, Verification, verify::verify_state};

/// One internally consistent, fully verified semantic view of a journal read.
///
/// The snapshot contains only logical entries and the canonical Datums they
/// retain. Physical bytes, storage locators, and backend state stay private to
/// the journal implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSnapshot {
    head: Option<JournalHead>,
    entries: Vec<JournalEntry>,
    datums: BTreeMap<ContentId, Datum>,
}

impl VerifiedSnapshot {
    pub(crate) fn from_state(state: StoredState) -> Result<Self, crate::JournalError> {
        let Verification {
            head,
            entries,
            object_ids,
        } = verify_state(&state)?;
        let datums = object_ids
            .into_iter()
            .map(|id| {
                let datum = state
                    .datums
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| crate::JournalError::MissingSemanticObject(id.clone()))?;
                Ok((id, datum))
            })
            .collect::<Result<_, crate::JournalError>>()?;
        Ok(Self {
            head,
            entries,
            datums,
        })
    }

    /// Returns the verified head from the same backend read as the entries.
    pub const fn head(&self) -> Option<&JournalHead> {
        self.head.as_ref()
    }

    /// Returns the complete verified entry chain in sequence order.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Resolves one retained semantic object from this exact snapshot.
    pub fn datum(&self, id: &ContentId) -> Option<&Datum> {
        self.datums.get(id)
    }

    /// Returns every retained semantic object keyed by its canonical identity.
    pub const fn datums(&self) -> &BTreeMap<ContentId, Datum> {
        &self.datums
    }
}
