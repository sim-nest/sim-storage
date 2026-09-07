use sim_kernel::{ContentId, Datum};
use thiserror::Error;

use crate::{JournalBackend, JournalError, JournalObject};

/// A verified crossing from semantic Datum identity to physical byte identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredDatumRef {
    /// Kernel Datum identity used by callers and journal entries.
    pub meaning: ContentId,
    /// Exact-byte identity used only by the storage adapter.
    pub storage: ContentId,
}

/// Failure of the owned-return persistent semantic object facet.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum StoreError {
    /// The journal object backend refused or could not verify the operation.
    #[error(transparent)]
    Journal(#[from] JournalError),
}

/// Owned-return persistent Datum operations over the journal object backend.
pub trait PersistentSemanticObjects {
    /// Stores a canonical value idempotently and returns both identities.
    fn put(&mut self, value: Datum) -> Result<StoredDatumRef, StoreError>;
    /// Resolves a value by semantic identity and verifies that identity again.
    fn get(&self, meaning: &ContentId) -> Result<Datum, StoreError>;
    /// Rebuilds and verifies the derived semantic-to-storage index.
    fn rebuild_index(&mut self) -> Result<Vec<StoredDatumRef>, StoreError>;
}

/// Persistent semantic object facet backed by an existing journal backend.
pub struct PersistentObjectStore<B> {
    backend: B,
}

impl<B: JournalBackend> PersistentObjectStore<B> {
    /// Opens the facet and verifies its rebuildable correspondence index.
    pub fn open(backend: B) -> Result<Self, StoreError> {
        backend.rebuild_datum_index()?;
        Ok(Self { backend })
    }

    /// Returns the wrapped backend.
    pub fn into_inner(self) -> B {
        self.backend
    }
}

impl<B: JournalBackend> PersistentSemanticObjects for PersistentObjectStore<B> {
    fn put(&mut self, value: Datum) -> Result<StoredDatumRef, StoreError> {
        Ok(self.backend.put_datum(JournalObject::from_datum(value)?)?)
    }

    fn get(&self, meaning: &ContentId) -> Result<Datum, StoreError> {
        let value = self.backend.get_datum(meaning)?;
        if value
            .content_id()
            .map_err(|_| JournalError::NonCanonicalDatum)?
            != *meaning
        {
            return Err(JournalError::CorruptObject(meaning.clone()).into());
        }
        Ok(value)
    }

    fn rebuild_index(&mut self) -> Result<Vec<StoredDatumRef>, StoreError> {
        Ok(self.backend.rebuild_datum_index()?)
    }
}
