//! Library registration for the lazy list backends: the [`LazyBackend`] and
//! [`IterBackend`] `ListBackend` implementations, the [`LazyListLib`] manifest,
//! and the [`install_lazy_list_lib`] entry point.

use std::sync::Arc;

use sim_kernel::{
    AbiVersion, Cx, Dependency, Error, Export, Lib, LibManifest, LibTarget, Linker, ListBackend,
    Result, Symbol, Value, Version,
};

use crate::{LazyConsList, LazyIterList};

/// The `lazy` [`ListBackend`]: constructs [`LazyConsList`] objects whose head
/// and tail are computed on demand.
pub struct LazyBackend;
/// The `iter` [`ListBackend`]: constructs [`LazyIterList`] objects backed by an
/// iterator over the supplied items.
pub struct IterBackend;

impl ListBackend for LazyBackend {
    fn name(&self) -> &str {
        "lazy"
    }

    fn new_list(&self, cx: &mut Cx, items: Vec<Value>) -> Result<Value> {
        cx.factory().opaque(finite_chain(items))
    }

    fn new_cons(&self, cx: &mut Cx, car: Value, cdr: Value) -> Result<Value> {
        ensure_list_tail(&cdr)?;
        let head = car.clone();
        let tail = cdr.clone();
        cx.factory().opaque(Arc::new(LazyConsList::new(
            move |_cx| Ok(head.clone()),
            move |_cx| Ok(Some(tail.clone())),
        )))
    }
}

impl ListBackend for IterBackend {
    fn name(&self) -> &str {
        "iter"
    }

    fn new_list(&self, cx: &mut Cx, items: Vec<Value>) -> Result<Value> {
        cx.factory().opaque(Arc::new(LazyIterList::new(Box::new(
            items.into_iter().map(Ok),
        ))))
    }

    fn new_cons(&self, cx: &mut Cx, car: Value, cdr: Value) -> Result<Value> {
        ensure_list_tail(&cdr)?;
        cx.factory()
            .opaque(Arc::new(LazyIterList::prepend(car, cdr)))
    }
}

fn finite_chain(items: Vec<Value>) -> Arc<LazyConsList> {
    chain_from(Arc::from(items), 0)
}

/// Builds the lazy cons node for `items[index..]` without copying the tail.
///
/// Every node shares the single `Arc<[Value]>` and carries only an integer
/// offset, so a fully traversed length-`n` list retains O(n) memory (one shared
/// slice plus O(1) per node) rather than the O(n^2) that per-node `Vec` clones
/// would cost.
fn chain_from(items: Arc<[Value]>, index: usize) -> Arc<LazyConsList> {
    if index >= items.len() {
        return Arc::new(LazyConsList::empty());
    }
    let head_items = Arc::clone(&items);
    Arc::new(LazyConsList::new(
        move |_cx| Ok(head_items[index].clone()),
        move |cx| {
            let next = index + 1;
            if next >= items.len() {
                Ok(None)
            } else {
                Ok(Some(
                    cx.factory().opaque(chain_from(Arc::clone(&items), next))?,
                ))
            }
        },
    ))
}

fn ensure_list_tail(value: &Value) -> Result<()> {
    if value.object().as_list().is_some() {
        Ok(())
    } else {
        Err(Error::TypeMismatch {
            expected: "list",
            found: "non-list",
        })
    }
}

/// The loadable [`Lib`] manifest for the lazy list backends, registered under
/// the `list/lazy` id by [`install_lazy_list_lib`].
pub struct LazyListLib;

impl Lib for LazyListLib {
    // Empty-manifest boilerplate is intentionally local: unlike the table
    // backends (which share sim_table_core::backend_manifest), the list crates
    // have no shared list-core owner yet, so a table-crate dep would be wrong.
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: Symbol::qualified("list", "lazy"),
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

/// Registers the [`LazyBackend`] and [`IterBackend`] in the list registry and
/// loads [`LazyListLib`], making lazy lists available to `cx`.
///
/// Idempotent: returns early if the `list/lazy` lib is already present.
pub fn install_lazy_list_lib(cx: &mut Cx) -> Result<()> {
    if cx
        .registry()
        .lib(&Symbol::qualified("list", "lazy"))
        .is_some()
    {
        return Ok(());
    }
    cx.list_registry_mut().register(Arc::new(LazyBackend));
    cx.list_registry_mut().register(Arc::new(IterBackend));
    cx.load_lib(&LazyListLib).map(|_| ())
}
