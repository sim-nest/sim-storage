//! Citizen descriptor for the hash table class, mapping table entries to and
//! from their serialized expression form via [`hash_table_class_symbol`] and
//! [`HashTableDescriptor`].

use sim_citizen_derive::Citizen;
use sim_kernel::{Expr, Symbol};

/// Serialized form of a [`HashTable`](crate::HashTable): the citizen descriptor
/// holding the table's entries as key/expression pairs.
///
/// Produced when a hash table is encoded and read back when a `table/HashTable`
/// constructor is decoded.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "table/HashTable", version = 0)]
pub struct HashTableDescriptor {
    /// Table entries as key/expression pairs, serialized via the shared
    /// `sim_table_core::citizen_fields::entries` codec.
    #[citizen(with = "sim_table_core::citizen_fields::entries")]
    pub entries: Vec<(Symbol, Expr)>,
}

/// The fully qualified class symbol (`table/HashTable`) for the hash table
/// citizen.
pub fn hash_table_class_symbol() -> Symbol {
    Symbol::qualified("table", "HashTable")
}
