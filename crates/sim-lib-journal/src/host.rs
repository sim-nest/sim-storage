use crate::{
    Admission, JournalBackend, JournalEntry, JournalError, JournalHead, Lease, StoredState,
};
use sim_kernel::{ContentId, Symbol};
use sim_storage_port::{HostDirErrorKind, HostDirPort, NeverCancel};
use std::{collections::BTreeMap, sync::Arc};

/// Evidence supplied by the concrete Table/Dir binding at construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    /// State-leaf compare-exchange is linearizable across coordinators.
    pub linearizable_cas: bool,
    /// Successful immutable-leaf writes carry the binding's durability receipt.
    pub durable_publish: bool,
}

/// Stable crash-injection boundaries in the publication protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failpoint {
    BeforeObjectPublish,
    AfterObjectPublish,
    AfterDurabilityReceipt,
    BeforeCas,
    AfterCas,
    BeforeAcknowledgement,
}

impl Failpoint {
    fn label(self) -> &'static str {
        match self {
            Self::BeforeObjectPublish => "before-object-publish",
            Self::AfterObjectPublish => "after-object-publish",
            Self::AfterDurabilityReceipt => "after-durability-receipt",
            Self::BeforeCas => "before-cas",
            Self::AfterCas => "after-cas",
            Self::BeforeAcknowledgement => "before-acknowledgement",
        }
    }
}

type FailHook = Arc<dyn Fn(Failpoint) -> bool + Send + Sync>;

/// Crash-durable journal composition over the canonical host-backed Table/Dir port.
///
/// `state` contains both fence and head. Its one compare-exchange is the only
/// publication linearization point. Objects and entries are immutable leaves.
pub struct HostDirJournalBackend {
    port: Arc<dyn HostDirPort>,
    capabilities: BackendCapabilities,
    work_bound: usize,
    fail: Option<FailHook>,
}

impl HostDirJournalBackend {
    /// Opens a binding. Read operations remain available when write safety is absent.
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
        backend.read_state()?; // full chain and content-closure verification before exposure
        Ok(backend)
    }

    /// Installs a deterministic test hook. Returning true simulates process death.
    pub fn with_failpoint_hook(
        mut self,
        hook: impl Fn(Failpoint) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.fail = Some(Arc::new(hook));
        self
    }

    /// Reports the binding evidence used to admit or refuse write mode.
    pub fn capabilities(&self) -> BackendCapabilities {
        self.capabilities
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
        for path in [["objects"], ["entries"], ["temporary"]] {
            self.port
                .create_dir(&path.map(str::to_owned))
                .map_err(port_error)?;
        }
        Ok(())
    }

    fn state_bytes(&self) -> Result<Option<Vec<u8>>, JournalError> {
        match self.port.read(&["state".into()]) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind == HostDirErrorKind::NotFound => Ok(None),
            Err(e) => Err(port_error(e)),
        }
    }

    fn put_immutable(&self, path: &[String], bytes: &[u8]) -> Result<(), JournalError> {
        let outcome = self
            .port
            .compare_exchange(path, None, Some(bytes), &NeverCancel)
            .map_err(port_error)?;
        if outcome.exchanged {
            return Ok(());
        }
        if outcome.observed.as_deref() == Some(bytes) {
            Ok(())
        } else {
            Err(JournalError::ConflictingObject)
        }
    }

    fn load(&self, path: &[String]) -> Result<Vec<u8>, JournalError> {
        self.port.read(path).map_err(port_error)
    }
}

impl JournalBackend for HostDirJournalBackend {
    fn acquire_lease(&self) -> Result<Lease, JournalError> {
        self.write_capable()?;
        self.ensure_layout()?;
        loop {
            let observed = self.state_bytes()?;
            let (fence, head) = observed
                .as_deref()
                .map(decode_state)
                .transpose()?
                .unwrap_or((0, None));
            let next = fence
                .checked_add(1)
                .ok_or_else(|| JournalError::Backend("fence exhausted".into()))?;
            let replacement = encode_state(next, head.as_ref());
            let result = self
                .port
                .compare_exchange(
                    &["state".into()],
                    observed.as_deref(),
                    Some(&replacement),
                    &NeverCancel,
                )
                .map_err(port_error)?;
            if result.exchanged {
                return Ok(Lease { fence: next });
            }
        }
    }

    fn read_state(&self) -> Result<StoredState, JournalError> {
        let (_, head) = self
            .state_bytes()?
            .as_deref()
            .map(decode_state)
            .transpose()?
            .unwrap_or((0, None));
        let Some(head) = head else {
            return Ok(StoredState::default());
        };
        let needed = (head.sequence as usize)
            .checked_add(1)
            .ok_or(JournalError::WorkBoundExceeded)?;
        if needed > self.work_bound {
            return Err(JournalError::WorkBoundExceeded);
        }
        let mut entries = BTreeMap::new();
        let mut objects = BTreeMap::new();
        for sequence in 0..=head.sequence {
            let bytes = self.load(&entry_path(sequence))?;
            let entry = decode_entry(&bytes)?;
            if entry.sequence != sequence {
                return Err(JournalError::CorruptState("entry location"));
            }
            for id in &entry.payloads {
                if !objects.contains_key(id) {
                    if entries.len() + objects.len() >= self.work_bound {
                        return Err(JournalError::WorkBoundExceeded);
                    }
                    objects.insert(id.clone(), self.load(&object_path(id))?);
                }
            }
            entries.insert(sequence, entry);
        }
        let state = StoredState {
            objects,
            entries,
            head: Some(head),
        };
        crate::verify::verify_state(&state)?;
        Ok(state)
    }

    fn admit(&self, admission: Admission) -> Result<JournalHead, JournalError> {
        self.write_capable()?;
        self.ensure_layout()?;
        let observed = self.state_bytes()?;
        let (fence, head) = observed
            .as_deref()
            .map(decode_state)
            .transpose()?
            .unwrap_or((0, None));
        if fence != admission.fence {
            return Err(JournalError::StaleLease);
        }
        if head != admission.expected {
            return Err(JournalError::WrongHead);
        }
        self.trip(Failpoint::BeforeObjectPublish)?;
        for object in &admission.objects {
            object.verify()?;
            self.put_immutable(&object_path(&object.id), &object.bytes)?;
        }
        self.trip(Failpoint::AfterObjectPublish)?;
        for entry in &admission.entries {
            self.put_immutable(&entry_path(entry.sequence), &encode_entry(entry))?;
        }
        self.trip(Failpoint::AfterDurabilityReceipt)?;
        let last = admission.entries.last().ok_or(JournalError::EmptyBatch)?;
        let new_head = JournalHead {
            sequence: last.sequence,
            entry: last.id.clone(),
        };
        self.trip(Failpoint::BeforeCas)?;
        let replacement = encode_state(fence, Some(&new_head));
        let result = self
            .port
            .compare_exchange(
                &["state".into()],
                observed.as_deref(),
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
}

fn port_error(error: sim_storage_port::HostDirError) -> JournalError {
    JournalError::Backend(error.to_string())
}
fn entry_path(sequence: u64) -> Vec<String> {
    vec!["entries".into(), format!("{sequence:016x}")]
}
fn object_path(id: &ContentId) -> Vec<String> {
    vec!["objects".into(), hex(&id.bytes)]
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn encode_state(fence: u64, head: Option<&JournalHead>) -> Vec<u8> {
    let mut out = b"SIMJSTATE1".to_vec();
    out.extend(fence.to_be_bytes());
    match head {
        Some(h) => {
            out.push(1);
            out.extend(h.sequence.to_be_bytes());
            put_id(&mut out, &h.entry);
        }
        None => out.push(0),
    }
    out
}
fn decode_state(bytes: &[u8]) -> Result<(u64, Option<JournalHead>), JournalError> {
    let mut c = Cursor::new(bytes);
    if c.take(10)? != b"SIMJSTATE1" {
        return Err(JournalError::CorruptState("state format"));
    }
    let fence = c.u64()?;
    let head = match c.byte()? {
        0 => None,
        1 => Some(JournalHead {
            sequence: c.u64()?,
            entry: c.id()?,
        }),
        _ => return Err(JournalError::CorruptState("state tag")),
    };
    c.end()?;
    Ok((fence, head))
}
fn encode_entry(entry: &JournalEntry) -> Vec<u8> {
    let mut out = b"SIMJENTRY1".to_vec();
    put_id(&mut out, &entry.id);
    out.extend(entry.sequence.to_be_bytes());
    match &entry.previous {
        Some(id) => {
            out.push(1);
            put_id(&mut out, id)
        }
        None => out.push(0),
    };
    put_text(&mut out, &entry.kind.as_qualified_str());
    out.extend((entry.payloads.len() as u32).to_be_bytes());
    for id in &entry.payloads {
        put_id(&mut out, id);
    }
    out
}
fn decode_entry(bytes: &[u8]) -> Result<JournalEntry, JournalError> {
    let mut c = Cursor::new(bytes);
    if c.take(10)? != b"SIMJENTRY1" {
        return Err(JournalError::CorruptState("entry format"));
    }
    let id = c.id()?;
    let sequence = c.u64()?;
    let previous = match c.byte()? {
        0 => None,
        1 => Some(c.id()?),
        _ => return Err(JournalError::CorruptState("entry tag")),
    };
    let kind = parse_symbol(&c.text()?)?;
    let count = c.u32()? as usize;
    let mut payloads = Vec::with_capacity(count);
    for _ in 0..count {
        payloads.push(c.id()?);
    }
    c.end()?;
    Ok(JournalEntry {
        id,
        sequence,
        previous,
        kind,
        payloads,
    })
}
fn put_id(out: &mut Vec<u8>, id: &ContentId) {
    put_text(out, &id.algorithm.as_qualified_str());
    out.extend(id.bytes)
}
fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend((text.len() as u32).to_be_bytes());
    out.extend(text.as_bytes())
}
fn parse_symbol(text: &str) -> Result<Symbol, JournalError> {
    match text.split_once('/') {
        Some((n, v)) if !n.is_empty() && !v.is_empty() => Ok(Symbol::qualified(n, v)),
        None => Symbol::checked(text).map_err(|_| JournalError::CorruptState("symbol")),
        _ => Err(JournalError::CorruptState("symbol")),
    }
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], JournalError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(JournalError::CorruptState("length"))?;
        let value = self
            .bytes
            .get(self.at..end)
            .ok_or(JournalError::CorruptState("truncated"))?;
        self.at = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, JournalError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, JournalError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, JournalError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn text(&mut self) -> Result<String, JournalError> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| JournalError::CorruptState("utf8"))
    }
    fn id(&mut self) -> Result<ContentId, JournalError> {
        let algorithm = parse_symbol(&self.text()?)?;
        let bytes = self.take(32)?.try_into().unwrap();
        Ok(ContentId::from_bytes(algorithm, bytes))
    }
    fn end(&self) -> Result<(), JournalError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(JournalError::CorruptState("trailing bytes"))
        }
    }
}
