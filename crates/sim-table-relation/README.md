# sim-table-relation

`sim-table-relation` projects admitted relational storage into the standard SIM
`Table` and `Dir` contracts. `RelationDir` stores a D9 node relation, resolves
paths with `sim-table-core`, performs mutations as checked atomic plans, and
publishes namespace generations with canonical Table compare-exchange.

`RelationView` projects rows from a sealed `CheckedQuery` only after the caller
admits a non-null unique key field. It sorts keys deterministically and refuses
every write. Relation authority and Table/Dir authority remain separate.
