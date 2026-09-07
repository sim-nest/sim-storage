use crate::{
    Admission, JournalBackend, JournalError, JournalHead, JournalObject, Lease, StoredDatumRef,
    StoredState,
};
use sim_kernel::{ContentId, Datum};
use std::sync::{Mutex, PoisonError};

#[derive(Default)]
struct Inner {
    fence: u64,
    state: StoredState,
}

/// Deterministic law-reference backend. It is deliberately not durable.
#[derive(Default)]
pub struct MemoryBackend {
    inner: Mutex<Inner>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl JournalBackend for MemoryBackend {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?;
        inner.fence = inner
            .fence
            .checked_add(1)
            .ok_or_else(|| JournalError::Backend("fence exhausted".into()))?;
        Ok(Lease { fence: inner.fence })
    }
    fn read_state(&self) -> Result<StoredState, JournalError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?
            .state
            .clone())
    }
    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?;
        if admission.fence != inner.fence {
            return Err(JournalError::StaleLease);
        }
        if admission.expected != inner.state.head {
            // A caller may retry an exactly committed batch after losing its
            // acknowledgement. This is the sole stale-head exception.
            let exact = admission
                .entries
                .iter()
                .all(|entry| inner.state.entries.get(&entry.sequence) == Some(entry))
                && !admission.entries.is_empty();
            if exact {
                return inner.state.head.clone().ok_or(JournalError::WrongHead);
            }
            if admission
                .entries
                .iter()
                .any(|entry| inner.state.entries.contains_key(&entry.sequence))
            {
                return Err(JournalError::ConflictingDelivery);
            }
            return Err(JournalError::WrongHead);
        }
        for object in admission.objects {
            match inner.state.objects.get(&object.id) {
                Some(bytes) if bytes != &object.bytes => {
                    return Err(JournalError::ConflictingObject);
                }
                Some(_) => {}
                None => {
                    inner
                        .state
                        .datums
                        .insert(object.id.clone(), object.datum().clone());
                    inner.state.objects.insert(object.id, object.bytes);
                }
            }
        }
        for entry in admission.entries {
            match inner.state.entries.get(&entry.sequence) {
                Some(existing) if existing == &entry => {}
                Some(_) => return Err(JournalError::ConflictingDelivery),
                None => {
                    inner.state.entries.insert(entry.sequence, entry);
                }
            }
        }
        let entry = inner
            .state
            .entries
            .last_key_value()
            .ok_or(JournalError::EmptyBatch)?
            .1;
        let head = JournalHead {
            sequence: entry.sequence,
            entry: entry.id.clone(),
        };
        inner.state.head = Some(head.clone());
        Ok(head)
    }

    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
        object.verify()?;
        let storage_bytes = object.storage_bytes()?;
        let reference = StoredDatumRef {
            meaning: object.id.clone(),
            storage: crate::object::storage_id(&storage_bytes),
        };
        let mut inner = self
            .inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?;
        match inner.state.datums.get(&object.id) {
            Some(value) if value != object.datum() => return Err(JournalError::ConflictingObject),
            _ => {
                let datum = object.datum().clone();
                inner.state.objects.insert(object.id.clone(), object.bytes);
                inner.state.datums.insert(object.id, datum);
            }
        }
        Ok(reference)
    }

    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
        self.inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?
            .state
            .datums
            .get(meaning)
            .cloned()
            .ok_or_else(|| JournalError::MissingSemanticObject(meaning.clone()))
    }

    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_: PoisonError<_>| JournalError::Backend("memory lock poisoned".into()))?;
        inner
            .state
            .datums
            .iter()
            .map(|(meaning, datum)| {
                let object = JournalObject::from_datum(datum.clone())?;
                let bytes = object.storage_bytes()?;
                Ok(StoredDatumRef {
                    meaning: meaning.clone(),
                    storage: crate::object::storage_id(&bytes),
                })
            })
            .collect()
    }
}
