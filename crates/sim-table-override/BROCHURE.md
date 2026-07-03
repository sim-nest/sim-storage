# sim-table-override

In one line: A stack of lookup tables where the top layers can cover entries in the ones beneath.

## What it gives you

This backend lets you lay one or more tables over a base table and treat the pile as a single table. A lookup checks the layers from top to bottom and returns the first match, so an upper layer shadows whatever a lower one holds under the same name. That makes it easy to keep a shared base untouched while a thin layer on top adds or replaces a few entries just for one setting or one run. Removing the top layer brings the original values back. It loads into SIM as an optional part, present when a program wants layered, overridable storage.

## Why you will be glad

- A small top layer changes a few entries without altering the shared base.
- Original values return the moment an overriding layer is removed.
- Several sets of settings can share one base while each keeps its own tweaks.

## Where it fits

The kernel states what a table must do and lets loadable parts supply the behavior. This crate is the layering answer: not a store of its own so much as a way to stack existing tables and read them as one. It fits wherever defaults and overrides meet, letting a base of common values stay fixed while local changes sit on top. Beside the plain, deferred, and tree backends, it is the overlay member, and it presents the same table behavior every backend shares.
