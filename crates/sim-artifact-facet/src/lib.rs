//! Pure artifact-facet comparison and bounded three-way merge.
//!
//! Base, observed, and intended images have distinct Rust roles. A facet binds
//! one artifact, owner, region, merge policy, and disclosure decision before
//! comparison. This crate performs no filesystem, Git, process, or network I/O.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod identity;
mod image;
mod merge;

pub use identity::*;
pub use image::*;
pub use merge::*;
