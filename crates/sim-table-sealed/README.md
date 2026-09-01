# sim-table-sealed

`sim-table-sealed` is a contract-preserving authenticated-encryption decorator
for any SIM `Table` and, when implemented by the wrapped value, `Dir` backend.
The host injects an opaque key identity, purpose-separated revocable AEAD and
name-blinding keys, plus a secure unique-nonce source;
the crate never stores a raw key or supplies a random generator.

Version 1 uses `ring`'s IETF ChaCha20-Poly1305: a 256-bit key, 96-bit nonce,
and 128-bit authentication tag. HMAC-SHA-256 produces lane-separated physical
names. A keyed opaque lane prefix permits enumeration and clearing without
touching sibling lanes. The associated data binds the format version, suite, lane, generation,
blinded physical key, and declared metadata class. Unsupported versions and
suites fail closed.

The backend still owns persistence, deletion, directory layout, concurrency,
and durability. Lisp receives only the resulting ordinary Table/Dir value and
uses standard operations; raw keys, nonce generation, sealing, and opening are
not Lisp-callable surfaces.

## Leakage and support policy

Ciphertext size, access patterns, operation timing, the configured metadata
class, and equality of repeated access to one physical name remain observable.
Plaintext keys and cross-lane key equality are hidden. Random nonces hide equal
plaintext values. Deleting ciphertext is only backend deletion; cryptographic
erasure additionally requires the host key provider to destroy or revoke every
managed key envelope, and cannot retract exported plaintext, OS caches,
screenshots, or unmanaged backups.

Only version 1 and suite 1 are supported. A future suite or wire version needs
an explicit reader and migration policy; it must never silently reinterpret an
existing object.
