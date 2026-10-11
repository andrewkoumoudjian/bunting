# Reusable package instructions

Packages remain protocol-focused, host-neutral and testable. `packages/bunting-engine` owns the only order book (ADR 0029).

Do not build a parallel matching engine. Bunting-owned domain packages avoid Cloudflare/Worker bindings, filesystem I/O, sockets, ambient time, ambient randomness and hidden global state. The participant-side exceptions are `bunting-client` (sockets, TLS, threads) and the native `session` and `local_venue` modules of `bunting-trading-state` (a thread over `bunting-client`; practice-venue files and the server process), each documented in its own `AGENTS.md`.
