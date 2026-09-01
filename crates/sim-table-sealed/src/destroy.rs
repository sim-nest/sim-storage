//! Bounded, inspectable managed-destruction evidence.

use crate::{Generation, GenerationId, GenerationManager, Lane, ManagedError, ProviderReceipt};

/// Residual information deliberately excluded from an erasure claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeakageDeclaration {
    /// Metadata that can remain after managed destruction.
    pub residual: Vec<String>,
    /// Mandatory boundary for material outside managed stores.
    pub unmanaged_plaintext_statement: String,
}

/// Evidence for a bounded managed generation destruction claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DestructionEvidence {
    /// Destroyed lane.
    pub lane: Lane,
    /// Destroyed generation.
    pub generation: GenerationId,
    /// Provider revocation and destruction acknowledgements.
    pub provider_receipts: Vec<ProviderReceipt>,
    /// Number of tracked live and backup envelopes removed.
    pub removed_envelopes: usize,
    /// Number of post-destruction negative probes that refused decryption.
    pub negative_probes: usize,
    /// Honest residual leakage and claim boundary.
    pub leakage: LeakageDeclaration,
}

impl GenerationManager {
    /// Revoke, remove tracked material, destroy, and negatively probe all ciphertext.
    pub fn destroy_generation(
        &self,
        generation: &Generation,
        tracked_backup_copies: &mut Vec<crate::BackupEnvelope>,
        residual: Vec<String>,
    ) -> Result<DestructionEvidence, ManagedError> {
        let lane = generation.manifest.lane;
        let id = generation.manifest.generation;
        let grant = self.grant(lane);
        let revoke = self
            .provider
            .revoke(&generation.manifest.key, grant)
            .map_err(|error| ManagedError(error.to_string()))?;
        self.store.remove_generation(lane, id)?;
        let before = tracked_backup_copies.len();
        tracked_backup_copies.retain(|backup| {
            backup.generation.manifest.lane != lane || backup.generation.manifest.generation != id
        });
        let removed_envelopes = 1 + before - tracked_backup_copies.len();
        let destroy = self
            .provider
            .destroy(&generation.manifest.key)
            .map_err(|error| ManagedError(error.to_string()))?;
        let mut negative_probes = 0;
        for ciphertext in generation.ciphertext.values() {
            if self
                .provider
                .probe(&generation.manifest.key, ciphertext)
                .is_ok()
            {
                return Err(ManagedError(
                    "destroyed key still decrypts ciphertext".into(),
                ));
            }
            negative_probes += 1;
        }
        Ok(DestructionEvidence {
            lane,
            generation: id,
            provider_receipts: vec![revoke, destroy],
            removed_envelopes,
            negative_probes,
            leakage: LeakageDeclaration {
                residual,
                unmanaged_plaintext_statement:
                    "exported or otherwise unmanaged plaintext is outside this destruction claim"
                        .into(),
            },
        })
    }
}
