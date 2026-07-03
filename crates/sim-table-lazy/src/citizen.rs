//! Citizen descriptor for the lazy table class, mapping forced table entries to
//! and from their serialized expression form via [`lazy_table_class_symbol`]
//! and [`LazyTableDescriptor`].

use sim_citizen_derive::Citizen;
use sim_kernel::{Expr, Symbol};

/// Serialized form of a [`LazyTable`](crate::LazyTable): the citizen descriptor
/// holding the table's (forced) entries as key/expression pairs.
///
/// Produced when a lazy table is encoded -- which forces every loader -- and
/// read back when a `table/LazyTable` constructor is decoded.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "table/LazyTable", version = 0)]
pub struct LazyTableDescriptor {
    /// Forced table entries as key/expression pairs, serialized via the shared
    /// `sim_table_core::citizen_fields::entries` codec.
    #[citizen(with = "sim_table_core::citizen_fields::entries")]
    pub entries: Vec<(Symbol, Expr)>,
}

/// The fully qualified class symbol (`table/LazyTable`) for the lazy table
/// citizen.
pub fn lazy_table_class_symbol() -> Symbol {
    Symbol::qualified("table", "LazyTable")
}
