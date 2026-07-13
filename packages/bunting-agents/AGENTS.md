# Built-in agent instructions

Every built-in policy emits participant intent through a bounded buffer and is
composed with `QuarccExecutionEngine`. There is no direct-to-engine mode.

- Use exact tick/lot/logical-time units and named deterministic RNG streams.
- Label these models Bunting-native unless a port record proves another source.
- Keep policy state, RNG state, and mandatory QUARCC state snapshotable.
- Do not add sockets, Worker bindings, ambient clocks, or mutable market-engine
  references.
