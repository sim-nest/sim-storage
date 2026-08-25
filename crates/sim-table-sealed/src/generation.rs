//! Crash-safe managed generation lifecycle.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

use crate::{GenerationKeyProvider, KeyGrant, KeyRef, ProviderReceipt};

/// Stable isolation lane. Each lane has an independent grant and generations.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Lane {
    /// Ordinary application data.
    Ordinary,
    /// Financial data.
    Finance,
    /// Private observations.
    PrivateObservation,
}

/// Monotonic identity within one lane.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GenerationId(pub u64);

/// Authenticated description of one complete generation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationManifest {
    /// Owning lane.
    pub lane: Lane,
    /// Generation identity.
    pub generation: GenerationId,
    /// Opaque provider key identity.
    pub key: KeyRef,
    /// Content identifiers for all ciphertext records.
    pub record_ids: Vec<String>,
    /// Authentication bytes covering every preceding field and record.
    pub authentication: Vec<u8>,
}

/// Complete managed generation selected or awaiting selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Generation {
    /// Authenticated manifest.
    pub manifest: GenerationManifest,
    /// Ciphertext keyed by the manifest's content identifiers.
    pub ciphertext: BTreeMap<String, Vec<u8>>,
}

/// Recovery result; recovery never combines records across generations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    /// One complete selected generation, if present.
    pub selected: Option<GenerationId>,
    /// Incomplete staged generations ignored during recovery.
    pub ignored_incomplete: Vec<GenerationId>,
}

/// Durable operations needed for atomic generation publication.
pub trait GenerationStore: Send + Sync {
    /// Persist all ciphertext for a staged generation.
    fn stage_records(&self, generation: &Generation) -> Result<(), ManagedError>;
    /// Persist the authenticated manifest last, marking the stage complete.
    fn complete_manifest(&self, manifest: &GenerationManifest) -> Result<(), ManagedError>;
    /// Atomically select a complete generation as lane head.
    fn select(&self, lane: Lane, generation: GenerationId) -> Result<(), ManagedError>;
    /// Read a complete generation, returning `None` for absent/incomplete state.
    fn complete(&self, lane: Lane, generation: GenerationId) -> Option<Generation>;
    /// Current selected generation.
    fn selected(&self, lane: Lane) -> Option<GenerationId>;
    /// Every staged generation id for recovery inspection.
    fn staged(&self, lane: Lane) -> Vec<GenerationId>;
    /// Remove all tracked live material for a generation.
    fn remove_generation(&self, lane: Lane, generation: GenerationId) -> Result<(), ManagedError>;
}

/// Managed generation operation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedError(pub String);

impl fmt::Display for ManagedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "managed sealed generation: {}", self.0)
    }
}

impl std::error::Error for ManagedError {}

/// Coordinates provider-owned keys and crash-safe storage publication.
pub struct GenerationManager {
    pub(crate) provider: Arc<dyn GenerationKeyProvider>,
    pub(crate) store: Arc<dyn GenerationStore>,
    grants: BTreeMap<Lane, KeyGrant>,
    rotation_guards: BTreeMap<Lane, Mutex<()>>,
}

impl GenerationManager {
    /// Construct with an explicit independent grant for every lane.
    pub fn new(
        provider: Arc<dyn GenerationKeyProvider>,
        store: Arc<dyn GenerationStore>,
        grants: [(Lane, KeyGrant); 3],
    ) -> Result<Self, ManagedError> {
        let grants: BTreeMap<_, _> = grants.into_iter().collect();
        if grants.len() != 3 || grants.values().any(|grant| grant.0.is_empty()) {
            return Err(ManagedError(
                "each lane requires one non-empty grant".into(),
            ));
        }
        let mut identities: Vec<_> = grants.values().map(|grant| &grant.0).collect();
        identities.sort();
        identities.dedup();
        if identities.len() != 3 {
            return Err(ManagedError("lane grants must be independent".into()));
        }
        Ok(Self {
            provider,
            store,
            grants,
            rotation_guards: [Lane::Ordinary, Lane::Finance, Lane::PrivateObservation]
                .into_iter()
                .map(|lane| (lane, Mutex::new(())))
                .collect(),
        })
    }

    /// Return the capability for a lane without permitting cross-lane fallback.
    pub fn grant(&self, lane: Lane) -> &KeyGrant {
        &self.grants[&lane]
    }

    /// Create, fully stage, authenticate, then atomically select a generation.
    pub fn rotate<F>(
        &self,
        lane: Lane,
        id: GenerationId,
        build: F,
    ) -> Result<(Generation, ProviderReceipt), ManagedError>
    where
        F: FnOnce(&KeyRef) -> Result<Generation, ManagedError>,
    {
        let _rotation = self.rotation_guards[&lane]
            .lock()
            .map_err(|_| ManagedError("lane rotation lock poisoned".into()))?;
        if self
            .store
            .selected(lane)
            .is_some_and(|current| id <= current)
        {
            return Err(ManagedError("rollback generation refused".into()));
        }
        let (key, receipt) = self
            .provider
            .create(&lane, id, self.grant(lane))
            .map_err(|error| ManagedError(error.to_string()))?;
        let generation = build(&key)?;
        if generation.manifest.lane != lane
            || generation.manifest.generation != id
            || generation.manifest.key != key
            || generation.manifest.authentication.is_empty()
            || generation.manifest.record_ids.len() != generation.ciphertext.len()
            || generation
                .manifest
                .record_ids
                .iter()
                .any(|record| !generation.ciphertext.contains_key(record))
        {
            return Err(ManagedError("incomplete or mismatched generation".into()));
        }
        self.store.stage_records(&generation)?;
        self.store.complete_manifest(&generation.manifest)?;
        self.store.select(lane, id)?;
        Ok((generation, receipt))
    }

    /// Recover one complete head without mixing records from incomplete stages.
    pub fn recover(&self, lane: Lane) -> Result<RecoveryReport, ManagedError> {
        let staged = self.store.staged(lane);
        let ignored_incomplete = staged
            .iter()
            .copied()
            .filter(|id| self.store.complete(lane, *id).is_none())
            .collect();
        let selected = staged
            .into_iter()
            .filter(|id| self.store.complete(lane, *id).is_some())
            .max();
        if let Some(selected) = selected
            && self.store.selected(lane) != Some(selected)
        {
            self.store.select(lane, selected)?;
        }
        Ok(RecoveryReport {
            selected,
            ignored_incomplete,
        })
    }
}
