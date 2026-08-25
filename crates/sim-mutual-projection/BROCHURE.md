# A mutual porch, not a shared archive

`sim-mutual-projection` lets two people exchange one chosen fact while their
private archives remain wholly separate. Each person accepts with an
independently held key. The invitation fixes exactly which fields may cross,
where they came from, and when access ends, so neither side can silently widen
the exchange.

The result is a copied view, not a mounted store: no table, directory, query,
sibling row, adjacent claim, or key-provider handle crosses the boundary.
Refusal and silence reveal nothing. Expiry or revocation removes the payload
and retains only the minimum receipt identity and reason needed for audit.
