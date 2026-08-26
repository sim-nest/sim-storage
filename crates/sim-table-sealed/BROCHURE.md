# sim-table-sealed

In one line: Authenticated encryption decorator for SIM Table and Dir backends.

## What it gives you

without replacing the store or teaching the kernel about encryption. `SealedTable` wraps a persistent Table or Dir. Values are authenticated and encrypted before reaching the backend, while logical names become keyed, lane-separated physical names. Purpose-separated keys prevent cross-protocol key reuse. A record copied to another key, lane, generation, or metadata class is rejected. The host retains control of keys, revocation, and secure nonce generation. Finance, private notes, and other lanes can have independent grants. Any conforming backend remains responsible for its own proven persistence and deletion behavior. Corruption and substitution. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-table-sealed owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
