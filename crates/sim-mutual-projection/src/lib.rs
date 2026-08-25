//! Bilateral, copied projections between mutually opaque archives.
//!
//! An [`Offer`] fixes the exact fields, provenance, recipient, expiry, and both
//! key identities before either person accepts. [`MutualPorch`] admits a copy
//! only after independently verified acceptances and never returns an archive,
//! table, directory, query, or key-provider handle.

mod model;
mod porch;

pub use model::{
    Acceptance, AuditEvent, CopiedFact, Fact, FieldShape, KeyId, Offer, OfferId, PartyId,
    ProjectionReceipt, ProvenanceLink, Tombstone,
};
pub use porch::{Clock, KeyVerifier, MutualPorch, PorchError, SystemClock};

#[cfg(test)]
mod tests;
