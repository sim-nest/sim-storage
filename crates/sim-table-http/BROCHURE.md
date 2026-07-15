# sim-table-http

In one line: It lets a trusted host treat direct HTTP resources as table entries under explicit network permission.

## What it gives you

This backend points a table at a base HTTP address. A key becomes one resource under that address: reading performs a bounded request, writing sends a bounded update, and the response body passes through the selected SIM codec. The caller must hold network permission before any socket opens, so local programs can name remote resources without receiving network authority by accident.

## Why you will be glad

- Direct HTTP resources fit the same table shape as local stores.
- Network access is checked at the operation boundary.
- Time and body limits keep remote calls from growing without bounds.

## Where it fits

SIM keeps storage behavior in loadable backends. This crate is the direct HTTP backend: useful when a program needs a remote document, endpoint, or fixture while still speaking the table contract. It sits beside the filesystem and in-memory stores, and it is distinct from the EvalFabric remote-table backend.
