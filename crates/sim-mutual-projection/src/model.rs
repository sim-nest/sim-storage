use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Symbol};
use std::collections::{BTreeMap, BTreeSet};

pub type PartyId = String;
pub type KeyId = String;
pub type OfferId = ContentId;

/// A private-archive fact supplied by value. No archive authority accompanies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    pub id: ContentId,
    pub fields: BTreeMap<String, Vec<u8>>,
    pub provenance: Vec<ProvenanceLink>,
}

impl Fact {
    pub fn synthetic(fields: BTreeMap<String, Vec<u8>>, provenance: Vec<ProvenanceLink>) -> Self {
        let id = content_id(b"sim-mutual-fact-v1", encode_fields(&fields));
        Self {
            id,
            fields,
            provenance,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProvenanceLink {
    pub relation: String,
    pub content: ContentId,
}

/// The closed field projection offered to the recipient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldShape {
    fields: BTreeSet<String>,
}

impl FieldShape {
    pub fn new(fields: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let fields: BTreeSet<_> = fields.into_iter().collect();
        if fields.is_empty() || fields.iter().any(|field| field.is_empty()) {
            return Err("a projection requires non-empty field names");
        }
        Ok(Self { fields })
    }

    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(String::as_str)
    }
}

/// Immutable invitation binding the exact projection to two independent keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub id: OfferId,
    pub inviter: PartyId,
    pub recipient: PartyId,
    pub inviter_key: KeyId,
    pub recipient_key: KeyId,
    pub fact: ContentId,
    pub shape: FieldShape,
    pub expires_at: u64,
}

impl Offer {
    pub fn new(
        inviter: PartyId,
        recipient: PartyId,
        inviter_key: KeyId,
        recipient_key: KeyId,
        fact: ContentId,
        shape: FieldShape,
        expires_at: u64,
    ) -> Self {
        let mut bytes = Vec::new();
        for value in [&inviter, &recipient, &inviter_key, &recipient_key] {
            push_bytes(&mut bytes, value.as_bytes());
        }
        push_id(&mut bytes, &fact);
        for field in shape.fields() {
            push_bytes(&mut bytes, field.as_bytes());
        }
        bytes.extend_from_slice(&expires_at.to_be_bytes());
        let id = content_id(b"sim-mutual-offer-v1", bytes);
        Self {
            id,
            inviter,
            recipient,
            inviter_key,
            recipient_key,
            fact,
            shape,
            expires_at,
        }
    }

    pub fn acceptance_message(&self) -> Vec<u8> {
        let mut bytes = b"sim-mutual-accept-v1\0".to_vec();
        push_id(&mut bytes, &self.id);
        bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceptance {
    pub offer: OfferId,
    pub party: PartyId,
    pub key: KeyId,
    pub proof: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopiedFact {
    pub source: ContentId,
    pub fields: BTreeMap<String, Vec<u8>>,
    pub provenance: Vec<ProvenanceLink>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionReceipt {
    pub id: ContentId,
    pub offer: OfferId,
    pub copied: ContentId,
    pub admitted_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub offer: OfferId,
    pub receipt: Option<ContentId>,
    pub ended_at: u64,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditEvent {
    Offered(OfferId),
    Refused(OfferId),
    Copied(ProjectionReceipt),
    Ended(Tombstone),
}

pub(crate) fn copied_id(fact: &CopiedFact) -> ContentId {
    let mut bytes = encode_fields(&fact.fields);
    for link in &fact.provenance {
        push_bytes(&mut bytes, link.relation.as_bytes());
        push_id(&mut bytes, &link.content);
    }
    content_id(b"sim-mutual-copy-v1", bytes)
}

pub(crate) fn receipt_id(offer: &OfferId, copied: &ContentId, at: u64) -> ContentId {
    let mut bytes = Vec::new();
    push_id(&mut bytes, offer);
    push_id(&mut bytes, copied);
    bytes.extend_from_slice(&at.to_be_bytes());
    content_id(b"sim-mutual-receipt-v1", bytes)
}

fn encode_fields(fields: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (name, value) in fields {
        push_bytes(&mut bytes, name.as_bytes());
        push_bytes(&mut bytes, value);
    }
    bytes
}
fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn push_id(out: &mut Vec<u8>, id: &ContentId) {
    push_bytes(out, id.algorithm.as_qualified_str().as_bytes());
    out.extend_from_slice(&id.bytes);
}
fn content_id(domain: &[u8], bytes: Vec<u8>) -> ContentId {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(bytes);
    ContentId::from_bytes(
        Symbol::qualified("mutual", "sha256-v1"),
        hash.finalize().into(),
    )
}
