use sim_kernel::{Datum, NumberLiteral, Symbol};

use crate::JournalError;

pub(crate) fn encode(value: &Datum) -> Result<Vec<u8>, JournalError> {
    value
        .canonical_bytes()
        .map_err(|_| JournalError::NonCanonicalDatum)?;
    let mut out = b"SIMJDATUM1".to_vec();
    put_datum(&mut out, value)?;
    Ok(out)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Datum, JournalError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(10)? != b"SIMJDATUM1" {
        return Err(JournalError::CorruptState("datum format"));
    }
    let value = cursor.datum()?;
    cursor.end()?;
    value
        .canonical_bytes()
        .map_err(|_| JournalError::NonCanonicalDatum)?;
    Ok(value)
}

fn put_datum(out: &mut Vec<u8>, value: &Datum) -> Result<(), JournalError> {
    match value {
        Datum::Nil => out.push(0),
        Datum::Bool(false) => out.push(1),
        Datum::Bool(true) => out.push(2),
        Datum::Number(number) => {
            out.push(3);
            put_symbol(out, &number.domain);
            put_bytes(out, number.canonical.as_bytes())?;
        }
        Datum::Symbol(symbol) => {
            out.push(4);
            put_symbol(out, symbol);
        }
        Datum::String(text) => {
            out.push(5);
            put_bytes(out, text.as_bytes())?;
        }
        Datum::Bytes(bytes) => {
            out.push(6);
            put_bytes(out, bytes)?;
        }
        Datum::List(items) => {
            out.push(7);
            put_items(out, items)?;
        }
        Datum::Vector(items) => {
            out.push(8);
            put_items(out, items)?;
        }
        Datum::Map(entries) => {
            out.push(9);
            let mut ordered = entries.to_vec();
            ordered.sort_by_key(|(key, _)| key.canonical_bytes().unwrap_or_default());
            put_len(out, ordered.len())?;
            for (key, value) in &ordered {
                put_datum(out, key)?;
                put_datum(out, value)?;
            }
        }
        Datum::Set(items) => {
            out.push(10);
            let mut ordered = items.to_vec();
            ordered.sort_by_key(|item| item.canonical_bytes().unwrap_or_default());
            put_items(out, &ordered)?;
        }
        Datum::Node { tag, fields } => {
            out.push(11);
            put_symbol(out, tag);
            let mut ordered = fields.to_vec();
            ordered.sort_by_key(|(name, _)| symbol_key(name));
            put_len(out, ordered.len())?;
            for (name, value) in &ordered {
                put_symbol(out, name);
                put_datum(out, value)?;
            }
        }
    }
    Ok(())
}

fn put_items(out: &mut Vec<u8>, items: &[Datum]) -> Result<(), JournalError> {
    put_len(out, items.len())?;
    for item in items {
        put_datum(out, item)?;
    }
    Ok(())
}

fn put_symbol(out: &mut Vec<u8>, symbol: &Symbol) {
    match &symbol.namespace {
        Some(namespace) => {
            out.push(1);
            put_bytes(out, namespace.as_bytes()).expect("symbol length fits u64");
        }
        None => out.push(0),
    }
    put_bytes(out, symbol.name.as_bytes()).expect("symbol length fits u64");
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), JournalError> {
    put_len(out, bytes.len())?;
    out.extend(bytes);
    Ok(())
}

fn put_len(out: &mut Vec<u8>, len: usize) -> Result<(), JournalError> {
    let len = u64::try_from(len).map_err(|_| JournalError::WorkBoundExceeded)?;
    out.extend(len.to_be_bytes());
    Ok(())
}

fn symbol_key(symbol: &Symbol) -> (Option<String>, String) {
    (
        symbol.namespace.as_ref().map(ToString::to_string),
        symbol.name.to_string(),
    )
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
            .ok_or(JournalError::CorruptState("datum length"))?;
        let value = self
            .bytes
            .get(self.at..end)
            .ok_or(JournalError::CorruptState("truncated datum"))?;
        self.at = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, JournalError> {
        Ok(self.take(1)?[0])
    }
    fn len(&mut self) -> Result<usize, JournalError> {
        usize::try_from(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| JournalError::CorruptState("datum length"))?,
        ))
        .map_err(|_| JournalError::WorkBoundExceeded)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, JournalError> {
        let len = self.len()?;
        Ok(self.take(len)?.to_vec())
    }
    fn text(&mut self) -> Result<String, JournalError> {
        String::from_utf8(self.bytes()?).map_err(|_| JournalError::CorruptState("datum utf8"))
    }
    fn symbol(&mut self) -> Result<Symbol, JournalError> {
        let namespace = match self.byte()? {
            0 => None,
            1 => Some(self.text()?),
            _ => return Err(JournalError::CorruptState("datum symbol")),
        };
        let name = self.text()?;
        match namespace {
            Some(namespace) => Ok(Symbol::qualified(namespace, name)),
            None => Ok(Symbol::new(name)),
        }
    }
    fn items(&mut self) -> Result<Vec<Datum>, JournalError> {
        let count = self.len()?;
        (0..count).map(|_| self.datum()).collect()
    }
    fn datum(&mut self) -> Result<Datum, JournalError> {
        Ok(match self.byte()? {
            0 => Datum::Nil,
            1 => Datum::Bool(false),
            2 => Datum::Bool(true),
            3 => Datum::Number(NumberLiteral {
                domain: self.symbol()?,
                canonical: self.text()?,
            }),
            4 => Datum::Symbol(self.symbol()?),
            5 => Datum::String(self.text()?),
            6 => Datum::Bytes(self.bytes()?),
            7 => Datum::List(self.items()?),
            8 => Datum::Vector(self.items()?),
            9 => {
                let count = self.len()?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push((self.datum()?, self.datum()?));
                }
                Datum::Map(entries)
            }
            10 => Datum::Set(self.items()?),
            11 => {
                let tag = self.symbol()?;
                let count = self.len()?;
                let mut fields = Vec::with_capacity(count);
                for _ in 0..count {
                    fields.push((self.symbol()?, self.datum()?));
                }
                Datum::Node { tag, fields }
            }
            _ => return Err(JournalError::CorruptState("datum tag")),
        })
    }
    fn end(&self) -> Result<(), JournalError> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(JournalError::CorruptState("trailing datum bytes"))
        }
    }
}
