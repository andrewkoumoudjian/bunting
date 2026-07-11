# Core crate instructions

Crates under this directory are protocol-neutral and platform-neutral unless their name explicitly says otherwise. Avoid Tokio, Worker bindings, wall-clock reads, filesystem I/O, sockets, global mutable state, and unbounded collections.
