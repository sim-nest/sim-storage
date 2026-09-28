// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Actual HostDirJournalBackend/codec/CAS paths over the existing modeled TestPort.
//! This is storage component evidence, not native filesystem or M5 qualification.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod cuts;
mod guarded;

const PHASES: [Failpoint; 6] = [
    Failpoint::BeforeObjectPublish,
    Failpoint::AfterObjectPublish,
    Failpoint::AfterDurabilityReceipt,
    Failpoint::BeforeCas,
    Failpoint::AfterCas,
    Failpoint::BeforeAcknowledgement,
];

fn backend(port: Arc<TestPort>) -> HostDirJournalBackend {
    HostDirJournalBackend::open(port, capabilities(), 100).unwrap()
}

fn fact(
    sequence: u64,
    previous: Option<ContentId>,
    kind: Symbol,
    object: &JournalObject,
) -> JournalEntry {
    JournalEntry::new(sequence, previous, kind, vec![object.id.clone()])
}

fn selected() -> Symbol {
    Symbol::qualified("specimen", "selected")
}

#[derive(Debug)]
struct Seen {
    phase: Failpoint,
    fence: u64,
    expected: Option<JournalHead>,
    objects: Vec<JournalObject>,
    entries: Vec<JournalEntry>,
}

impl Seen {
    fn capture(phase: Failpoint, admission: &Admission) -> Self {
        Self {
            phase,
            fence: admission.fence,
            expected: admission.expected.clone(),
            objects: admission.objects.clone(),
            entries: admission.entries.clone(),
        }
    }
}

#[test]
fn context_borrows_the_complete_original_batch_at_all_existing_cuts() {
    let port = Arc::new(TestPort::default());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&seen);
    let owner = Arc::new(
        backend(port).with_admission_failpoint_hook(move |phase, admission| {
            observed
                .try_lock()
                .unwrap()
                .push(Seen::capture(phase, admission));
            false
        }),
    );
    let journal = Journal::new(Arc::clone(&owner));
    let lease = journal.acquire_lease().unwrap();
    owner.put_datum(object(99)).unwrap();
    assert!(
        seen.lock().unwrap().is_empty(),
        "lease and standalone object writes are not admission cuts"
    );
    let first_object = object(1);
    let first = entry(0, None, &first_object);
    let previous = journal
        .publish(&lease, None, vec![first_object], vec![first])
        .unwrap();
    seen.lock().unwrap().clear();

    let objects = vec![JournalObject::from_bytes(vec![0, 255, 128]), object(3)];
    let a = fact(1, Some(previous.entry.clone()), selected(), &objects[0]);
    let b = fact(
        2,
        Some(a.id.clone()),
        Symbol::qualified("other", "selected"),
        &objects[1],
    );
    let entries = vec![a, b];
    let head = journal
        .publish(&lease, Some(&previous), objects.clone(), entries.clone())
        .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.iter().map(|value| value.phase).collect::<Vec<_>>(),
        PHASES
    );
    for value in seen.iter() {
        assert_eq!(value.fence, lease.fence());
        assert_eq!(value.expected, Some(previous.clone()));
        assert_eq!(value.objects, objects);
        assert_eq!(value.entries, entries);
    }
    assert_eq!(head.entry, entries[1].id);
    assert_eq!(journal.verified_snapshot().unwrap().entries().len(), 3);
}

#[test]
fn existing_hook_runs_first_and_refusal_suppresses_only_the_same_context_cut() {
    for refuse in [false, true] {
        let order = Arc::new(Mutex::new(Vec::new()));
        let legacy = Arc::clone(&order);
        let context = Arc::clone(&order);
        let owner = backend(Arc::new(TestPort::default()))
            .with_failpoint_hook(move |phase| {
                if PHASES.contains(&phase) {
                    legacy.try_lock().unwrap().push((0, phase));
                }
                refuse && phase == Failpoint::BeforeCas
            })
            .with_admission_failpoint_hook(move |phase, _| {
                context.try_lock().unwrap().push((1, phase));
                false
            });
        let journal = Journal::new(owner);
        let lease = journal.acquire_lease().unwrap();
        let payload = object(1);
        let fact = entry(0, None, &payload);
        let result = journal.publish(&lease, None, vec![payload], vec![fact]);
        let mut expected = Vec::new();
        for phase in PHASES {
            expected.push((0, phase));
            if refuse && phase == Failpoint::BeforeCas {
                break;
            }
            expected.push((1, phase));
        }
        assert_eq!(*order.lock().unwrap(), expected);
        if refuse {
            assert_eq!(result, Err(JournalError::InjectedCrash("before-cas")));
            assert!(journal.head().unwrap().is_none());
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn stale_original_writer_refuses_before_context_hook_and_changes_no_state() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let owner = Arc::new(
        backend(Arc::new(TestPort::default())).with_admission_failpoint_hook(move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            false
        }),
    );
    let journal = Journal::new(Arc::clone(&owner));
    let stale = journal.acquire_lease().unwrap();
    let live = journal.acquire_lease().unwrap();
    let before = owner.state_envelope().unwrap();
    let payload = object(1);
    let fact = fact(0, None, selected(), &payload);
    assert_eq!(
        journal.publish(&stale, None, vec![payload.clone()], vec![fact.clone()]),
        Err(JournalError::StaleLease),
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(owner.state_envelope().unwrap(), before);
    journal
        .publish(&live, None, vec![payload], vec![fact])
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), PHASES.len());
}
