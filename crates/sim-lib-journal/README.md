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

## Admission-scoped conformance cuts

`with_failpoint_hook` and `with_admission_failpoint_hook` are `#[cfg(test)]`
only: they exist for this crate's own conformance test suite and do not
compile into a normal build, so no consumer of this crate can reach them.
This section documents that test-internal contract, not a supported public
API.

`HostDirJournalBackend::with_admission_failpoint_hook` borrows the actual
`Admission` alongside each existing publication `Failpoint`. A trusted
conformance owner can select the exact entry kind, object, expected head and
writer fence without relying on unrelated callback counts. The original
`with_failpoint_hook` remains supported and runs first; a refusal there prevents
the admission callback at that boundary. Returning `true` injects the existing
boundary error, while `false` leaves the original publication/CAS path intact.
The callback explicitly owns any one-shot arming and must retain its fired
admission identity: successful historical redelivery does not mean a cut fired.

Admission callbacks are not invoked by lease acquisition, format migration,
standalone object writes or already-committed confirmation. They must be
bounded, nonblocking and must not reenter journal or port operations. Guarded
`AfterCas` runs under the original exclusion before the action: refusal leaves
committed data but suppresses that action. `BeforeAcknowledgement` can fail
after the action ran. Neither error permits replaying an uncertain action.

The `admission_context` conformance tests exercise the real host backend,
physical codec, head CAS and guarded action path over its existing modeled
`TestPort`. They establish component ordering and binding, not native filesystem
durability, administrative process cleanup or native M5 qualification.

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
