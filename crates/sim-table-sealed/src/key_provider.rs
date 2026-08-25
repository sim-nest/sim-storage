//! Opaque managed-generation key-provider contract.

use std::fmt;

use crate::{GenerationId, Lane};

/// Opaque authorization for exactly one storage lane.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyGrant(pub String);

/// Opaque provider-owned key identity. It contains no key bytes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyRef(pub String);

/// Provider-wrapped generation key suitable for managed backup storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrappedKey {
    /// Provider policy required to unwrap this envelope.
    pub policy: String,
    /// Opaque wrapped bytes. These are not usable key material.
    pub envelope: Vec<u8>,
}

/// Auditable provider acknowledgement for a key lifecycle action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderReceipt {
    /// Provider-defined unique receipt id.
    pub id: String,
    /// Action acknowledged by the provider.
    pub action: &'static str,
    /// Key affected by the action.
    pub key: KeyRef,
}

/// Redacted provider refusal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderError(pub &'static str);

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generation key provider refused {}", self.0)
    }
}

impl std::error::Error for ProviderError {}

/// Injected, capability-gated owner of managed generation keys.
///
/// Key bytes never cross this contract. Cryptographic transforms and probes
/// happen behind the provider boundary and address keys only by [`KeyRef`].
pub trait GenerationKeyProvider: Send + Sync {
    /// Create a key bound to `lane`, `generation`, and its independent grant.
    fn create(
        &self,
        lane: &Lane,
        generation: GenerationId,
        grant: &KeyGrant,
    ) -> Result<(KeyRef, ProviderReceipt), ProviderError>;

    /// Wrap a key under a named restore policy.
    fn wrap(
        &self,
        key: &KeyRef,
        grant: &KeyGrant,
        policy: &str,
    ) -> Result<(WrappedKey, ProviderReceipt), ProviderError>;

    /// Recover an opaque key reference under the named policy and lane grant.
    fn unwrap(
        &self,
        wrapped: &WrappedKey,
        grant: &KeyGrant,
        policy: &str,
    ) -> Result<(KeyRef, ProviderReceipt), ProviderError>;

    /// Revoke the lane grant for this key.
    fn revoke(&self, key: &KeyRef, grant: &KeyGrant) -> Result<ProviderReceipt, ProviderError>;

    /// Permanently destroy provider-managed material for this key.
    fn destroy(&self, key: &KeyRef) -> Result<ProviderReceipt, ProviderError>;

    /// Authenticate/decrypt a managed ciphertext, returning only success/failure.
    fn probe(&self, key: &KeyRef, ciphertext: &[u8]) -> Result<(), ProviderError>;
}
