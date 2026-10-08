# Reusable package instructions

Packages remain protocol-focused and testable. `packages/bunting-engine` owns the only order book (ADR 0029).

Do not build a parallel matching engine. Other Bunting-owned domain packages should avoid Worker bindings, filesystem I/O, sockets, ambient time, and hidden global state.
