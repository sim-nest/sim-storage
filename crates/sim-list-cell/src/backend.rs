//! Library registration for the cons-cell list backend: the [`ConsBackend`]
//! `ListBackend` implementation, the [`ConsListLib`] manifest, and the
//! [`install_cons_list_lib`] entry point.

use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Cx, Dependency, Error, Export, Lib, LibManifest, LibTarget, Linker, ListBackend,
    Result, Symbol, Value, Version,
};

use crate::ConsList;

/// The `cons` [`ListBackend`]: constructs [`ConsList`] objects for the
/// runtime's list-construction and `cons` operations.
pub struct ConsBackend;

impl ListBackend for ConsBackend {
    fn name(&self) -> &str {
        "cons"
    }

    fn new_list(&self, cx: &mut Cx, items: Vec<Value>) -> Result<Value> {
        cx.factory().opaque(ConsList::from_vec(items))
    }

    fn new_cons(&self, cx: &mut Cx, car: Value, cdr: Value) -> Result<Value> {
        match cdr.object().downcast_ref::<ConsList>() {
            Some(cons) => cx
                .factory()
                .opaque(Arc::new(ConsList::cell(car, Arc::new(cons.clone())))),
            None => {
                // A non-`ConsList` tail is kept lazily rather than materialized,
                // so consing onto a lazy or unbounded list does not realize its
                // spine.
                if cdr.object().as_list().is_none() {
                    return Err(Error::TypeMismatch {
                        expected: "list",
                        found: "non-list",
                    });
                }
                cx.factory()
                    .opaque(Arc::new(ConsList::cell_foreign(car, cdr)))
            }
        }
    }
}

/// The loadable [`Lib`] manifest for the cons-cell list backend, registered
/// under the `list/cons` id by [`install_cons_list_lib`].
pub struct ConsListLib;

impl Lib for ConsListLib {
    // Empty-manifest boilerplate is intentionally local: unlike the table
    // backends (which share sim_table_core::backend_manifest), the list crates
    // have no shared list-core owner yet, so a table-crate dep would be wrong.
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("list", "cons"),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::<Dependency>::new(),
            capabilities: Vec::new(),
            exports: Vec::<Export>::new(),
        }
    }

    fn load(&self, _cx: &mut sim_kernel::LoadCx, _linker: &mut Linker<'_>) -> Result<()> {
        Ok(())
    }
}

/// Registers the [`ConsBackend`] in the list registry and loads
/// [`ConsListLib`], making cons-cell lists available to `cx`.
///
/// Idempotent: returns early if the `list/cons` lib is already present.
pub fn install_cons_list_lib(cx: &mut Cx) -> Result<()> {
    if cx
        .registry()
        .lib(&Symbol::qualified("list", "cons"))
        .is_some()
    {
        return Ok(());
    }
    cx.list_registry_mut().register(Arc::new(ConsBackend));
    cx.load_lib(&ConsListLib).map(|_| ())
}
