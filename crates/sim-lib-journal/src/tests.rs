use crate::*;
use sim_kernel::{ContentId, Symbol};
use std::sync::Arc;

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
    let corrupt = JournalObject {
        id: good.id.clone(),
        bytes: vec![2],
    };
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
