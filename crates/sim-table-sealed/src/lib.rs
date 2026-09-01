//! Authenticated-encryption decorator for any SIM `Table` or `Dir` backend.
//!
//! Version 1 uses standard IETF ChaCha20-Poly1305 from `ring`: 256-bit keys,
//! 96-bit nonces, and 128-bit tags. HMAC-SHA-256 blinds names with lane
//! separation. Suite/version support is fail-closed; incompatible migrations
//! require an explicit reader rather than silent fallback.
//!
//! Ciphertext length, operation timing, per-lane key equality, and access
//! patterns remain visible. Randomized sealing hides value equality; lane-bound
//! key blinding hides plaintext names and cross-lane name equality. Key and
//! randomness ownership stay injected, and diagnostics contain neither.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backup;
mod blind;
mod codec;
mod destroy;
mod error;
mod format;
mod generation;
mod key_provider;
mod table;

pub use backup::{BackupEnvelope, BackupPart};
pub use destroy::{DestructionEvidence, LeakageDeclaration};
pub use error::SealedError;
pub use format::{Binding, NONCE_LEN, SUITE_CHACHA20_POLY1305, SealedObject, TAG_LEN, VERSION};
pub use generation::{
    Generation, GenerationId, GenerationManager, GenerationManifest, GenerationStore, Lane,
    ManagedError, RecoveryReport,
};
pub use key_provider::{
    GenerationKeyProvider, KeyGrant, KeyRef, ProviderError, ProviderReceipt, WrappedKey,
};
pub use table::{KeyProvider, NonceSource, SealedConfig, SealedTable, SecretKey};

#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod tests;
