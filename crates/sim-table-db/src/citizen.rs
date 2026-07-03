//! Citizen descriptor for the db-directory class, mapping a directory node to
//! and from its serialized path form via [`db_dir_class_symbol`] and
//! [`DbDirDescriptor`].

use sim_citizen_derive::Citizen;
use sim_kernel::Symbol;

/// Serialized form of a [`DbDir`](crate::DbDir): the citizen descriptor holding
/// the directory node's path within the store.
///
/// Produced when a directory node is encoded and read back when a `table/DbDir`
/// constructor is decoded. It records location only, not the store contents.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "table/DbDir", version = 0)]
pub struct DbDirDescriptor {
    /// Path segments from the store root to this directory node, serialized via
    /// the shared `sim_table_core::citizen_fields::path_segments` codec.
    #[citizen(with = "sim_table_core::citizen_fields::path_segments")]
    pub path: Vec<String>,
}

/// The fully qualified class symbol (`table/DbDir`) for the db-directory
/// citizen.
pub fn db_dir_class_symbol() -> Symbol {
    Symbol::qualified("table", "DbDir")
}
