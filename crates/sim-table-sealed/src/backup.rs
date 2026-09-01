//! Managed backup envelopes.

use crate::{Generation, GenerationManager, KeyGrant, KeyRef, Lane, ManagedError, WrappedKey};

/// Required part of a managed backup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupPart {
    /// Ciphertext records.
    Ciphertext,
    /// Authenticated generation manifest.
    Manifest,
    /// Provider-wrapped generation envelope.
    WrappedGenerationKey,
}

/// Backup containing no raw generation key or plaintext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackupEnvelope {
    /// Complete encrypted generation.
    pub generation: Generation,
    /// Provider-wrapped generation key.
    pub wrapped_key: WrappedKey,
}

impl GenerationManager {
    /// Export ciphertext, its authenticated manifest, and a wrapped key only.
    pub fn backup(
        &self,
        generation: &Generation,
        policy: &str,
    ) -> Result<BackupEnvelope, ManagedError> {
        if policy.is_empty() || generation.manifest.authentication.is_empty() {
            return Err(ManagedError(
                "backup policy or authentication missing".into(),
            ));
        }
        let (wrapped_key, _) = self
            .provider
            .wrap(
                &generation.manifest.key,
                self.grant(generation.manifest.lane),
                policy,
            )
            .map_err(|error| ManagedError(error.to_string()))?;
        Ok(BackupEnvelope {
            generation: generation.clone(),
            wrapped_key,
        })
    }

    /// Restore only under the envelope's named policy and lane grant.
    pub fn restore(&self, backup: &BackupEnvelope, policy: &str) -> Result<KeyRef, ManagedError> {
        if backup.wrapped_key.policy != policy
            || backup.generation.manifest.authentication.is_empty()
            || backup.generation.ciphertext.len() != backup.generation.manifest.record_ids.len()
        {
            return Err(ManagedError(
                "backup is incomplete or policy mismatched".into(),
            ));
        }
        let lane = backup.generation.manifest.lane;
        let (key, _) = self
            .provider
            .unwrap(&backup.wrapped_key, self.grant(lane), policy)
            .map_err(|error| ManagedError(error.to_string()))?;
        if key != backup.generation.manifest.key {
            return Err(ManagedError("stale generation envelope".into()));
        }
        Ok(key)
    }

    /// Exposes a lane grant only to document its explicit restore dependency.
    pub fn restore_grant(&self, lane: Lane) -> &KeyGrant {
        self.grant(lane)
    }
}
