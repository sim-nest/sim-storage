// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Immutable publication and guarded head admission.

use super::*;
use crate::{AdmissionDisposition, CommitAction, GuardedAdmission};

impl HostDirJournalBackend {
    pub(super) fn admit_with_action(
        &self,
        admission: Admission,
        action: Option<&mut dyn CommitAction>,
    ) -> Result<GuardedAdmission, JournalError> {
        if action.is_some() && !self.port.supports_guarded_compare_exchange() {
            return Err(JournalError::WriteRefused(
                "guarded host admission unsupported",
            ));
        }
        self.write_capable()?;
        self.ensure_layout()?;
        let observed = self.state_bytes()?.ok_or(JournalError::WriteRefused(
            "acquire a v2 lease before append",
        ))?;
        if observed.starts_with(b"SIMJSTATE1") {
            return Err(JournalError::WriteRefused(
                "acquire a v2 lease before append",
            ));
        }
        let mut envelope = decode_envelope(&observed)?;
        if envelope.fence != admission.fence {
            return Err(JournalError::StaleLease);
        }
        if envelope.head != admission.expected {
            let state = self.read_v2(&envelope)?;
            if admission.entries.is_empty()
                || !admission
                    .entries
                    .iter()
                    .all(|entry| state.entries.get(&entry.sequence) == Some(entry))
            {
                return Err(JournalError::WrongHead);
            }
            // Only the committed, verified prefix proves historical admission.
            // Physical orphan leaves cannot authorize redelivery. Confirm the
            // same envelope/fence atomically; never replay its external action.
            let confirmation = self
                .port
                .compare_exchange(
                    &["state".into()],
                    Some(&observed),
                    Some(&observed),
                    &NeverCancel,
                )
                .map_err(port_error)?;
            if !confirmation.exchanged {
                return Err(JournalError::WrongHead);
            }
            return Ok(GuardedAdmission {
                head: envelope.head.ok_or(JournalError::WrongHead)?,
                disposition: AdmissionDisposition::AlreadyCommittedActionNotInvoked,
            });
        }
        self.trip_admission(Failpoint::BeforeObjectPublish, &admission)?;
        let mut references = BTreeMap::new();
        for object in &admission.objects {
            let reference = self.put_object(object)?;
            references.insert(reference.meaning.clone(), reference);
        }
        self.trip_admission(Failpoint::AfterObjectPublish, &admission)?;
        let mut previous_location = envelope.head_location.clone();
        let mut last_location = None;
        for entry in &admission.entries {
            let mut payloads = Vec::with_capacity(entry.payloads.len());
            for meaning in &entry.payloads {
                let reference = match references.get(meaning) {
                    Some(reference) => reference.clone(),
                    None => self.find_object(meaning)?.0,
                };
                payloads.push(reference);
            }
            let physical = PhysicalEntry {
                entry: entry.clone(),
                previous_location: previous_location.clone(),
                payloads,
            };
            let bytes = encode_v2_entry(&physical);
            let storage = crate::object::storage_id(&bytes);
            let location = EntryLocation {
                namespace: envelope.namespace.clone(),
                sequence: entry.sequence,
                entry: entry.id.clone(),
                storage,
            };
            ensure_entry_dirs(self.port.as_ref(), &location)?;
            self.put_immutable(&entry_path(&location), &bytes)?;
            previous_location = Some(location.clone());
            last_location = Some(location);
        }
        self.trip_admission(Failpoint::AfterDurabilityReceipt, &admission)?;
        let last = admission.entries.last().ok_or(JournalError::EmptyBatch)?;
        let new_head = JournalHead {
            sequence: last.sequence,
            entry: last.id.clone(),
        };
        envelope.head = Some(new_head.clone());
        envelope.head_location = last_location;
        self.trip_admission(Failpoint::BeforeCas, &admission)?;
        let replacement = encode_envelope(&envelope);
        let mut post_commit_error = None;
        let result = if let Some(action) = action {
            let mut called = false;
            let mut continuation = || {
                if called {
                    return;
                }
                called = true;
                post_commit_error = self.trip_admission(Failpoint::AfterCas, &admission).err();
                if post_commit_error.is_none() {
                    action.after_commit();
                }
            };
            let result = self
                .port
                .compare_exchange_then(
                    &["state".into()],
                    Some(&observed),
                    Some(&replacement),
                    &NeverCancel,
                    &mut continuation,
                )
                .map_err(port_error)?;
            if result.exchanged && !called {
                return Err(JournalError::WriteRefused(
                    "guarded commit acknowledgement is incomplete",
                ));
            }
            result
        } else {
            let result = self
                .port
                .compare_exchange(
                    &["state".into()],
                    Some(&observed),
                    Some(&replacement),
                    &NeverCancel,
                )
                .map_err(port_error)?;
            if result.exchanged {
                self.trip_admission(Failpoint::AfterCas, &admission)?;
            }
            result
        };
        if !result.exchanged {
            return Err(JournalError::WrongHead);
        }
        if let Some(error) = post_commit_error {
            return Err(error);
        }
        self.trip_admission(Failpoint::BeforeAcknowledgement, &admission)?;
        Ok(GuardedAdmission {
            head: new_head,
            disposition: AdmissionDisposition::CommittedActionInvoked,
        })
    }
}
