//! Override (overlay) table backend for the SIM constellation.
//!
//! Provides an [`OverrideTable`] that layers one or more tables over a base
//! table, resolving lookups front-to-back so upper layers shadow lower ones.
//! It satisfies the kernel table contract and is registered as a loadable
//! library through [`install_override_table_lib`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod class;
mod install;
mod override_table;

pub use class::construct_override_table;
pub use install::install_override_table_lib;
pub use override_table::OverrideTable;

#[cfg(test)]
mod tests;
