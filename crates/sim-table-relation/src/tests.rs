use super::*;
use sim_kernel::{
    Cx, DefaultFactory, Dir, EagerPolicy, Error, Expr, HandleSeed, Result, Symbol, Table, Value,
};
use std::sync::Arc;

struct TextCodec(Symbol);
impl RelationValueCodec for TextCodec {
    fn identity(&self) -> Symbol {
        self.0.clone()
    }
    fn encode(&self, cx: &mut Cx, value: &Value) -> Result<Vec<u8>> {
        match value.object().as_expr(cx)? {
            Expr::String(v) => Ok(v.into_bytes()),
            _ => Err(Error::Eval("test codec expects string".into())),
        }
    }
    fn decode(&self, cx: &mut Cx, bytes: &[u8]) -> Result<Value> {
        cx.factory()
            .string(String::from_utf8(bytes.to_vec()).map_err(|_| Error::Eval("utf8".into()))?)
    }
}
fn cx() -> Cx {
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(8),
    );
    cx.grant(relation_namespace_capability());
    cx.grant(relation_table_read_capability());
    cx.grant(relation_table_write_capability());
    cx
}
fn root() -> RelationDir {
    RelationDir::open(Arc::new(TextCodec(Symbol::qualified("codec", "test"))))
}

#[test]
fn d9_root_nested_operations_and_invariants() {
    let mut cx = cx();
    let root = root();
    assert_eq!(
        (root.nodes().unwrap()[0].id, root.nodes().unwrap()[0].parent),
        (0, 0)
    );
    let child = root.mkdir(&mut cx, Symbol::new("nested")).unwrap();
    let dir = child.object().as_dir().unwrap();
    dir.mkdir(&mut cx, Symbol::new("deep")).unwrap();
    assert!(root.rmdir(&mut cx, Symbol::new("nested")).is_err());
    assert!(root.mkdir(&mut cx, Symbol::new("nested")).is_err());
    assert!(root.mkdir(&mut cx, Symbol::new("bad/name")).is_err());
    assert_eq!(
        root.nodes()
            .unwrap()
            .iter()
            .map(|n| n.id)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[test]
fn values_round_trip_and_failed_mutation_rolls_back() {
    let mut cx = cx();
    let root = root();
    let value = cx.factory().string("hello".into()).unwrap();
    root.set(&mut cx, Symbol::new("item"), value).unwrap();
    assert_eq!(
        root.get(&mut cx, Symbol::new("item"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("hello".into())
    );
    let before = root.nodes().unwrap().len();
    assert!(root.mkdir(&mut cx, Symbol::new("item")).is_err());
    assert_eq!(root.nodes().unwrap().len(), before);
    root.del(&mut cx, Symbol::new("item")).unwrap();
    assert!(!root.has(&mut cx, Symbol::new("item")).unwrap());
}

#[test]
fn capabilities_are_independent() {
    let root = root();
    let mut relation_only = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(9),
    );
    relation_only.grant(relation_namespace_capability());
    assert!(root.keys(&mut relation_only).is_err());
    let mut table_only = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        HandleSeed::new(10),
    );
    table_only.grant(relation_table_read_capability());
    assert!(root.keys(&mut table_only).is_err());
}

#[test]
fn codec_mismatch_fails_closed() {
    let mut cx = cx();
    let root = root();
    let value = cx.factory().string("x".into()).unwrap();
    root.set(&mut cx, Symbol::new("x"), value).unwrap();
    root.replace_codec_identity(1, Symbol::qualified("codec", "other"));
    assert!(root.get(&mut cx, Symbol::new("x")).is_err());
}
