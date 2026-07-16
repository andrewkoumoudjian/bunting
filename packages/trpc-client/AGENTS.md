# Native tRPC client instructions

Implement only the frozen Bunting tRPC subset. Keep transport injected, retries bounded and mutations non-retryable unless the caller supplies an idempotent command identifier.
