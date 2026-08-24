# Relational directories without another VFS

`RelationDir` turns a durable D9 node relation into the ordinary Table/Dir
surface: atomic mutations, codec-identified bytes, strict path handling, and a
CAS-published namespace head. `RelationView` exposes admitted relational results
as a deterministic read-only keyed table without weakening relation authority.
