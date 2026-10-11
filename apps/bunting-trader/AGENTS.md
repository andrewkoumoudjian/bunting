# bunting-trader instructions

A participant and operator app that connects to a hosted Bunting venue
over the Bunting Native Protocol (ADR 0040; `operator` subcommands for
instructor and administrator certificates, ADR 0042; `tender`, `otc` and
`deals` subcommands, ADR 0043). It owns command-line parsing and presentation
only; every protocol, TLS, probe, heartbeat and resume behavior lives in
`packages/bunting-client`. It holds no market authority: it prints what
the venue committed.

- Never build protocol handling here; extend `bunting-client`.
- Client order IDs must be unique for the participant for the whole run;
  the default (microseconds since the Unix epoch) is for manual use.
- Native only; the binary compiles to an empty `main` on wasm32.
