# sim-lib-journal

`sim-lib-journal` is SIM's domain-free canonical journal. A logical entry is a
`journal/entry-v2` `Datum`; its `core/sha256-datum-v1` identity links the next
entry and determines the public head. Payload ids likewise name semantic
values. Exact-byte storage ids, namespaces, and locators stay inside the
backend.

The host backend reads both native generations. It verifies retained
`SIMJSTATE1` entries and byte-addressed payloads under their original laws,
then derives the canonical chain without rewriting the old bytes. The first
writer reserves a fresh namespace and uses one state CAS to select the verified
prefix, format, fence, canonical head, and locator. V2 entry leaves include the
sequence, full canonical entry id, and storage id, so a losing writer cannot
reserve a winner's sequence path. Replay follows the committed locator closure;
directory order and orphan leaves carry no authority.

`PersistentObjectStore` provides owned-return `Datum` persistence over that
same object backend. `StoredDatumRef` keeps the semantic and storage identities
separate, and its derived correspondence index can be rebuilt and verified.
`Journal::verified_snapshot` returns entries and their canonical payload Datums
from one verified backend read, without exposing physical storage state.

```rust
use std::sync::Arc;
use sim_kernel::Datum;
use sim_lib_journal::{
    MemoryBackend, PersistentObjectStore, PersistentSemanticObjects,
};

let backend = Arc::new(MemoryBackend::new());
let mut objects = PersistentObjectStore::open(backend).unwrap();
let stored = objects.put(Datum::String("evidence".into())).unwrap();
assert_eq!(objects.get(&stored.meaning).unwrap(), Datum::String("evidence".into()));
assert_eq!(objects.rebuild_index().unwrap(), vec![stored]);
```
