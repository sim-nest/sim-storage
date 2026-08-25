//! Versioned sealed-object wire format and authenticated binding.

use crate::SealedError;

/// Current stable object format version.
pub const VERSION: u8 = 1;
/// ChaCha20-Poly1305 suite identifier.
pub const SUITE_CHACHA20_POLY1305: u8 = 1;
/// IETF ChaCha20-Poly1305 nonce length.
pub const NONCE_LEN: usize = 12;
/// Poly1305 authentication tag length.
pub const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 2 + NONCE_LEN + 4;

/// Metadata cryptographically bound to an object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Binding {
    /// Opaque lane identity. Moving an object between lanes is refused.
    pub lane: Vec<u8>,
    /// Storage generation. Restoring into another generation is refused.
    pub generation: u64,
    /// Blinded physical key at which the object must reside.
    pub physical_key: String,
    /// Stable metadata class; prevents substituting bytes from another use.
    pub metadata_class: String,
}

impl Binding {
    pub(crate) fn aad(&self) -> Vec<u8> {
        let mut out = b"sim-table-sealed\0".to_vec();
        out.push(VERSION);
        out.push(SUITE_CHACHA20_POLY1305);
        put_bytes(&mut out, &self.lane);
        out.extend_from_slice(&self.generation.to_be_bytes());
        put_bytes(&mut out, self.physical_key.as_bytes());
        put_bytes(&mut out, self.metadata_class.as_bytes());
        out
    }
}

/// Parsed sealed object which owns only ciphertext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SealedObject {
    /// Wire suite identifier.
    pub suite: u8,
    /// Unique per-key nonce.
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext with its appended authentication tag.
    pub ciphertext_and_tag: Vec<u8>,
}

impl SealedObject {
    /// Encode the stable, self-bounded binary representation.
    pub fn encode(&self) -> Result<Vec<u8>, SealedError> {
        let len =
            u32::try_from(self.ciphertext_and_tag.len()).map_err(|_| SealedError::Oversized)?;
        let mut out = Vec::with_capacity(HEADER_LEN + self.ciphertext_and_tag.len());
        out.push(VERSION);
        out.push(self.suite);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&self.ciphertext_and_tag);
        Ok(out)
    }

    /// Parse strictly, refusing versions, suites, truncation, and trailing data.
    pub fn decode(bytes: &[u8], max_object_bytes: usize) -> Result<Self, SealedError> {
        if bytes.len() > max_object_bytes || bytes.len() < HEADER_LEN + TAG_LEN {
            return Err(if bytes.len() > max_object_bytes {
                SealedError::Oversized
            } else {
                SealedError::Authentication
            });
        }
        if bytes[0] != VERSION || bytes[1] != SUITE_CHACHA20_POLY1305 {
            return Err(SealedError::Authentication);
        }
        let nonce = bytes[2..2 + NONCE_LEN]
            .try_into()
            .map_err(|_| SealedError::Authentication)?;
        let len = u32::from_be_bytes(
            bytes[14..18]
                .try_into()
                .map_err(|_| SealedError::Authentication)?,
        ) as usize;
        if len < TAG_LEN || HEADER_LEN.checked_add(len) != Some(bytes.len()) {
            return Err(SealedError::Authentication);
        }
        Ok(Self {
            suite: bytes[1],
            nonce,
            ciphertext_and_tag: bytes[HEADER_LEN..].to_vec(),
        })
    }
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
