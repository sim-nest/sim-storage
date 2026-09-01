# sim-relation-site

In one line: Bounded provider-neutral relation effect site for SIM.

## What it gives you

Run the same admitted logical plan against any loaded database provider without letting SQL text, provider locators, unbounded results, or raw transaction state escape into application code. Typed bindings, pushed rows, capability checks, kernel effects, redacted receipts, and total rollback live at one durable seam. The contract keeps inputs, outputs, limits, and refusal cases explicit, so callers can compose the capability without acquiring unrelated host, transport, or product authority. Stable records make the result suitable for tests, inspection, and deterministic integration.

## Why you will be glad

- The public contract makes supported behavior, limits, and typed failures visible before integration.
- One owning crate prevents neighboring libraries from growing competing copies of the same policy.
- Deterministic records and checked tests keep adapters reviewable when implementations evolve.

## Where it fits

Within SIM, sim-relation-site owns only the focused contract described above. Adjacent runtime libraries, platform adapters, codecs, and user surfaces can build around it while retaining their own policy. That boundary keeps the kernel small, avoids competing implementations, and lets this capability evolve without forcing unrelated components to change.
