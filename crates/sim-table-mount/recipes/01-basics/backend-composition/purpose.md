This recipe records a mounted storage namespace assembled from independent
backends. A root directory receives a database-backed directory mount and a
hash-table leaf mount, then inspects the explicit mount table.

Mounted mutation preserves the backing table's atomic compare-exchange. The
three canonical transitions are absent-to-value (`Absent` to `Value(v)`),
value-to-value (`Value(v)` to `Value(next)`), and value-to-delete
(`Value(next)` to `Delete`). A stored `nil` uses `Value(nil)` and is never
treated as absence. Mount points themselves refuse compare-exchange rather
than splitting the check and mutation across routing operations.
