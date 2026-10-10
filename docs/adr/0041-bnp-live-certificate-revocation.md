# ADR 0041: Live certificate revocation for BNP

- Status: **Accepted** (2026-10-10), implemented in slice 29. Amends the
  operational impact of [ADR 0040](0040-bunting-native-protocol-v1.md)
  ("a revocation needs a restart in v1"); every other ADR 0040 decision
  stands.

## Context

ADR 0040 reads the operator CA and CRLs once at start-up. A competition
cannot restart its venue to cut off a leaked or misused team certificate:
a restart interrupts every team and the round. The owner asked for revocation
without a restart (2026-10-10). Observed before this slice: the BNP listener
built one `rustls` server configuration at start-up, and a session's
certificate was checked only during its handshake.

## Decision

1. A watcher thread (`apps/bunting-server/src/bnp_trust.rs`) re-reads
   `bnp.client_ca` and every `bnp.revocation_lists` file once a second and
   hashes their bytes. When the hash changes it builds a new verifier and
   TLS configuration and bumps a generation counter.
2. New handshakes always use the current configuration.
3. Every live session re-verifies its own certificate chain against the new
   verifier when the generation changes, on the next turn of its loop
   (at most 20 ms later). A chain that no longer verifies gets `Logout`
   ("certificate no longer trusted: …") and the connection closes. Resting
   orders stay, as after any disconnect.
4. Trust never silently widens: a file that cannot be read or parsed, or a
   listed CRL file that holds no CRL, leaves the previous trust in force and
   is reported on stderr. At start-up the same conditions refuse to start.
5. The roster and the server certificate are still read at start-up. A
   revocation is a CRL from the CA, the standard X.509 mechanism (RFC 5280).

## Consequences

- An operator revokes a team by replacing the CRL file (write a new file and
  rename it over the old one); the team is logged out within about a second
  and cannot reconnect with that certificate.
- Rotating the operator CA file applies the same way.
- One extra thread and two small file reads per second per venue.

## Rejected alternatives

- **Reload on a signal (SIGHUP):** not portable to every host and needs
  signal handling the server does not otherwise have.
- **An admin HTTP endpoint:** adds a control surface; the file is already the
  operator's source of truth.
- **Roster removal as revocation:** the roster is part of the run
  configuration and would need config reloading; a CRL is the standard
  mechanism and works across runs.
- **Applying an empty or unreadable CRL:** would drop every revocation.

## Validation

`apps/bunting-server/tests/bnp.rs`
(`a_revocation_list_update_logs_out_the_live_session_without_a_restart`):
revoking a live team's certificate logs that session out with the reason,
new handshakes with it fail, another team's session continues, and a
damaged CRL file afterwards keeps the revocation in force.

## Operational impact

RUNBOOK "Bunting Native Protocol" step 4. Replace CRL files atomically
(write then rename) so the watcher never reads a half-written file; a
half-written file is refused and retried on the next poll anyway.

## Security impact

Revocation now takes effect for live sessions, closing the window in which a
compromised certificate stayed connected until restart. The fail-closed
rules keep a damaged file from re-admitting revoked certificates.

## References

- [ADR 0040](0040-bunting-native-protocol-v1.md), [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md)
- RFC 5280 (X.509 certificates and CRLs)
