# strategy-dispatch-consumer instructions

This Worker consumes committed strategy invocation requests and calls `services/strategy-loader` through a service binding. Queue delivery is at-least-once, so every request and result is keyed by an immutable invocation ID and expected strategy-state revision.

Validate envelope sizes and provenance before dispatch. Never invent market context, mutate strategy state or submit orders directly. Return the bounded loader result to the authoritative `MarketRun` object as an authenticated internal command.

Duplicate execution is allowed but duplicate acceptance is not. The run object accepts only the first valid result for a pending invocation ID/state revision before its deadline. Retries, dead-letter handling and observability are explicit and cannot alter canonical order without a committed result event.