# Origin-store instructions

Keep persistence models Worker-independent and exact. IDs and sequences must survive JSON and database round trips without JavaScript numeric coercion. A commit binds idempotency, canonical events, projections, version advancement, and snapshot metadata into one atomic expected-version operation.
