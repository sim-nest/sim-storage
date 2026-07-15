# sim-storage

Where SIM keeps things: lists and lookup tables -- eager or lazy, layered or on
disk -- behind one uniform runtime contract, so you pick a storage strategy by
installing the matching library instead of rewriting call sites.

## Quickstart

Build a shared cons-cell list and read it back by head and tail:

```bash
cargo add sim-list-cell
```

```rust
use std::sync::Arc;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Factory, ListValue, LengthResult};
use sim_list_cell::ConsList;

let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));

let a = cx.factory().bool(true).unwrap();
let b = cx.factory().bool(false).unwrap();
let xs = ConsList::from_vec(vec![a, b]);

assert_eq!(xs.len(&mut cx).unwrap(), LengthResult::Known(2));
let tail = xs.cdr(&mut cx).unwrap().unwrap();
let tail = tail.object().as_list().unwrap();
assert_eq!(tail.len(&mut cx).unwrap(), LengthResult::Known(1));
```

(from the `ConsList` doctest, `crates/sim-list-cell/src/cons.rs:30`)

Store symbol-keyed values in an in-memory table and look one up:

```bash
cargo add sim-table-hash
```

```rust
use std::sync::Arc;
use sim_kernel::{Cx, DefaultFactory, NoopEvalPolicy, Symbol, Table};
use sim_table_hash::HashTable;

let mut cx = Cx::new(Arc::new(NoopEvalPolicy), Arc::new(DefaultFactory));
let value = cx.factory().bool(true).unwrap();
let table = HashTable::with_entries(vec![(Symbol::new("a"), value.clone())]);

assert_eq!(table.len(&mut cx).unwrap(), 1);
assert_eq!(table.get(&mut cx, Symbol::new("a")).unwrap(), value);
```

(from the `HashTable::with_entries` doctest, `crates/sim-table-hash/src/hash.rs:67`)

## How it works

`sim-storage` is the storage-backend surface of the SIM constellation. SIM is
an expandable Rust runtime built around a small protocol kernel plus a large
set of loadable libraries: the kernel defines contracts, libraries provide
behavior. This repository provides the concrete list and table backends that
satisfy the kernel collection contracts.

Each crate registers itself as a loadable library through an `install_*` entry
point and exposes one storage strategy behind a kernel contract -- the
`ListBackend` and `TableBackend` traits and the directory contract. The kernel
types these backends build on (the contract traits, `Datum`, the citizen class
machinery, and the runtime `Cx`/`Lib`) are defined in `sim-kernel`; this
repository supplies the behavior, not the protocol.

## Crates

### List backends

| Crate | Role |
| --- | --- |
| `sim-list-cell` | Cell-based list backend: a mutable cons-cell list built from shared `ConsList` cells, satisfying the kernel `ListBackend` contract. Installs via `install_cons_list_lib`. |
| `sim-list-lazy` | Lazy list backend: `LazyConsList` computes head and tail on demand and `LazyIterList` adapts an iterator into a list, with an `unfold` constructor. Installs via `install_lazy_list_lib`. |

### Table backends

| Crate | Role |
| --- | --- |
| `sim-table-hash` | Hash-map table backend: `HashTable` stores symbol-keyed entries in an in-memory hash map, satisfying the kernel `TableBackend` contract. Installs via `install_hash_table_lib`. |
| `sim-table-lazy` | Lazy table backend: `LazyTable` produces entry values through `ValueLoader` closures that run at most once and memoize their result. Installs via `install_lazy_table_lib`. |
| `sim-table-override` | Overlay table backend: `OverrideTable` layers one or more tables over a base table, resolving lookups front-to-back so upper layers shadow lower ones. Installs via `install_override_table_lib`. |
| `sim-table-db` | Db-backed table backend: `DbDir` is a path-addressed directory tree of symbol-keyed values that satisfies the kernel table and directory contracts under capability control. Installs via `install_db_dir_lib`. |
| `sim-table-fs` | Filesystem directory backend: `FsDir` exposes a host directory as a path-addressed table with codec-by-extension leaves and capability-gated mutation. Installs via `install_fs_dir_lib`. |

## Backends as loadable libraries

Every backend follows the same contract. It implements a kernel collection
trait (`ListBackend` or `TableBackend`, plus the directory contract for
`sim-table-db` and `sim-table-fs`), registers a citizen class so its values are
first-class runtime objects, and exposes a single `install_*` function that adds
the library to a runtime `Cx`. A program selects a storage strategy -- eager
cell versus lazy, hashed versus overlay versus db versus filesystem -- by
installing the matching library, while the kernel contract keeps the collection
surface uniform across backends.

## Validation

These commands run in the constellation workspace; only `sim-kernel` builds from a lone clone today (see `DEVELOPING.md` in `sim-sdk`). A single-repo build lands with the first crates.io publish.

```bash
cargo fmt --check && cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo doc --workspace --no-deps
cargo run -p xtask -- simdoc --check
```

## Documentation Lanes

`cargo run -p xtask -- simdoc` builds the public documentation lanes:

- API docs: `target/doc/`
- Agent cards: `docs/agents/cards.jsonl` and `docs/agents/card-index.json`
- Human docs: `docs/humans/`
- Diagrams: `docs/diagrams/src/` and `docs/diagrams/generated/`

The same command writes split contract files under `docs/generated/`. Everything
under `docs/` is generated; do not hand-edit it.

### Rustdoc conventions

Public API documentation in `src/` follows one house style:

- Every public item opens with a one-line summary sentence, then context.
- Each backend is framed by its storage strategy (eager cell vs lazy, hashed vs
  overlay vs db) and the kernel list/table/`Datum` contract it satisfies.
- The first-reach types carry a `# Examples` doctest that compiles and passes.
- Cross-reference with intra-doc links, and link back to this README rather than
  restating it.

The public API is documentation-gated: each crate's `lib.rs` denies
`missing_docs`, so every public item, field, and variant must be documented for
the crate to build.

### Examples and recipes

These crates ship no `recipes/` tree; their examples are their rustdoc doctests.
Recipes that exercise the storage backends end to end live in the crates that
load a runtime to drive them.
