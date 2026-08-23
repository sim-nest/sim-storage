# sim-relation-site

`sim-relation-site` is the bounded, provider-neutral host effect seam for
checked relational plans and migration programs. It has no SQL parser or text
execution API. Callers choose a loaded site symbol plus an opaque `Datum`
locator; only the selected provider interprets that locator.

Every operation requires an operation-specific capability, records exactly one
kernel effect, and applies mandatory row, cell, byte, and work limits. Query
results are pushed into a caller-owned sink. Transactions and savepoints are
closure-managed so commit and rollback state cannot be manipulated directly.
