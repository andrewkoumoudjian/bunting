# Core crate instructions

Crates remain protocol-focused and testable. The intentional exception is `crates/orderbook`, which wraps the approved `OrderBook-rs` production dependency and therefore inherits its transitive concurrency/runtime packages.

Do not build a parallel matching engine. Other Bunting-owned domain crates should avoid Worker bindings, filesystem I/O, sockets, ambient time, and hidden global state.

`crates/worker-cache` is a platform adapter and may depend on `workers-rs`.
