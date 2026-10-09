# Command-transaction instructions

Keep orchestration sans-I/O and deterministic. Recover and invoke `bunting-engine` without re-owning matching, ledger, risk, or mutable run state. Commit engine-produced results to the origin store before anything is acknowledged or published.

Current behavior clones the run on load and after prepare; the target (exploration note Step 3) is a writer-owned live state with a command journal. Do not add further whole-state copies.
