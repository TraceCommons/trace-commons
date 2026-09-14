# Schema 1 compatibility baseline

These bytes were emitted by the production serializer at routed stack commit
`b9501e56a710b4054ebe5e7cf4301730676cd9ee`. The capture uses the existing
`comparison_specs` synthetic helpers and includes accepted, partial, rejected,
pending, unknown, and unassessed outcomes across the two canonical cohorts.

The regression test deserializes and validates both records, then requires the
current serializer to reproduce every pre-change byte. This freezes the old
analytical, saved-record, audit, and estimation-input digests as well as the
schema 1 field set. The fixtures contain no private source data.
