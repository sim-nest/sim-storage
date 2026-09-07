use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Datum, Symbol};

use crate::{JournalError, datum_codec};

/// Immutable semantic value and its convenient payload-byte projection.
///
/// [`JournalObject::from_bytes`] wraps bytes in `journal/exact-bytes-v1`, so
/// their public id is a kernel Datum id rather than a physical storage digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalObject {
    pub id: ContentId,
    pub bytes: Vec<u8>,
    datum: Datum,
}

impl JournalObject {
    /// Constructs a tagged exact-byte semantic value.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let datum = Datum::Node {
            tag: Symbol::qualified("journal", "exact-bytes-v1"),
            fields: vec![(Symbol::new("bytes"), Datum::Bytes(bytes.clone()))],
        };
        let id = datum
            .content_id()
            .expect("exact byte payload is a canonical datum");
        Self { id, bytes, datum }
    }

    /// Constructs an object from any canonical Datum.
    pub fn from_datum(datum: Datum) -> Result<Self, JournalError> {
        let id = datum
            .content_id()
            .map_err(|_| JournalError::NonCanonicalDatum)?;
        let bytes = exact_bytes(&datum).unwrap_or(datum_codec::encode(&datum)?);
        Ok(Self { id, bytes, datum })
    }

    /// Borrows the semantic value.
    pub fn datum(&self) -> &Datum {
        &self.datum
    }

    /// Rejects any object whose claimed semantic id or byte projection differs.
    pub fn verify(&self) -> Result<(), JournalError> {
        if self
            .datum
            .content_id()
            .map_err(|_| JournalError::NonCanonicalDatum)?
            != self.id
        {
            return Err(JournalError::CorruptObject(self.id.clone()));
        }
        if let Some(exact) = exact_bytes(&self.datum) {
            if exact != self.bytes {
                return Err(JournalError::CorruptObject(self.id.clone()));
            }
        } else if datum_codec::encode(&self.datum)? != self.bytes {
            return Err(JournalError::CorruptObject(self.id.clone()));
        }
        Ok(())
    }

    pub(crate) fn storage_bytes(&self) -> Result<Vec<u8>, JournalError> {
        let encoded = datum_codec::encode(&self.datum)?;
        let mut out = b"SIMJOBJECT2".to_vec();
        out.extend((encoded.len() as u64).to_be_bytes());
        out.extend(encoded);
        Ok(out)
    }

    pub(crate) fn from_storage_bytes(bytes: &[u8]) -> Result<Self, JournalError> {
        if bytes.len() < 19 || &bytes[..11] != b"SIMJOBJECT2" {
            return Err(JournalError::CorruptState("object format"));
        }
        let len = u64::from_be_bytes(
            bytes[11..19]
                .try_into()
                .map_err(|_| JournalError::CorruptState("object length"))?,
        ) as usize;
        let encoded = bytes
            .get(19..)
            .filter(|tail| tail.len() == len)
            .ok_or(JournalError::CorruptState("object length"))?;
        let datum = datum_codec::decode(encoded)?;
        let mut object = Self::from_datum(datum)?;
        if let Some(exact) = exact_bytes(&object.datum) {
            object.bytes = exact;
        }
        object.verify()?;
        Ok(object)
    }
}

fn exact_bytes(datum: &Datum) -> Option<Vec<u8>> {
    let Datum::Node { tag, fields } = datum else {
        return None;
    };
    if *tag != Symbol::qualified("journal", "exact-bytes-v1") || fields.len() != 1 {
        return None;
    }
    match &fields[0] {
        (name, Datum::Bytes(bytes)) if *name == Symbol::new("bytes") => Some(bytes.clone()),
        _ => None,
    }
}

/// Exact-byte physical identity used only for storage locators.
pub(crate) fn storage_id(bytes: &[u8]) -> ContentId {
    ContentId::from_bytes(
        Symbol::qualified("journal", "sha256-storage-v1"),
        Sha256::digest(bytes).into(),
    )
}
