//! Lazy table backend for the SIM constellation.
//!
//! Provides a [`LazyTable`] implementation satisfying the kernel `TableBackend`
//! contract, where entry values are produced by [`ValueLoader`] closures that
//! run at most once and memoize their result. Registered as a loadable library
//! through [`install_lazy_table_lib`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod citizen;
mod lazy;

pub use backend::{LazyBackend, LazyTableLib, install_lazy_table_lib};
pub use citizen::{LazyTableDescriptor, lazy_table_class_symbol};
pub use lazy::{LazyTable, ValueLoader};

#[cfg(test)]
mod tests;
