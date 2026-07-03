use std::sync::Arc;

use sim_kernel::{
    Cx, DefaultFactory, NoopEvalPolicy, ObjectEncoding, Symbol, Table, read_construct_capability,
};

use crate::{
    HashBackend, HashTable, HashTableDescriptor, hash_table_class_symbol, install_hash_table_lib,
};

fn test_cx() -> Cx {
    Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory))
}

#[test]
fn hash_table_lookup() {
    let mut cx = test_cx();
    let one = cx.factory().bool(true).unwrap();
    let two = cx.factory().nil().unwrap();
    let table = HashTable::with_entries(vec![
        (Symbol::new("a"), one.clone()),
        (Symbol::new("b"), two),
    ]);

    assert_eq!(table.len(&mut cx).unwrap(), 2);
    assert!(table.has(&mut cx, Symbol::new("a")).unwrap());
    assert_eq!(table.get(&mut cx, Symbol::new("a")).unwrap(), one);
}

#[test]
fn install_registers_hash_backend() {
    let mut cx = test_cx();
    install_hash_table_lib(&mut cx).unwrap();
    cx.table_registry_mut().set_active("hash").unwrap();

    let backend = cx.table_registry().active();
    assert_eq!(backend, "hash");

    let name = <HashBackend as sim_kernel::TableBackend>::name(&HashBackend);
    assert_eq!(name, "hash");
}

#[test]
fn hash_table_citizen_round_trips_as_descriptor() {
    let mut cx = test_cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let value = cx.factory().string("value".to_owned()).unwrap();
    let table = HashTable::with_entries(vec![(Symbol::new("key"), value)]);
    let original = cx.factory().opaque(std::sync::Arc::new(table)).unwrap();

    sim_citizen::check_value_fixture(&mut cx, original.clone()).unwrap();

    let ObjectEncoding::Constructor { args, .. } = original
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .unwrap()
    else {
        panic!("expected constructor encoding");
    };
    let args = args
        .iter()
        .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();
    let decoded = cx.read_construct(&hash_table_class_symbol(), args).unwrap();

    assert!(
        decoded
            .object()
            .as_any()
            .downcast_ref::<HashTableDescriptor>()
            .is_some()
    );
    assert!(decoded.object().as_table_impl().is_none());
}
