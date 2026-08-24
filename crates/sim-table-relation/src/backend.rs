//! Relation-backed implementation details.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

use sim_kernel::{
    CapabilityName, Cx, Datum, Error, Expr, Object, Result, Symbol, TableCompareExchange,
    TableExpected, TableReplacement, Value,
    id::CORE_TABLE_CLASS_ID,
    object::ClassRef,
    table::{Dir, Table},
};
use sim_relation_core::Row;
use sim_relation_plan::CheckedQuery;
use sim_table_core::TablePath;

/// Authority to use the relational backend itself.
pub fn relation_namespace_capability() -> CapabilityName {
    CapabilityName::new("relation.namespace")
}
/// Authority to read the projected Table/Dir surface.
pub fn relation_table_read_capability() -> CapabilityName {
    CapabilityName::new("table.relation.read")
}
/// Authority to mutate the projected Table/Dir surface.
pub fn relation_table_write_capability() -> CapabilityName {
    CapabilityName::new("table.relation.write")
}

/// Persistent node kind in the D9 relation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A directory node.
    Dir,
    /// A value node.
    Value,
}

/// One D9 node row. Root is always id zero and parent zero.
#[derive(Clone)]
pub struct Node {
    /// Positive generated id, except root zero.
    pub id: u64,
    /// Parent node id.
    pub parent: u64,
    /// Unique name within the parent.
    pub name: Symbol,
    /// Typed node kind.
    pub kind: NodeKind,
    /// Encoded value bytes; empty for directories.
    pub bytes: Vec<u8>,
    /// Exact codec identity used for bytes.
    pub codec: Symbol,
    value: Option<Value>,
}

#[derive(Clone)]
struct State {
    nodes: BTreeMap<u64, Node>,
    next_id: u64,
    generation: u64,
}

/// A value codec whose stable identity is persisted beside every value.
pub trait RelationValueCodec: Send + Sync {
    /// Stable codec identity.
    fn identity(&self) -> Symbol;
    /// Encode a value to durable bytes.
    fn encode(&self, cx: &mut Cx, value: &Value) -> Result<Vec<u8>>;
    /// Decode durable bytes.
    fn decode(&self, cx: &mut Cx, bytes: &[u8]) -> Result<Value>;
}

/// Checked namespace mutation executed atomically against one state snapshot.
#[derive(Clone)]
enum Plan {
    Set(Vec<String>, Symbol, Value, Vec<u8>),
    Delete(Vec<String>, Symbol),
    Mkdir(Vec<String>, Symbol),
    Rmdir(Vec<String>, Symbol),
    Clear(Vec<String>),
}

/// Relation-backed directory view.
#[derive(Clone)]
pub struct RelationDir {
    state: Arc<Mutex<State>>,
    path: Vec<String>,
    codec: Arc<dyn RelationValueCodec>,
    head: Option<(Value, Symbol)>,
}

impl RelationDir {
    /// Open a new D9 relation. The root row is `(0, 0, "", Dir)`.
    pub fn open(codec: Arc<dyn RelationValueCodec>) -> Self {
        let root = Node {
            id: 0,
            parent: 0,
            name: Symbol::new(""),
            kind: NodeKind::Dir,
            bytes: Vec::new(),
            codec: codec.identity(),
            value: None,
        };
        Self {
            state: Arc::new(Mutex::new(State {
                nodes: BTreeMap::from([(0, root)]),
                next_id: 1,
                generation: 0,
            })),
            path: Vec::new(),
            codec,
            head: None,
        }
    }
    /// Attach the canonical Table slot used to publish generation heads by CAS.
    pub fn with_head(mut self, table: Value, key: Symbol) -> Result<Self> {
        if table.object().as_table_impl().is_none() {
            return Err(Error::Eval("table/relation: head is not a Table".into()));
        }
        self.head = Some((table, key));
        Ok(self)
    }
    /// Resolve an absolute or relative Table path using `sim-table-core`.
    pub fn resolve(&self, path: &TablePath) -> Result<Self> {
        let mut out = Vec::new();
        for segment in path.segments() {
            if !sim_table_core::is_legal_table_segment(segment) {
                return Err(Error::Eval("table/relation: illegal path".into()));
            }
            out.push(segment.to_owned());
        }
        let state = self.lock()?;
        let id = self.resolve_id(&state, &out)?;
        if state.nodes[&id].kind != NodeKind::Dir {
            return Err(Error::Eval(
                "table/relation: path is not a directory".into(),
            ));
        }
        drop(state);
        Ok(Self {
            state: self.state.clone(),
            path: out,
            codec: self.codec.clone(),
            head: self.head.clone(),
        })
    }
    /// Snapshot the canonical node rows for inspection or persistence.
    pub fn nodes(&self) -> Result<Vec<Node>> {
        Ok(self.lock()?.nodes.values().cloned().collect())
    }
    #[cfg(test)]
    pub(crate) fn replace_codec_identity(&self, id: u64, codec: Symbol) {
        self.state
            .lock()
            .expect("test state")
            .nodes
            .get_mut(&id)
            .expect("test node")
            .codec = codec;
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| Error::Eval("table/relation: lock poisoned".into()))
    }
    fn resolve_id(&self, state: &State, path: &[String]) -> Result<u64> {
        let mut id = 0;
        for part in path {
            id = state
                .nodes
                .values()
                .find(|n| n.parent == id && n.name.name.as_ref() == part)
                .map(|n| n.id)
                .ok_or_else(|| {
                    Error::Eval(format!("table/relation: missing path /{}", path.join("/")))
                })?;
        }
        Ok(id)
    }
    fn child<'a>(state: &'a State, parent: u64, name: &Symbol) -> Option<&'a Node> {
        state
            .nodes
            .values()
            .find(|n| n.parent == parent && n.name == *name)
    }
    fn execute(&self, cx: &mut Cx, plan: Plan) -> Result<Option<Value>> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_write_capability())?;
        let mut original = self.lock()?;
        let mut state = original.clone();
        let parent_path = match &plan {
            Plan::Set(p, ..)
            | Plan::Delete(p, ..)
            | Plan::Mkdir(p, ..)
            | Plan::Rmdir(p, ..)
            | Plan::Clear(p) => p,
        };
        let parent = self.resolve_id(&state, parent_path)?;
        let result = match plan {
            Plan::Set(_, name, value, bytes) => {
                if !sim_table_core::is_legal_table_segment(&name.name) {
                    return Err(Error::Eval("table/relation: illegal name".into()));
                }
                if let Some(n) = Self::child(&state, parent, &name) {
                    if n.kind == NodeKind::Dir {
                        return Err(Error::Eval("table/relation: name is a directory".into()));
                    }
                    let id = n.id;
                    let n = state.nodes.get_mut(&id).expect("node");
                    n.bytes = bytes;
                    n.codec = self.codec.identity();
                    n.value = Some(value);
                } else {
                    let id = state.next_id;
                    state.next_id += 1;
                    state.nodes.insert(
                        id,
                        Node {
                            id,
                            parent,
                            name,
                            kind: NodeKind::Value,
                            bytes,
                            codec: self.codec.identity(),
                            value: Some(value),
                        },
                    );
                }
                None
            }
            Plan::Delete(_, name) => {
                let Some(n) = Self::child(&state, parent, &name) else {
                    return Ok(None);
                };
                if n.kind == NodeKind::Dir {
                    return Err(Error::Eval(
                        "table/relation: delete cannot remove directory".into(),
                    ));
                }
                let id = n.id;
                state.nodes.remove(&id).and_then(|n| n.value)
            }
            Plan::Mkdir(_, name) => {
                if !sim_table_core::is_legal_table_segment(&name.name) {
                    return Err(Error::Eval("table/relation: illegal name".into()));
                }
                if Self::child(&state, parent, &name).is_some() {
                    return Err(Error::Eval("table/relation: duplicate sibling name".into()));
                }
                let id = state.next_id;
                state.next_id += 1;
                state.nodes.insert(
                    id,
                    Node {
                        id,
                        parent,
                        name,
                        kind: NodeKind::Dir,
                        bytes: Vec::new(),
                        codec: self.codec.identity(),
                        value: None,
                    },
                );
                None
            }
            Plan::Rmdir(_, name) => {
                let n = Self::child(&state, parent, &name)
                    .ok_or_else(|| Error::Eval("table/relation: directory absent".into()))?;
                if n.id == 0 {
                    return Err(Error::Eval("table/relation: root cannot be removed".into()));
                }
                if n.kind != NodeKind::Dir {
                    return Err(Error::Eval("table/relation: not a directory".into()));
                }
                let id = n.id;
                if state.nodes.values().any(|c| c.parent == id && c.id != id) {
                    return Err(Error::Eval("table/relation: directory not empty".into()));
                }
                state.nodes.remove(&id);
                None
            }
            Plan::Clear(_) => {
                let ids = state
                    .nodes
                    .values()
                    .filter(|n| n.parent == parent && n.kind == NodeKind::Value)
                    .map(|n| n.id)
                    .collect::<Vec<_>>();
                for id in ids {
                    state.nodes.remove(&id);
                }
                None
            }
        };
        state.generation += 1;
        if let Some((head, key)) = &self.head {
            let table = head.object().as_table_impl().expect("checked");
            let generation = state.generation.to_string();
            let replacement = cx.factory().string(generation)?;
            let expected = if original.generation == 0 {
                TableExpected::Absent
            } else {
                TableExpected::Value(Expr::String(original.generation.to_string()))
            };
            let exchanged = table.compare_exchange(
                cx,
                key.clone(),
                expected,
                TableReplacement::Value(replacement),
            )?;
            if !exchanged.exchanged {
                return Err(Error::Eval("table/relation: stale namespace head".into()));
            }
        }
        *original = state;
        Ok(result)
    }
}

impl Object for RelationDir {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok(format!("table/relation[/{}]", self.path.join("/")))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for RelationDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(CORE_TABLE_CLASS_ID, Symbol::qualified("core", "Table"))
    }
    fn as_expr(&self, _: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(Symbol::qualified("table/relation", "dir")))
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_dir(&self) -> Option<&dyn Dir> {
        Some(self)
    }
}
impl Table for RelationDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "relation")
    }
    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_read_capability())?;
        let s = self.lock()?;
        let p = self.resolve_id(&s, &self.path)?;
        let Some(n) = Self::child(&s, p, &key) else {
            return cx.factory().nil();
        };
        if n.kind == NodeKind::Dir {
            return Err(Error::Eval("table/relation: key is directory".into()));
        }
        if n.codec != self.codec.identity() {
            return Err(Error::Eval(
                "table/relation: codec identity mismatch".into(),
            ));
        }
        self.codec.decode(cx, &n.bytes)
    }
    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        let bytes = self.codec.encode(cx, &value)?;
        self.execute(cx, Plan::Set(self.path.clone(), key, value, bytes))
            .map(drop)
    }
    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_read_capability())?;
        let s = self.lock()?;
        Ok(Self::child(&s, self.resolve_id(&s, &self.path)?, &key).is_some())
    }
    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        self.execute(cx, Plan::Delete(self.path.clone(), key))?
            .map_or_else(|| cx.factory().nil(), Ok)
    }
    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_read_capability())?;
        let s = self.lock()?;
        let p = self.resolve_id(&s, &self.path)?;
        Ok(s.nodes
            .values()
            .filter(|n| n.parent == p && n.id != p)
            .map(|n| n.name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        let keys = self.keys(cx)?;
        keys.into_iter()
            .filter_map(|k| self.get(cx, k.clone()).ok().map(|v| (k, v)))
            .collect::<Vec<_>>()
            .pipe(Ok)
    }
    fn len(&self, cx: &mut Cx) -> Result<usize> {
        Ok(self.entries(cx)?.len())
    }
    fn clear(&self, cx: &mut Cx) -> Result<()> {
        self.execute(cx, Plan::Clear(self.path.clone())).map(drop)
    }
    fn compare_exchange(
        &self,
        _: &mut Cx,
        _: Symbol,
        _: TableExpected,
        _: TableReplacement,
    ) -> Result<TableCompareExchange> {
        Err(Error::Eval(
            "table/relation: use structured transaction".into(),
        ))
    }
}
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
impl Dir for RelationDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        self.execute(cx, Plan::Mkdir(self.path.clone(), name.clone()))?;
        cx.factory().opaque(Arc::new(Self {
            state: self.state.clone(),
            path: self
                .path
                .iter()
                .cloned()
                .chain([name.name.to_string()])
                .collect(),
            codec: self.codec.clone(),
            head: self.head.clone(),
        }))
    }
    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_read_capability())?;
        let s = self.lock()?;
        let p = self.resolve_id(&s, &self.path)?;
        let Some(n) = Self::child(&s, p, &name) else {
            return Ok(None);
        };
        if n.kind != NodeKind::Dir {
            return Err(Error::Eval("table/relation: not a directory".into()));
        }
        drop(s);
        Ok(Some(
            cx.factory().opaque(Arc::new(Self {
                state: self.state.clone(),
                path: self
                    .path
                    .iter()
                    .cloned()
                    .chain([name.name.to_string()])
                    .collect(),
                codec: self.codec.clone(),
                head: self.head.clone(),
            }))?,
        ))
    }
    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        self.execute(cx, Plan::Rmdir(self.path.clone(), name))?;
        cx.factory().nil()
    }
    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        cx.require(&relation_namespace_capability())?;
        cx.require(&relation_table_read_capability())?;
        let s = self.lock()?;
        let p = self.resolve_id(&s, &self.path)?;
        Ok(Self::child(&s, p, &name).is_some_and(|n| n.kind == NodeKind::Dir))
    }
}

/// Admitted unique key projection for a query view.
pub struct UniqueKeyProjection {
    index: usize,
}
impl UniqueKeyProjection {
    /// Admit a non-null field as the view key.
    pub fn admit(query: &CheckedQuery, field: &Symbol) -> Result<Self> {
        let index = query
            .output()
            .fields()
            .iter()
            .position(|f| f.name.symbol() == field)
            .ok_or_else(|| Error::Eval("table/relation-view: keyless plan".into()))?;
        if query.output().fields()[index].nullable {
            return Err(Error::Eval("table/relation-view: nullable key".into()));
        }
        Ok(Self { index })
    }
}

/// Deterministic, read-only Table projection of already executed checked rows.
pub struct RelationView {
    query: CheckedQuery,
    rows: BTreeMap<Symbol, Value>,
}
impl RelationView {
    /// Build a view, rejecting duplicate keys and rows not typed by the checked plan.
    pub fn new(
        query: CheckedQuery,
        key: UniqueKeyProjection,
        rows: impl IntoIterator<Item = (Row, Value)>,
    ) -> Result<Self> {
        let mut out = BTreeMap::new();
        for (row, value) in rows {
            if row.row_type() != query.output() {
                return Err(Error::Eval("table/relation-view: row type mismatch".into()));
            }
            let symbol = match row.cells()[key.index].value() {
                Some(Datum::String(v)) => Symbol::new(v.as_str()),
                Some(Datum::Symbol(v)) => v.clone(),
                _ => {
                    return Err(Error::Eval(
                        "table/relation-view: key is not symbolic".into(),
                    ));
                }
            };
            if out.insert(symbol, value).is_some() {
                return Err(Error::Eval("table/relation-view: duplicate key".into()));
            }
        }
        Ok(Self { query, rows: out })
    }
    /// Sealed plan backing this view.
    pub fn query(&self) -> &CheckedQuery {
        &self.query
    }
}
impl Object for RelationView {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("table/relation-view".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl sim_kernel::ObjectCompat for RelationView {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory()
            .class_stub(CORE_TABLE_CLASS_ID, Symbol::qualified("core", "Table"))
    }
    fn as_expr(&self, _: &mut Cx) -> Result<Expr> {
        Ok(Expr::Symbol(Symbol::qualified("table/relation", "view")))
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
}
impl Table for RelationView {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table/relation", "view")
    }
    fn get(&self, _: &mut Cx, key: Symbol) -> Result<Value> {
        self.rows
            .get(&key)
            .cloned()
            .ok_or_else(|| Error::Eval("table/relation-view: missing key".into()))
    }
    fn set(&self, _: &mut Cx, _: Symbol, _: Value) -> Result<()> {
        Err(Error::Eval("table/relation-view is read-only".into()))
    }
    fn has(&self, _: &mut Cx, key: Symbol) -> Result<bool> {
        Ok(self.rows.contains_key(&key))
    }
    fn del(&self, _: &mut Cx, _: Symbol) -> Result<Value> {
        Err(Error::Eval("table/relation-view is read-only".into()))
    }
    fn keys(&self, _: &mut Cx) -> Result<Vec<Symbol>> {
        Ok(self.rows.keys().cloned().collect())
    }
    fn entries(&self, _: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        Ok(self
            .rows
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
    fn len(&self, _: &mut Cx) -> Result<usize> {
        Ok(self.rows.len())
    }
    fn clear(&self, _: &mut Cx) -> Result<()> {
        Err(Error::Eval("table/relation-view is read-only".into()))
    }
}
