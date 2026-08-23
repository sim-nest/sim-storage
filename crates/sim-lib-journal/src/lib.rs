//! Domain-free atomic journal behavior over content-addressed objects.
//!
//! A journal publishes immutable objects before making entries visible and
//! advances its head through one fenced compare-and-swap transition. Backends
//! implement [`JournalBackend::admit`]; callers use [`Journal`] so the gapless,
//! closure, redelivery, and replay laws are enforced once.

mod backend;
mod entry;
mod head;
mod lease;
mod memory;
mod object;
mod projection;
mod replay;
mod verify;

pub use backend::{Admission, JournalBackend, StoredState};
pub use entry::JournalEntry;
pub use head::JournalHead;
pub use lease::Lease;
pub use memory::MemoryBackend;
pub use object::JournalObject;
pub use projection::{DirProjection, ProjectionRow, TableProjection};
pub use replay::{Replay, replay};
pub use verify::{JournalError, Verification};

use sim_kernel::{ContentId, Symbol};

/// The single state-machine implementation shared by every backend.
pub struct Journal<B> {
    backend: B,
}

impl<B: JournalBackend> Journal<B> {
    /// Wraps a backend. Durable production backends must report write safety.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Obtains a new fencing generation, invalidating every older lease.
    pub fn acquire_lease(&self) -> Result<Lease, JournalError> {
        self.backend.acquire_lease()
    }

    /// Returns the current verified head.
    pub fn head(&self) -> Result<Option<JournalHead>, JournalError> {
        let state = self.backend.read_state()?;
        verify::verify_state(&state).map(|v| v.head)
    }

    /// Atomically publishes objects and an ordered entry batch under one fence.
    ///
    /// Exact redelivery is a no-op. Any conflicting delivery, missing/corrupt
    /// payload, stale fence, sequence gap, or wrong predecessor is rejected.
    pub fn publish(
        &self,
        lease: &Lease,
        expected: Option<&JournalHead>,
        objects: Vec<JournalObject>,
        entries: Vec<JournalEntry>,
    ) -> Result<JournalHead, JournalError> {
        if entries.is_empty() {
            return Err(JournalError::EmptyBatch);
        }
        let before = self.backend.read_state()?;
        verify::verify_state(&before)?;
        verify::verify_batch(&before, expected, &objects, &entries)?;
        let head = self.backend.admit(Admission {
            fence: lease.fence,
            expected: expected.cloned(),
            objects,
            entries: entries.clone(),
        })?;
        Ok(head)
    }

    /// Reads and verifies the complete journal closure.
    pub fn verify(&self) -> Result<Verification, JournalError> {
        verify::verify_state(&self.backend.read_state()?)
    }

    /// Replays verified entries in sequence order.
    pub fn replay(&self) -> Result<Replay, JournalError> {
        replay(self.backend.read_state()?)
    }

    /// Creates a detached, read-only table projection.
    pub fn table_projection(&self) -> Result<TableProjection, JournalError> {
        Ok(TableProjection::from_verification(self.verify()?))
    }

    /// Creates a detached, read-only directory projection.
    pub fn dir_projection(&self) -> Result<DirProjection, JournalError> {
        Ok(DirProjection::from_verification(self.verify()?))
    }

    /// Convenience constructor for an entry with an open kind symbol.
    pub fn entry(
        sequence: u64,
        previous: Option<ContentId>,
        kind: Symbol,
        payloads: Vec<ContentId>,
    ) -> JournalEntry {
        JournalEntry::new(sequence, previous, kind, payloads)
    }
}

#[cfg(test)]
mod tests;
