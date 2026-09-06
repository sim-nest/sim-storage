# sim-artifact-facet

In one line: Preserve independent edits while changing only the exact artifact region owned by the caller.

## What it gives you

`sim-artifact-facet` gives change tools a shared, deterministic answer to whether a proposal is already true, unchanged, directly applicable, safely mergeable, or conflicting. It binds ownership and region before comparison, keeps base, observed, and intended content in separate roles, and preserves disjoint work under explicit limits. Generated regions remain under their generator's authority.

## Why you will be glad

- Unchanged proposals leave someone else's nearby work intact.
- Overlapping edits become visible conflicts instead of silent winners.
- Maintenance tools and durable mutation engines can share one comparison law.

## Where it fits

Within SIM, sim-artifact-facet is the pure decision layer below packet checking and repository mutation. Filesystem durability, rollback, Git transactions, disclosure policy, and outward publication remain with their dedicated owners.
