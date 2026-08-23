# One trustworthy append law

`sim-lib-journal` gives every SIM domain the same small foundation for immutable
objects, fenced writers, atomic batches, verified replay, and disposable
read-only projections. A deterministic memory backend is supplied as a law
reference, never as durable production storage.
