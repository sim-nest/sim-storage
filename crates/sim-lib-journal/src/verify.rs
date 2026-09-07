use crate::{JournalEntry, JournalHead, JournalObject, StoredState};
use sim_kernel::ContentId;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    pub head: Option<JournalHead>,
    pub entries: Vec<JournalEntry>,
    pub object_ids: BTreeSet<ContentId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum JournalError {
    #[error("journal batch is empty")]
    EmptyBatch,
    #[error("journal head does not match the expected head")]
    WrongHead,
    #[error("writer lease is stale")]
    StaleLease,
    #[error("sequence is not gapless")]
    SequenceGap,
    #[error("entry has the wrong previous id")]
    WrongPrevious,
    #[error("entry identity is not canonical")]
    CorruptEntry,
    #[error("object {0:?} has bytes that do not match its id")]
    CorruptObject(ContentId),
    #[error("payload object {0:?} is missing")]
    MissingPayload(ContentId),
    #[error("content id was redelivered with conflicting bytes")]
    ConflictingObject,
    #[error("datum is not canonical")]
    NonCanonicalDatum,
    #[error("semantic object {0:?} is missing")]
    MissingSemanticObject(ContentId),
    #[error("sequence was redelivered with a conflicting entry")]
    ConflictingDelivery,
    #[error("backend state is corrupt: {0}")]
    CorruptState(&'static str),
    #[error("backend failure: {0}")]
    Backend(String),
    #[error("backend cannot satisfy durable journal writes: {0}")]
    WriteRefused(&'static str),
    #[error("journal verification exceeded its caller-supplied work bound")]
    WorkBoundExceeded,
    #[error("injected crash at {0}")]
    InjectedCrash(&'static str),
}

pub(crate) fn verify_batch(
    state: &StoredState,
    expected: Option<&JournalHead>,
    objects: &[JournalObject],
    entries: &[JournalEntry],
) -> Result<(), JournalError> {
    let mut available: BTreeMap<ContentId, Vec<u8>> = state.objects.clone();
    for object in objects {
        object.verify()?;
        if let Some(bytes) = available.get(&object.id)
            && bytes != &object.bytes
        {
            return Err(JournalError::ConflictingObject);
        }
        available.insert(object.id.clone(), object.bytes.clone());
    }
    let first_sequence = expected.map_or(0, |h| h.sequence + 1);
    let mut previous = expected.map(|h| h.entry.clone());
    for (sequence, entry) in (first_sequence..).zip(entries) {
        if entry.canonical_id()? != entry.id {
            return Err(JournalError::CorruptEntry);
        }
        if entry.sequence != sequence {
            return Err(JournalError::SequenceGap);
        }
        if entry.previous != previous {
            return Err(JournalError::WrongPrevious);
        }
        for payload in &entry.payloads {
            if !available.contains_key(payload) {
                return Err(JournalError::MissingPayload(payload.clone()));
            }
        }
        if let Some(existing) = state.entries.get(&entry.sequence)
            && existing != entry
        {
            return Err(JournalError::ConflictingDelivery);
        }
        previous = Some(entry.id.clone());
    }
    Ok(())
}

pub(crate) fn verify_state(state: &StoredState) -> Result<Verification, JournalError> {
    let mut prior = None;
    let mut object_ids = BTreeSet::new();
    for (expected_sequence, entry) in state.entries.values().enumerate() {
        if entry.sequence != expected_sequence as u64 {
            return Err(JournalError::CorruptState("sequence gap"));
        }
        if entry.previous != prior {
            return Err(JournalError::CorruptState("previous id"));
        }
        if entry.canonical_id()? != entry.id {
            return Err(JournalError::CorruptEntry);
        }
        for payload in &entry.payloads {
            object_ids.insert(payload.clone());
            let bytes = state
                .objects
                .get(payload)
                .ok_or_else(|| JournalError::MissingPayload(payload.clone()))?;
            let datum = state
                .datums
                .get(payload)
                .ok_or_else(|| JournalError::MissingSemanticObject(payload.clone()))?;
            let object = JournalObject::from_datum(datum.clone())?;
            if object.id != *payload || object.bytes != *bytes {
                return Err(JournalError::CorruptObject(payload.clone()));
            }
        }
        prior = Some(entry.id.clone());
    }
    let computed = state.entries.last_key_value().map(|(_, e)| JournalHead {
        sequence: e.sequence,
        entry: e.id.clone(),
    });
    if computed != state.head {
        return Err(JournalError::CorruptState("head"));
    }
    Ok(Verification {
        head: computed,
        entries: state.entries.values().cloned().collect(),
        object_ids,
    })
}
