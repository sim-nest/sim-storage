// conformance: immutable journal objects and fenced heads replay exactly after reopen.

use crate::*;
use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Datum, Symbol, datum_content_algorithm};
use sim_storage_port::{
    Cancellation, HostCompareExchange, HostDirError, HostDirErrorKind, HostDirPort, HostEntry,
};
use std::sync::{Arc, Barrier};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

fn object(byte: u8) -> JournalObject {
    JournalObject::from_bytes([byte])
}
fn entry(sequence: u64, previous: Option<ContentId>, object: &JournalObject) -> JournalEntry {
    JournalEntry::new(
        sequence,
        previous,
        Symbol::qualified("example", "fact"),
        vec![object.id.clone()],
    )
}

#[test]
fn replay_survives_deleting_every_projection() {
    let journal = Journal::new(MemoryBackend::new());
    let lease = journal.acquire_lease().unwrap();
    let a = object(1);
    let first = entry(0, None, &a);
    let head = journal
        .publish(&lease, None, vec![a.clone()], vec![first.clone()])
        .unwrap();
    let b = object(2);
    let second = entry(1, Some(head.entry.clone()), &b);
    journal
        .publish(&lease, Some(&head), vec![b], vec![second.clone()])
        .unwrap();

    let table = journal.table_projection().unwrap();
    let dir = journal.dir_projection().unwrap();
    assert!(!table.rows().is_empty());
    assert!(!dir.journal().is_empty());
    drop((table, dir)); // projections are disposable and carry no authority

    assert_eq!(
        journal.replay().unwrap().collect::<Vec<_>>(),
        vec![first, second]
    );
    assert_eq!(journal.verify().unwrap().object_ids.len(), 2);
}

#[test]
fn verified_snapshot_binds_entries_and_semantic_payloads_to_one_read() {
    let journal = Journal::new(MemoryBackend::new());
    let lease = journal.acquire_lease().unwrap();
    let value = Datum::Node {
        tag: Symbol::qualified("example", "semantic-value-v1"),
        fields: vec![(Symbol::new("answer"), Datum::String("forty-two".into()))],
    };
    let object = JournalObject::from_datum(value.clone()).unwrap();
    let fact = entry(0, None, &object);
    let expected_id = object.id.clone();
    let expected_entry = fact.clone();
    journal
        .publish(&lease, None, vec![object], vec![fact])
        .unwrap();

    let snapshot = journal.verified_snapshot().unwrap();
    assert_eq!(snapshot.entries(), &[expected_entry]);
    assert_eq!(snapshot.datum(&expected_id), Some(&value));
    assert_eq!(snapshot.datums().len(), 1);
    assert_eq!(snapshot.head().unwrap().sequence, 0);
}

#[test]
fn stale_fence_wrong_previous_gap_and_missing_payload_are_rejected() {
    let journal = Journal::new(MemoryBackend::new());
    let stale = journal.acquire_lease().unwrap();
    let live = journal.acquire_lease().unwrap();
    let a = object(1);
    let first = entry(0, None, &a);
    assert_eq!(
        journal.publish(&stale, None, vec![a.clone()], vec![first.clone()]),
        Err(JournalError::StaleLease)
    );
    let head = journal.publish(&live, None, vec![a], vec![first]).unwrap();
    let b = object(2);
    assert_eq!(
        journal.publish(
            &live,
            Some(&head),
            vec![b.clone()],
            vec![entry(3, Some(head.entry.clone()), &b)]
        ),
        Err(JournalError::SequenceGap)
    );
    assert_eq!(
        journal.publish(
            &live,
            Some(&head),
            vec![b.clone()],
            vec![entry(1, None, &b)]
        ),
        Err(JournalError::WrongPrevious)
    );
    let missing = object(9);
    assert!(matches!(
        journal.publish(
            &live,
            Some(&head),
            vec![],
            vec![entry(1, Some(head.entry.clone()), &missing)]
        ),
        Err(JournalError::MissingPayload(_))
    ));
}

#[test]
fn corrupt_bytes_and_conflicting_content_are_rejected() {
    let journal = Journal::new(MemoryBackend::new());
    let lease = journal.acquire_lease().unwrap();
    let good = object(1);
    let mut corrupt = good.clone();
    corrupt.bytes = vec![2];
    let first = entry(0, None, &good);
    assert!(matches!(
        journal.publish(&lease, None, vec![corrupt], vec![first]),
        Err(JournalError::CorruptObject(_))
    ));
}

#[test]
fn exact_batch_redelivery_is_idempotent_but_conflict_is_not() {
    let backend = Arc::new(MemoryBackend::new());
    let journal = Journal::new(backend.clone());
    let lease = journal.acquire_lease().unwrap();
    let a = object(1);
    let first = entry(0, None, &a);
    let head = journal
        .publish(&lease, None, vec![a.clone()], vec![first.clone()])
        .unwrap();

    // Redelivery arrives with its original expected head after acknowledgement
    // loss. The backend recognizes the exact committed batch as a no-op.
    let admission = Admission {
        fence: lease.fence(),
        expected: None,
        objects: vec![a],
        entries: vec![first.clone()],
    };
    assert_eq!(backend.admit(admission).unwrap(), head);
    let mut conflict = first;
    conflict.kind = Symbol::qualified("example", "different");
    let admission = Admission {
        fence: lease.fence(),
        expected: None,
        objects: vec![],
        entries: vec![conflict],
    };
    assert_eq!(
        backend.admit(admission),
        Err(JournalError::ConflictingDelivery)
    );

    let conflicting = JournalEntry::new(0, None, Symbol::qualified("example", "different"), vec![]);
    assert_eq!(
        journal.publish(&lease, None, vec![], vec![conflicting]),
        Err(JournalError::ConflictingDelivery)
    );
}

#[test]
fn atomic_batch_is_all_or_nothing() {
    let journal = Journal::new(MemoryBackend::new());
    let lease = journal.acquire_lease().unwrap();
    let a = object(1);
    let first = entry(0, None, &a);
    let b = object(2);
    let bad = entry(2, Some(first.id.clone()), &b);
    assert_eq!(
        journal.publish(&lease, None, vec![a, b], vec![first, bad]),
        Err(JournalError::SequenceGap)
    );
    assert_eq!(journal.head().unwrap(), None);
    assert!(journal.verify().unwrap().object_ids.is_empty());
}

#[test]
fn every_short_two_writer_interleaving_has_one_sequence_zero_winner() {
    // Explore all schedules of acquire/publish for two coordinators. A lease
    // acquisition invalidates older generations; no schedule admits two
    // different entries at sequence zero.
    for schedule in [
        [0, 1, 2, 3],
        [0, 2, 1, 3],
        [0, 2, 3, 1],
        [2, 0, 1, 3],
        [2, 0, 3, 1],
        [2, 3, 0, 1],
    ] {
        let backend = Arc::new(MemoryBackend::new());
        let mut leases = [None, None];
        let objects = [object(1), object(2)];
        let mut accepted = Vec::new();
        for op in schedule {
            let writer = op / 2;
            if op % 2 == 0 {
                leases[writer] = Some(backend.acquire_lease().unwrap());
            } else if let Some(lease) = &leases[writer] {
                let candidate = entry(0, None, &objects[writer]);
                if backend
                    .admit(Admission {
                        fence: lease.fence(),
                        expected: None,
                        objects: vec![objects[writer].clone()],
                        entries: vec![candidate.clone()],
                    })
                    .is_ok()
                {
                    accepted.push(candidate.id);
                }
            }
        }
        accepted.sort();
        accepted.dedup();
        assert!(
            accepted.len() <= 1,
            "schedule {schedule:?} accepted two identities"
        );
    }
}

#[derive(Default)]
struct TestPort {
    files: Mutex<BTreeMap<Vec<String>, Vec<u8>>>,
    dirs: Mutex<BTreeSet<Vec<String>>>,
}
impl TestPort {
    fn corrupt_one_byte(&self) {
        let mut files = self.files.lock().unwrap();
        let value = files
            .iter_mut()
            .find(|(p, _)| p.first().is_some_and(|v| v == "objects-v2"))
            .unwrap()
            .1;
        value[0] ^= 1;
    }
}
impl HostDirPort for TestPort {
    fn label(&self) -> &str {
        "test"
    }
    fn list(&self, dir: &[String]) -> Result<Vec<HostEntry>, HostDirError> {
        let files = self.files.lock().unwrap();
        let dirs = self.dirs.lock().unwrap();
        let mut rows = BTreeMap::new();
        for path in dirs.iter() {
            if path.len() == dir.len() + 1 && path.starts_with(dir) {
                rows.insert(
                    path.last().unwrap().clone(),
                    HostEntry {
                        name: path.last().unwrap().clone(),
                        kind: sim_storage_port::HostEntryKind::Directory,
                        len: 0,
                    },
                );
            }
        }
        for (path, bytes) in files.iter() {
            if path.len() == dir.len() + 1 && path.starts_with(dir) {
                rows.insert(
                    path.last().unwrap().clone(),
                    HostEntry {
                        name: path.last().unwrap().clone(),
                        kind: sim_storage_port::HostEntryKind::File,
                        len: bytes.len() as u64,
                    },
                );
            }
        }
        Ok(rows.into_values().collect())
    }
    fn metadata(&self, path: &[String]) -> Result<Option<HostEntry>, HostDirError> {
        Ok(self.files.lock().unwrap().get(path).map(|v| HostEntry {
            name: path.last().unwrap().clone(),
            kind: sim_storage_port::HostEntryKind::File,
            len: v.len() as u64,
        }))
    }
    fn read(&self, path: &[String]) -> Result<Vec<u8>, HostDirError> {
        self.files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| HostDirError::new(HostDirErrorKind::NotFound, "absent"))
    }
    fn replace(
        &self,
        path: &[String],
        bytes: &[u8],
        _: &dyn Cancellation,
    ) -> Result<(), HostDirError> {
        self.files
            .lock()
            .unwrap()
            .insert(path.to_vec(), bytes.to_vec());
        Ok(())
    }
    fn compare_exchange(
        &self,
        path: &[String],
        expected: Option<&[u8]>,
        replacement: Option<&[u8]>,
        _: &dyn Cancellation,
    ) -> Result<HostCompareExchange, HostDirError> {
        let mut files = self.files.lock().unwrap();
        let observed = files.get(path).cloned();
        let exchanged = observed.as_deref() == expected;
        if exchanged {
            match replacement {
                Some(v) => {
                    files.insert(path.to_vec(), v.to_vec());
                }
                None => {
                    files.remove(path);
                }
            }
        }
        Ok(HostCompareExchange {
            exchanged,
            observed,
        })
    }
    fn remove_file(&self, path: &[String]) -> Result<(), HostDirError> {
        self.files.lock().unwrap().remove(path);
        Ok(())
    }
    fn create_dir(&self, path: &[String]) -> Result<(), HostDirError> {
        self.dirs.lock().unwrap().insert(path.to_vec());
        Ok(())
    }
    fn remove_dir_all(&self, _: &[String]) -> Result<(), HostDirError> {
        Ok(())
    }
    fn child(&self, _: &str) -> Result<Arc<dyn HostDirPort>, HostDirError> {
        Err(HostDirError::new(HostDirErrorKind::Unsupported, "unused"))
    }
}

fn capabilities() -> BackendCapabilities {
    BackendCapabilities {
        linearizable_cas: true,
        durable_publish: true,
    }
}

#[test]
fn host_backend_refuses_unsafe_writes_and_read_open_is_bounded() {
    let port = Arc::new(TestPort::default());
    let unsafe_backend = HostDirJournalBackend::open(
        port.clone(),
        BackendCapabilities {
            linearizable_cas: false,
            durable_publish: true,
        },
        10,
    )
    .unwrap();
    assert!(matches!(
        unsafe_backend.acquire_lease(),
        Err(JournalError::WriteRefused(_))
    ));
    assert!(matches!(
        HostDirJournalBackend::open(port, capabilities(), 0),
        Err(JournalError::WorkBoundExceeded)
    ));
}

#[test]
fn host_failpoints_reopen_to_old_or_new_head_and_verify_content() {
    for point in [
        Failpoint::BeforeObjectPublish,
        Failpoint::AfterObjectPublish,
        Failpoint::AfterDurabilityReceipt,
        Failpoint::BeforeCas,
        Failpoint::AfterCas,
        Failpoint::BeforeAcknowledgement,
    ] {
        let port = Arc::new(TestPort::default());
        let backend = HostDirJournalBackend::open(port.clone(), capabilities(), 20).unwrap();
        let lease = backend.acquire_lease().unwrap();
        let crashing = Journal::new(backend.with_failpoint_hook(move |seen| seen == point));
        let payload = object(7);
        let fact = entry(0, None, &payload);
        assert!(matches!(
            crashing.publish(&lease, None, vec![payload], vec![fact]),
            Err(JournalError::InjectedCrash(_))
        ));
        let reopened = Journal::new(HostDirJournalBackend::open(port, capabilities(), 20).unwrap());
        let head = reopened.head().unwrap();
        if matches!(
            point,
            Failpoint::AfterCas | Failpoint::BeforeAcknowledgement
        ) {
            assert!(head.is_some());
            assert_eq!(reopened.verify().unwrap().entries.len(), 1)
        } else {
            assert!(head.is_none())
        }
        let lease = reopened.acquire_lease().unwrap();
        let prior = reopened.head().unwrap();
        let progress = object(8);
        let next = entry(
            prior.as_ref().map_or(0, |value| value.sequence + 1),
            prior.as_ref().map(|value| value.entry.clone()),
            &progress,
        );
        reopened
            .publish(&lease, prior.as_ref(), vec![progress], vec![next])
            .unwrap();
        assert_eq!(
            reopened.replay().unwrap().count(),
            usize::from(head.is_some()) + 1
        );
    }
}

#[test]
fn host_put_if_absent_contention_projection_and_corruption_laws() {
    let port = Arc::new(TestPort::default());
    let a = HostDirJournalBackend::open(port.clone(), capabilities(), 20).unwrap();
    let b = HostDirJournalBackend::open(port.clone(), capabilities(), 20).unwrap();
    let lease_a = a.acquire_lease().unwrap();
    let lease_b = b.acquire_lease().unwrap();
    let one = object(1);
    let two = object(2);
    assert_eq!(
        a.admit(Admission {
            fence: lease_a.fence(),
            expected: None,
            objects: vec![one.clone()],
            entries: vec![entry(0, None, &one)]
        }),
        Err(JournalError::StaleLease)
    );
    b.admit(Admission {
        fence: lease_b.fence(),
        expected: None,
        objects: vec![two.clone()],
        entries: vec![entry(0, None, &two)],
    })
    .unwrap();
    let reopened =
        Journal::new(HostDirJournalBackend::open(port.clone(), capabilities(), 20).unwrap());
    assert!(!reopened.table_projection().unwrap().rows().is_empty());
    assert!(!reopened.dir_projection().unwrap().journal().is_empty());
    port.corrupt_one_byte();
    assert!(HostDirJournalBackend::open(port, capabilities(), 20).is_err());
}

#[test]
fn entry_and_payload_ids_are_kernel_datum_identities() {
    let payload = JournalObject::from_bytes(b"semantic bytes".to_vec());
    let entry = JournalEntry::new(
        0,
        None,
        Symbol::qualified("example", "semantic"),
        vec![payload.id.clone()],
    );
    assert_eq!(payload.id.algorithm, datum_content_algorithm());
    assert_eq!(entry.id.algorithm, datum_content_algorithm());
    assert_eq!(entry.id, entry.canonical_datum().content_id().unwrap());
    assert_ne!(
        payload.id,
        ContentId::from_bytes(
            Symbol::qualified("journal", "sha256-storage-v1"),
            Sha256::digest(payload.storage_bytes().unwrap()).into(),
        )
    );
}

#[test]
fn host_persistent_index_is_disposable_and_rebuildable() {
    let port = Arc::new(TestPort::default());
    let backend = HostDirJournalBackend::open(port, capabilities(), 20).unwrap();
    let mut store = PersistentObjectStore::open(backend).unwrap();
    let datum = Datum::String("persistent evidence root".into());
    let reference = store.put(datum.clone()).unwrap();
    assert_eq!(store.rebuild_index().unwrap(), vec![reference.clone()]);
    assert_eq!(store.get(&reference.meaning).unwrap(), datum);
}

mod native;
