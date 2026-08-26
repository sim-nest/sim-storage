# sim-mutual-projection

In one line: Two-key copied projections without shared archive authority.

## What it gives you

`sim-mutual-projection` lets two people exchange one chosen fact while their private archives remain wholly separate. Each person accepts with an independently held key. The invitation fixes exactly which fields may cross, where they came from, and when access ends, so neither side can silently widen the exchange. The result is a copied view, not a mounted store: no table, directory, query, sibling row, adjacent claim, or key-provider handle crosses the boundary. Refusal and silence reveal nothing. Expiry or revocation removes the payload and retains only the minimum receipt identity and. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-mutual-projection owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
