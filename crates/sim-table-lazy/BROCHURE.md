# sim-table-lazy

In one line: A lookup table whose values are worked out the first time you ask and then remembered.

## What it gives you

This table holds names now but puts off producing their values until someone reads them. Each entry carries a small piece of work that runs at most once; the first read computes the value, and every read after that returns the saved result. That means a program can list many entries cheaply and pay only for the ones it actually opens, and never pay twice for the same one. It suits values that are expensive to build or drawn from somewhere slow. It loads into SIM as an optional part, present when a program wants keyed storage with deferred, remembered values.

## Why you will be glad

- Costly values are built only when first read.
- Each value is computed once and then reused.
- A table can name many entries while paying for only the opened ones.

## Where it fits

The kernel defines the table contract and leaves each concrete store to a loadable part. Where the hash backend holds values already made, this one holds the recipe and runs it on demand, the choice when preparing every value up front would be wasteful. It reads through the same shared table behavior as the others, so a program treats a deferred value exactly like a ready one. It fits alongside the plain, overlay, and tree backends as the wait-and-remember member of the set.
