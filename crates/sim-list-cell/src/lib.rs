//! Cell-based list backend for the SIM constellation.
//!
//! Provides a mutable cons-cell list implementation satisfying the kernel
//! `ListBackend` contract, registered as a loadable library through
//! [`install_cons_list_lib`]. Lists are built from shared [`ConsList`] cells.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod citizen;
mod cons;

pub use backend::{ConsBackend, ConsListLib, install_cons_list_lib};
pub use citizen::{ConsListDescriptor, cons_list_class_symbol};
pub use cons::ConsList;

#[cfg(test)]
mod tests;
