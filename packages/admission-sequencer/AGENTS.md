# Admission-sequencer instructions

Implements ADR 0035 latency as a sans-I/O, clock-free
library: the measured access-latency estimator (published, never used for
ordering), the latency map (teams, venues and the hub at named locations;
every directed path, including team to team, from location links, with a
seeded jitter stream per direction; `release = t_rx + L(p, d)`, outbound
`L(s, p)`), and
the bounded `(release, arrival)` sequencer.

Do not reintroduce modes, equalization or compensation (ADR 0035 supersedes
ADR 0034).

- The host supplies every time value (monotonic microseconds) and every RTT
  sample; this crate never reads a clock, socket or ambient randomness, and it
  must compile for `wasm32-unknown-unknown`.
- Every value that decides admission order is returned in an
  `AdmissionRecord` so the journal can carry it; replay uses recorded values
  and never re-runs the model against a network.
- Keep the ordering invariant: nothing admitted after a release can be
  ordered before it. Keep every queue and map bounded.
