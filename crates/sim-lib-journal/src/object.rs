use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Symbol};

use crate::JournalError;

/// Immutable canonical bytes and their content identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalObject {
    pub id: ContentId,
    pub bytes: Vec<u8>,
}

impl JournalObject {
    /// Constructs an object using the journal's canonical byte identity.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        Self {
            id: ContentId::from_bytes(Symbol::qualified("journal", "sha256-bytes-v1"), digest),
            bytes,
        }
    }

    /// Rejects bytes that do not match their claimed identity.
    pub fn verify(&self) -> Result<(), JournalError> {
        if Self::from_bytes(self.bytes.clone()).id == self.id {
            Ok(())
        } else {
            Err(JournalError::CorruptObject(self.id.clone()))
        }
    }
}
