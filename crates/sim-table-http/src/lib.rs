#![forbid(unsafe_code)]
#![deny(missing_docs)]
//! HTTP-backed table directory backend for SIM.
//!
//! `HttpDir` treats each table key as a resource below a configured base URL.
//! Reads perform bounded `GET` requests, writes perform bounded `PUT` or `POST`
//! requests, and deletes perform bounded `DELETE` requests. Every effectful
//! operation requires the canonical `net/http` capability, with the compatibility
//! aliases accepted by `sim-table-core`.

mod capabilities;
mod citizen;
mod http_dir;
mod transport;

pub use capabilities::{require_table_http, table_http_capability};
pub use citizen::{HttpDirDescriptor, http_dir_class_symbol};
pub use http_dir::{HttpDir, HttpDirOptions, HttpWriteMethod, install_http_dir_lib};

/// Cookbook recipes for this lib, embedded at build time.
pub static RECIPES: sim_cookbook::EmbeddedDir =
    include!(concat!(env!("OUT_DIR"), "/cookbook_recipes.rs"));

#[cfg(test)]
mod tests;
