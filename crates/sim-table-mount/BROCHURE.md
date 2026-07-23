# Mounted Table/Dir Namespace

Applications often need one namespace assembled from several storage places:
an in-memory control tree, a filesystem project folder, a remote table, and a
small local table of overrides. This crate provides that composition point.

`MountedDir` keeps the normal Table and Dir contract. A caller sees one
directory, while each mounted backend keeps its own authority checks and
mutation rules. Longest-prefix routing makes nested mounts predictable, and
conflicts fail closed instead of silently shadowing data.
