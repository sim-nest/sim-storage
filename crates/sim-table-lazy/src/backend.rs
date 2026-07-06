//! Library registration for the lazy table backend: the [`LazyBackend`]
//! `TableBackend` implementation, the [`LazyTableLib`] manifest, and the
//! [`install_lazy_table_lib`] entry point.

use std::sync::Arc;

use sim_kernel::{Cx, Lib, LibManifest, Linker, Result, Symbol, TableBackend, Value};

use crate::LazyTable;

/// Table backend that constructs [`LazyTable`] objects for the `lazy` backend
/// name.
///
/// Registered with the context table registry by [`install_lazy_table_lib`].
/// Tables built through this backend hold the supplied entries eagerly; lazy
/// loaders are added afterwards via [`LazyTable::put_lazy`].
pub struct LazyBackend;

impl TableBackend for LazyBackend {
    fn name(&self) -> &str {
        "lazy"
    }

    fn new_table(&self, cx: &mut Cx, entries: Vec<(Symbol, Value)>) -> Result<Value> {
        cx.factory()
            .opaque(Arc::new(LazyTable::with_entries(entries)))
    }
}

/// Loadable library manifest for the lazy table backend.
///
/// Declares the `table/lazy` library identity and ABI; backend registration is
/// performed separately by [`install_lazy_table_lib`].
pub struct LazyTableLib;

impl Lib for LazyTableLib {
    fn manifest(&self) -> LibManifest {
        sim_table_core::backend_manifest(
            Symbol::qualified("table", "lazy"),
            env!("CARGO_PKG_VERSION"),
        )
    }

    fn load(&self, _cx: &mut sim_kernel::LoadCx, _linker: &mut Linker<'_>) -> Result<()> {
        Ok(())
    }
}

/// Register the [`LazyBackend`] and load the [`LazyTableLib`] into `cx`.
///
/// Idempotent: if the `table/lazy` library is already present the call is a
/// no-op. After installation the `lazy` backend can be selected as the active
/// table backend via the context table registry.
pub fn install_lazy_table_lib(cx: &mut Cx) -> Result<()> {
    let lib_id = Symbol::qualified("table", "lazy");
    if cx.registry().lib(&lib_id).is_some() {
        return Ok(());
    }
    cx.table_registry_mut().register(Arc::new(LazyBackend));
    cx.load_lib(&LazyTableLib).map(|_| ())
}
