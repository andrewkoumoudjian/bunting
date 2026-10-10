# Admission-sequencer instructions

Implements ADR 0030's latency model as amended by ADR 0034 as a sans-I/O,
clock-free library: the delay estimator (minimum of kernel TCP RTT and
lifetime probe RTT, never increasing within a connection), the
`physical | equalized | geographic` release formula with a seeded per-path
jitter stream, and the bounded `(release, arrival)` sequencer.

ADR 0034's contract is the test for every change here: distance up to `D`
must not change outcomes, nobody may gain by looking slower, and a client's
own processing speed must keep its full advantage.

- The host supplies every time value (monotonic microseconds) and every RTT
  sample; this crate never reads a clock, socket or ambient randomness, and it
  must compile for `wasm32-unknown-unknown`.
- Every value that decides admission order is returned in an
  `AdmissionRecord` so the journal can carry it; replay uses recorded values
  and never re-runs the model against a network.
- Keep the ordering invariant: nothing admitted after a release can be
  ordered before it. Keep every queue and map bounded.
