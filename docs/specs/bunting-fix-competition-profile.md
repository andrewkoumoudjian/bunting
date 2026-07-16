# Bunting FIX Latest competition profile

Status: canonical target profile, version `bunting.fixlatest.competition.v1`

This profile uses a FIXT.1.1 session, FIX 5.0 SP2 application semantics and
standard FIX Latest messages where their semantics fit, with the FIX Latest
Orchestra repository as the normative standard dictionary. The released
`rustyfix-dictionary` crate supplies the runtime FIXT.1.1 and FIX 5.0 SP2
QuickFIX dictionaries; it is a validation implementation, not the normative
source. Bunting's generated Orchestra overlay defines only project-owned
Bunting `U*` messages plus tags `10000`-`10020` for simulation concepts. The
machine-readable dictionary is
[`schemas/fix/bunting.fixlatest.competition.v1.json`](../../schemas/fix/bunting.fixlatest.competition.v1.json).
The existing `simfix-*` packages implement only the subset marked implemented
in the corrected plan; listing a message here is a downstream requirement, not
an implementation claim.

## Session, authentication and topology

The wire format is standard SOH-delimited FIXT.1.1 with BodyLength and CheckSum.
Native Bunting may be a TCP/TLS acceptor. Cloudflare Bunting is always the TCP
initiator to an external acceptor; there is no Worker raw-TCP ingress.

Logon `A` requires standard sender/target IDs, sequence/time, encryption method,
heartbeat interval, `DefaultApplVerID(1137)=9`, Username `553`, Password `554` or an approved credential
token, and `BuntingProfileVersion(10000)`. The authenticated session binds actor,
role, tenant, team, participant and allowed runs. Application fields cannot
change that binding. ResetSeqNumFlag is honored only when both peers and the
server policy authorize reset; otherwise Logon rejects.

Heartbeat `0`, TestRequest `1`, ResendRequest `2`, Reject `3`, SequenceReset `4`
and Logout `5` follow FIX session semantics. Session sequence is independent of
`BuntingCommittedSequence(10010)`, which identifies origin-committed market
facts. Resent messages use PossDupFlag and OrigSendingTime.

## Discovery and lifecycle

SecurityListRequest `x` and SecurityList `y` expose eligible instruments with
standard security fields plus run/scenario/version, listing, tick, lot, bounds,
currency, run lifecycle, logical time, capability and policy tags. Participant
sessions cannot start or control runs.

Instructor/admin sessions use `UA` for publish/create/start/pause/resume/advance,
pacing and terminate operations. Each mutation carries tags `10007`-`10009` and
receives status plus committed sequence. Capability discovery precedes use;
unsupported operations receive BusinessMessageReject `j` with a stable code.

## Orders

NewOrderSingle `D`, OrderCancelRequest `F`, OrderCancelReplaceRequest `G` and
OrderStatusRequest `H` cover submit, cancel, replace and status. Standard IDs,
side, quantity, order type, price and TIF are used. Bunting tags carry run,
command, correlation, expected version and logical time. Composite, bulk-cancel
and special order actions use `U7` or an explicitly versioned `U*` payload until
a suitable standard FIX workflow is selected.

ExecutionReport `8`, OrderCancelReject `9` and BusinessMessageReject `j` report
only committed outcomes. Reports preserve ClOrdID/OrigClOrdID/OrderID/ExecID,
CumQty/LeavesQty/LastQty/LastPx, status, stable reject reason, fees and committed
sequence. Acceptance, partial fill, full fill, cancel, replace, expiry and
rejection remain distinct. Transport receipt never implies venue acceptance.

## Public market data

MarketDataRequest `V`, SnapshotFullRefresh `W`, IncrementalRefresh `X` and
RequestReject `Y` provide L1, aggregated/raw L2, trades and status. Depth is
bounded and updates contain absolute resulting quantity; zero/delete removes a
level. TradeCaptureReportRequest `AD` and Report `AE` provide bounded committed
trades and history, including time-and-sales and versioned OHLC intervals.

Every output carries run, listing/instrument and committed sequence. Public
state may coalesce. A gap outside retention produces `UC` reset followed by a
snapshot. Public data never contains participant-private ownership or ledger
facts.

## Private account and competition state

Order status/execution reports expose private live and historical orders/fills.
RequestForPositions `AN` and PositionReport `AP` provide positions and exact
cash by currency, settled/reserved/accrued/scheduled balances,
buying power, NLV, realized/unrealized P&L, cost basis, fees/rebates, limits,
warnings, penalties and risk groups. All wide integers use canonical decimal
strings and all marks name their policy version.

UserRequest `BE`/UserResponse `BF` covers bounded session/participant metadata,
never credential disclosure. Participant-private data is filtered by the
authenticated participant/team binding before serialization.

## News, tenders, OTC, assets, leases, reports and score

- Standard News `B` publishes ordered news with public, participant or team audience and logical time.
- `U6` lists tenders and performs bid/accept/decline with expiry and committed outcome.
- `U9` requests or delivers transaction logs, P&L, time-and-sales, OTC activity, reports, rankings and score.
- `UB` is instructor/admin-only risk, limit, compliance, fine and participant-control mutation/query traffic.

Product areas deferred beyond the competition MVC, including OTC and
asset/facility workflows, receive a reviewed standard-message mapping before
they enter this profile. Each extension message uses
`BuntingResourceKind(10016)`, resource ID, action, status and a
bounded canonical JSON payload only where standard FIX groups cannot express the
versioned structure. Payload schemas are separately versioned and unknown fields
reject. A generic JSON escape hatch is prohibited.

## Audience rules

Public messages contain competition-public run, instrument, book, trade,
history and public news data. Private messages contain only the authenticated
participant's or authorized team's orders, fills, accounts, tenders, OTC,
assets, reports and score. Instructor messages contain explicitly projected
instructional monitoring/control data. Admin messages require administrator
claims. Built-in agents are participant-scoped and receive no privileged data.

Every application output includes `BuntingAudience(10012)`. Audience filtering
occurs before encoding, and tests must prove that another participant's order,
fill, balance, risk, private news, tender, OTC or report cannot appear.

## Replay, reset and slow consumers

FIX resend repairs the session journal; it does not reconstruct market history.
Market recovery uses `BuntingRecoveryCursor(10011)` and committed sequence. The
server returns a retained tail or `UC` reset plus current public/private
snapshots. Public book updates may coalesce, but trades and private facts are
replayable or force reset. A slow consumer receives Logout with the last usable
cursor and reconnect instructions; it is never silently advanced.

## Competition completeness gate

A profile release is competition-complete only when automated coverage proves
that every participant action and every participant-visible field in the
product/parity matrices has a FIX request/report path, audience test, committed
sequence behavior and recovery test. Browser or TUI-only functionality fails
this gate.
