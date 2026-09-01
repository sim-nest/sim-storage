# sim-mutual-projection

`sim-mutual-projection` copies one explicitly shaped fact between two people
without mounting, querying, or sharing either person's archive. The immutable
offer binds the fact, admitted fields, provenance, expiry, parties, and two
independently held verification keys. Both acceptances are required.

The receiving view contains only copied field bytes and provenance content
links. Expiry, refusal, or either party's revocation removes that payload and
leaves a minimum content-identity tombstone for policy audit.
