use super::*;

#[test]
fn synthetic_v1_prefix_replays_canonically_without_rewriting_bytes() {
    let port = Arc::new(TestPort::default());
    let old = OldProtocolModel::seed(port.clone(), &[b"old-a", b"old-b"]);
    let before = port.files.lock().unwrap().clone();
    let backend = HostDirJournalBackend::open(port.clone(), capabilities(), 100).unwrap();
    let state = backend.read_state().unwrap();
    assert_eq!(state.entries.len(), 2);
    assert!(
        state
            .entries
            .values()
            .all(|entry| entry.id.algorithm == datum_content_algorithm())
    );
    assert_eq!(*port.files.lock().unwrap(), before);

    let lease = backend.acquire_lease().unwrap();
    let envelope = backend.state_envelope().unwrap().unwrap();
    assert_eq!(envelope.format, NativeFormatId::V2);
    assert_eq!(envelope.prefix.as_ref().unwrap().entries, 2);
    assert_eq!(envelope.head, state.head);
    assert_eq!(
        old.captured,
        before.get(&vec!["state".into()]).unwrap().clone()
    );
    assert_eq!(lease.fence(), envelope.fence);
    for (path, bytes) in before {
        if path
            .first()
            .is_some_and(|part| part == "entries" || part == "objects")
        {
            assert_eq!(port.files.lock().unwrap().get(&path), Some(&bytes));
        }
    }
}

#[test]
fn paused_old_writer_loses_after_upgrade_and_cannot_poison_v2_sequence() {
    let port = Arc::new(TestPort::default());
    let old = OldProtocolModel::seed(port.clone(), &[b"old"]);
    let backend = HostDirJournalBackend::open(port.clone(), capabilities(), 100).unwrap();
    let journal = Journal::new(backend);
    let lease = journal.acquire_lease().unwrap();
    assert!(!old.fresh_writer_accepts_state());
    assert!(!old.publish_after_capture(b"late-old"));

    let prior = journal.head().unwrap().unwrap();
    let payload = JournalObject::from_bytes(b"canonical-next".to_vec());
    let next = JournalEntry::new(
        prior.sequence + 1,
        Some(prior.entry.clone()),
        Symbol::qualified("example", "next"),
        vec![payload.id.clone()],
    );
    let accepted = journal
        .publish(&lease, Some(&prior), vec![payload], vec![next.clone()])
        .unwrap();
    assert_eq!(accepted.entry, next.id);
    assert_eq!(journal.replay().unwrap().count(), 2);
    assert!(
        port.files
            .lock()
            .unwrap()
            .contains_key(&vec!["entries".into(), "0000000000000001".into()])
    );
}

#[test]
fn competing_upgrades_select_one_namespace_and_leave_the_loser_inert() {
    let port = Arc::new(TestPort::default());
    OldProtocolModel::seed(port.clone(), &[b"old"]);
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let port = port.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            HostDirJournalBackend::open(port, capabilities(), 100)
                .unwrap()
                .with_failpoint_hook(move |point| {
                    if point == Failpoint::BeforeFormatCas {
                        barrier.wait();
                    }
                    false
                })
                .acquire_lease()
                .unwrap()
        }));
    }
    let leases: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_ne!(leases[0], leases[1]);
    let backend = HostDirJournalBackend::open(port.clone(), capabilities(), 100).unwrap();
    let selected = backend.state_envelope().unwrap().unwrap().namespace;
    let namespaces: Vec<_> = port
        .dirs
        .lock()
        .unwrap()
        .iter()
        .filter(|path| path.len() == 2 && path[0] == "namespaces-v2")
        .cloned()
        .collect();
    assert_eq!(namespaces.len(), 2);
    assert!(namespaces.iter().any(|path| path[1] == selected.0));
    assert_eq!(Journal::new(backend).replay().unwrap().count(), 1);
}

#[test]
fn conflicting_v2_writers_publish_distinct_leaves_but_accept_one_head() {
    let port = Arc::new(TestPort::default());
    let lease = HostDirJournalBackend::open(port.clone(), capabilities(), 100)
        .unwrap()
        .acquire_lease()
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for byte in [1_u8, 2] {
        let port = port.clone();
        let lease = lease.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let backend = HostDirJournalBackend::open(port, capabilities(), 100)
                .unwrap()
                .with_failpoint_hook(move |point| {
                    if point == Failpoint::BeforeCas {
                        barrier.wait();
                    }
                    false
                });
            let journal = Journal::new(backend);
            let payload = object(byte);
            let entry = entry(0, None, &payload);
            journal.publish(&lease, None, vec![payload], vec![entry])
        }));
    }
    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(JournalError::WrongHead)))
            .count(),
        1
    );
    let journal = Journal::new(HostDirJournalBackend::open(port, capabilities(), 100).unwrap());
    assert_eq!(journal.replay().unwrap().count(), 1);
    let prior = journal.head().unwrap().unwrap();
    let lease = journal.acquire_lease().unwrap();
    let payload = object(3);
    let next = entry(1, Some(prior.entry.clone()), &payload);
    journal
        .publish(&lease, Some(&prior), vec![payload], vec![next])
        .unwrap();
    assert_eq!(journal.replay().unwrap().count(), 2);
}

#[test]
fn old_crash_residue_and_old_winner_before_upgrade_have_exact_outcomes() {
    let residue_port = Arc::new(TestPort::default());
    let residue = OldProtocolModel::seed(residue_port.clone(), &[b"old"]);
    residue.publish_leaf_only(b"orphan");
    let residue_backend = HostDirJournalBackend::open(residue_port, capabilities(), 100).unwrap();
    let residue_journal = Journal::new(residue_backend);
    let residue_lease = residue_journal.acquire_lease().unwrap();
    let prior = residue_journal.head().unwrap().unwrap();
    let object = JournalObject::from_bytes(b"winner".to_vec());
    let entry = JournalEntry::new(
        1,
        Some(prior.entry.clone()),
        Symbol::new("winner"),
        vec![object.id.clone()],
    );
    residue_journal
        .publish(&residue_lease, Some(&prior), vec![object], vec![entry])
        .unwrap();
    assert_eq!(residue_journal.replay().unwrap().count(), 2);

    let winner_port = Arc::new(TestPort::default());
    let winner = OldProtocolModel::seed(winner_port.clone(), &[b"old"]);
    assert!(winner.publish_after_capture(b"old-winner"));
    let winner_backend = HostDirJournalBackend::open(winner_port, capabilities(), 100).unwrap();
    assert_eq!(winner_backend.read_state().unwrap().entries.len(), 2);
    winner_backend.acquire_lease().unwrap();
    assert_eq!(winner_backend.read_state().unwrap().entries.len(), 2);
}

#[test]
fn every_upgrade_crash_recovers_old_or_new_authority() {
    for point in [
        Failpoint::BeforePrefixDescriptor,
        Failpoint::AfterPrefixDescriptor,
        Failpoint::AfterNamespaceReservation,
        Failpoint::BeforeFormatCas,
        Failpoint::AfterFormatCas,
    ] {
        let port = Arc::new(TestPort::default());
        OldProtocolModel::seed(port.clone(), &[b"old"]);
        let crashing = HostDirJournalBackend::open(port.clone(), capabilities(), 100)
            .unwrap()
            .with_failpoint_hook(move |seen| seen == point);
        assert!(matches!(
            crashing.acquire_lease(),
            Err(JournalError::InjectedCrash(_))
        ));
        let recovered = HostDirJournalBackend::open(port, capabilities(), 100).unwrap();
        assert_eq!(recovered.read_state().unwrap().entries.len(), 1);
        let journal = Journal::new(recovered);
        let lease = journal.acquire_lease().unwrap();
        let prior = journal.head().unwrap().unwrap();
        let object = JournalObject::from_bytes(b"progress".to_vec());
        let entry = JournalEntry::new(
            1,
            Some(prior.entry.clone()),
            Symbol::new("progress"),
            vec![object.id.clone()],
        );
        journal
            .publish(&lease, Some(&prior), vec![object], vec![entry])
            .unwrap();
        assert_eq!(journal.replay().unwrap().count(), 2);
    }
}

#[test]
fn corrupt_truncated_and_disagreeing_native_state_fail_closed() {
    let corrupt_port = Arc::new(TestPort::default());
    OldProtocolModel::seed(corrupt_port.clone(), &[b"old"]);
    corrupt_port.corrupt_v1_object();
    assert!(HostDirJournalBackend::open(corrupt_port, capabilities(), 100).is_err());

    let truncated = Arc::new(TestPort::default());
    truncated
        .files
        .lock()
        .unwrap()
        .insert(vec!["state".into()], b"SIMJSTATE2\0".to_vec());
    assert!(HostDirJournalBackend::open(truncated, capabilities(), 100).is_err());

    let mismatch = Arc::new(TestPort::default());
    OldProtocolModel::seed(mismatch.clone(), &[b"old"]);
    let backend = HostDirJournalBackend::open(mismatch.clone(), capabilities(), 100).unwrap();
    backend.acquire_lease().unwrap();
    let mut state = mismatch
        .files
        .lock()
        .unwrap()
        .get(&vec!["state".into()])
        .unwrap()
        .clone();
    let last = state.len() - 1;
    state[last] ^= 1;
    mismatch
        .files
        .lock()
        .unwrap()
        .insert(vec!["state".into()], state);
    assert!(HostDirJournalBackend::open(mismatch, capabilities(), 100).is_err());
}

#[derive(Clone)]
struct OldProtocolModel {
    port: Arc<TestPort>,
    captured: Vec<u8>,
    fence: u64,
    head: JournalHead,
}

impl OldProtocolModel {
    fn seed(port: Arc<TestPort>, payloads: &[&[u8]]) -> Self {
        port.dirs
            .lock()
            .unwrap()
            .extend([vec!["objects".into()], vec!["entries".into()]]);
        let mut previous = None;
        let mut head = None;
        for (sequence, payload) in payloads.iter().enumerate() {
            let object = old_object_id(payload);
            port.files.lock().unwrap().insert(
                vec!["objects".into(), hex_test(&object.bytes)],
                payload.to_vec(),
            );
            let entry = old_entry(
                sequence as u64,
                previous.clone(),
                Symbol::qualified("old-model", "fact"),
                vec![object],
            );
            port.files.lock().unwrap().insert(
                vec!["entries".into(), format!("{sequence:016x}")],
                encode_old_entry(&entry),
            );
            previous = Some(entry.id.clone());
            head = Some(JournalHead {
                sequence: entry.sequence,
                entry: entry.id,
            });
        }
        let head = head.expect("fixture is non-empty");
        let captured = encode_old_state(7, Some(&head));
        port.files
            .lock()
            .unwrap()
            .insert(vec!["state".into()], captured.clone());
        Self {
            port,
            captured,
            fence: 7,
            head,
        }
    }

    fn publish_leaf_only(&self, payload: &[u8]) -> JournalHead {
        let sequence = self.head.sequence + 1;
        let object = old_object_id(payload);
        self.port.files.lock().unwrap().insert(
            vec!["objects".into(), hex_test(&object.bytes)],
            payload.to_vec(),
        );
        let entry = old_entry(
            sequence,
            Some(self.head.entry.clone()),
            Symbol::qualified("old-model", "late"),
            vec![object],
        );
        self.port.files.lock().unwrap().insert(
            vec!["entries".into(), format!("{sequence:016x}")],
            encode_old_entry(&entry),
        );
        JournalHead {
            sequence,
            entry: entry.id,
        }
    }

    fn publish_after_capture(&self, payload: &[u8]) -> bool {
        let head = self.publish_leaf_only(payload);
        let replacement = encode_old_state(self.fence, Some(&head));
        self.port
            .compare_exchange(
                &["state".into()],
                Some(&self.captured),
                Some(&replacement),
                &sim_storage_port::NeverCancel,
            )
            .unwrap()
            .exchanged
    }

    fn fresh_writer_accepts_state(&self) -> bool {
        self.port
            .files
            .lock()
            .unwrap()
            .get(&vec!["state".into()])
            .is_some_and(|state| state.starts_with(b"SIMJSTATE1"))
    }
}

impl TestPort {
    fn corrupt_v1_object(&self) {
        let mut files = self.files.lock().unwrap();
        let bytes = files
            .iter_mut()
            .find(|(path, _)| path.first().is_some_and(|part| part == "objects"))
            .unwrap()
            .1;
        bytes[0] ^= 1;
    }
}

#[derive(Clone)]
struct OldEntry {
    id: ContentId,
    sequence: u64,
    previous: Option<ContentId>,
    kind: Symbol,
    payloads: Vec<ContentId>,
}

fn old_entry(
    sequence: u64,
    previous: Option<ContentId>,
    kind: Symbol,
    payloads: Vec<ContentId>,
) -> OldEntry {
    let mut hasher = Sha256::new();
    hasher.update(b"sim-journal-entry-v1\0");
    hasher.update(sequence.to_be_bytes());
    match &previous {
        Some(id) => {
            hasher.update([1]);
            hash_old_id(&mut hasher, id);
        }
        None => hasher.update([0]),
    }
    hash_old_symbol(&mut hasher, &kind);
    hasher.update((payloads.len() as u64).to_be_bytes());
    for id in &payloads {
        hash_old_id(&mut hasher, id);
    }
    OldEntry {
        id: ContentId::from_bytes(
            Symbol::qualified("journal", "sha256-entry-v1"),
            hasher.finalize().into(),
        ),
        sequence,
        previous,
        kind,
        payloads,
    }
}

fn old_object_id(bytes: &[u8]) -> ContentId {
    ContentId::from_bytes(
        Symbol::qualified("journal", "sha256-bytes-v1"),
        Sha256::digest(bytes).into(),
    )
}

fn encode_old_state(fence: u64, head: Option<&JournalHead>) -> Vec<u8> {
    let mut out = b"SIMJSTATE1".to_vec();
    out.extend(fence.to_be_bytes());
    match head {
        Some(head) => {
            out.push(1);
            out.extend(head.sequence.to_be_bytes());
            put_old_id(&mut out, &head.entry);
        }
        None => out.push(0),
    }
    out
}

fn encode_old_entry(entry: &OldEntry) -> Vec<u8> {
    let mut out = b"SIMJENTRY1".to_vec();
    put_old_id(&mut out, &entry.id);
    out.extend(entry.sequence.to_be_bytes());
    match &entry.previous {
        Some(previous) => {
            out.push(1);
            put_old_id(&mut out, previous);
        }
        None => out.push(0),
    }
    put_old_text(&mut out, &entry.kind.as_qualified_str());
    out.extend((entry.payloads.len() as u32).to_be_bytes());
    for payload in &entry.payloads {
        put_old_id(&mut out, payload);
    }
    out
}

fn put_old_id(out: &mut Vec<u8>, id: &ContentId) {
    put_old_text(out, &id.algorithm.as_qualified_str());
    out.extend(id.bytes);
}

fn put_old_text(out: &mut Vec<u8>, text: &str) {
    out.extend((text.len() as u32).to_be_bytes());
    out.extend(text.as_bytes());
}

fn hash_old_id(hasher: &mut Sha256, id: &ContentId) {
    hash_old_symbol(hasher, &id.algorithm);
    hasher.update(id.bytes);
}

fn hash_old_symbol(hasher: &mut Sha256, symbol: &Symbol) {
    let text = symbol.as_qualified_str();
    hasher.update((text.len() as u64).to_be_bytes());
    hasher.update(text.as_bytes());
}

fn hex_test(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
