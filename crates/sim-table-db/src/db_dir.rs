//! The [`DbDir`] object and its library registration: a path-addressed,
//! capability-gated directory tree of symbol-keyed values implementing the
//! kernel table and directory contracts, plus the [`install_db_dir_lib`] entry
//! point.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use sim_kernel::{
    Cx, Error, Expr, Object, ObjectEncode, ObjectEncoding, Result, Symbol, Value,
    capability::{
        table_db_capability, table_db_mkdir_capability, table_db_read_capability,
        table_db_rmdir_capability, table_db_write_capability,
    },
    id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
    table::{Dir, Table},
};

use crate::citizen::db_dir_class_symbol;

struct Store {
    values: BTreeMap<(String, Symbol), Value>,
    dirs: BTreeSet<String>,
}

/// A node in a path-addressed, capability-gated directory tree of symbol-keyed
/// values.
///
/// This is an in-memory table/directory backend, not an external database
/// engine: every `DbDir` is a view onto a shared store (a `BTreeMap` of values
/// and a `BTreeSet` of directory paths behind a `Mutex`) rooted at a particular
/// path. As a [`Table`] it reads and writes the values at its own path; as a
/// [`Dir`] it creates, opens, and removes child directories, each of which is
/// another `DbDir` sharing the same store. Cloning a `DbDir` shares the
/// underlying store. Every operation requires the relevant `table/db`
/// capability (read, write, mkdir, or rmdir), and child names must be legal
/// single path segments.
#[derive(Clone)]
pub struct DbDir {
    store: Arc<Mutex<Store>>,
    path: String,
}

impl DbDir {
    /// Open a fresh store and return a `DbDir` rooted at its top-level
    /// directory.
    pub fn open() -> Self {
        let mut dirs = BTreeSet::new();
        dirs.insert(String::new());
        Self {
            store: Arc::new(Mutex::new(Store {
                values: BTreeMap::new(),
                dirs,
            })),
            path: String::new(),
        }
    }

    fn with_store(store: Arc<Mutex<Store>>, path: String) -> Self {
        Self { store, path }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>> {
        self.store
            .lock()
            .map_err(|_| Error::Eval("table/db lock poisoned".into()))
    }

    fn child_path(&self, name: &Symbol) -> Result<String> {
        let segment = name.name.as_ref();
        if !sim_table_core::is_legal_table_segment(segment) {
            return Err(Error::Eval(format!("table/db: illegal name {segment:?}")));
        }
        Ok(if self.path.is_empty() {
            segment.to_owned()
        } else {
            format!("{}/{segment}", self.path)
        })
    }

    fn direct_subdirs(&self, store: &Store) -> Vec<Symbol> {
        let prefix = if self.path.is_empty() {
            None
        } else {
            Some(format!("{}/", self.path))
        };
        let mut names = BTreeSet::new();
        for path in &store.dirs {
            if path.is_empty() || *path == self.path {
                continue;
            }
            let Some(rest) = prefix
                .as_ref()
                .map_or_else(|| Some(path.as_str()), |prefix| path.strip_prefix(prefix))
            else {
                continue;
            };
            if let Some((head, tail)) = rest.split_once('/') {
                if !head.is_empty() && !tail.is_empty() {
                    names.insert(Symbol::new(head));
                }
            } else if !rest.is_empty() {
                names.insert(Symbol::new(rest));
            }
        }
        names.into_iter().collect()
    }

    fn descriptor_path(&self) -> Vec<String> {
        if self.path.is_empty() {
            Vec::new()
        } else {
            self.path
                .split('/')
                .map(std::borrow::ToOwned::to_owned)
                .collect()
        }
    }
}

impl Default for DbDir {
    fn default() -> Self {
        Self::open()
    }
}

impl Object for DbDir {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        if self.path.is_empty() {
            Ok("table/db[/]".to_owned())
        } else {
            Ok(format!("table/db[/{}]", self.path))
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for DbDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = db_dir_class_symbol();
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
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_dir(&self) -> Option<&dyn Dir> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for DbDir {
    fn object_encoding(&self, _cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: db_dir_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                sim_table_core::citizen_fields::path_segments::encode(&self.descriptor_path()),
            ],
        })
    }
}

impl sim_citizen::Citizen for DbDir {
    fn citizen_symbol() -> Symbol {
        db_dir_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        1
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["path"]
    }
}

impl Table for DbDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "db")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        cx.require(&table_db_read_capability())?;
        let value = self.lock()?.values.get(&(self.path.clone(), key)).cloned();
        match value {
            Some(value) => Ok(value),
            None => cx.factory().nil(),
        }
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        cx.require(&table_db_write_capability())?;
        let path = self.child_path(&key)?;
        let mut store = self.lock()?;
        if store.dirs.contains(&path) {
            return Err(Error::Eval(format!("table/db: {key} is a directory")));
        }
        store.values.insert((self.path.clone(), key), value);
        Ok(())
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        cx.require(&table_db_read_capability())?;
        let path = self.child_path(&key)?;
        let store = self.lock()?;
        Ok(store.values.contains_key(&(self.path.clone(), key)) || store.dirs.contains(&path))
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        cx.require(&table_db_write_capability())?;
        let value = self.lock()?.values.remove(&(self.path.clone(), key));
        match value {
            Some(value) => Ok(value),
            None => cx.factory().nil(),
        }
    }

    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        cx.require(&table_db_read_capability())?;
        let store = self.lock()?;
        let mut keys = BTreeSet::new();
        for (path, key) in store.values.keys() {
            if *path == self.path {
                keys.insert(key.clone());
            }
        }
        for key in self.direct_subdirs(&store) {
            keys.insert(key);
        }
        Ok(keys.into_iter().collect())
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        cx.require(&table_db_read_capability())?;
        let store = self.lock()?;
        Ok(store
            .values
            .iter()
            .filter(|((path, _), _)| *path == self.path)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect())
    }

    fn len(&self, cx: &mut Cx) -> Result<usize> {
        Ok(self.entries(cx)?.len())
    }

    fn clear(&self, cx: &mut Cx) -> Result<()> {
        cx.require(&table_db_write_capability())?;
        self.lock()?
            .values
            .retain(|(path, _), _| *path != self.path);
        Ok(())
    }
}

impl Dir for DbDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        cx.require(&table_db_mkdir_capability())?;
        let path = self.child_path(&name)?;
        let mut store = self.lock()?;
        if store
            .values
            .contains_key(&(self.path.clone(), name.clone()))
        {
            return Err(Error::Eval(format!("table/db: {name} is a file")));
        }
        store.dirs.insert(path.clone());
        cx.factory()
            .opaque(Arc::new(Self::with_store(self.store.clone(), path)))
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        cx.require(&table_db_read_capability())?;
        let path = self.child_path(&name)?;
        let store = self.lock()?;
        if store.dirs.contains(&path) {
            return Ok(Some(
                cx.factory()
                    .opaque(Arc::new(Self::with_store(self.store.clone(), path)))?,
            ));
        }
        if store
            .values
            .contains_key(&(self.path.clone(), name.clone()))
        {
            return Err(Error::Eval(format!("table/db: {name} is not a directory")));
        }
        Ok(None)
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        cx.require(&table_db_rmdir_capability())?;
        let path = self.child_path(&name)?;
        let mut store = self.lock()?;
        if !store.dirs.contains(&path) {
            return Err(Error::Eval(format!("table/db: {name} is not a directory")));
        }
        let prefix = format!("{path}/");
        store
            .values
            .retain(|(entry_path, _), _| *entry_path != path && !entry_path.starts_with(&prefix));
        store
            .dirs
            .retain(|dir_path| *dir_path != path && !dir_path.starts_with(&prefix));
        cx.factory().nil()
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        cx.require(&table_db_read_capability())?;
        let path = self.child_path(&name)?;
        Ok(self.lock()?.dirs.contains(&path))
    }
}

/// Open a fresh db-directory store and return its root [`DbDir`] wrapped as an
/// opaque table/directory object.
///
/// # Errors
///
/// Returns a capability error if the `table/db` capability has not been granted
/// to `cx`. Note that the individual read, write, mkdir, and rmdir operations
/// on the returned directory are gated by their own capabilities.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use sim_kernel::{
///     Cx, DefaultFactory, EagerPolicy, Symbol, Table,
///     capability::{table_db_capability, table_db_read_capability, table_db_write_capability},
/// };
/// use sim_table_db::install_db_dir_lib;
///
/// let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
/// cx.grant(table_db_capability());
/// cx.grant(table_db_read_capability());
/// cx.grant(table_db_write_capability());
///
/// let root = install_db_dir_lib(&mut cx).unwrap();
/// let table = root.object().as_table_impl().unwrap();
/// let value = cx.factory().string("v".to_owned()).unwrap();
/// table.set(&mut cx, Symbol::new("k"), value.clone()).unwrap();
/// assert_eq!(table.get(&mut cx, Symbol::new("k")).unwrap(), value);
/// ```
pub fn install_db_dir_lib(cx: &mut Cx) -> Result<Value> {
    cx.require(&table_db_capability())?;
    cx.factory().opaque(Arc::new(DbDir::open()))
}
