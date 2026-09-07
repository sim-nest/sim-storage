use crate::native_codec::*;
use crate::{
    Admission, JournalBackend, JournalEntry, JournalError, JournalHead, JournalObject, Lease,
    StoredDatumRef, StoredState,
};
use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Datum};
use sim_storage_port::{HostDirErrorKind, HostDirPort, NeverCancel};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

type FailHook = Arc<dyn Fn(Failpoint) -> bool + Send + Sync>;
static NAMESPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Crash-durable journal composition over the canonical host-backed Table/Dir port.
///
/// V1 storage is a read-only compatibility prefix. The first v2 lease verifies
/// that prefix and atomically selects its descriptor, a higher fence, and a
/// fresh namespace. Every later append publishes immutable content-addressed
/// leaves before comparing the complete state envelope.
pub struct HostDirJournalBackend {
    port: Arc<dyn HostDirPort>,
    capabilities: BackendCapabilities,
    work_bound: usize,
    fail: Option<FailHook>,
}

impl HostDirJournalBackend {
    /// Opens a binding and verifies the complete committed closure.
    pub fn open(
        port: Arc<dyn HostDirPort>,
        capabilities: BackendCapabilities,
        work_bound: usize,
    ) -> Result<Self, JournalError> {
        if work_bound == 0 {
            return Err(JournalError::WorkBoundExceeded);
        }
        let backend = Self {
            port,
            capabilities,
            work_bound,
            fail: None,
        };
        backend.read_state()?;
        Ok(backend)
    }

    /// Installs a deterministic crash hook for conformance models.
    pub fn with_failpoint_hook(
        mut self,
        hook: impl Fn(Failpoint) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.fail = Some(Arc::new(hook));
        self
    }

    /// Reports the capability evidence used to admit or refuse writes.
    pub fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
    }

    /// Returns the selected v2 state envelope, if migration has occurred.
    pub fn state_envelope(&self) -> Result<Option<NativeStateEnvelope>, JournalError> {
        let Some(bytes) = self.state_bytes()? else {
            return Ok(None);
        };
        if bytes.starts_with(b"SIMJSTATE1") {
            return Ok(None);
        }
        Ok(Some(decode_envelope(&bytes)?))
    }

    fn trip(&self, point: Failpoint) -> Result<(), JournalError> {
        if self.fail.as_ref().is_some_and(|hook| hook(point)) {
            Err(JournalError::InjectedCrash(point.label()))
        } else {
            Ok(())
        }
    }

    fn write_capable(&self) -> Result<(), JournalError> {
        if !self.capabilities.linearizable_cas {
            return Err(JournalError::WriteRefused(
                "linearizable table/cas unavailable",
            ));
        }
        if !self.capabilities.durable_publish {
            return Err(JournalError::WriteRefused(
                "storage durability receipt unavailable",
            ));
        }
        Ok(())
    }

    fn ensure_layout(&self) -> Result<(), JournalError> {
        for path in [
            vec!["objects-v2".into()],
            vec!["namespaces-v2".into()],
            vec!["compat-v1".into()],
            vec!["temporary".into()],
        ] {
            self.port.create_dir(&path).map_err(port_error)?;
        }
        Ok(())
    }

    fn state_bytes(&self) -> Result<Option<Vec<u8>>, JournalError> {
        match self.port.read(&["state".into()]) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind == HostDirErrorKind::NotFound => Ok(None),
            Err(error) => Err(port_error(error)),
        }
    }

    fn put_immutable(&self, path: &[String], bytes: &[u8]) -> Result<(), JournalError> {
        let outcome = self
            .port
            .compare_exchange(path, None, Some(bytes), &NeverCancel)
            .map_err(port_error)?;
        if outcome.exchanged || outcome.observed.as_deref() == Some(bytes) {
            Ok(())
        } else {
            Err(JournalError::ConflictingObject)
        }
    }

    fn load(&self, path: &[String]) -> Result<Vec<u8>, JournalError> {
        self.port.read(path).map_err(port_error)
    }

    fn reserve_namespace(&self, seed: &[u8]) -> Result<EntryNamespace, JournalError> {
        for _ in 0..128 {
            let nonce = NAMESPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| JournalError::Backend("system time before epoch".into()))?
                .as_nanos();
            let mut hash = Sha256::new();
            hash.update(b"sim-journal-namespace-v2\0");
            hash.update(seed);
            hash.update(std::process::id().to_be_bytes());
            hash.update(now.to_be_bytes());
            hash.update(nonce.to_be_bytes());
            let token = hex(&hash.finalize());
            let namespace = EntryNamespace(token);
            let root = namespace_root(&namespace);
            self.port.create_dir(&root).map_err(port_error)?;
            let marker = [root.clone(), vec!["format".into()]].concat();
            let result = self
                .port
                .compare_exchange(&marker, None, Some(b"SIMJNAMESPACE2"), &NeverCancel)
                .map_err(port_error)?;
            if result.exchanged {
                self.port
                    .create_dir(&[root, vec!["entries".into()]].concat())
                    .map_err(port_error)?;
                return Ok(namespace);
            }
        }
        Err(JournalError::Backend(
            "could not reserve a fresh journal namespace".into(),
        ))
    }

    fn put_object(&self, object: &JournalObject) -> Result<StoredDatumRef, JournalError> {
        object.verify()?;
        let bytes = object.storage_bytes()?;
        let storage = crate::object::storage_id(&bytes);
        let meaning_dir = vec!["objects-v2".into(), id_key(&object.id)];
        self.port.create_dir(&meaning_dir).map_err(port_error)?;
        let path = [meaning_dir, vec![id_key(&storage)]].concat();
        self.put_immutable(&path, &bytes)?;
        Ok(StoredDatumRef {
            meaning: object.id.clone(),
            storage,
        })
    }

    fn find_object(
        &self,
        meaning: &ContentId,
    ) -> Result<(StoredDatumRef, JournalObject), JournalError> {
        let dir = vec!["objects-v2".into(), id_key(meaning)];
        let entries = self.port.list(&dir).map_err(port_error)?;
        let files: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.kind == sim_storage_port::HostEntryKind::File)
            .collect();
        if files.is_empty() {
            return Err(JournalError::MissingSemanticObject(meaning.clone()));
        }
        if files.len() != 1 {
            return Err(JournalError::CorruptState("ambiguous semantic object"));
        }
        let storage = parse_id_key(&files[0].name)?;
        let bytes = self.load(&[dir, vec![files[0].name.clone()]].concat())?;
        if crate::object::storage_id(&bytes) != storage {
            return Err(JournalError::CorruptState("object storage id"));
        }
        let object = JournalObject::from_storage_bytes(&bytes)?;
        if object.id != *meaning {
            return Err(JournalError::CorruptObject(meaning.clone()));
        }
        Ok((
            StoredDatumRef {
                meaning: meaning.clone(),
                storage,
            },
            object,
        ))
    }

    fn load_object_ref(&self, reference: &StoredDatumRef) -> Result<JournalObject, JournalError> {
        let bytes = self.load(&object_path(reference))?;
        if crate::object::storage_id(&bytes) != reference.storage {
            return Err(JournalError::CorruptState("object storage id"));
        }
        let object = JournalObject::from_storage_bytes(&bytes)?;
        if object.id != reference.meaning {
            return Err(JournalError::CorruptObject(reference.meaning.clone()));
        }
        Ok(object)
    }

    fn read_v1(&self, state_bytes: &[u8]) -> Result<(StoredState, JournalHead), JournalError> {
        let (_, head) = decode_v1_state(state_bytes)?;
        let Some(physical_head) = head else {
            return Err(JournalError::CorruptState("empty v1 prefix"));
        };
        let needed = usize::try_from(physical_head.sequence)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(JournalError::WorkBoundExceeded)?;
        if needed > self.work_bound {
            return Err(JournalError::WorkBoundExceeded);
        }
        let mut entries = BTreeMap::new();
        let mut objects = BTreeMap::new();
        let mut datums = BTreeMap::new();
        let mut physical_previous = None;
        let mut canonical_previous = None;
        for sequence in 0..=physical_head.sequence {
            let bytes = self.load(&v1_entry_path(sequence))?;
            let old = decode_v1_entry(&bytes)?;
            if old.sequence != sequence || old.previous != physical_previous {
                return Err(JournalError::CorruptState("v1 chain"));
            }
            if old.id != old.canonical_id() {
                return Err(JournalError::CorruptEntry);
            }
            let mut payloads = Vec::with_capacity(old.payloads.len());
            for old_id in &old.payloads {
                let payload = self.load(&v1_object_path(old_id))?;
                if v1_object_id(&payload) != *old_id {
                    return Err(JournalError::CorruptObject(old_id.clone()));
                }
                let object = JournalObject::from_bytes(payload);
                payloads.push(object.id.clone());
                objects.insert(object.id.clone(), object.bytes.clone());
                datums.insert(object.id.clone(), object.datum().clone());
            }
            let entry = JournalEntry::new(sequence, canonical_previous.clone(), old.kind, payloads);
            physical_previous = Some(old.id);
            canonical_previous = Some(entry.id.clone());
            entries.insert(sequence, entry);
        }
        if physical_previous.as_ref() != Some(&physical_head.entry) {
            return Err(JournalError::CorruptState("v1 head"));
        }
        let canonical_head = entries
            .last_key_value()
            .map(|(_, entry)| JournalHead {
                sequence: entry.sequence,
                entry: entry.id.clone(),
            })
            .ok_or(JournalError::CorruptState("empty v1 prefix"))?;
        let state = StoredState {
            objects,
            datums,
            entries,
            head: Some(canonical_head.clone()),
        };
        crate::verify::verify_state(&state)?;
        Ok((state, canonical_head))
    }

    fn read_prefix(&self, prefix: &VerifiedNativePrefixRef) -> Result<StoredState, JournalError> {
        let bytes = self.load(&descriptor_path(&prefix.descriptor))?;
        if crate::object::storage_id(&bytes) != prefix.descriptor {
            return Err(JournalError::CorruptState("prefix descriptor id"));
        }
        let (old_state, described_canonical_head) = decode_descriptor(&bytes)?;
        let (_, physical) = decode_v1_state(&old_state)?;
        if physical.as_ref() != Some(&prefix.physical_head) {
            return Err(JournalError::CorruptState("prefix physical head"));
        }
        let (state, canonical) = self.read_v1(&old_state)?;
        if described_canonical_head != prefix.canonical_head
            || canonical != prefix.canonical_head
            || prefix.entries != canonical.sequence.saturating_add(1)
        {
            return Err(JournalError::CorruptState("prefix canonical head"));
        }
        Ok(state)
    }

    fn read_v2(&self, envelope: &NativeStateEnvelope) -> Result<StoredState, JournalError> {
        if envelope.format != NativeFormatId::V2 {
            return Err(JournalError::CorruptState("native format"));
        }
        let marker =
            self.load(&[namespace_root(&envelope.namespace), vec!["format".into()]].concat())?;
        if marker != b"SIMJNAMESPACE2" {
            return Err(JournalError::CorruptState("namespace format"));
        }
        let mut state = match &envelope.prefix {
            Some(prefix) => self.read_prefix(prefix)?,
            None => StoredState::default(),
        };
        let prefix_len = state.entries.len();
        let mut location = envelope.head_location.clone();
        let mut suffix = Vec::new();
        let mut seen = BTreeSet::new();
        while let Some(current) = location {
            if current.namespace != envelope.namespace || !seen.insert(current.clone()) {
                return Err(JournalError::CorruptState("entry locator chain"));
            }
            if prefix_len + suffix.len() >= self.work_bound {
                return Err(JournalError::WorkBoundExceeded);
            }
            let bytes = self.load(&entry_path(&current))?;
            if crate::object::storage_id(&bytes) != current.storage {
                return Err(JournalError::CorruptState("entry storage id"));
            }
            let physical = decode_v2_entry(&bytes)?;
            if physical.entry.id != current.entry
                || physical.entry.sequence != current.sequence
                || physical.entry.canonical_id()? != physical.entry.id
            {
                return Err(JournalError::CorruptEntry);
            }
            for (meaning, reference) in physical.entry.payloads.iter().zip(&physical.payloads) {
                if meaning != &reference.meaning {
                    return Err(JournalError::CorruptState("payload locator"));
                }
                let object = self.load_object_ref(reference)?;
                state
                    .objects
                    .insert(object.id.clone(), object.bytes.clone());
                state
                    .datums
                    .insert(object.id.clone(), object.datum().clone());
            }
            if physical.entry.payloads.len() != physical.payloads.len() {
                return Err(JournalError::CorruptState("payload locator count"));
            }
            location = physical.previous_location.clone();
            suffix.push(physical);
        }
        suffix.reverse();
        for physical in suffix {
            if state
                .entries
                .insert(physical.entry.sequence, physical.entry)
                .is_some()
            {
                return Err(JournalError::CorruptState("overlapping suffix"));
            }
        }
        state.head = envelope.head.clone();
        crate::verify::verify_state(&state)?;
        let expected_location = state.entries.len() > prefix_len;
        if expected_location != envelope.head_location.is_some() {
            return Err(JournalError::CorruptState("head locator"));
        }
        Ok(state)
    }

    fn install_v2(&self, observed: Option<&[u8]>) -> Result<Option<Lease>, JournalError> {
        let (old_fence, prefix, canonical_head) = match observed {
            Some(bytes) => {
                let (fence, physical_head) = decode_v1_state(bytes)?;
                match physical_head {
                    Some(physical_head) => {
                        let (_, canonical_head) = self.read_v1(bytes)?;
                        self.trip(Failpoint::BeforePrefixDescriptor)?;
                        let descriptor_bytes = encode_descriptor(bytes, &canonical_head);
                        let descriptor = crate::object::storage_id(&descriptor_bytes);
                        self.put_immutable(&descriptor_path(&descriptor), &descriptor_bytes)?;
                        self.trip(Failpoint::AfterPrefixDescriptor)?;
                        (
                            fence,
                            Some(VerifiedNativePrefixRef {
                                entries: canonical_head.sequence + 1,
                                physical_head,
                                canonical_head: canonical_head.clone(),
                                descriptor,
                            }),
                            Some(canonical_head),
                        )
                    }
                    None => (fence, None, None),
                }
            }
            None => (0, None, None),
        };
        let fence = old_fence
            .checked_add(1)
            .ok_or_else(|| JournalError::Backend("fence exhausted".into()))?;
        let namespace = self.reserve_namespace(observed.unwrap_or_default())?;
        self.trip(Failpoint::AfterNamespaceReservation)?;
        let envelope = NativeStateEnvelope {
            format: NativeFormatId::V2,
            fence,
            namespace,
            prefix,
            head: canonical_head,
            head_location: None,
        };
        let replacement = encode_envelope(&envelope);
        self.trip(Failpoint::BeforeFormatCas)?;
        let result = self
            .port
            .compare_exchange(
                &["state".into()],
                observed,
                Some(&replacement),
                &NeverCancel,
            )
            .map_err(port_error)?;
        if !result.exchanged {
            return Ok(None);
        }
        self.trip(Failpoint::AfterFormatCas)?;
        Ok(Some(Lease { fence }))
    }
}

impl JournalBackend for HostDirJournalBackend {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        self.write_capable()?;
        self.ensure_layout()?;
        loop {
            let observed = self.state_bytes()?;
            match observed.as_deref() {
                None => {
                    if let Some(lease) = self.install_v2(observed.as_deref())? {
                        return Ok(lease);
                    }
                }
                Some(bytes) if bytes.starts_with(b"SIMJSTATE1") => {
                    if let Some(lease) = self.install_v2(Some(bytes))? {
                        return Ok(lease);
                    }
                }
                Some(bytes) => {
                    let mut envelope = decode_envelope(bytes)?;
                    self.read_v2(&envelope)?;
                    envelope.fence = envelope
                        .fence
                        .checked_add(1)
                        .ok_or_else(|| JournalError::Backend("fence exhausted".into()))?;
                    let replacement = encode_envelope(&envelope);
                    let result = self
                        .port
                        .compare_exchange(
                            &["state".into()],
                            Some(bytes),
                            Some(&replacement),
                            &NeverCancel,
                        )
                        .map_err(port_error)?;
                    if result.exchanged {
                        return Ok(Lease {
                            fence: envelope.fence,
                        });
                    }
                }
            }
        }
    }

    fn read_state(&self) -> Result<StoredState, JournalError> {
        let Some(bytes) = self.state_bytes()? else {
            return Ok(StoredState::default());
        };
        if bytes.starts_with(b"SIMJSTATE1") {
            let (_, head) = decode_v1_state(&bytes)?;
            return match head {
                Some(_) => self.read_v1(&bytes).map(|(state, _)| state),
                None => Ok(StoredState::default()),
            };
        }
        let envelope = decode_envelope(&bytes)?;
        self.read_v2(&envelope)
    }

    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
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
            if admission.entries.last().is_some_and(|entry| {
                envelope
                    .head
                    .as_ref()
                    .is_some_and(|head| head.entry == entry.id)
            }) {
                return envelope.head.ok_or(JournalError::WrongHead);
            }
            return Err(JournalError::WrongHead);
        }
        self.trip(Failpoint::BeforeObjectPublish)?;
        let mut references = BTreeMap::new();
        for object in &admission.objects {
            let reference = self.put_object(object)?;
            references.insert(reference.meaning.clone(), reference);
        }
        self.trip(Failpoint::AfterObjectPublish)?;
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
        self.trip(Failpoint::AfterDurabilityReceipt)?;
        let last = admission.entries.last().ok_or(JournalError::EmptyBatch)?;
        let new_head = JournalHead {
            sequence: last.sequence,
            entry: last.id.clone(),
        };
        envelope.head = Some(new_head.clone());
        envelope.head_location = last_location;
        self.trip(Failpoint::BeforeCas)?;
        let replacement = encode_envelope(&envelope);
        let result = self
            .port
            .compare_exchange(
                &["state".into()],
                Some(&observed),
                Some(&replacement),
                &NeverCancel,
            )
            .map_err(port_error)?;
        if !result.exchanged {
            return Err(JournalError::WrongHead);
        }
        self.trip(Failpoint::AfterCas)?;
        self.trip(Failpoint::BeforeAcknowledgement)?;
        Ok(new_head)
    }

    fn put_datum(&self, object: JournalObject) -> Result<StoredDatumRef, JournalError> {
        self.write_capable()?;
        self.ensure_layout()?;
        self.put_object(&object)
    }

    fn get_datum(&self, meaning: &ContentId) -> Result<Datum, JournalError> {
        Ok(self.find_object(meaning)?.1.datum().clone())
    }

    fn rebuild_datum_index(&self) -> Result<Vec<StoredDatumRef>, JournalError> {
        let mut rebuilt = Vec::new();
        let entries = match self.port.list(&["objects-v2".into()]) {
            Ok(entries) => entries,
            Err(error) if error.kind == HostDirErrorKind::NotFound => return Ok(rebuilt),
            Err(error) => return Err(port_error(error)),
        };
        for entry in entries {
            if entry.kind != sim_storage_port::HostEntryKind::Directory {
                return Err(JournalError::CorruptState("object index entry"));
            }
            if rebuilt.len() >= self.work_bound {
                return Err(JournalError::WorkBoundExceeded);
            }
            let meaning = parse_id_key(&entry.name)?;
            rebuilt.push(self.find_object(&meaning)?.0);
        }
        rebuilt.sort_by(|left, right| left.meaning.cmp(&right.meaning));
        Ok(rebuilt)
    }
}
