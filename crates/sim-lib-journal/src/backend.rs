use crate::{JournalEntry, JournalError, JournalHead, JournalObject, Lease};
use sim_kernel::ContentId;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A consistent backend snapshot used for verification and replay.
#[derive(Clone, Debug, Default)]
pub struct StoredState {
    pub objects: BTreeMap<ContentId, Vec<u8>>,
    pub entries: BTreeMap<u64, JournalEntry>,
    pub head: Option<JournalHead>,
}

/// One atomic admission request. Implementations publish immutable objects and
/// compare the head/fence in the same linearized operation.
pub struct Admission {
    pub fence: u64,
    pub expected: Option<JournalHead>,
    pub objects: Vec<JournalObject>,
    pub entries: Vec<JournalEntry>,
}

/// Object-safe storage seam. A Table backend implements `admit` with canonical
/// `table/cas`; it must refuse writes unless CAS and durability are provable.
pub trait JournalBackend: Send + Sync {
    fn acquire_lease(&self) -> Result<Lease, JournalError>;
    fn read_state(&self) -> Result<StoredState, JournalError>;
    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError>;
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
}
