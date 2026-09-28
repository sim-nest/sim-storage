// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use std::sync::mpsc;
use std::time::Duration;

fn backends() -> Vec<Arc<dyn JournalBackend>> {
    vec![
        Arc::new(MemoryBackend::new()),
        Arc::new(
            HostDirJournalBackend::open(
                Arc::new(TestPort {
                    guarded: true,
                    ..TestPort::default()
                }),
                capabilities(),
                100,
            )
            .unwrap(),
        ),
    ]
}

#[test]
fn equivalent_unordered_object_redelivery_matches_on_every_backend() {
    for backend in backends() {
        let a = Datum::Node {
            tag: Symbol::qualified("fixture", "canonical-object"),
            fields: vec![
                (
                    Symbol::new("z"),
                    Datum::Set(vec![Datum::Bool(true), Datum::Bool(false)]),
                ),
                (Symbol::new("a"), Datum::Nil),
            ],
        };
        let b = Datum::Node {
            tag: Symbol::qualified("fixture", "canonical-object"),
            fields: vec![
                (Symbol::new("a"), Datum::Nil),
                (
                    Symbol::new("z"),
                    Datum::Set(vec![Datum::Bool(false), Datum::Bool(true)]),
                ),
            ],
        };
        let first = backend
            .put_datum(JournalObject::from_datum(a.clone()).unwrap())
            .unwrap();
        let repeated = backend
            .put_datum(JournalObject::from_datum(b).unwrap())
            .unwrap();
        assert_eq!(first, repeated);
        assert_eq!(
            backend
                .get_datum(&first.meaning)
                .unwrap()
                .canonical_bytes()
                .unwrap(),
            a.canonical_bytes().unwrap()
        );
    }
}

#[test]
fn historical_redelivery_requires_current_fence_and_never_repeats_action() {
    for backend in backends() {
        let journal = Journal::new(backend);
        let writer = journal.acquire_lease().unwrap();
        let a = object(1);
        let first = entry(0, None, &a);
        let head = journal
            .publish_then(
                &writer,
                None,
                vec![a.clone()],
                vec![first.clone()],
                &mut || {},
            )
            .unwrap()
            .head;
        let b = object(2);
        let next = entry(1, Some(head.entry.clone()), &b);
        let current = journal
            .publish(&writer, Some(&head), vec![b], vec![next])
            .unwrap();
        let delivered = journal
            .publish_then(
                &writer,
                None,
                vec![a.clone()],
                vec![first.clone()],
                &mut || panic!("historical action repeated"),
            )
            .unwrap();
        assert_eq!(delivered.head, current);
        assert_eq!(
            delivered.disposition,
            AdmissionDisposition::AlreadyCommittedActionNotInvoked
        );
        let fresh = journal.acquire_lease().unwrap();
        assert_eq!(
            journal.publish_then(
                &writer,
                None,
                vec![a.clone()],
                vec![first.clone()],
                &mut || panic!("stale action")
            ),
            Err(JournalError::StaleLease)
        );
        let restored = journal
            .publish_then(&fresh, None, vec![a], vec![first], &mut || {
                panic!("reopen repeated action")
            })
            .unwrap();
        assert_eq!(restored.head, current);
        assert_eq!(
            restored.disposition,
            AdmissionDisposition::AlreadyCommittedActionNotInvoked
        );
        assert_eq!(journal.verified_snapshot().unwrap().entries().len(), 2);
    }
}

#[test]
fn guarded_commit_is_readable_and_redelivery_never_repeats_the_action() {
    for backend in backends() {
        let journal = Journal::new(backend);
        let lease = journal.acquire_lease().unwrap();
        let a = object(1);
        let fact = entry(0, None, &a);
        let mut calls = 0;
        let mut action = || {
            calls += 1;
            assert_eq!(
                journal.verified_snapshot().unwrap().entries(),
                std::slice::from_ref(&fact)
            );
        };
        let first = journal
            .publish_then(
                &lease,
                None,
                vec![a.clone()],
                vec![fact.clone()],
                &mut action,
            )
            .unwrap();
        assert_eq!(
            first.disposition,
            AdmissionDisposition::CommittedActionInvoked
        );
        let again = journal
            .publish_then(&lease, None, vec![a], vec![fact], &mut || {
                panic!("redelivery repeated the action");
            })
            .unwrap();
        assert_eq!(again.head, first.head);
        assert_eq!(
            again.disposition,
            AdmissionDisposition::AlreadyCommittedActionNotInvoked
        );
        assert_eq!(calls, 1);
    }
}

#[test]
fn superseded_fence_cannot_commit_or_invoke_action() {
    for backend in backends() {
        let journal = Journal::new(backend);
        let stale = journal.acquire_lease().unwrap();
        let _live = journal.acquire_lease().unwrap();
        let a = object(1);
        let fact = entry(0, None, &a);
        let result = journal.publish_then(&stale, None, vec![a], vec![fact], &mut || {
            panic!("stale fence invoked action");
        });
        assert_eq!(result, Err(JournalError::StaleLease));
        assert_eq!(journal.head().unwrap(), None);
    }
}

#[test]
fn guarded_action_excludes_competing_lease_until_it_returns() {
    for backend in backends() {
        let journal = Journal::new(backend.clone());
        let lease = journal.acquire_lease().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let a = object(1);
        let fact = entry(0, None, &a);
        let mut worker = None;
        journal
            .publish_then(&lease, None, vec![a], vec![fact], &mut || {
                let backend = backend.clone();
                let started = started_tx.clone();
                let done = done_tx.clone();
                worker = Some(std::thread::spawn(move || {
                    started.send(()).unwrap();
                    done.send(backend.acquire_lease()).unwrap();
                }));
                started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                assert_eq!(
                    done_rx.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout),
                    "lease advanced before guarded action returned"
                );
                assert_eq!(journal.head().unwrap().unwrap().sequence, 0);
            })
            .unwrap();
        assert!(
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_ok()
        );
        worker.unwrap().join().unwrap();
    }
}

#[test]
fn committed_intent_survives_action_or_acknowledgement_loss_without_replay() {
    for (cut, expected_calls) in [
        (Failpoint::AfterCas, 0),
        (Failpoint::BeforeAcknowledgement, 1),
    ] {
        let port = Arc::new(TestPort {
            guarded: true,
            ..TestPort::default()
        });
        let backend = HostDirJournalBackend::open(port.clone(), capabilities(), 100)
            .unwrap()
            .with_failpoint_hook(move |point| point == cut);
        let journal = Journal::new(backend);
        let lease = journal.acquire_lease().unwrap();
        let a = object(1);
        let fact = entry(0, None, &a);
        let mut calls = 0;
        assert!(matches!(
            journal.publish_then(
                &lease,
                None,
                vec![a.clone()],
                vec![fact.clone()],
                &mut || {
                    calls += 1;
                }
            ),
            Err(JournalError::InjectedCrash(_))
        ));
        assert_eq!(calls, expected_calls);
        let reopened =
            Journal::new(HostDirJournalBackend::open(port, capabilities(), 100).unwrap());
        assert_eq!(reopened.head().unwrap().unwrap().sequence, 0);
        let replay = reopened
            .publish_then(&lease, None, vec![a], vec![fact], &mut || {
                panic!("uncertain committed intent was replayed");
            })
            .unwrap();
        assert_eq!(
            replay.disposition,
            AdmissionDisposition::AlreadyCommittedActionNotInvoked
        );
    }
}

#[test]
fn unsupported_guard_refuses_before_publishing_any_input() {
    let port = Arc::new(TestPort::default());
    let journal =
        Journal::new(HostDirJournalBackend::open(port.clone(), capabilities(), 100).unwrap());
    let lease = journal.acquire_lease().unwrap();
    let before = port.files.lock().unwrap().clone();
    let a = object(1);
    let fact = entry(0, None, &a);
    assert_eq!(
        journal.publish_then(&lease, None, vec![a], vec![fact], &mut || panic!(
            "unsupported action"
        )),
        Err(JournalError::WriteRefused(
            "guarded host admission unsupported"
        ))
    );
    assert_eq!(*port.files.lock().unwrap(), before);
}

impl TestPort {
    pub(super) fn exchange(
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
}
