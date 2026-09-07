use sim_kernel::{ContentId, Datum, NumberLiteral, Symbol};

use crate::JournalError;

/// One immutable journal fact whose identity is the canonical semantic Datum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: ContentId,
    pub sequence: u64,
    pub previous: Option<ContentId>,
    pub kind: Symbol,
    pub payloads: Vec<ContentId>,
}

impl JournalEntry {
    /// Constructs a canonical `journal/entry-v2` value.
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
        value.id = value
            .canonical_datum()
            .content_id()
            .expect("journal entry construction is canonical");
        value
    }

    /// Returns the exact semantic value whose content id is this entry's id.
    pub fn canonical_datum(&self) -> Datum {
        Datum::Node {
            tag: Symbol::qualified("journal", "entry-v2"),
            fields: vec![
                (
                    Symbol::new("sequence"),
                    Datum::Number(NumberLiteral {
                        domain: Symbol::qualified("numbers", "u64"),
                        canonical: self.sequence.to_string(),
                    }),
                ),
                (
                    Symbol::new("previous"),
                    self.previous.as_ref().map_or(Datum::Nil, id_datum),
                ),
                (Symbol::new("kind"), Datum::Symbol(self.kind.clone())),
                (
                    Symbol::new("payloads"),
                    Datum::Vector(self.payloads.iter().map(id_datum).collect()),
                ),
            ],
        }
    }

    pub(crate) fn canonical_id(&self) -> Result<ContentId, JournalError> {
        self.canonical_datum()
            .content_id()
            .map_err(|_| JournalError::CorruptEntry)
    }
}

pub(crate) fn id_datum(id: &ContentId) -> Datum {
    Datum::Node {
        tag: Symbol::qualified("journal", "content-id-v1"),
        fields: vec![
            (
                Symbol::new("algorithm"),
                Datum::Symbol(id.algorithm.clone()),
            ),
            (Symbol::new("digest"), Datum::Bytes(id.bytes.to_vec())),
        ],
    }
}
