# Command-transaction instructions

Keep orchestration sans-I/O and deterministic. Recover and invoke `bunting-engine` without re-owning matching, ledger, risk, or mutable run state. Commit engine-produced results to the origin store before anything is acknowledged or published.

Since slice 14 the origin owns and journals the live run, so this crate only gives order flow and simulation commands one call shape and error type. It is a candidate to fold into its callers; do not grow it a second state path or add whole-state copies.
