use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    thread,
};

use crate::{
    Generation, GenerationId, GenerationKeyProvider, GenerationManager, GenerationManifest,
    GenerationStore, KeyGrant, KeyRef, Lane, ManagedError, ProviderError, ProviderReceipt,
    WrappedKey,
};

#[derive(Default)]
struct Provider {
    live: Mutex<BTreeSet<String>>,
    refused: Mutex<BTreeSet<&'static str>>,
}

impl Provider {
    fn receipt(action: &'static str, key: &KeyRef) -> ProviderReceipt {
        ProviderReceipt {
            id: format!("{action}-{}", key.0),
            action,
            key: key.clone(),
        }
    }
    fn allowed(&self, action: &'static str) -> Result<(), ProviderError> {
        if self.refused.lock().unwrap().contains(action) {
            Err(ProviderError(action))
        } else {
            Ok(())
        }
    }
}

impl GenerationKeyProvider for Provider {
    fn create(
        &self,
        lane: &Lane,
        generation: GenerationId,
        grant: &KeyGrant,
    ) -> Result<(KeyRef, ProviderReceipt), ProviderError> {
        self.allowed("create")?;
        let key = KeyRef(format!("{lane:?}-{}-{}", generation.0, grant.0));
        self.live.lock().unwrap().insert(key.0.clone());
        Ok((key.clone(), Self::receipt("create", &key)))
    }
    fn wrap(
        &self,
        key: &KeyRef,
        grant: &KeyGrant,
        policy: &str,
    ) -> Result<(WrappedKey, ProviderReceipt), ProviderError> {
        self.allowed("wrap")?;
        if !self.live.lock().unwrap().contains(&key.0) || !key.0.ends_with(&grant.0) {
            return Err(ProviderError("grant"));
        }
        Ok((
            WrappedKey {
                policy: policy.into(),
                envelope: key.0.as_bytes().to_vec(),
            },
            Self::receipt("wrap", key),
        ))
    }
    fn unwrap(
        &self,
        wrapped: &WrappedKey,
        grant: &KeyGrant,
        policy: &str,
    ) -> Result<(KeyRef, ProviderReceipt), ProviderError> {
        self.allowed("unwrap")?;
        if wrapped.policy != policy {
            return Err(ProviderError("policy"));
        }
        let key = KeyRef(
            String::from_utf8(wrapped.envelope.clone()).map_err(|_| ProviderError("envelope"))?,
        );
        if !key.0.ends_with(&grant.0) || !self.live.lock().unwrap().contains(&key.0) {
            return Err(ProviderError("grant"));
        }
        Ok((key.clone(), Self::receipt("unwrap", &key)))
    }
    fn revoke(&self, key: &KeyRef, _grant: &KeyGrant) -> Result<ProviderReceipt, ProviderError> {
        self.allowed("revoke")?;
        self.live.lock().unwrap().remove(&key.0);
        Ok(Self::receipt("revoke", key))
    }
    fn destroy(&self, key: &KeyRef) -> Result<ProviderReceipt, ProviderError> {
        self.allowed("destroy")?;
        self.live.lock().unwrap().remove(&key.0);
        Ok(Self::receipt("destroy", key))
    }
    fn probe(&self, key: &KeyRef, _ciphertext: &[u8]) -> Result<(), ProviderError> {
        self.live
            .lock()
            .unwrap()
            .contains(&key.0)
            .then_some(())
            .ok_or(ProviderError("decrypt"))
    }
}

#[derive(Default)]
struct StoreState {
    staged: BTreeMap<(Lane, GenerationId), Generation>,
    complete: BTreeSet<(Lane, GenerationId)>,
    selected: BTreeMap<Lane, GenerationId>,
    tear_manifest: bool,
    refuse_select_once: bool,
}

#[derive(Default)]
struct Store(Mutex<StoreState>);

impl GenerationStore for Store {
    fn stage_records(&self, generation: &Generation) -> Result<(), ManagedError> {
        let key = (generation.manifest.lane, generation.manifest.generation);
        self.0
            .lock()
            .unwrap()
            .staged
            .insert(key, generation.clone());
        Ok(())
    }
    fn complete_manifest(&self, manifest: &GenerationManifest) -> Result<(), ManagedError> {
        let mut state = self.0.lock().unwrap();
        if state.tear_manifest {
            state.tear_manifest = false;
            return Err(ManagedError("injected torn manifest".into()));
        }
        state.complete.insert((manifest.lane, manifest.generation));
        Ok(())
    }
    fn select(&self, lane: Lane, generation: GenerationId) -> Result<(), ManagedError> {
        let mut state = self.0.lock().unwrap();
        if state.refuse_select_once {
            state.refuse_select_once = false;
            return Err(ManagedError("injected torn selection".into()));
        }
        if !state.complete.contains(&(lane, generation)) {
            return Err(ManagedError("incomplete selection".into()));
        }
        state.selected.insert(lane, generation);
        Ok(())
    }
    fn complete(&self, lane: Lane, generation: GenerationId) -> Option<Generation> {
        let state = self.0.lock().unwrap();
        state
            .complete
            .contains(&(lane, generation))
            .then(|| state.staged[&(lane, generation)].clone())
    }
    fn selected(&self, lane: Lane) -> Option<GenerationId> {
        self.0.lock().unwrap().selected.get(&lane).copied()
    }
    fn staged(&self, lane: Lane) -> Vec<GenerationId> {
        self.0
            .lock()
            .unwrap()
            .staged
            .keys()
            .filter_map(|(candidate, id)| (*candidate == lane).then_some(*id))
            .collect()
    }
    fn remove_generation(&self, lane: Lane, generation: GenerationId) -> Result<(), ManagedError> {
        let mut state = self.0.lock().unwrap();
        state.staged.remove(&(lane, generation));
        state.complete.remove(&(lane, generation));
        if state.selected.get(&lane) == Some(&generation) {
            state.selected.remove(&lane);
        }
        Ok(())
    }
}

fn manager(provider: Arc<Provider>, store: Arc<Store>) -> GenerationManager {
    GenerationManager::new(
        provider,
        store,
        [
            (Lane::Ordinary, KeyGrant("ordinary-grant".into())),
            (Lane::Finance, KeyGrant("finance-grant".into())),
            (Lane::PrivateObservation, KeyGrant("private-grant".into())),
        ],
    )
    .unwrap()
}

fn generation(lane: Lane, id: GenerationId, key: &KeyRef, value: u8) -> Generation {
    Generation {
        manifest: GenerationManifest {
            lane,
            generation: id,
            key: key.clone(),
            record_ids: vec!["record".into()],
            authentication: vec![value, 0xa5],
        },
        ciphertext: BTreeMap::from([("record".into(), vec![value, 0x5a])]),
    }
}

#[test]
fn torn_rotation_recovers_old_complete_generation_and_refuses_rollback() {
    let provider = Arc::new(Provider::default());
    let store = Arc::new(Store::default());
    let manager = manager(provider, store.clone());
    manager
        .rotate(Lane::Finance, GenerationId(1), |key| {
            Ok(generation(Lane::Finance, GenerationId(1), key, 1))
        })
        .unwrap();
    store.0.lock().unwrap().tear_manifest = true;
    assert!(
        manager
            .rotate(Lane::Finance, GenerationId(2), |key| Ok(generation(
                Lane::Finance,
                GenerationId(2),
                key,
                2
            )))
            .is_err()
    );
    let report = manager.recover(Lane::Finance).unwrap();
    assert_eq!(report.selected, Some(GenerationId(1)));
    assert_eq!(report.ignored_incomplete, vec![GenerationId(2)]);
    store.0.lock().unwrap().refuse_select_once = true;
    assert!(
        manager
            .rotate(Lane::Finance, GenerationId(3), |key| Ok(generation(
                Lane::Finance,
                GenerationId(3),
                key,
                3
            )))
            .is_err()
    );
    let report = manager.recover(Lane::Finance).unwrap();
    assert_eq!(report.selected, Some(GenerationId(3)));
    assert!(
        manager
            .rotate(Lane::Finance, GenerationId(2), |key| Ok(generation(
                Lane::Finance,
                GenerationId(2),
                key,
                4
            )))
            .is_err()
    );
}

#[test]
fn backup_requires_every_part_named_policy_grant_and_current_envelope() {
    let provider = Arc::new(Provider::default());
    let store = Arc::new(Store::default());
    let manager = manager(provider.clone(), store);
    let (generation, _) = manager
        .rotate(Lane::PrivateObservation, GenerationId(7), |key| {
            Ok(generation(
                Lane::PrivateObservation,
                GenerationId(7),
                key,
                7,
            ))
        })
        .unwrap();
    let backup = manager.backup(&generation, "vault-policy").unwrap();
    assert_eq!(
        manager.restore(&backup, "vault-policy").unwrap(),
        generation.manifest.key
    );
    assert!(manager.restore(&backup, "other-policy").is_err());
    let mut missing = backup.clone();
    missing.generation.ciphertext.clear();
    assert!(manager.restore(&missing, "vault-policy").is_err());
    let mut stale = backup;
    stale.wrapped_key.envelope = b"stale-private-grant".to_vec();
    assert!(manager.restore(&stale, "vault-policy").is_err());
    provider.refused.lock().unwrap().insert("unwrap");
    assert!(manager.restore(&stale, "vault-policy").is_err());
    provider.refused.lock().unwrap().remove("unwrap");
    provider.refused.lock().unwrap().insert("wrap");
    assert!(manager.backup(&generation, "vault-policy").is_err());
}

#[test]
fn concurrent_same_lane_rotations_never_roll_the_head_back() {
    let provider = Arc::new(Provider::default());
    let store = Arc::new(Store::default());
    let manager = Arc::new(manager(provider, store.clone()));
    let workers: Vec<_> = [GenerationId(2), GenerationId(1)]
        .into_iter()
        .map(|id| {
            let manager = manager.clone();
            thread::spawn(move || {
                manager.rotate(Lane::Ordinary, id, |key| {
                    Ok(generation(Lane::Ordinary, id, key, id.0 as u8))
                })
            })
        })
        .collect();
    let successes = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(Result::is_ok)
        .count();
    assert!(successes >= 1);
    assert_eq!(store.selected(Lane::Ordinary), Some(GenerationId(2)));
}

#[test]
fn provider_refusal_is_fail_closed_at_create_and_revoke() {
    let provider = Arc::new(Provider::default());
    let store = Arc::new(Store::default());
    let manager = manager(provider.clone(), store);
    provider.refused.lock().unwrap().insert("create");
    assert!(
        manager
            .rotate(Lane::Ordinary, GenerationId(1), |key| Ok(generation(
                Lane::Ordinary,
                GenerationId(1),
                key,
                1
            )))
            .is_err()
    );
    provider.refused.lock().unwrap().remove("create");
    let (generation, _) = manager
        .rotate(Lane::Ordinary, GenerationId(1), |key| {
            Ok(generation(Lane::Ordinary, GenerationId(1), key, 1))
        })
        .unwrap();
    provider.refused.lock().unwrap().insert("revoke");
    assert!(
        manager
            .destroy_generation(&generation, &mut Vec::new(), Vec::new())
            .is_err()
    );
}

#[test]
fn concurrent_lane_use_and_destruction_are_isolated_and_evidenced() {
    let provider = Arc::new(Provider::default());
    let store = Arc::new(Store::default());
    let manager = Arc::new(manager(provider, store.clone()));
    let mut workers = vec![];
    for (lane, value) in [
        (Lane::Ordinary, 3),
        (Lane::Finance, 4),
        (Lane::PrivateObservation, 5),
    ] {
        let manager = manager.clone();
        workers.push(thread::spawn(move || {
            manager
                .rotate(lane, GenerationId(1), |key| {
                    Ok(generation(lane, GenerationId(1), key, value))
                })
                .unwrap()
                .0
        }));
    }
    let generations: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    let finance = generations
        .iter()
        .find(|generation| generation.manifest.lane == Lane::Finance)
        .unwrap();
    let ordinary_before = store.complete(Lane::Ordinary, GenerationId(1)).unwrap();
    let private_before = store
        .complete(Lane::PrivateObservation, GenerationId(1))
        .unwrap();
    let mut backups = vec![manager.backup(finance, "vault").unwrap()];
    let evidence = manager
        .destroy_generation(
            finance,
            &mut backups,
            vec!["lane identity".into(), "ciphertext size".into()],
        )
        .unwrap();
    assert!(backups.is_empty());
    assert_eq!(evidence.provider_receipts.len(), 2);
    assert_eq!(evidence.negative_probes, 1);
    assert!(
        evidence
            .leakage
            .unmanaged_plaintext_statement
            .contains("outside")
    );
    assert_eq!(
        store.complete(Lane::Ordinary, GenerationId(1)),
        Some(ordinary_before)
    );
    assert_eq!(
        store.complete(Lane::PrivateObservation, GenerationId(1)),
        Some(private_before)
    );
}
