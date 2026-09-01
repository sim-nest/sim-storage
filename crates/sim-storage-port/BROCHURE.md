# sim-storage-port

In one line: Portable host-directory port contracts for SIM storage.

## What it gives you

Validated relative components, deterministic directory entries, sanitized errors, cancellation, atomic replacement, and compare-exchange through one HostDirPort. Platform adapters retain native filesystem authority while table and directory policy sees only the bounded contract. Mounted-root escape and special files have explicit refusal categories. Atomic writes and compare-exchange are testable without exposing host paths. Portable storage policy remains independent of operating-system APIs. This crate is the host seam beneath SIM's file-backed table and directory implementations. Platforms implement it; storage libraries consume it. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-storage-port owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
