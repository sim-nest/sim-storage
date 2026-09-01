/// A writer generation. Only the backend can mint a live fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub(crate) fence: u64,
}

impl Lease {
    pub fn fence(&self) -> u64 {
        self.fence
    }
}
