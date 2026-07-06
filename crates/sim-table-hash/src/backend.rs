//! Library registration for the hash-map table backend: the [`HashBackend`]
//! `TableBackend` implementation, the [`HashTableLib`] manifest, and the
//! [`install_hash_table_lib`] entry point.

use std::sync::Arc;

use sim_kernel::{Cx, Lib, LibManifest, Linker, Result, Symbol, TableBackend, Value};

use crate::HashTable;

/// Table backend that constructs [`HashTable`] objects for the `hash` backend
/// name.
///
/// Registered with the context table registry by [`install_hash_table_lib`] so
/// that table operations resolved against the `hash` backend build hash-map
/// tables.
pub struct HashBackend;

impl TableBackend for HashBackend {
    fn name(&self) -> &str {
        "hash"
    }

    fn new_table(&self, cx: &mut Cx, entries: Vec<(Symbol, Value)>) -> Result<Value> {
        cx.factory()
            .opaque(Arc::new(HashTable::with_entries(entries)))
    }
}

/// Loadable library manifest for the hash table backend.
///
/// Declares the `table/hash` library identity and ABI; backend registration is
/// performed separately by [`install_hash_table_lib`].
pub struct HashTableLib;

impl Lib for HashTableLib {
    fn manifest(&self) -> LibManifest {
        sim_table_core::backend_manifest(
            Symbol::qualified("table", "hash"),
            env!("CARGO_PKG_VERSION"),
        )
    }

    fn load(&self, _cx: &mut sim_kernel::LoadCx, _linker: &mut Linker<'_>) -> Result<()> {
        Ok(())
    }
}

/// Register the [`HashBackend`] and load the [`HashTableLib`] into `cx`.
///
/// Idempotent: if the `table/hash` library is already present the call is a
/// no-op. After installation the `hash` backend can be selected as the active
/// table backend via the context table registry.
pub fn install_hash_table_lib(cx: &mut Cx) -> Result<()> {
    if cx
        .registry()
        .lib(&Symbol::qualified("table", "hash"))
        .is_some()
    {
        return Ok(());
    }
    cx.table_registry_mut().register(Arc::new(HashBackend));
    cx.load_lib(&HashTableLib).map(|_| ())
}
