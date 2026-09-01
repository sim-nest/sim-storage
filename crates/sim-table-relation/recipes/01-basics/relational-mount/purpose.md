# Relational plus filesystem namespace

The recipe mounts the same relation-backed root and an injected filesystem root
under one `MountedDir`. Mount authority changes routing only: relation, table,
and filesystem capabilities are still independently required by their owners.
