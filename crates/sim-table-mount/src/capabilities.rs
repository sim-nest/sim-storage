//! Capability names used by mounted Table/Dir namespaces.

use sim_kernel::CapabilityName;

/// Capability required to create or mutate a mounted namespace registry.
pub fn table_mount_capability() -> CapabilityName {
    CapabilityName::new("table.mount")
}

#[cfg(test)]
mod tests {
    use super::table_mount_capability;

    #[test]
    fn capability_token_is_stable() {
        assert_eq!(table_mount_capability().as_str(), "table.mount");
    }
}
