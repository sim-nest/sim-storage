//! The [`HashTable`] object: an in-memory, hash-map-backed table that
//! implements the kernel table and object-encoding contracts.

use std::{
    collections::HashMap,
    sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use sim_kernel::{
    Cx, Error, Expr, Object, ObjectEncode, ObjectEncoding, Result, Symbol, Table, Value,
    id::CORE_TABLE_CLASS_ID, object::ClassRef,
};

use crate::citizen::hash_table_class_symbol;

/// In-memory table backed by a hash map of symbol keys to [`Value`]s.
///
/// Implements the kernel [`Table`] contract (`get`/`set`/`has`/`del`/`keys`/
/// `entries`/`len`/`clear`) and the object-encoding contracts, so it can be
/// stored as an opaque object and round-tripped through its
/// [`HashTableDescriptor`](crate::HashTableDescriptor) citizen form. The map is
/// guarded by an `RwLock`, so a `HashTable` is shareable and mutable through a
/// shared reference.
pub struct HashTable {
    inner: RwLock<HashMap<Symbol, Value>>,
}

impl Clone for HashTable {
    fn clone(&self) -> Self {
        Self {
            inner: RwLock::new(
                self.inner
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone(),
            ),
        }
    }
}

impl HashTable {
    fn read(&self) -> Result<RwLockReadGuard<'_, HashMap<Symbol, Value>>> {
        self.inner
            .read()
            .map_err(|_| Error::Eval("table/hash lock poisoned".into()))
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, HashMap<Symbol, Value>>> {
        self.inner
            .write()
            .map_err(|_| Error::Eval("table/hash lock poisoned".into()))
    }

    /// Construct an empty hash table.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Construct a hash table pre-populated with `entries`.
    ///
    /// Later entries with the same key overwrite earlier ones.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use sim_kernel::{Cx, DefaultFactory, NoopEvalPolicy, Symbol, Table};
    /// use sim_table_hash::HashTable;
    ///
    /// let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
    /// let value = cx.factory().bool(true).unwrap();
    /// let table = HashTable::with_entries(vec![(Symbol::new("a"), value.clone())]);
    ///
    /// assert_eq!(table.len(&mut cx).unwrap(), 1);
    /// assert!(table.has(&mut cx, Symbol::new("a")).unwrap());
    /// assert_eq!(table.get(&mut cx, Symbol::new("a")).unwrap(), value);
    /// ```
    pub fn with_entries(entries: Vec<(Symbol, Value)>) -> Self {
        Self {
            inner: RwLock::new(entries.into_iter().collect()),
        }
    }

    fn descriptor_entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Expr)>> {
        let mut entries = self
            .read()?
            .iter()
            .map(|(key, value)| Ok((key.clone(), value.object().as_expr(cx)?)))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(entries)
    }
}

impl Default for HashTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Object for HashTable {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("table/hash[{}]", self.read()?.len()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for HashTable {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = hash_table_class_symbol();
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

impl ObjectEncode for HashTable {
    fn object_encoding(&self, cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: hash_table_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                sim_table_core::citizen_fields::entries::encode(&self.descriptor_entries(cx)?),
            ],
        })
    }
}

impl sim_citizen::Citizen for HashTable {
    fn citizen_symbol() -> Symbol {
        hash_table_class_symbol()
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

impl Table for HashTable {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "hash")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        match self.read()?.get(&key) {
            Some(value) => Ok(value.clone()),
            None => cx.factory().nil(),
        }
    }

    fn set(&self, _cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        self.write()?.insert(key, value);
        Ok(())
    }

    fn has(&self, _cx: &mut Cx, key: Symbol) -> Result<bool> {
        Ok(self.read()?.contains_key(&key))
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        match self.write()?.remove(&key) {
            Some(value) => Ok(value),
            None => cx.factory().nil(),
        }
    }

    fn keys(&self, _cx: &mut Cx) -> Result<Vec<Symbol>> {
        Ok(self.read()?.keys().cloned().collect())
    }

    fn entries(&self, _cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        Ok(self
            .read()?
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }

    fn len(&self, _cx: &mut Cx) -> Result<usize> {
        Ok(self.read()?.len())
    }

    fn clear(&self, _cx: &mut Cx) -> Result<()> {
        self.write()?.clear();
        Ok(())
    }
}
