# market-run-do instructions

One object is the authoritative sequencer for one run. Persist accepted event batches before acknowledgement or fan-out, use SQLite for recovery, and make alarm/Queue work idempotent.

Accept native market-data sockets with the Durable Object Hibernation WebSocket API. Publish only committed projections. Implement `stream_epoch`/`record_id`, reset-plus-snapshot recovery, absolute L1/L2 updates, application ACKs, bounded outstanding bytes/records and explicit slow-consumer disconnects. Do not use periodic timers solely for stream flushing; batch by committed event batch and bounded frame size.

Hibernation attachments contain only small identity, authorization, subscription, protocol and last-ACK metadata. Canonical books, queues and strategy state stay in SQLite/snapshots.

Never execute untrusted strategy code synchronously in the command transaction. Commit invocation requests, dispatch them asynchronously, and accept results only as authenticated idempotent commands with matching invocation/state revisions.