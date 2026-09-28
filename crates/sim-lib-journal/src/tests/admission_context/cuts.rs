// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

#[test]
fn one_shot_exact_kind_selection_preserves_before_and_after_commit_truth() {
    for phase in [
        Failpoint::BeforeCas,
        Failpoint::AfterCas,
        Failpoint::BeforeAcknowledgement,
    ] {
        let port = Arc::new(TestPort::default());
        let armed = Arc::new(AtomicBool::new(true));
        let selected_arm = Arc::clone(&armed);
        let fired = Arc::new(Mutex::new(Vec::new()));
        let selected_fired = Arc::clone(&fired);
        let owner = Arc::new(backend(Arc::clone(&port)).with_admission_failpoint_hook(
            move |seen, admission| {
                if seen == phase
                    && admission
                        .entries
                        .iter()
                        .any(|entry| entry.kind == selected())
                    && selected_arm.swap(false, Ordering::SeqCst)
                {
                    selected_fired
                        .try_lock()
                        .unwrap()
                        .push(Seen::capture(seen, admission));
                    true
                } else {
                    false
                }
            },
        ));
        let journal = Journal::new(Arc::clone(&owner));
        let lease = journal.acquire_lease().unwrap();
        let other = object(1);
        let unrelated = fact(0, None, Symbol::qualified("other", "selected"), &other);
        let previous = journal
            .publish(&lease, None, vec![other], vec![unrelated])
            .unwrap();
        assert!(
            armed.load(Ordering::SeqCst),
            "similar leaf in another namespace cannot consume the cut"
        );
        assert!(fired.lock().unwrap().is_empty());
        let payload = object(2);
        let target = fact(1, Some(previous.entry.clone()), selected(), &payload);
        let before = owner.state_envelope().unwrap().unwrap();
        let failure = journal.publish(
            &lease,
            Some(&previous),
            vec![payload.clone()],
            vec![target.clone()],
        );
        assert_eq!(failure, Err(JournalError::InjectedCrash(phase.label())));
        let seen = fired.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].phase, phase);
        assert_eq!(seen[0].fence, lease.fence());
        assert_eq!(seen[0].expected, Some(previous.clone()));
        assert_eq!(seen[0].entries, vec![target.clone()]);
        assert_eq!(seen[0].objects, vec![payload.clone()]);
        drop(seen);
        let reopened = backend(Arc::clone(&port));
        let state = reopened.state_envelope().unwrap().unwrap();
        assert_eq!(state.fence, before.fence);
        assert_eq!(state.namespace, before.namespace);
        let committed = phase != Failpoint::BeforeCas;
        assert_eq!(
            Journal::new(reopened)
                .verified_snapshot()
                .unwrap()
                .entries()
                .len(),
            if committed { 2 } else { 1 }
        );
        assert_eq!(
            state.head.as_ref().unwrap().entry,
            if committed {
                target.id.clone()
            } else {
                previous.entry.clone()
            }
        );

        let acknowledged = journal
            .publish(&lease, Some(&previous), vec![payload], vec![target.clone()])
            .unwrap();
        assert_eq!(acknowledged.entry, target.id);
        let last_object = object(3);
        let last = fact(2, Some(target.id), selected(), &last_object);
        journal
            .publish(&lease, Some(&acknowledged), vec![last_object], vec![last])
            .unwrap();
        assert_eq!(
            fired.lock().unwrap().len(),
            1,
            "one-shot consumption belongs to the explicit callback"
        );
        assert_eq!(journal.verified_snapshot().unwrap().entries().len(), 3);
    }
}

#[test]
fn committed_confirmation_does_not_fire_a_newly_selected_cut_or_repeat_action() {
    let port = Arc::new(TestPort {
        guarded: true,
        ..TestPort::default()
    });
    let original = Journal::new(backend(Arc::clone(&port)));
    let lease = original.acquire_lease().unwrap();
    let payload = object(7);
    let target = fact(0, None, selected(), &payload);
    let committed = original
        .publish(&lease, None, vec![payload.clone()], vec![target.clone()])
        .unwrap();
    let armed = Arc::new(AtomicBool::new(true));
    let selected_arm = Arc::clone(&armed);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let owner = Arc::new(backend(Arc::clone(&port)).with_admission_failpoint_hook(
        move |phase, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            phase == Failpoint::BeforeAcknowledgement && selected_arm.swap(false, Ordering::SeqCst)
        },
    ));
    let state = owner.state_envelope().unwrap();
    let confirmed = Journal::new(Arc::clone(&owner))
        .publish_then(&lease, None, vec![payload], vec![target], &mut || {
            panic!("confirmation replayed action")
        })
        .unwrap();
    assert_eq!(confirmed.head, committed);
    assert_eq!(
        confirmed.disposition,
        AdmissionDisposition::AlreadyCommittedActionNotInvoked
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        armed.load(Ordering::SeqCst),
        "confirmed historical DATA is a MISSED cut, not an injected failure"
    );
    assert_eq!(owner.state_envelope().unwrap(), state);
    let next_object = object(8);
    let next = fact(1, Some(committed.entry.clone()), selected(), &next_object);
    assert_eq!(
        Journal::new(owner).publish(&lease, Some(&committed), vec![next_object], vec![next]),
        Err(JournalError::InjectedCrash("before-acknowledgement")),
    );
    assert_eq!(calls.load(Ordering::SeqCst), PHASES.len());
    assert!(!armed.load(Ordering::SeqCst));
    assert_eq!(
        Journal::new(backend(port))
            .verified_snapshot()
            .unwrap()
            .entries()
            .len(),
        2
    );
}

#[test]
fn persistent_context_hook_is_not_implicitly_consumed_by_backend() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let journal = Journal::new(
        backend(Arc::new(TestPort::default())).with_admission_failpoint_hook(move |phase, _| {
            if phase == Failpoint::BeforeCas {
                seen.fetch_add(1, Ordering::SeqCst);
                true
            } else {
                false
            }
        }),
    );
    let lease = journal.acquire_lease().unwrap();
    let payload = object(1);
    let target = fact(0, None, selected(), &payload);
    for _ in 0..2 {
        assert_eq!(
            journal.publish(&lease, None, vec![payload.clone()], vec![target.clone()]),
            Err(JournalError::InjectedCrash("before-cas")),
        );
        assert!(journal.head().unwrap().is_none());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
