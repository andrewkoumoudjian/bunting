# Built-in agent instructions

Every built-in policy emits participant intent through a bounded buffer and is
composed with `QuarccExecutionEngine`. There is no direct-to-engine mode.

- Use exact tick/lot/logical-time units and named deterministic RNG streams.
- Label these models Bunting-native unless a port record proves another source.
- Keep policy state, RNG state, and mandatory QUARCC state snapshotable; hosts
  must persist those snapshots with checkpoints so restarts resume identically.
- Agent commands are ordinary participant inputs: they go through the same
  admission and recording path as human orders (ADR 0030 gives agents a
  location in the latency model).
- Do not add sockets, Worker bindings, ambient clocks, or mutable market-engine
  references.
