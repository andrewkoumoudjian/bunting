# Origin-store instructions

Keep persistence adapters Worker-independent and exact. Persist the engine-owned snapshot envelope without redefining or mutating its state. IDs and sequences must survive JSON and database round trips without JavaScript numeric coercion. A commit binds idempotency, canonical events, projections, version advancement, and snapshot metadata into one atomic expected-version operation.
