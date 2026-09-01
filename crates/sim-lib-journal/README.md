# sim-lib-journal

`sim-lib-journal` is SIM's domain-free atomic content journal. It publishes
immutable content-addressed objects and advances one gapless, fenced head. The
backend contract has one atomic admission operation so durable implementations
can compose canonical `table/cas` without inventing another lock or store.

Entry kinds are open `Symbol` values and payloads are opaque `ContentId`s. The
crate contains no study, model, attempt, grading, or report vocabulary.
