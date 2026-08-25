# Sealed Table/Dir

In one line: Put authenticated confidentiality around an existing SIM table
without replacing the store or teaching the kernel about encryption.

## What it gives you

`SealedTable` wraps a persistent Table or Dir. Values are authenticated and
encrypted before reaching the backend, while logical names become keyed,
lane-separated physical names. Purpose-separated keys prevent cross-protocol
key reuse. A record copied to another key, lane,
generation, or metadata class is rejected. The host retains control of keys,
revocation, and secure nonce generation.

## Why you will be glad

- Finance, private notes, and other lanes can have independent grants.
- Any conforming backend remains responsible for its own proven persistence
  and deletion behavior.
- Corruption and substitution fail closed with redacted diagnostics.
- Existing Lisp code continues to use ordinary Table and Dir operations.

## Honest boundary

Encryption does not hide ciphertext size, access timing, or access patterns,
and backend deletion is not proof that exported plaintext disappeared.
Cryptographic erasure is meaningful only within the named key-provider and
managed-backup boundary.
