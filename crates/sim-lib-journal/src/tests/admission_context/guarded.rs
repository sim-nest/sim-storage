// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
use sim_storage_port::NeverCancel;

#[test]
fn guarded_cut_preserves_committed_data_and_exact_action_order() {
    for phase in [
        Failpoint::BeforeCas,
        Failpoint::AfterCas,
        Failpoint::BeforeAcknowledgement,
    ] {
        let port = Arc::new(TestPort {
            guarded: true,
            ..TestPort::default()
        });
        let observed_port = Arc::clone(&port);
        let fired = Arc::new(AtomicBool::new(false));
        let selected_fired = Arc::clone(&fired);
        let owner = Arc::new(backend(Arc::clone(&port)).with_admission_failpoint_hook(
            move |seen, admission| {
                if seen != phase {
                    return false;
                }
                assert_eq!(admission.entries.len(), 1);
                assert_eq!(admission.entries[0].kind, selected());
                if phase == Failpoint::AfterCas {
                    assert!(
                        matches!(
                            observed_port.admission.try_lock(),
                            Err(std::sync::TryLockError::WouldBlock)
                        ),
                        "AfterCas must remain inside original guarded exclusion"
                    );
                } else {
                    assert!(observed_port.admission.try_lock().is_ok());
                }
                assert!(!selected_fired.swap(true, Ordering::SeqCst));
                true
            },
        ));
        let journal = Journal::new(Arc::clone(&owner));
        let lease = journal.acquire_lease().unwrap();
        let payload = object(1);
        let target = fact(0, None, selected(), &payload);
        // A real callback write through the existing modeled port, not a
        // fabricated native stop. No filesystem/native durability is claimed.
        let effect = vec!["action-effect".to_owned()];
        let mut action = || {
            port.replace(&effect, b"action actually entered", &NeverCancel)
                .unwrap()
        };
        assert_eq!(
            journal.publish_then(
                &lease,
                None,
                vec![payload.clone()],
                vec![target.clone()],
                &mut action
            ),
            Err(JournalError::InjectedCrash(phase.label())),
        );
        assert!(fired.load(Ordering::SeqCst));
        let action_entered = phase == Failpoint::BeforeAcknowledgement;
        match port.read(&effect) {
            Ok(bytes) => {
                assert!(action_entered);
                assert_eq!(bytes, b"action actually entered");
            }
            Err(error) => {
                assert!(!action_entered);
                assert_eq!(error.kind, HostDirErrorKind::NotFound);
            }
        }
        let reopened = Journal::new(backend(Arc::clone(&port)));
        let committed = phase != Failpoint::BeforeCas;
        assert_eq!(
            reopened.verified_snapshot().unwrap().entries().len(),
            usize::from(committed)
        );
        if committed {
            let result = reopened
                .publish_then(&lease, None, vec![payload], vec![target], &mut || {
                    panic!("redelivery repeated/supplied a missing action")
                })
                .unwrap();
            assert_eq!(
                result.disposition,
                AdmissionDisposition::AlreadyCommittedActionNotInvoked
            );
        }
    }
}
