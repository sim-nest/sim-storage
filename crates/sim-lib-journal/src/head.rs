use sim_kernel::ContentId;

/// The sole authoritative append position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalHead {
    pub sequence: u64,
    pub entry: ContentId,
}
