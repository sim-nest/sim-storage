//! Capability names used by database-backed tables.

use sim_kernel::CapabilityName;

/// The capability gating database-backed tables (`table.db`).
pub fn table_db_capability() -> CapabilityName {
    CapabilityName::new("table.db")
}

/// The capability gating database table reads (`table.db.read`).
pub fn table_db_read_capability() -> CapabilityName {
    CapabilityName::new("table.db.read")
}

/// The capability gating database table writes (`table.db.write`).
pub fn table_db_write_capability() -> CapabilityName {
    CapabilityName::new("table.db.write")
}

/// The capability gating database table directory creation (`table.db.mkdir`).
pub fn table_db_mkdir_capability() -> CapabilityName {
    CapabilityName::new("table.db.mkdir")
}

/// The capability gating database table directory removal (`table.db.rmdir`).
pub fn table_db_rmdir_capability() -> CapabilityName {
    CapabilityName::new("table.db.rmdir")
}

#[cfg(test)]
mod tests {
    use super::{
        table_db_capability, table_db_mkdir_capability, table_db_read_capability,
        table_db_rmdir_capability, table_db_write_capability,
    };

    #[test]
    fn capability_tokens_are_stable() {
        assert_eq!(table_db_capability().as_str(), "table.db");
        assert_eq!(table_db_read_capability().as_str(), "table.db.read");
        assert_eq!(table_db_write_capability().as_str(), "table.db.write");
        assert_eq!(table_db_mkdir_capability().as_str(), "table.db.mkdir");
        assert_eq!(table_db_rmdir_capability().as_str(), "table.db.rmdir");
    }
}
