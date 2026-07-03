# sim-table-hash

In one line: A quick name-to-value lookup store that finds any entry by its key almost instantly.

## What it gives you

This is the plain, fast lookup table for SIM. It keeps entries in memory, each under a symbol key, and finds, adds, or removes one by its name without scanning the rest. Because it uses a hash map underneath, the time to reach an entry stays about the same whether the table holds a handful of names or many thousands. It is the sensible default when a program just needs to associate names with values and get them back quickly. It loads into the runtime as an optional part, present whenever straightforward keyed storage is called for.

## Why you will be glad

- Any entry is found by name without searching the whole table.
- Lookup stays quick as the table grows.
- It is a clear default when you simply need names paired with values.

## Where it fits

The kernel says what a table must do but builds none itself. This crate is the everyday, in-memory answer to that contract, the one reached for first when a program needs keyed storage and nothing fancier. It stands alongside the lazy, overlay, and tree-shaped backends, each suited to a different need, and all of them present the same table behavior to the code above. When speed and simplicity matter more than structure, this is the plain choice.
