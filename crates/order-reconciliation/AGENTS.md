# Order-reconciliation crate instructions

This crate contains protocol-neutral desired-versus-live order reconciliation and local/external identifier mapping.

Model acknowledgements, rejections, partial fills, cancel-pending, replace-pending, stale orders, duplicate venue messages, reconnect snapshots and out-of-order reports as explicit state transitions. The crate consumes canonical events or normalized venue reports and emits reconciliation actions; it never sends network requests itself.

Use strongly typed local, client and venue order IDs. All transitions are deterministic, idempotent under duplicate reports, bounded, snapshot-capable and covered by generated transition tests.

Do not import the C++ callback/thread model or infer fills from successful request submission. Kill-switch actions and generated cancels still pass through the normal command and risk pipeline.