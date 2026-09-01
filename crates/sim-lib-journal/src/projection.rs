use crate::{JournalEntry, Verification};
use sim_kernel::ContentId;

/// Detached row in a read-only projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionRow {
    Head { sequence: u64, entry: ContentId },
    Entry(JournalEntry),
    Object(ContentId),
}

/// Read-only flat Table-shaped snapshot. It exposes no mutation method and is
/// never consulted by journal authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableProjection {
    rows: Vec<ProjectionRow>,
}

impl TableProjection {
    pub(crate) fn from_verification(value: Verification) -> Self {
        Self { rows: rows(value) }
    }
    pub fn rows(&self) -> &[ProjectionRow] {
        &self.rows
    }
}

/// Read-only Dir-shaped snapshot grouped by journal/object namespaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirProjection {
    journal: Vec<ProjectionRow>,
    objects: Vec<ProjectionRow>,
}

impl DirProjection {
    pub(crate) fn from_verification(value: Verification) -> Self {
        let all = rows(value);
        let (objects, journal) = all
            .into_iter()
            .partition(|r| matches!(r, ProjectionRow::Object(_)));
        Self { journal, objects }
    }
    pub fn journal(&self) -> &[ProjectionRow] {
        &self.journal
    }
    pub fn objects(&self) -> &[ProjectionRow] {
        &self.objects
    }
}

fn rows(value: Verification) -> Vec<ProjectionRow> {
    let mut rows = Vec::new();
    if let Some(h) = value.head {
        rows.push(ProjectionRow::Head {
            sequence: h.sequence,
            entry: h.entry,
        });
    }
    rows.extend(value.entries.into_iter().map(ProjectionRow::Entry));
    rows.extend(value.object_ids.into_iter().map(ProjectionRow::Object));
    rows
}
