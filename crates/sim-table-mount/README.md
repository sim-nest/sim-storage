# sim-table-mount

`sim-table-mount` provides `MountedDir`, a reusable Table/Dir namespace that
routes one directory tree across explicit mounted Table and Dir backends.

The root is an ordinary `Dir`. Each mount point is an absolute
`sim-table-core` path. Directory mounts route reads and mutations by longest
valid prefix; table mounts are leaves. The mounted namespace delegates to the
owning backend, so capability checks, read-only behavior, backend errors, and
live state changes remain visible instead of being copied into a flattened
store.

```rust
use std::sync::Arc;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, Symbol, Table};
use sim_table_core::TablePath;
use sim_table_mount::{MountedDir, table_mount_capability};

let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
cx.grant(table_mount_capability());

let root = cx.factory().table(Vec::new()).unwrap();
let table = cx.factory().table(vec![
    (Symbol::new("answer"), cx.factory().bool(true).unwrap()),
]).unwrap();
let namespace = MountedDir::new(root).unwrap();
namespace
    .mount_table(&mut cx, TablePath::parse_absolute("/lookup").unwrap(), table)
    .unwrap();

let mounted = namespace.get(&mut cx, Symbol::new("lookup")).unwrap();
assert!(mounted.object().as_table_impl().is_some());
```
