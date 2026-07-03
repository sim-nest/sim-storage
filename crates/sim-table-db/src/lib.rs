//! Db-backed table backend for the SIM constellation.
//!
//! Provides [`DbDir`], a path-addressed directory tree of symbol-keyed values
//! that satisfies the kernel table and directory contracts under capability
//! control. Registered as a loadable library through [`install_db_dir_lib`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod citizen;
mod db_dir;

pub use citizen::{DbDirDescriptor, db_dir_class_symbol};
pub use db_dir::{DbDir, install_db_dir_lib};

#[cfg(test)]
mod tests;
