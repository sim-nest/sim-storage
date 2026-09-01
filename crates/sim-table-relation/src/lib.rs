//! Relation-backed implementations of the standard Table and Dir contracts.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;

pub use backend::{
    Node, NodeKind, RelationDir, RelationValueCodec, RelationView, UniqueKeyProjection,
    relation_namespace_capability, relation_table_read_capability, relation_table_write_capability,
};

#[cfg(test)]
mod tests;
