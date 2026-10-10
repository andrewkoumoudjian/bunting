# bnp-wire instructions

Own the Bunting Native Protocol (BNP) frame layout and message codec
(ADR 0040): length-prefixed frames, one message type byte, fixed-width
little-endian fields, bounded strings and repeating groups. Nothing else:
no sockets, TLS, time, identity, engine types or mapping to commands.

- Stay dependency-free and host-neutral (`wasm32-unknown-unknown`).
- Every decoder path is bounded: frame size, string bytes and group counts
  are checked before allocation. Unknown types, trailing bytes and invalid
  enum values are errors, never skipped.
- The layout is a published contract. Changing a field's width, order or
  meaning needs a new `PROTOCOL_VERSION` and an ADR; adding a message type
  is additive only when old peers can refuse it cleanly.
- Keep `docs/specs/bnp-v1.md` in step with this file.
