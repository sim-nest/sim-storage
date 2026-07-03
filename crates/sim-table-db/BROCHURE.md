# sim-table-db

In one line: A file-cabinet style store that keeps named values in a tree of folders you reach by path.

## What it gives you

This backend arranges named values as a directory tree, the way folders hold files on a computer. Each value has a name, and folders can hold more folders, so you reach a value by walking a path from the top. Reading and writing pass through permission checks, so a program only touches the parts it has been allowed to touch. It behaves both as a table of names and as a browsable tree, which suits settings, records, and anything that reads better when grouped and nested. It joins SIM as a loadable part, added when a program needs path-addressed storage under access control.

## Why you will be glad

- Values are grouped in folders and reached by a clear path.
- Permission checks guard every read and write.
- A nested layout keeps large sets of names tidy and easy to browse.

## Where it fits

SIM keeps the kernel small: it states what a table and a directory must do and leaves the building to parts like this. This crate is the tree-shaped, path-addressed answer, chosen when values want structure and grouping rather than one flat pile. It sits among the other table backends, each a different trade, all speaking the shared table contract. Because access runs under capability control, it fits places where who-can-see-what matters as much as the values themselves.
