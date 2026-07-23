#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! Mounted Table/Dir namespace for SIM storage backends.
//!
//! [`MountedDir`] composes a root directory with explicit Table or Dir mount
//! points. Directory operations route by the longest valid mounted prefix, and
//! table leaves remain leaves. Each operation delegates to the mounted backend
//! that owns the selected path, preserving that backend's capability checks,
//! read-only behavior, errors, and live state.

pub mod capabilities;
mod mount_dir;
mod ops;
mod routing;

pub use capabilities::table_mount_capability;
pub use mount_dir::{MountInspection, MountKind, MountedDir};
pub use ops::{
    install_mount_dir_lib, mount_create_symbol, mount_dir_symbol, mount_inspect_symbol,
    mount_table_symbol, mount_unmount_symbol,
};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
