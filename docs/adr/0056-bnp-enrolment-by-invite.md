# ADR 0056: BNP enrolment by invite (identities created on first connection)

- Status: **Accepted (2026-10-22)**. Owner, 2026-10-22, on joining the
  hosted venue: "i think the certificates should be generated on
  connection". Amends [ADR 0040](0040-bunting-native-protocol-v1.md)
  decision 3 (how a fingerprint enters the roster). Every other ADR 0040
  and [ADR 0041](0041-bnp-live-certificate-revocation.md) decision stands.
- Target until a slice is recorded in `docs/implementation-log/`.

## Context

Today an operator generates every participant's key and certificate
(`bunting host-init`, observed in `bunting-trading-state` `prepare_with`)
and hands each trader three files: `ca.pem`, `participant-N.pem` and
`participant-N.key`. The private key is created on the operator's machine
and travels to the trader. Joining is a file-copying exercise, and the
desk's connect screen asks for three paths.

RIT's client creates an account from the login screen (observed,
`docs/research/rit-binary-audit/desk-client-inventory-1.8.466.md`,
"Onboarding": unknown trader ID, then "create a new account?"). Bunting
must keep ADR 0040's certified identity: no passwords, and a fingerprint
in a roster names the participant.

## Decision

1. **Invites.** An operator mints a one-time invite for one participant
   with `bunting invite`.
   - **The token** is
     `bnpi1:<host:port>:<ca-sha256-hex>:<secret>`:
     - the venue's BNP address;
     - the SHA-256 of the venue's client CA certificate (DER);
     - 128 random bits as lowercase hex.
     Traders paste one line; nothing else is configured.
   - **What the venue stores:** only `sha256(secret)`, the participant ID
     and an optional expiry, in `bnp.enrolment.invites` (a JSON file,
     mode 0600).
   - **Scope:** an invite enrols the participant role only. Instructors
     and administrators keep operator-issued certificates (ADR 0042).
2. **Enrolment on the same listener.** When `bnp.enrolment` is
   configured, the BNP TLS 1.3 listener also accepts a client that
   presents no certificate. Such a session may send exactly one message,
   `Enroll`, before anything else; any other first message is logged out.
   Wire (BNP 1.1, client type 0x06 and server type 0x4e):

   | Type | Name | Body |
   |---|---|---|
   | 0x06 | Enroll | `secret:string csr_pem:string` |
   | 0x4e | Enrolled | `participant_id:u128 certificate_pem:string ca_pem:string` |

   - **Checks:**
     - the invite exists, is unused and is unexpired;
     - its participant has no enrolled certificate yet;
     - the CSR's self-signature verifies (proof of key possession).
   - **On success:**
     - the venue issues a client-auth certificate from its client CA
       (`bnp.enrolment.ca_key`, validity `certificate_days`);
     - it appends `{certificate_sha256, participant_id, role:
       participant}` to `bnp.enrolment.enrolled` (atomic rename), so the
       file outlives a restart;
     - it marks the invite used and answers `Enrolled`, then `Logout`
       ("enrolled; reconnect with your certificate").
   - **On failure:** one generic `Logout` ("invite refused"); the reason
     goes to the venue log only.
   - **Bounds:** the handshake and the `Enroll` message keep the session's
     handshake timeout and frame bounds.
3. **The roster** is the configured `bnp.roster` plus every entry in the
   enrolled file. The venue holds it in memory and adds an entry at
   enrolment, so the new certificate works on the next connection without
   a restart. Revocation is unchanged: CRLs (ADR 0041) apply to enrolled
   certificates exactly as to issued ones.
4. **The client trusts the pin.**
   - During enrolment the client presents no certificate. It accepts the
     venue's TLS chain only if it ends in a CA certificate whose SHA-256
     equals the token's pin, and the leaf verifies against that CA for
     the host it dialled.
   - It stores `ca_pem` (re-checked against the pin), its certificate and
     its private key. The key is generated locally (P-256) and is never
     sent.
   - Every later connection is the ordinary mutual-TLS session of ADR
     0040.
5. **The desk and the CLI.**
   - **Desk:** the connect screen's "Hosted venue" accepts an invite. The
     desk enrols once, saves the identity under its config directory
     (key mode 0600), and remembers the venue. The certificate-file
     fields remain for operator-issued identities.
   - **CLI:** `bunting-trader enroll <token> --out <dir>` writes the same
     three files for scripts.
6. **Provisioning.**
   - `bunting host-init --enrolment` keeps the CA key on the venue host
     (`ca.key`, mode 0600) and writes an empty invites file.
   - `bunting invite --dir <venue> --participant N [--hours H]` prints a
     token.
   - Without `--enrolment` nothing changes: no CA key on the host and
     no unauthenticated handshakes.

## Consequences

- Traders join with one pasted line. Private keys are created where they
  are used.
- The enrolment CA key lives on the venue host. Whoever reads it can mint
  client certificates, but a certificate alone is not an identity: only
  the roster (configured plus enrolled) names a participant. Keep the venue
  directory private, and revoke through CRLs.
- Unauthenticated TLS handshakes reach the venue, as for any TLS service.
  They can only enrol, and they are bounded by the handshake timeout, the
  frame limit and one message per session.
- **FIX parity (ADR 0031):** FIX identity is the roster's username and
  password. Enrolment is a BNP transport bootstrap, not participant data,
  so there is no FIX counterpart (recorded reason).

## Rejected alternatives

- **Self-signed client certificates trusted on first use.** These would
  accept any key the first time and drop ADR 0040's operator CA, so a
  leaked fingerprint list could not be told from an enrolled one.
- **A password per participant over BNP.** ADR 0040 rejects shared
  secrets as identity. The invite secret is single-use and only buys a
  certificate.
- **Operator-generated keys sent to traders (today).** The key travels, and
  joining needs three files.
- **A separate HTTPS enrolment service.** It would be a REST surface, which
  ADR 0031 forbids.

## Validation

- **`bnp-wire`:** `Enroll` and `Enrolled` round-trip.
- **Server integration:**
  - an invite enrols once;
  - the enrolled certificate logs in as its participant without a
    restart;
  - a reused or expired invite, a wrong secret, a CSR with a bad
    signature, and a non-`Enroll` first message are all logged out;
  - the enrolled roster survives a restart.
- **Client:** a token whose pin does not match the venue's CA is refused
  before any secret is sent.
- **Live:** a desk on another machine enrols by invite on the hosted
  venue and trades.
