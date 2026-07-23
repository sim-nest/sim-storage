//! Mounted directory object and routing implementation.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use sim_kernel::{
    Cx, Error, Expr, Object, Result, Symbol, Value,
    id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
    table::{Dir, Table},
};
use sim_table_core::TablePath;

use crate::{
    routing::{
        ResolvedNode, check_segment, format_path, has_mount_descendant, is_prefix, join_child,
        longest_mount, owned_path, require_dir_value, require_table_value, resolve_mounted_target,
        traverse_dir_path,
    },
    table_mount_capability,
};

pub(crate) type MountPath = Vec<String>;

#[derive(Clone)]
pub(crate) struct MountTarget {
    pub(crate) kind: MountKind,
    pub(crate) value: Value,
}

/// The mounted target kind at a mount point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountKind {
    /// A Table leaf mounted at one exact path.
    Table,
    /// A Dir mounted at one path and available below that path.
    Dir,
}

impl MountKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Dir => "dir",
        }
    }
}

/// One row returned by [`MountedDir::inspect`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountInspection {
    /// Absolute mount point.
    pub path: String,
    /// Mounted target kind.
    pub kind: MountKind,
}

/// A live Table/Dir namespace assembled from a root Dir and explicit mounts.
#[derive(Clone)]
pub struct MountedDir {
    root: Value,
    mounts: Arc<Mutex<BTreeMap<MountPath, MountTarget>>>,
    path: MountPath,
}

impl MountedDir {
    /// Create a mounted namespace rooted at `root`.
    ///
    /// `root` must be a directory value. The namespace starts with no mounts.
    pub fn new(root: Value) -> Result<Self> {
        require_dir_value(&root, "table/mount: root must be a Dir")?;
        Ok(Self {
            root,
            mounts: Arc::new(Mutex::new(BTreeMap::new())),
            path: Vec::new(),
        })
    }

    /// Mount `target` as a directory at absolute `path`.
    pub fn mount_dir(&self, cx: &mut Cx, path: TablePath, target: Value) -> Result<()> {
        cx.require(&table_mount_capability())?;
        require_dir_value(&target, "table/mount: Dir mount target must be a Dir")?;
        self.insert_mount(cx, path, MountKind::Dir, target)
    }

    /// Mount `target` as a table leaf at absolute `path`.
    pub fn mount_table(&self, cx: &mut Cx, path: TablePath, target: Value) -> Result<()> {
        cx.require(&table_mount_capability())?;
        require_table_value(&target, "table/mount: table mount target must be a Table")?;
        self.insert_mount(cx, path, MountKind::Table, target)
    }

    /// Remove the exact mount at absolute `path`.
    pub fn unmount(&self, cx: &mut Cx, path: &TablePath) -> Result<Option<Value>> {
        cx.require(&table_mount_capability())?;
        let path = owned_path(path);
        if path.is_empty() {
            return Err(Error::Eval("table/mount: cannot unmount root".to_owned()));
        }
        Ok(self.lock_mounts()?.remove(&path).map(|target| target.value))
    }

    /// Return the current mount table as absolute paths and target kinds.
    pub fn inspect(&self) -> Result<Vec<MountInspection>> {
        Ok(self
            .lock_mounts()?
            .iter()
            .map(|(path, target)| MountInspection {
                path: format_path(path),
                kind: target.kind,
            })
            .collect())
    }

    fn at_path(&self, path: MountPath) -> Self {
        Self {
            root: self.root.clone(),
            mounts: self.mounts.clone(),
            path,
        }
    }

    fn insert_mount(
        &self,
        cx: &mut Cx,
        path: TablePath,
        kind: MountKind,
        target: Value,
    ) -> Result<()> {
        let path = owned_path(&path);
        if path.is_empty() {
            return Err(Error::Eval("table/mount: cannot mount root".to_owned()));
        }
        if self.root_exact_kind(cx, &path)?.is_some() {
            return Err(Error::Eval(format!(
                "table/mount: mount point {} shadows an existing root node",
                format_path(&path)
            )));
        }

        let mut mounts = self.lock_mounts()?;
        if mounts.contains_key(&path) {
            return Err(Error::Eval(format!(
                "table/mount: duplicate mount point {}",
                format_path(&path)
            )));
        }
        for (existing_path, existing) in mounts.iter() {
            if is_prefix(existing_path, &path)
                && existing.kind == MountKind::Table
                && existing_path.len() < path.len()
            {
                return Err(Error::Eval(format!(
                    "table/mount: cannot mount below table leaf {}",
                    format_path(existing_path)
                )));
            }
            if is_prefix(&path, existing_path) && kind == MountKind::Table {
                return Err(Error::Eval(format!(
                    "table/mount: table mount {} would parent existing mount {}",
                    format_path(&path),
                    format_path(existing_path)
                )));
            }
        }
        mounts.insert(
            path,
            MountTarget {
                kind,
                value: target,
            },
        );
        Ok(())
    }

    fn lock_mounts(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<MountPath, MountTarget>>> {
        self.mounts
            .lock()
            .map_err(|_| Error::Eval("table/mount: mount registry lock poisoned".to_owned()))
    }

    fn mount_snapshot(&self) -> Result<BTreeMap<MountPath, MountTarget>> {
        Ok(self.lock_mounts()?.clone())
    }

    fn resolve_current(&self, cx: &mut Cx) -> Result<ResolvedNode> {
        let mounts = self.mount_snapshot()?;
        if let Some((prefix_len, target)) = longest_mount(&mounts, &self.path) {
            return resolve_mounted_target(cx, target, &self.path[prefix_len..]);
        }
        match traverse_dir_path(cx, self.root.clone(), &self.path)? {
            Some(value) => Ok(ResolvedNode::Backed(value)),
            None if has_mount_descendant(&mounts, &self.path) => Ok(ResolvedNode::Virtual),
            None => Err(Error::Eval(format!(
                "table/mount: {} has no backing directory",
                format_path(&self.path)
            ))),
        }
    }

    fn root_exact_kind(&self, cx: &mut Cx, path: &[String]) -> Result<Option<MountKind>> {
        if path.is_empty() {
            return Ok(Some(MountKind::Dir));
        }
        let parent_path = &path[..path.len() - 1];
        let Some(parent) = traverse_dir_path(cx, self.root.clone(), parent_path)? else {
            return Ok(None);
        };
        let dir = require_dir_value(&parent, "table/mount: root path parent is not a Dir")?;
        let table = require_table_value(&parent, "table/mount: root path parent is not a Table")?;
        let name = Symbol::new(path.last().expect("non-empty path").as_str());
        if dir.is_dir(cx, name.clone())? {
            return Ok(Some(MountKind::Dir));
        }
        if table.has(cx, name)? {
            return Ok(Some(MountKind::Table));
        }
        Ok(None)
    }

    fn direct_mount_children(&self) -> Result<BTreeMap<Symbol, MountKind>> {
        let mut children = BTreeMap::new();
        for (path, target) in self.lock_mounts()?.iter() {
            if path.len() <= self.path.len() || !is_prefix(&self.path, path) {
                continue;
            }
            let name = Symbol::new(path[self.path.len()].as_str());
            let kind = if path.len() == self.path.len() + 1 {
                target.kind
            } else {
                MountKind::Dir
            };
            children.entry(name).or_insert(kind);
        }
        Ok(children)
    }

    fn child_has_mount(&self, name: &Symbol) -> Result<bool> {
        let mut child = self.path.clone();
        child.push(name.name.to_string());
        Ok(self
            .lock_mounts()?
            .keys()
            .any(|path| is_prefix(&child, path)))
    }

    fn direct_mount_value(&self, name: &Symbol) -> Result<Option<MountTarget>> {
        let mut child = self.path.clone();
        child.push(name.name.to_string());
        Ok(self.lock_mounts()?.get(&child).cloned())
    }

    fn check_no_mount_child(&self, name: &Symbol, action: &str) -> Result<()> {
        if self.child_has_mount(name)? {
            return Err(Error::Eval(format!(
                "table/mount: cannot {action} {} while a mount exists there",
                join_child(&self.path, name)
            )));
        }
        Ok(())
    }

    fn backed_table(node: ResolvedNode) -> Result<Option<Value>> {
        match node {
            ResolvedNode::Backed(value) => {
                require_table_value(&value, "table/mount: backing node is not a Table")?;
                Ok(Some(value))
            }
            ResolvedNode::Virtual => Ok(None),
        }
    }
}

impl Object for MountedDir {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!(
            "table/mount[{}; mounts={}]",
            format_path(&self.path),
            self.lock_mounts()?.len()
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for MountedDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
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
}

impl Table for MountedDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "mount")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        if let Some(target) = self.direct_mount_value(&key)? {
            return match target.kind {
                MountKind::Table => Ok(target.value),
                MountKind::Dir => cx.factory().nil(),
            };
        }
        let node = self.resolve_current(cx)?;
        let Some(value) = Self::backed_table(node)? else {
            return cx.factory().nil();
        };
        require_table_value(&value, "table/mount: backing node is not a Table")?.get(cx, key)
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        self.check_no_mount_child(&key, "set")?;
        let node = self.resolve_current(cx)?;
        let Some(backing) = Self::backed_table(node)? else {
            return Err(Error::Eval(format!(
                "table/mount: {} has no writable backing table",
                format_path(&self.path)
            )));
        };
        require_table_value(&backing, "table/mount: backing node is not a Table")?
            .set(cx, key, value)
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        if self.child_has_mount(&key)? {
            return Ok(true);
        }
        let node = self.resolve_current(cx)?;
        let Some(backing) = Self::backed_table(node)? else {
            return Ok(false);
        };
        require_table_value(&backing, "table/mount: backing node is not a Table")?.has(cx, key)
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        self.check_no_mount_child(&key, "delete")?;
        let node = self.resolve_current(cx)?;
        let Some(backing) = Self::backed_table(node)? else {
            return cx.factory().nil();
        };
        require_table_value(&backing, "table/mount: backing node is not a Table")?.del(cx, key)
    }

    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        let mut keys = BTreeSet::new();
        let node = self.resolve_current(cx)?;
        if let Some(backing) = Self::backed_table(node)? {
            keys.extend(
                require_table_value(&backing, "table/mount: backing node is not a Table")?
                    .keys(cx)?,
            );
        }
        keys.extend(self.direct_mount_children()?.into_keys());
        Ok(keys.into_iter().collect())
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let mount_children = self.direct_mount_children()?;
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        let node = self.resolve_current(cx)?;
        if let Some(backing) = Self::backed_table(node)? {
            for (key, value) in
                require_table_value(&backing, "table/mount: backing node is not a Table")?
                    .entries(cx)?
            {
                if mount_children.contains_key(&key) {
                    return Err(Error::Eval(format!(
                        "table/mount: backing entry {key} conflicts with a mount"
                    )));
                }
                seen.insert(key.clone());
                out.push((key, value));
            }
        }
        for (key, kind) in mount_children {
            if kind == MountKind::Table && seen.insert(key.clone()) {
                let target = self
                    .direct_mount_value(&key)?
                    .expect("mount child collected from registry");
                out.push((key, target.value));
            }
        }
        Ok(out)
    }

    fn len(&self, cx: &mut Cx) -> Result<usize> {
        Ok(self.entries(cx)?.len())
    }

    fn clear(&self, cx: &mut Cx) -> Result<()> {
        if let Some(name) = self.direct_mount_children()?.keys().next().cloned() {
            return Err(Error::Eval(format!(
                "table/mount: cannot clear {} while mount child {name} exists",
                format_path(&self.path)
            )));
        }
        let node = self.resolve_current(cx)?;
        let Some(backing) = Self::backed_table(node)? else {
            return Ok(());
        };
        require_table_value(&backing, "table/mount: backing node is not a Table")?.clear(cx)
    }
}

impl Dir for MountedDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        check_segment(&name)?;
        self.check_no_mount_child(&name, "mkdir")?;
        let ResolvedNode::Backed(value) = self.resolve_current(cx)? else {
            return Err(Error::Eval(format!(
                "table/mount: {} has no backing directory",
                format_path(&self.path)
            )));
        };
        require_dir_value(&value, "table/mount: backing node is not a Dir")?.mkdir(cx, name)
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        check_segment(&name)?;
        let mut child_path = self.path.clone();
        child_path.push(name.name.to_string());
        if let Some(target) = self.direct_mount_value(&name)? {
            return match target.kind {
                MountKind::Dir => cx
                    .factory()
                    .opaque(Arc::new(self.at_path(child_path)))
                    .map(Some),
                MountKind::Table => Err(Error::Eval(format!(
                    "table/mount: {} is a table mount, not a directory",
                    join_child(&self.path, &name)
                ))),
            };
        }
        if self.child_has_mount(&name)? {
            return cx
                .factory()
                .opaque(Arc::new(self.at_path(child_path)))
                .map(Some);
        }
        let ResolvedNode::Backed(value) = self.resolve_current(cx)? else {
            return Ok(None);
        };
        let dir = require_dir_value(&value, "table/mount: backing node is not a Dir")?;
        if dir.opendir(cx, name)?.is_some() {
            return cx
                .factory()
                .opaque(Arc::new(self.at_path(child_path)))
                .map(Some);
        }
        Ok(None)
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        check_segment(&name)?;
        self.check_no_mount_child(&name, "remove directory")?;
        let ResolvedNode::Backed(value) = self.resolve_current(cx)? else {
            return cx.factory().nil();
        };
        require_dir_value(&value, "table/mount: backing node is not a Dir")?.rmdir(cx, name)
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        check_segment(&name)?;
        if let Some(target) = self.direct_mount_value(&name)? {
            return Ok(target.kind == MountKind::Dir);
        }
        if self.child_has_mount(&name)? {
            return Ok(true);
        }
        let ResolvedNode::Backed(value) = self.resolve_current(cx)? else {
            return Ok(false);
        };
        require_dir_value(&value, "table/mount: backing node is not a Dir")?.is_dir(cx, name)
    }
}
