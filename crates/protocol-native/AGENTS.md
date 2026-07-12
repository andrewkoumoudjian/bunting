# protocol-native instructions

Own native HTTP and WebSocket schemas, subscriptions, snapshots, deltas, acknowledgements and recovery. Translate only at the boundary; canonical events and market types remain protocol-neutral.

The initial stream is versioned JSON. L2 deltas carry absolute resulting level quantities, zero removes a level, and snapshots are sorted and checksummed. Every stream batch includes run ID, activation-local stream epoch/record ID and the highest canonical event sequence reflected.

Keep public and private channels separate. Public book/status data may be coalesced to latest absolute state; trades and private order/execution/account/risk records may be batched but never silently dropped. Define typed slow-consumer warnings, reset reasons and recovery cursors. Bound subscriptions, depth, frame bytes, messages and unacknowledged records.