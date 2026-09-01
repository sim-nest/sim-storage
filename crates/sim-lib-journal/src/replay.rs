use crate::{JournalEntry, JournalError, StoredState, verify::verify_state};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    entries: Vec<JournalEntry>,
    cursor: usize,
}

impl Iterator for Replay {
    type Item = JournalEntry;
    fn next(&mut self) -> Option<Self::Item> {
        let item = self.entries.get(self.cursor).cloned();
        self.cursor += usize::from(item.is_some());
        item
    }
}

pub fn replay(state: StoredState) -> Result<Replay, JournalError> {
    let verified = verify_state(&state)?;
    Ok(Replay {
        entries: verified.entries,
        cursor: 0,
    })
}
