//! Lazy list backend for the SIM constellation.
//!
//! Provides deferred and streamed list implementations satisfying the kernel
//! `ListBackend` contract: [`LazyConsList`] computes head and tail on demand,
//! and [`LazyIterList`] adapts an iterator into a list. Registered as a
//! loadable library through [`install_lazy_list_lib`].

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod citizen;
mod iter;
mod lazy;

pub use backend::{IterBackend, LazyBackend, LazyListLib, install_lazy_list_lib};
pub use citizen::{
    LazyConsListDescriptor, LazyIterListDescriptor, lazy_cons_list_class_symbol,
    lazy_iter_list_class_symbol,
};
pub use iter::LazyIterList;
pub use lazy::{HeadFn, LazyConsList, TailFn, UnfoldStep, unfold};

#[cfg(test)]
mod tests;
