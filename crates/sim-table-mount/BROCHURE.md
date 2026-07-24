# Mounted Table/Dir Namespace

In one line: It turns several Table and Dir backends into one predictable namespace without weakening each backend's authority checks.

## What it gives you

Applications often need one namespace assembled from several storage places: an
in-memory control tree, a filesystem project folder, a remote table, and a small
local table of overrides. This crate provides that composition point. A caller
sees one directory, while each mounted backend still owns its path rules,
capabilities, mutation policy, and error behavior.

`MountedDir` keeps the normal Table and Dir contract. Longest-prefix routing
makes nested mounts predictable, visible mount leaves make inspection clear, and
conflicts fail closed instead of silently shadowing data.

## Why you will be glad

- Mount a project tree, a generated cache, and a remote table behind one Dir.
- Keep read, write, mkdir, delete, and list authority at the mounted backend.
- Reject overlapping mounts and protected roots before they can hide data.

## Where it fits

This is the storage composition layer for SIM's Table/Dir stack. Low-level table
crates provide concrete backends, and runtime organs use this crate when a
single feature needs a coherent namespace assembled from several places.
