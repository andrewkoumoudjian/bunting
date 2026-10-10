# Admission-sequencer instructions

Implements ADR 0030's latency model as a sans-I/O, clock-free library: the
windowed-minimum RTT estimator, the `physical | equalized | geographic`
release formula with a seeded per-path jitter stream, and the bounded
`(release, arrival)` sequencer.

- The host supplies every time value (monotonic microseconds) and every RTT
  sample; this crate never reads a clock, socket or ambient randomness, and it
  must compile for `wasm32-unknown-unknown`.
- Every value that decides admission order is returned in an
  `AdmissionRecord` so the journal can carry it; replay uses recorded values
  and never re-runs the model against a network.
- Keep the ordering invariant: nothing admitted after a release can be
  ordered before it. Keep every queue and map bounded.
