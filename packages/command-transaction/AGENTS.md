# Command-transaction instructions

Keep orchestration sans-I/O and deterministic. Recover and invoke `bunting-engine` without re-owning matching, ledger, risk, or mutable run state. Commit engine-produced candidate state before cache writes, and treat cache failures as recovery conditions.
