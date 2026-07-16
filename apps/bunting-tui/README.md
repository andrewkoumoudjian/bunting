# Bunting trading terminal

This native Ratatui workstation adapts the Longbridge Terminal component
hierarchy and interaction model to Bunting. Production mode is a FIXT.1.1
session with FIX 5.0 SP2 application semantics and is a
initiator over configured TCP or TLS; it never reads or mutates
`bunting-engine`. The embedded engine-backed acceptor remains available only
behind `--fixture` for deterministic native testing.

The terminal ships named `local`, `remote` and `cloudflare-gateway` profile
templates. Profiles select endpoint, transport, CompIDs, username, credential
environment variable, team, run, requested role, heartbeat and reset policy.
Passwords and tokens are read from the named environment variable and are
never persisted or shown in the bounded raw FIX journal.

The default configuration path is `~/.config/bunting/terminal.json` (or
`$XDG_CONFIG_HOME/bunting/terminal.json`). A custom file and profile can be
selected without changing saved defaults:

```bash
export BUNTING_LOCAL_PASSWORD='...'
cargo run --locked -p bunting-tui -- --profile local
cargo run --locked -p bunting-tui -- \
  --config ./terminal.json --profile remote
```

TLS profiles use platform trust roots and may add a PEM CA through `ca_file`:

```json
{
  "kind": "tls",
  "server_name": "fix.example.com",
  "ca_file": "/absolute/path/to/competition-ca.pem"
}
```

Use `/workspace save NAME`, `/workspace load NAME`, and
`/workspace remove NAME` to manage bounded named layouts. `R` reconnects a
disconnected session while preserving its in-memory sequence/journal state;
`/session reset` works only for a profile with `allow_sequence_reset: true` and
requests `ResetSeqNumFlag(141)=Y`. The server remains authoritative over reset
authorization and verified identity.

The embedded server also starts a deterministic local agent population. Each
policy wake emits bounded intent into its own QUARCC execution engine, then the
QUARCC adapter maps the resulting action to the same canonical market-command
boundary used by a remote server. The human remains an independent FIX
participant alongside those agents.

The account, simulation, collaboration and administration workspaces are
capability-gated. They show `BACKEND UNAVAILABLE` until the corresponding FIX
report type is actually observed, and privileged projections remain hidden
because the current server mapping does not return a verified role claim.

Select any built-in policies by repeating or comma-separating `--agent`; the
default population is a static liquidity provider, a zero-intelligence noise
trader and a long-momentum trader. `--agent-tick-ms` controls wall-clock pacing
without changing deterministic logical wake times:

```bash
cargo run --locked -p bunting-tui -- --fixture \
  --agent avellaneda_stoikov,poisson_noise --agent-tick-ms 250
```

Use `--endpoint 127.0.0.1:9880` for a process-local endpoint override. The
loopback acceptor is a native test harness; the Cloudflare Worker remains
outbound-TCP-only and does not accept raw TCP. A Cloudflare participant profile
therefore points at the external gateway/acceptor from the product contract,
never at raw Worker ingress.

The component adaptations and their licenses are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
