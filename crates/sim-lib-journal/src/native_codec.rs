use crate::{JournalEntry, JournalError, JournalHead, StoredDatumRef};
use sha2::{Digest, Sha256};
use sim_kernel::{ContentId, Datum, Symbol};
use sim_storage_port::HostDirPort;

/// Evidence supplied by the concrete Table/Dir binding at construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendCapabilities {
    /// State-leaf compare-exchange is linearizable across coordinators.
    pub linearizable_cas: bool,
    /// Successful immutable-leaf writes carry the binding's durability receipt.
    pub durable_publish: bool,
}

/// Native physical state version. It never enters a semantic entry identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeFormatId {
    /// Retained `SIMJSTATE1`/`SIMJENTRY1` prefix.
    V1,
    /// Canonical semantic entries in isolated content-addressed leaves.
    V2,
}

/// Backend-allocated routing token for one v2 entry namespace.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EntryNamespace(pub String);

/// Exact physical locator for a committed v2 entry leaf.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct EntryLocation {
    pub namespace: EntryNamespace,
    pub sequence: u64,
    pub entry: ContentId,
    pub storage: ContentId,
}

/// Durable proof reference for the immutable v1 committed prefix.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedNativePrefixRef {
    pub entries: u64,
    pub physical_head: JournalHead,
    pub canonical_head: JournalHead,
    pub descriptor: ContentId,
}

/// The one physical state selected by the native state CAS.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeStateEnvelope {
    pub format: NativeFormatId,
    pub fence: u64,
    pub namespace: EntryNamespace,
    pub prefix: Option<VerifiedNativePrefixRef>,
    pub head: Option<JournalHead>,
    pub head_location: Option<EntryLocation>,
}

/// Stable crash-injection boundaries in upgrade and append publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failpoint {
    BeforePrefixDescriptor,
    AfterPrefixDescriptor,
    AfterNamespaceReservation,
    BeforeFormatCas,
    AfterFormatCas,
    BeforeObjectPublish,
    AfterObjectPublish,
    AfterDurabilityReceipt,
    BeforeCas,
    AfterCas,
    BeforeAcknowledgement,
}

impl Failpoint {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::BeforePrefixDescriptor => "before-prefix-descriptor",
            Self::AfterPrefixDescriptor => "after-prefix-descriptor",
            Self::AfterNamespaceReservation => "after-namespace-reservation",
            Self::BeforeFormatCas => "before-format-cas",
            Self::AfterFormatCas => "after-format-cas",
            Self::BeforeObjectPublish => "before-object-publish",
            Self::AfterObjectPublish => "after-object-publish",
            Self::AfterDurabilityReceipt => "after-durability-receipt",
            Self::BeforeCas => "before-cas",
            Self::AfterCas => "after-cas",
            Self::BeforeAcknowledgement => "before-acknowledgement",
        }
    }
}

#[derive(Clone)]
pub(crate) struct PhysicalEntry {
    pub(crate) entry: JournalEntry,
    pub(crate) previous_location: Option<EntryLocation>,
    pub(crate) payloads: Vec<StoredDatumRef>,
}

#[derive(Clone)]
pub(crate) struct V1Entry {
    pub(crate) id: ContentId,
    pub(crate) sequence: u64,
    pub(crate) previous: Option<ContentId>,
    pub(crate) kind: Symbol,
    pub(crate) payloads: Vec<ContentId>,
}

impl V1Entry {
    pub(crate) fn canonical_id(&self) -> ContentId {
        let mut hasher = Sha256::new();
        hasher.update(b"sim-journal-entry-v1\0");
        hasher.update(self.sequence.to_be_bytes());
        put_v1_optional_id_hash(&mut hasher, self.previous.as_ref());
        put_v1_symbol_hash(&mut hasher, &self.kind);
        hasher.update((self.payloads.len() as u64).to_be_bytes());
        for id in &self.payloads {
            put_v1_id_hash(&mut hasher, id);
        }
        ContentId::from_bytes(
            Symbol::qualified("journal", "sha256-entry-v1"),
            hasher.finalize().into(),
        )
    }
}

pub(crate) fn encode_envelope(value: &NativeStateEnvelope) -> Vec<u8> {
    let mut out = b"SIMJSTATE2".to_vec();
    out.extend(value.fence.to_be_bytes());
    put_text(&mut out, &value.namespace.0);
    match &value.prefix {
        Some(prefix) => {
            out.push(1);
            out.extend(prefix.entries.to_be_bytes());
            put_head(&mut out, &prefix.physical_head);
            put_head(&mut out, &prefix.canonical_head);
            put_id(&mut out, &prefix.descriptor);
        }
        None => out.push(0),
    }
    put_optional_head(&mut out, value.head.as_ref());
    put_optional_location(&mut out, value.head_location.as_ref());
    out
}

pub(crate) fn decode_envelope(bytes: &[u8]) -> Result<NativeStateEnvelope, JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(10)? != b"SIMJSTATE2" {
        return Err(JournalError::CorruptState("state format"));
    }
    let fence = cursor.u64()?;
    let namespace = EntryNamespace(cursor.text()?);
    let prefix = match cursor.byte()? {
        0 => None,
        1 => Some(VerifiedNativePrefixRef {
            entries: cursor.u64()?,
            physical_head: cursor.head()?,
            canonical_head: cursor.head()?,
            descriptor: cursor.id()?,
        }),
        _ => return Err(JournalError::CorruptState("prefix tag")),
    };
    let head = cursor.optional_head()?;
    let head_location = cursor.optional_location()?;
    cursor.end()?;
    if prefix.is_none() && head.is_some() && head_location.is_none() {
        return Err(JournalError::CorruptState("head without locator"));
    }
    Ok(NativeStateEnvelope {
        format: NativeFormatId::V2,
        fence,
        namespace,
        prefix,
        head,
        head_location,
    })
}

pub(crate) fn encode_descriptor(old_state: &[u8], canonical_head: &JournalHead) -> Vec<u8> {
    let mut out = b"SIMJPREFIX1".to_vec();
    out.extend((old_state.len() as u64).to_be_bytes());
    out.extend(old_state);
    put_head(&mut out, canonical_head);
    out
}

pub(crate) fn decode_descriptor(bytes: &[u8]) -> Result<(Vec<u8>, JournalHead), JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(11)? != b"SIMJPREFIX1" {
        return Err(JournalError::CorruptState("prefix descriptor"));
    }
    let len = usize::try_from(cursor.u64()?).map_err(|_| JournalError::WorkBoundExceeded)?;
    let state = cursor.take(len)?.to_vec();
    let canonical_head = cursor.head()?;
    cursor.end()?;
    Ok((state, canonical_head))
}

pub(crate) fn encode_v2_entry(value: &PhysicalEntry) -> Vec<u8> {
    let mut out = b"SIMJENTRY2".to_vec();
    let datum = crate::datum_codec::encode(&value.entry.canonical_datum())
        .expect("canonical entry datum encodes");
    out.extend((datum.len() as u64).to_be_bytes());
    out.extend(datum);
    put_optional_location(&mut out, value.previous_location.as_ref());
    out.extend((value.payloads.len() as u64).to_be_bytes());
    for reference in &value.payloads {
        put_id(&mut out, &reference.meaning);
        put_id(&mut out, &reference.storage);
    }
    out
}

pub(crate) fn decode_v2_entry(bytes: &[u8]) -> Result<PhysicalEntry, JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(10)? != b"SIMJENTRY2" {
        return Err(JournalError::CorruptState("entry format"));
    }
    let datum_len = usize::try_from(cursor.u64()?).map_err(|_| JournalError::WorkBoundExceeded)?;
    let datum = crate::datum_codec::decode(cursor.take(datum_len)?)?;
    let entry = entry_from_datum(datum)?;
    let previous_location = cursor.optional_location()?;
    let count = usize::try_from(cursor.u64()?).map_err(|_| JournalError::WorkBoundExceeded)?;
    let mut payloads = Vec::with_capacity(count);
    for _ in 0..count {
        payloads.push(StoredDatumRef {
            meaning: cursor.id()?,
            storage: cursor.id()?,
        });
    }
    cursor.end()?;
    Ok(PhysicalEntry {
        entry,
        previous_location,
        payloads,
    })
}

pub(crate) fn entry_from_datum(datum: Datum) -> Result<JournalEntry, JournalError> {
    let id = datum.content_id().map_err(|_| JournalError::CorruptEntry)?;
    let Datum::Node { tag, fields } = datum else {
        return Err(JournalError::CorruptEntry);
    };
    if tag != Symbol::qualified("journal", "entry-v2") || fields.len() != 4 {
        return Err(JournalError::CorruptEntry);
    }
    let field = |name: &str| {
        fields
            .iter()
            .find(|(field, _)| *field == Symbol::new(name))
            .map(|(_, value)| value)
            .ok_or(JournalError::CorruptEntry)
    };
    let sequence = match field("sequence")? {
        Datum::Number(number)
            if number.domain == Symbol::qualified("numbers", "u64")
                && number
                    .canonical
                    .parse::<u64>()
                    .ok()
                    .is_some_and(|value| value.to_string() == number.canonical) =>
        {
            number
                .canonical
                .parse()
                .map_err(|_| JournalError::CorruptEntry)?
        }
        _ => return Err(JournalError::CorruptEntry),
    };
    let previous = match field("previous")? {
        Datum::Nil => None,
        value => Some(id_from_datum(value)?),
    };
    let kind = match field("kind")? {
        Datum::Symbol(kind) => kind.clone(),
        _ => return Err(JournalError::CorruptEntry),
    };
    let payloads = match field("payloads")? {
        Datum::Vector(values) => values.iter().map(id_from_datum).collect::<Result<_, _>>()?,
        _ => return Err(JournalError::CorruptEntry),
    };
    let entry = JournalEntry {
        id,
        sequence,
        previous,
        kind,
        payloads,
    };
    if entry.canonical_id()? != entry.id {
        return Err(JournalError::CorruptEntry);
    }
    Ok(entry)
}

pub(crate) fn id_from_datum(value: &Datum) -> Result<ContentId, JournalError> {
    let Datum::Node { tag, fields } = value else {
        return Err(JournalError::CorruptEntry);
    };
    if *tag != Symbol::qualified("journal", "content-id-v1") || fields.len() != 2 {
        return Err(JournalError::CorruptEntry);
    }
    let algorithm = fields
        .iter()
        .find_map(|(name, value)| (*name == Symbol::new("algorithm")).then_some(value));
    let digest = fields
        .iter()
        .find_map(|(name, value)| (*name == Symbol::new("digest")).then_some(value));
    let (Some(Datum::Symbol(algorithm)), Some(Datum::Bytes(bytes))) = (algorithm, digest) else {
        return Err(JournalError::CorruptEntry);
    };
    let digest: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| JournalError::CorruptEntry)?;
    Ok(ContentId::from_bytes(algorithm.clone(), digest))
}

pub(crate) fn decode_v1_state(bytes: &[u8]) -> Result<(u64, Option<JournalHead>), JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(10)? != b"SIMJSTATE1" {
        return Err(JournalError::CorruptState("state format"));
    }
    let fence = cursor.u64()?;
    let head = cursor.optional_head()?;
    cursor.end()?;
    Ok((fence, head))
}

pub(crate) fn decode_v1_entry(bytes: &[u8]) -> Result<V1Entry, JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(10)? != b"SIMJENTRY1" {
        return Err(JournalError::CorruptState("entry format"));
    }
    let id = cursor.id()?;
    let sequence = cursor.u64()?;
    let previous = match cursor.byte()? {
        0 => None,
        1 => Some(cursor.id()?),
        _ => return Err(JournalError::CorruptState("entry tag")),
    };
    let kind = parse_symbol(&cursor.text()?)?;
    let count = cursor.u32()? as usize;
    let mut payloads = Vec::with_capacity(count);
    for _ in 0..count {
        payloads.push(cursor.id()?);
    }
    cursor.end()?;
    Ok(V1Entry {
        id,
        sequence,
        previous,
        kind,
        payloads,
    })
}

pub(crate) fn v1_object_id(bytes: &[u8]) -> ContentId {
    ContentId::from_bytes(
        Symbol::qualified("journal", "sha256-bytes-v1"),
        Sha256::digest(bytes).into(),
    )
}

pub(crate) fn put_v1_optional_id_hash(hasher: &mut Sha256, id: Option<&ContentId>) {
    match id {
        Some(id) => {
            hasher.update([1]);
            put_v1_id_hash(hasher, id);
        }
        None => hasher.update([0]),
    }
}

pub(crate) fn put_v1_id_hash(hasher: &mut Sha256, id: &ContentId) {
    put_v1_symbol_hash(hasher, &id.algorithm);
    hasher.update(id.bytes);
}

pub(crate) fn put_v1_symbol_hash(hasher: &mut Sha256, symbol: &Symbol) {
    let text = symbol.as_qualified_str();
    hasher.update((text.len() as u64).to_be_bytes());
    hasher.update(text.as_bytes());
}

pub(crate) fn namespace_root(namespace: &EntryNamespace) -> Vec<String> {
    vec!["namespaces-v2".into(), namespace.0.clone()]
}

pub(crate) fn ensure_entry_dirs(
    port: &dyn HostDirPort,
    location: &EntryLocation,
) -> Result<(), JournalError> {
    let root = [namespace_root(&location.namespace), vec!["entries".into()]].concat();
    let sequence = [root, vec![format!("{:016x}", location.sequence)]].concat();
    port.create_dir(&sequence).map_err(port_error)?;
    port.create_dir(&[sequence, vec![id_key(&location.entry)]].concat())
        .map_err(port_error)
}

pub(crate) fn entry_path(location: &EntryLocation) -> Vec<String> {
    vec![
        "namespaces-v2".into(),
        location.namespace.0.clone(),
        "entries".into(),
        format!("{:016x}", location.sequence),
        id_key(&location.entry),
        id_key(&location.storage),
    ]
}

pub(crate) fn object_path(reference: &StoredDatumRef) -> Vec<String> {
    vec![
        "objects-v2".into(),
        id_key(&reference.meaning),
        id_key(&reference.storage),
    ]
}

pub(crate) fn descriptor_path(id: &ContentId) -> Vec<String> {
    vec!["compat-v1".into(), id_key(id)]
}

pub(crate) fn v1_entry_path(sequence: u64) -> Vec<String> {
    vec!["entries".into(), format!("{sequence:016x}")]
}

pub(crate) fn v1_object_path(id: &ContentId) -> Vec<String> {
    vec!["objects".into(), hex(&id.bytes)]
}

pub(crate) fn id_key(id: &ContentId) -> String {
    format!(
        "{}-{}",
        hex(id.algorithm.as_qualified_str().as_bytes()),
        hex(&id.bytes)
    )
}

pub(crate) fn parse_id_key(text: &str) -> Result<ContentId, JournalError> {
    let (algorithm, digest) = text
        .split_once('-')
        .ok_or(JournalError::CorruptState("id path"))?;
    let algorithm = String::from_utf8(unhex(algorithm)?)
        .map_err(|_| JournalError::CorruptState("id path utf8"))?;
    let digest = unhex(digest)?;
    let bytes: [u8; 32] = digest
        .try_into()
        .map_err(|_| JournalError::CorruptState("id path digest"))?;
    Ok(ContentId::from_bytes(parse_symbol(&algorithm)?, bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn unhex(text: &str) -> Result<Vec<u8>, JournalError> {
    if !text.len().is_multiple_of(2) {
        return Err(JournalError::CorruptState("hex path"));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or(JournalError::CorruptState("hex path"))?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or(JournalError::CorruptState("hex path"))?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

pub(crate) fn put_head(out: &mut Vec<u8>, head: &JournalHead) {
    out.extend(head.sequence.to_be_bytes());
    put_id(out, &head.entry);
}

pub(crate) fn put_optional_head(out: &mut Vec<u8>, head: Option<&JournalHead>) {
    match head {
        Some(head) => {
            out.push(1);
            put_head(out, head);
        }
        None => out.push(0),
    }
}

pub(crate) fn put_optional_location(out: &mut Vec<u8>, location: Option<&EntryLocation>) {
    match location {
        Some(location) => {
            out.push(1);
            put_text(out, &location.namespace.0);
            out.extend(location.sequence.to_be_bytes());
            put_id(out, &location.entry);
            put_id(out, &location.storage);
        }
        None => out.push(0),
    }
}

pub(crate) fn put_id(out: &mut Vec<u8>, id: &ContentId) {
    put_text(out, &id.algorithm.as_qualified_str());
    out.extend(id.bytes);
}

pub(crate) fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend((text.len() as u32).to_be_bytes());
    out.extend(text.as_bytes());
}

pub(crate) fn parse_symbol(text: &str) -> Result<Symbol, JournalError> {
    match text.split_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            Ok(Symbol::qualified(namespace, name))
        }
        None => Symbol::checked(text).map_err(|_| JournalError::CorruptState("symbol")),
        _ => Err(JournalError::CorruptState("symbol")),
    }
}

pub(crate) fn port_error(error: sim_storage_port::HostDirError) -> JournalError {
    JournalError::Backend(error.to_string())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], JournalError> {
        let end = self
            .at
            .checked_add(len)
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
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| JournalError::CorruptState("u32"))?,
        ))
    }
    fn u64(&mut self) -> Result<u64, JournalError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| JournalError::CorruptState("u64"))?,
        ))
    }
    fn text(&mut self) -> Result<String, JournalError> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| JournalError::CorruptState("utf8"))
    }
    fn id(&mut self) -> Result<ContentId, JournalError> {
        let algorithm = parse_symbol(&self.text()?)?;
        let bytes = self
            .take(32)?
            .try_into()
            .map_err(|_| JournalError::CorruptState("id"))?;
        Ok(ContentId::from_bytes(algorithm, bytes))
    }
    fn head(&mut self) -> Result<JournalHead, JournalError> {
        Ok(JournalHead {
            sequence: self.u64()?,
            entry: self.id()?,
        })
    }
    fn optional_head(&mut self) -> Result<Option<JournalHead>, JournalError> {
        match self.byte()? {
            0 => Ok(None),
            1 => Ok(Some(self.head()?)),
            _ => Err(JournalError::CorruptState("head tag")),
        }
    }
    fn optional_location(&mut self) -> Result<Option<EntryLocation>, JournalError> {
        match self.byte()? {
            0 => Ok(None),
            1 => Ok(Some(EntryLocation {
                namespace: EntryNamespace(self.text()?),
                sequence: self.u64()?,
                entry: self.id()?,
                storage: self.id()?,
            })),
            _ => Err(JournalError::CorruptState("location tag")),
        }
    }
    fn end(&self) -> Result<(), JournalError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(JournalError::CorruptState("trailing bytes"))
        }
    }
}
