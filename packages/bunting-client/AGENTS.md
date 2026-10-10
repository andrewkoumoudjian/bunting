# bunting-client instructions

The one Bunting Native Protocol client (ADR 0031, ADR 0040): connection,
TLS 1.3 with the participant's certificate, `Hello`/`Welcome`, immediate
probe replies, heartbeats, the private-stream resume cursor, and
snapshot-plus-update feed books. The trader app, the TUI, bindings and
tests use this crate; none of them may implement BNP themselves.

- This is a participant-side network client, the one package allowed
  sockets, threads and TLS. It never holds market authority: it sends
  ordinary requests and reports what the venue committed.
- Keep it a good client, never a compensating one (ADR 0035): Nagle off,
  probes answered from the reader thread the moment they are decrypted,
  one TLS write per send, a reader that drains the socket continuously.
  Do not add anything that smooths, delays or equalizes timing.
- Networking is native-only (`cfg(not(target_arch = "wasm32"))`);
  [`FeedBook`] and other pure helpers stay host-neutral.
- Every buffer is bounded; a full event queue blocks the reader, which is
  ordinary TCP backpressure (the venue disconnects a client that stops
  reading, as a real venue does).
