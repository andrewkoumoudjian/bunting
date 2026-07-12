# Reusable package instructions

Packages remain protocol-focused and testable. The intentional exception is `packages/orderbook`, which wraps the approved `OrderBook-rs` production dependency and therefore inherits its transitive concurrency/runtime packages.

Do not build a parallel matching engine. Other Bunting-owned domain packages should avoid Worker bindings, filesystem I/O, sockets, ambient time, and hidden global state.

`packages/worker-cache` is a platform adapter and may depend on `workers-rs`.
