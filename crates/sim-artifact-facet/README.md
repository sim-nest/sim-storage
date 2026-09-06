# sim-artifact-facet

`sim-artifact-facet` owns the effect-free law for comparing and merging one
owned artifact region. A facet fixes the artifact, owner, region, semantic
projection, merge policy, disclosure decision, and generated-owner status
before base, observed, and intended images are admitted into distinct Rust
roles. Every identity-bearing facet field becomes read-only at construction, so
the facet id and the specification inspected by consumers cannot diverge.

The law preserves an observed foreign edit when a proposal is unchanged,
accepts an already-true result, applies an intended image when the base is
still present, and merges only bounded disjoint UTF-8 line changes under an
explicit policy. Overlap, ambiguous ownership, invalid regions, binary
two-sided edits, and exceeded work bounds fail closed. The crate performs no
filesystem or Git operation.

The `descriptor` recipe-policy applies because its recipe exposes the pure
three-way contract without claiming host execution. Integration tests cover
idempotence, disjoint preservation, conflict symmetry, role separation,
generated ownership, and work limits.
