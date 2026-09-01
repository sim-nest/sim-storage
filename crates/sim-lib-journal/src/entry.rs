use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Symbol};

/// One immutable journal fact. Kinds and payload interpretation remain open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: ContentId,
    pub sequence: u64,
    pub previous: Option<ContentId>,
    pub kind: Symbol,
    pub payloads: Vec<ContentId>,
}

impl JournalEntry {
    pub fn new(
        sequence: u64,
        previous: Option<ContentId>,
        kind: Symbol,
        payloads: Vec<ContentId>,
    ) -> Self {
        let mut value = Self {
            id: ContentId::from_bytes(Symbol::new("unset"), [0; 32]),
            sequence,
            previous,
            kind,
            payloads,
        };
        value.id = value.canonical_id();
        value
    }

    pub(crate) fn canonical_id(&self) -> ContentId {
        let mut hasher = Sha256::new();
        hasher.update(b"sim-journal-entry-v1\0");
        hasher.update(self.sequence.to_be_bytes());
        encode_optional_id(&mut hasher, self.previous.as_ref());
        encode_symbol(&mut hasher, &self.kind);
        hasher.update((self.payloads.len() as u64).to_be_bytes());
        for id in &self.payloads {
            encode_id(&mut hasher, id);
        }
        ContentId::from_bytes(
            Symbol::qualified("journal", "sha256-entry-v1"),
            hasher.finalize().into(),
        )
    }
}

fn encode_optional_id(hasher: &mut Sha256, id: Option<&ContentId>) {
    match id {
        Some(id) => {
            hasher.update([1]);
            encode_id(hasher, id);
        }
        None => {
            hasher.update([0]);
        }
    }
}
fn encode_id(hasher: &mut Sha256, id: &ContentId) {
    encode_symbol(hasher, &id.algorithm);
    hasher.update(id.bytes);
}
fn encode_symbol(hasher: &mut Sha256, symbol: &Symbol) {
    let text = symbol.as_qualified_str();
    hasher.update((text.len() as u64).to_be_bytes());
    hasher.update(text.as_bytes());
}
