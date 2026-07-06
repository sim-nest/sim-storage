use std::sync::Arc;

use sim_kernel::{
    Cx, DefaultFactory, NoopEvalPolicy, ObjectEncoding, Symbol, Table, read_construct_capability,
};

use crate::{
    HashBackend, HashTable, HashTableDescriptor, HashTableLib, hash_table_class_symbol,
    install_hash_table_lib,
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
fn install_is_idempotent_and_manifest_is_stable() {
    use sim_kernel::Lib;

    let mut cx = test_cx();
    let lib_id = Symbol::qualified("table", "hash");

    // First install registers the backend and the loadable lib.
    assert!(cx.registry().lib(&lib_id).is_none());
    install_hash_table_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&lib_id).is_some());

    // Second install is a no-op: it returns early because the lib is present.
    install_hash_table_lib(&mut cx).unwrap();
    assert!(cx.registry().lib(&lib_id).is_some());

    // The manifest identity/version comes from the shared constructor and is
    // stable across calls.
    let manifest = HashTableLib.manifest();
    assert_eq!(manifest.id, lib_id);
    assert_eq!(manifest.version.0, env!("CARGO_PKG_VERSION"));
    assert_eq!(HashTableLib.manifest().version.0, manifest.version.0);
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

#[test]
fn keys_and_entries_are_deterministically_ordered() {
    // Regression guard for F39: `keys`/`entries` must not leak the
    // nondeterministic HashMap order. Insert out of order, expect sorted.
    let mut cx = test_cx();
    let value = cx.factory().bool(true).unwrap();
    let table = HashTable::with_entries(vec![
        (Symbol::new("delta"), value.clone()),
        (Symbol::new("alpha"), value.clone()),
        (Symbol::new("charlie"), value.clone()),
        (Symbol::new("bravo"), value),
    ]);

    let expected = vec![
        Symbol::new("alpha"),
        Symbol::new("bravo"),
        Symbol::new("charlie"),
        Symbol::new("delta"),
    ];

    assert_eq!(table.keys(&mut cx).unwrap(), expected);
    // Stable across repeated calls.
    assert_eq!(table.keys(&mut cx).unwrap(), expected);

    let entry_keys: Vec<Symbol> = table
        .entries(&mut cx)
        .unwrap()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert_eq!(entry_keys, expected);
}
