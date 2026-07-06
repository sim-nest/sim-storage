//! The [`LazyTable`] object: a table whose entry values are produced on demand
//! by [`ValueLoader`] closures and memoized after their first force, while
//! satisfying the kernel table and object-encoding contracts.

use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use sim_kernel::{
    Cx, Error, Expr, Object, ObjectEncode, ObjectEncoding, Result, Symbol, Table, Value,
    id::CORE_TABLE_CLASS_ID, object::ClassRef,
};

use crate::citizen::lazy_table_class_symbol;

/// A value loader, called at most once and memoized with its first result.
pub type ValueLoader = Arc<dyn Fn(&mut Cx) -> Result<Value> + Send + Sync>;

struct LazyEntry {
    loader: ValueLoader,
    cache: OnceLock<Result<Value>>,
}

impl LazyEntry {
    fn eager(value: Value) -> Self {
        let cache = OnceLock::new();
        let _ = cache.set(Ok(value.clone()));
        Self {
            loader: Arc::new(move |_| Ok(value.clone())),
            cache,
        }
    }

    fn force(&self, cx: &mut Cx) -> Result<Value> {
        if let Some(cached) = self.cache.get() {
            return cached.clone();
        }
        let result = (self.loader)(cx);
        let _ = self.cache.set(result.clone());
        result
    }
}

/// Table whose entry values are produced on demand by [`ValueLoader`] closures
/// and memoized after their first force.
///
/// Each entry's loader runs at most once; the first result (value *or* error)
/// is cached and returned for every later access. Metadata operations
/// (`has`/`keys`/`len`) do not force loaders, while `get`/`del`/`entries` and
/// encoding do. Implements the kernel [`Table`] contract and the
/// object-encoding contracts, round-tripping through its
/// [`LazyTableDescriptor`](crate::LazyTableDescriptor) citizen form. The entry
/// map is guarded by an `RwLock`, so a `LazyTable` is shareable and mutable
/// through a shared reference.
pub struct LazyTable {
    entries: RwLock<HashMap<Symbol, Arc<LazyEntry>>>,
}

impl Clone for LazyTable {
    fn clone(&self) -> Self {
        Self {
            entries: RwLock::new(
                self.entries
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            ),
        }
    }
}

impl LazyTable {
    fn read(&self) -> Result<RwLockReadGuard<'_, HashMap<Symbol, Arc<LazyEntry>>>> {
        self.entries
            .read()
            .map_err(|_| Error::Eval("table/lazy lock poisoned".into()))
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, HashMap<Symbol, Arc<LazyEntry>>>> {
        self.entries
            .write()
            .map_err(|_| Error::Eval("table/lazy lock poisoned".into()))
    }

    /// Construct an empty lazy table.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Construct a lazy table whose entries are produced on demand by the given
    /// loaders.
    ///
    /// Each loader runs at most once, on first access of its key, and its
    /// result is memoized.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use sim_kernel::{Cx, DefaultFactory, NoopEvalPolicy, Result, Symbol, Table, Value};
    /// use sim_table_lazy::{LazyTable, ValueLoader};
    ///
    /// let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    /// let loader: ValueLoader = Arc::new(|cx: &mut Cx| cx.factory().bool(true));
    /// let table = LazyTable::with_loaders(vec![(Symbol::new("x"), loader)]);
    ///
    /// // Metadata does not force the loader.
    /// assert!(table.has(&mut cx, Symbol::new("x")).unwrap());
    /// // get forces it (once) and memoizes the result.
    /// let forced = table.get(&mut cx, Symbol::new("x")).unwrap();
    /// assert_eq!(forced, table.get(&mut cx, Symbol::new("x")).unwrap());
    /// ```
    pub fn with_loaders(pairs: Vec<(Symbol, ValueLoader)>) -> Self {
        let entries = pairs
            .into_iter()
            .map(|(key, loader)| {
                (
                    key,
                    Arc::new(LazyEntry {
                        loader,
                        cache: OnceLock::new(),
                    }),
                )
            })
            .collect();
        Self {
            entries: RwLock::new(entries),
        }
    }

    /// Construct a lazy table pre-populated with already-computed values.
    ///
    /// Each entry is stored as an eager (pre-cached) loader, so accessing it
    /// performs no further computation. Later entries with the same key
    /// overwrite earlier ones.
    pub fn with_entries(entries: Vec<(Symbol, Value)>) -> Self {
        let entries = entries
            .into_iter()
            .map(|(key, value)| (key, Arc::new(LazyEntry::eager(value))))
            .collect();
        Self {
            entries: RwLock::new(entries),
        }
    }

    /// Insert (or replace) a lazily computed entry under `key`.
    ///
    /// The `loader` runs at most once, on first access of `key`, and its result
    /// is memoized.
    pub fn put_lazy(&self, key: Symbol, loader: ValueLoader) {
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                key,
                Arc::new(LazyEntry {
                    loader,
                    cache: OnceLock::new(),
                }),
            );
    }

    fn descriptor_entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Expr)>> {
        let mut entries = self
            .entries(cx)?
            .into_iter()
            .map(|(key, value)| Ok((key, value.object().as_expr(cx)?)))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }
}

impl Default for LazyTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Object for LazyTable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("table/lazy[{}]", self.read()?.len()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for LazyTable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = lazy_table_class_symbol();
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        let symbol = Symbol::qualified("core", "Table");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol)
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }
    fn truth(&self, _cx: &mut Cx) -> Result<bool> {
        Ok(!self.read()?.is_empty())
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for LazyTable {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: lazy_table_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                sim_table_core::citizen_fields::entries::encode(&self.descriptor_entries(cx)?),
            ],
        })
    }
}

impl sim_citizen::Citizen for LazyTable {
    fn citizen_symbol() -> Symbol {
        lazy_table_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        1
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["entries"]
    }
}

impl Table for LazyTable {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "lazy")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        match self.read()?.get(&key).cloned() {
            Some(entry) => entry.force(cx),
            None => cx.factory().nil(),
        }
    }

    fn set(&self, _cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        self.write()?.insert(key, Arc::new(LazyEntry::eager(value)));
        Ok(())
    }

    fn has(&self, _cx: &mut Cx, key: Symbol) -> Result<bool> {
        Ok(self.read()?.contains_key(&key))
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        match self.write()?.remove(&key) {
            Some(entry) => entry.force(cx),
            None => cx.factory().nil(),
        }
    }

    fn keys(&self, _cx: &mut Cx) -> Result<Vec<Symbol>> {
        // Sort so iteration order is deterministic across runs, matching the
        // sorted encoder (`descriptor_entries`) and the other table backends;
        // a raw `HashMap` iteration leaks nondeterministic order.
        let mut keys: Vec<Symbol> = self.read()?.keys().cloned().collect();
        keys.sort();
        Ok(keys)
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let mut snapshot: Vec<(Symbol, Arc<LazyEntry>)> = self
            .read()?
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect();
        // Force in a deterministic key order so callers see a stable sequence.
        snapshot.sort_by(|left, right| left.0.cmp(&right.0));
        let mut out = Vec::with_capacity(snapshot.len());
        for (key, entry) in snapshot {
            out.push((key, entry.force(cx)?));
        }
        Ok(out)
    }

    fn len(&self, _cx: &mut Cx) -> Result<usize> {
        Ok(self.read()?.len())
    }

    fn clear(&self, _cx: &mut Cx) -> Result<()> {
        self.write()?.clear();
        Ok(())
    }
}
