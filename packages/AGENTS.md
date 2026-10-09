# Reusable package instructions

Packages remain protocol-focused, host-neutral and testable. `packages/bunting-engine` owns the only order book (ADR 0029).

Do not build a parallel matching engine. Bunting-owned domain packages avoid Cloudflare/Worker bindings, filesystem I/O, sockets, ambient time, ambient randomness and hidden global state.
