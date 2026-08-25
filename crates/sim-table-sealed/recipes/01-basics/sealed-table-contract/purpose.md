The host constructs a sealed value only after injecting its revocable key grant
and secure unique-nonce service. Lisp code receives that value through its
application composition and can use only the canonical Table/Dir operations
listed by this specimen. There is deliberately no Lisp encryption, decryption,
raw-key, nonce, suite-selection, or backend-specific operation.

The wrapper authenticates version, suite, lane, generation, blinded physical
name, and metadata class. The backend still owns persistence and deletion.
Ciphertext size, timing, access patterns, and repeated access to one physical
name remain visible; key destruction says nothing about unmanaged plaintext or
backups.
