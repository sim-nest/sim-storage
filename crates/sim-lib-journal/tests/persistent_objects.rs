use std::sync::Arc;

use sim_kernel::{Datum, Symbol};
use sim_lib_journal::{MemoryBackend, PersistentObjectStore, PersistentSemanticObjects};

// conformance: persistent Datums retain separate semantic and storage identity.

#[test]
fn owned_values_keep_semantic_and_storage_identity_distinct_across_reopen() {
    let backend = Arc::new(MemoryBackend::new());
    let mut store = PersistentObjectStore::open(backend.clone()).unwrap();
    let value = Datum::Node {
        tag: Symbol::qualified("example", "evidence-set-v1"),
        fields: vec![(Symbol::new("members"), Datum::Vector(vec![]))],
    };

    let reference = store.put(value.clone()).unwrap();
    assert_ne!(reference.meaning, reference.storage);
    assert_eq!(store.get(&reference.meaning).unwrap(), value);
    assert_eq!(store.rebuild_index().unwrap(), vec![reference.clone()]);

    drop(store);
    let reopened = PersistentObjectStore::open(backend).unwrap();
    assert_eq!(reopened.get(&reference.meaning).unwrap(), value);
}
