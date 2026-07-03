//! Hash-map table backend for the SIM constellation.
//!
//! Provides a [`HashTable`] implementation satisfying the kernel `TableBackend`
//! contract, storing symbol-keyed entries in an in-memory hash map and
//! registered as a loadable library through [`install_hash_table_lib`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod citizen;
mod hash;

pub use backend::{HashBackend, HashTableLib, install_hash_table_lib};
pub use citizen::{HashTableDescriptor, hash_table_class_symbol};
pub use hash::HashTable;

#[cfg(test)]
mod tests;
