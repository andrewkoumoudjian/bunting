# Bunting as a trading-competition platform

Status: proposal. Companion to `docs/streamlining-audit.md`, which records what
the repository is today.

Event shape (owner input): a **live trading competition**. Participants compete
against each other on one market run by this engine. They write trading agents;
they do not modify the engine.

Audience: whoever runs the competition, plus the maintainers who must keep the
engine honest while N teams trade against it for the duration of the event.

---

## 0. The reframe, and a correction

An earlier draft of this document assumed participants would build *on* the
engine. They do not. That changes who the audiences are:

- **Participants are clients.** They need credentials, a protocol, a practice
  venue, and a published scoring function. Most of them should never clone this
  repository or install a Rust toolchain.
- **Organizers are operators.** They need to run one live market for N
  concurrent teams, control rounds, watch it happen, and defend the final
  scores.

The important discovery is that **the repository already knows it is a
competition platform.** This is not a general market simulator that could be
adapted; the competition surface is already named and partially built:

| Artifact | Evidence |
| --- | --- |
| FIX competition profile | `schemas/fix/bunting.fixlatest.competition.v1.json` + `.orchestra.xml` |
| Competition wire validation | `simfix-wire`: `competition_rule()` (line 415), `validate_competition()` (430), `CompetitionDictionary` (457) |
| Competition policies | `docs/specs/competition-policies-v1.md`, `docs/specs/bunting-fix-competition-profile.md` |
| Competition views + scoring | `bunting-application/src/competition.rs`: `competition_policies()`, `AccountView`, `RiskScoreView`, `NewsTenderView`, `discovery()`, `risk_score()` |
| Server-side competition path | `bunting-server/src/runtime.rs`: `competition_messages()` (416), `operator_command()` (572), `OpenTenderPayload` (562) |
| End-to-end golden | `tests/goldens/competition-full-run.v1.json` |
| Rounds | `IterationId` in `market-types` |
| RIT/RITC heritage | `docs/specs/rit-class-market-simulation.md`, `docs/specs/rit-tui-parity-matrix.md`, `docs/research/rit-binary-audit/`, `ref/ritc_mm` |

So the target is a RITC-class competition venue, and the specification work is
largely done. The gap is not design. **The gap is that the venue is
single-connection, single-participant, and has no operator surface for running
rounds.** That is a much better position to be in than the audit alone
suggested, and it sharpens what to build.

Two products come out of this repository:

1. **The venue** — one native server, many concurrent participants, operated
   live, fully replayable.
2. **The competitor pack** — protocol document, sample clients, credentials,
   scenario descriptions, and a *practice venue* each team runs locally.

"Runnable locally" means the competitor pack. "Auditable" means the venue's
record can be replayed to recompute every score.

---

## 1. Existential: the concurrent multi-session venue

A trading competition is N teams trading against each other on one market at
the same time. Everything else is secondary to that.

**Today** (`docs/deployment.md`): each server process "accepts at most one FIX
connection", binds "one immutable participant/run binding", and the native
origin store is "intentionally single-writer" — "Put no second server process
on the same origin file."

That is one team per market. There is no competition.

**Most of the fix is already written, in the wrong crate.**
`apps/bunting-tui/src/local_market.rs` (1,052 lines) is an async `tokio`
acceptor that serves FIX connections and calls `bunting-engine` in-process:
`TcpListener::bind` at line 116, `serve_connections` at 120, `serve` at 131.
Meanwhile `apps/bunting-server/src/runtime.rs` uses blocking `std::net`
(lines 164, 819) with no `tokio` in `[dependencies]`.

Promote the TUI's tokio acceptor into `bunting-server`, delete the blocking one,
and generalize to N sessions against one run with one authoritative writer
serializing commits. This resolves three audit findings at once — divergent I/O
stacks, single-session limit, the 940-line `runtime.rs` god-module — largely by
relocating working code.

Requirements a live competition adds beyond "more than one socket":

- **Participant roster.** One run, N credentialed participants, each with
  starting cash and inventory. Today there is one immutable participant binding
  per process and a single `bunting-local-admin-token`.
- **Per-session isolation.** One team disconnecting, flooding, or sending
  garbage must not disturb any other session or the run.
- **Reconnect without loss.** `simfix-session` already has `SessionSnapshot`,
  `restore()`, and journal entries. A team whose laptop sleeps mid-round must
  resume, not forfeit. This is a support incident waiting to happen and the
  machinery is already there.
- **Bounded fairness controls** — see §2.

Do not touch `simfix-wire`/`simfix-session`. Their sans-I/O design
(`receive_bytes`, `poll`, `SessionAction`) is exactly what makes this change
cheap and safe.

---

## 2. Fairness is a venue feature, not a policy document

This is the concern most likely to be discovered too late, and it does not
appear anywhere in the current docs.

In a live FIX-over-TCP competition, **network position is alpha.** A team on the
venue's switch beats a team on hotel wifi, regardless of strategy quality. A
team that discovers they can submit 10,000 orders per second degrades the market
for everyone and wins on throughput rather than insight. Neither outcome is the
competition you meant to run.

Concrete mechanisms, in rough order of value:

1. **Per-session rate limits and order-count caps.** `simfix-wire::WireLimits`
   (line 91) already bounds messages, and `orderbook-rs` supplies per-book open
   order counts, notional limits, and price bands (`architecture.md` §5, §12).
   These need to be enforced *per participant session* and to reject with a
   message that names the limit. Without this, one team can degrade the venue.
2. **Decide and publish the latency model.** Three defensible choices:
   - *Co-locate everyone* — same room, same switch. What RITC does. Simplest and
     fairest, but constrains the event format.
   - *Normalize with a gateway delay* — a fixed inbound delay per session that
     dominates network variance, making everyone's effective latency equal.
   - *Remove latency from the game* — discrete matching intervals (frequent
     batch auctions) so arrival order within an interval does not matter.
     This makes remote participation genuinely fair and changes the competition
     into a pricing/strategy contest rather than a speed contest.

   The engine's deterministic scenario clock makes the third option unusually
   cheap to implement, and it is the only one that survives remote teams.
3. **Symmetric market data.** Every participant must receive the same book
   updates on the same schedule. Publish the depth, the update cadence, and
   whether it is snapshot or incremental, in the protocol document.
4. **Publish it all before the event.** Teams will optimize against whatever
   the venue rewards. If the reward function is undocumented, they will
   reverse-engineer it, and that becomes the competition.

Pick a latency model early. It is close to unchangeable once teams have written
code against it.

---

## 3. Auditability: replay the venue, not the agent

For a live competition, the auditable record is the **venue's**, not the
participant's. A network client reacting to a live market cannot be re-run
deterministically — but the venue's decisions can.

The engine already produces everything needed and does not yet exploit it:
`origin-store` holds accepted commands, canonical events, and idempotency
records; `command-transaction` commits before publishing; snapshots are
checksum-addressed; `EventSequence` is monotonic; scenarios are deterministic
(ADR 0010).

So the audit primitive is:

```bash
bunting replay <run-id>     # re-run the recorded command stream through the
                            # engine; recompute every fill, position, and score
```

Every accepted command is recorded with its arrival sequence. Replay feeds that
exact stream back through the engine and must reproduce the identical canonical
event stream, final checksum, and scores. Byte-identical or it fails.

What this buys:

- **Disputes resolve mechanically.** "My order should have filled" is answered
  by the event log plus a replay, not by argument.
- **Scores are recomputable** by anyone with the run archive, including after
  the event, including by a suspicious participant.
- **Cheating is structurally hard.** The venue is the sole source of truth for
  fills; a participant cannot fabricate one.
- **It hardens the engine for free.** Any nondeterminism — map iteration order,
  wall-clock leakage into matching, unseeded RNG — surfaces as a failed replay.
  That is a better determinism suite than anyone would write deliberately, and
  it protects ADR 0010 permanently.

Ship the run archive — scenario, seeds, engine version, command stream, event
stream, final checksums — as a single publishable artifact per round.
`tests/goldens/competition-full-run.v1.json` is already close to this shape;
generalize that format rather than inventing one.

Scoring stays a pure function of the archive:

```bash
bunting score  <run-archive>     # -> per-participant scores
bunting judge  <round-archives>  # -> leaderboard.json + leaderboard.html
```

No database, no scoring service. A leaderboard that is a pure function of an
immutable archive is reproducible offline by anyone — which is where disputes
actually get settled.

---

## 4. The operator surface: rounds, roster, control

A competition is not one continuous market. It is heats, rounds, resets, and
published results. This is the largest genuinely *missing* piece, and it is the
difference between an engine and a platform someone can run an event on.

Partially present already: `IterationId` in `market-types`,
`operator_command()` and `OpenTenderPayload` in `runtime.rs`, and an admin HTTP
surface at `runtime.rs:818–900`. What a live event needs on top:

- **Roster management** — create N participants with credentials and starting
  balances; export a credential sheet to hand out.
- **Round lifecycle** — arm, start, pause, resume, halt, end, reset; carry or
  reset scores across rounds. The engine already has halt-and-drain and a kill
  switch via `orderbook-rs`.
- **Live event injection** — news, tenders, and shocks on a schedule or on
  operator command. `NewsTenderView` and `OpenTenderPayload` exist; the
  scheduling surface does not.
- **A visible operator console.** This is where the TUI earns its keep — see §5.
- **One panic button** that halts trading cleanly, commits, and leaves a
  replayable record. Something will go wrong mid-round.

Note the tension to resolve: the admin surface is currently a hand-written
HTTP/1 parser (`write_http`, `runtime.rs:882`). It is ~40 lines that will grow
under exactly this pressure. Either freeze it deliberately and drive operations
from the CLI against the origin store, or accept one small framework. Do not let
it grow organically during event prep.

---

## 5. Promote the TUI: it is the operator and spectator console

The audit flagged `apps/bunting-tui` as the largest component (6,251 lines) and
the source of roughly half the 243-crate dependency graph. Under a live
competition framing, that assessment changes.

A trading competition needs a big screen at the front of the room: the book,
the tape, the leaderboard, positions, and news as it drops. `bunting-tui`
already renders order books, candlestick charts, volume, and a log panel
(`chart/`, `tui/views/market.rs`, `tui/widgets/candlestick_chart.rs`), and
`docs/specs/rit-tui-parity-matrix.md` shows it was built against RIT's
operator/trader surface.

So: **keep it, reframe it as the operator/spectator console, and stop treating
it as a participant client.** Participants connect over FIX with their own code;
the TUI is what the organizers and the room watch.

Still feature-gate it out of the server build. `apps/bunting-cli` depends on
`bunting-tui` unconditionally, so `bunting server` currently links `ratatui`,
`crossterm`, `rustls`, `ring`, and the `windows-*` families. A default-off `tui`
feature keeps the venue binary small without giving up the console.

---

## 6. Baseline agents are the market, not examples

`packages/bunting-agents` (1,295 lines) holds deterministic built-in policies
composed with mandatory QUARCC execution, scheduled by `bunting-runtime`.

Under a "build on the engine" framing those were examples for participants.
Under a competition framing they are **the market environment**, and they become
essential:

- **Liquidity.** Early in a round, participant agents have no one to trade
  against. Baseline market makers solve the cold-start problem that otherwise
  makes the first minutes of every round dead.
- **Realism.** Noise traders and directional flow give strategies something to
  detect and exploit. A market of only competitors is a strange, thin,
  unrealistic game.
- **Calibration.** A published baseline score tells participants whether they
  are actually good, and gives organizers a sanity check that a round is
  well-formed.
- **Fairness.** Because they are deterministic and seeded, every team faces the
  *same* environment. This is what makes cross-team comparison legitimate, and
  it is only true because the agents are deterministic.

Publish their behaviour and their scores. A hidden market environment is a
reverse-engineering contest.

`ref/abides` (agent-based interactive discrete event simulation),
`ref/market-maker-rs`, and `ref/ritc_mm` are the right prior art for expanding
this set — and note this is the participant-facing use of the QUARCC execution
engine that `README.md` already describes ("built-in agents always use it").

---

## 7. The competitor pack, and what "runnable locally" means

Participants need a **practice venue**: the same engine, the same scenarios, the
same scoring, running on their own laptop, so they arrive on competition day
with working code. This is how RITC practice cases work, and it is the single
highest-leverage thing for participant experience.

It also means the "build cliff" from the audit matters in a specific way: for
*participants*, not for the engine.

The pack:

1. **`PROTOCOL.md`** — the FIX profile: supported messages, fields, limits,
   reject codes, market-data cadence. Generate it from
   `schemas/fix/bunting.fixlatest.competition.v1.orchestra.xml`, which already
   exists, rather than hand-writing a second source of truth.
2. **Sample clients that actually trade** — Python first (the median
   competitor writes Python), then Java/Go via off-the-shelf FIX libraries, then
   Rust and C++. `tests/interop/quickfixgo` already proves the
   `github.com/quickfixgo/quickfix v0.9.10` path works; that test is a working
   sample client in disguise.
3. **A practice venue in one command**, with prebuilt binaries so no Rust
   toolchain is needed. `install.sh` already ships macOS arm64/x86_64, Linux
   x86_64, and Windows x86_64 archives — make that the documented default path.
   Note `git clone` must work *without* `--recursive`: CI already runs
   `submodules: false`, but no document says so, and a naive recursive clone
   pulls 483 MB.
4. **`SCORING.md`** with the scoring function published up front, and
   `bunting score` runnable locally so teams self-evaluate.
5. **Practice scenarios** matching the competition rounds in structure but not
   in seed.
6. **`bunting doctor`** — see §8.

Deliberately *not* in the pack: the Rust workspace, the ADRs, `AGENTS.md`, the
engine internals. Participants are clients. `AGENTS.md`'s instruction-precedence
chain and the strict lint set (`unsafe_code = "forbid"`,
`unwrap_used`/`panic` denied) remain correct for the engine and are irrelevant
to a competitor — which conveniently resolves the governance tension entirely:
there is no ungoverned tree in this repository, because participant code lives
in participants' own repositories.

---

## 8. `bunting doctor`: buy this before the event

The largest hidden cost of running a live competition is not judging. It is
twenty teams whose FIX session will not establish, each consuming an organizer
for twenty minutes, during setup, simultaneously.

The venue's strictness makes this worse, and the strictness is correct: bounded
messages, dictionary validation (`validate_competition`,
`CompetitionDictionary`), constant-time credential comparison, mTLS
expectations. Every one is a silent rejection to a confused competitor.

Two commands that answer "why isn't it working" without an organizer:

```bash
bunting doctor                      # venue reachable? port open? credentials
                                    # valid? dictionary/version compatible?
bunting conformance --agent <cmd>   # drive a team's client through N scenarios
                                    # and report exactly which message was
                                    # wrong, which tag, and why
```

The conformance harness is the participant-facing mirror of machinery that
already exists — `tests/oracles/nbc-matcher`, the QuickFIX-Go interop gate, and
`ref/liquibook`/`ref/exchange-core` as differential oracles per
`architecture.md` §14. The same tooling that proves the venue correct can prove
a competitor's client correct. Build it once, aim it in both directions.

Budget real time on error-message quality. Every reject should name the field,
the limit, and the remedy. It is far cheaper than staffing a help desk during
setup.

---

## 9. Cloudflare: the spectator edge, not the venue

Workers cannot accept inbound raw TCP (`docs/deployment.md`, ADR 0020).
A competition venue is *defined* by participants connecting to it. Therefore a
Worker can never be the venue — not as a limitation to route around, but as a
fact that assigns roles cleanly and settles the "wrapper only" intent on
technical grounds:

- **The venue is the native server**, on one machine on the competition
  network. Inbound TCP, filesystem persistence, long-lived sessions, live
  operator control.
- **The Worker serves the public read path**: live leaderboard, published run
  archives, book snapshots, the spectator view. Read-only, checksum-addressed,
  globally cached, and extremely bursty at exactly the moment the leaderboard
  goes on screen and gets shared. This is what Workers are genuinely good at,
  and `packages/worker-cache` — immutable content-addressed snapshot storage —
  is already precisely that shape.

This also deletes `apps/bunting-server/src/relay.rs` (9.5 KB). The relay exists
only to work around Workers' inbound-TCP limitation. Once the Worker stops
pretending to be the venue, the workaround is unnecessary.

Cloudflare becomes a real wrapper over a genuinely different concern, which
answers the "Worker parity" question left open in
`docs/streamlining-audit.md` §6. The decoupling mechanics are unchanged from
that document's §4.2 — and its step 0 (rewrite the docs and the CI policy grep
that currently *requires* browser-JS shims in the engine manifest) still has to
land first, or the change fails CI and gets reverted.

---

## 10. Scenarios are the competition content

The scenario machinery exists — `schemas/nbc/config.v1.json`, `scenarios/nbc/`,
`bunting-engine/src/compatibility/nbc/config.rs` — and the engine exposes
halts, price bands, kill switch, self-trade prevention, and expiry sweeps
through `orderbook-rs` (`architecture.md` §5).

Design the rounds as named, deterministic, separately scored scenarios. A
RITC-shaped set:

| Round | Tests |
| --- | --- |
| Liquidity / market making | two-sided quoting, inventory risk |
| Trending market | directional risk, position sizing |
| News shock | reaction speed, risk limits under stress |
| Tender offer | valuation under a decision deadline (`NewsTenderView`, `OpenTenderPayload` exist) |
| Halt and resume | operational robustness, reconnect handling |
| Thin book / wide spreads | execution quality, patience |

This is mostly **data plus policy**, not new engine code — the cheapest
high-value content in the whole plan. Determinism also makes rounds comparable
across teams, which a live random market never is.

`ref/nbc_engine` and `ref/nbc-hft-simulation` are the prior art if you want a
latency-sensitive round; note §2 first, because an HFT round makes the latency
model decision unavoidable.

---

## 11. Documents

There are 20+ top-level docs and 21 ADRs, several superseded and unmarked
(0004 by 0020; 0013/0014 by 0018/0019; 0015 by 0016). Split them by audience,
which the current tree does not do:

**Competitor-facing** (the pack, §7): `PROTOCOL.md`, `SCORING.md`,
`GETTING_STARTED.md` (practice venue in three commands), `RULES.md` (limits,
fairness model, conduct).

**Organizer-facing**: `RUNBOOK.md` — set up the venue, create the roster, run a
round, handle a mid-round failure, publish results, settle a dispute by replay.
This does not exist today and is what an event actually runs on.

**Maintainer-facing**: everything currently in `docs/`, moved under
`docs/internals/`, with superseded ADRs marked superseded in-file. `AGENTS.md`
keeps full authority over the engine tree; it is simply not what a competitor
or an operator reads.

---

## 12. What I would do first

Ordered by whether the competition can happen at all.

| # | Work | Why here |
| --- | --- | --- |
| 1 | Concurrent multi-session FIX venue with a participant roster (§1) | Without it there is no competition. Mostly promoting `local_market.rs`. |
| 2 | Decide and publish the fairness/latency model (§2); enforce per-session limits | Near-unchangeable once teams write against it. One team can otherwise degrade the venue. |
| 3 | `bunting replay` + the run archive format (§3) | Defines the audit story and the dispute process. Generalize the existing golden. |
| 4 | Operator surface: rounds, roster, event injection, panic button (§4) | The missing platform layer; an event cannot be run without it. |
| 5 | Competitor pack v1: `PROTOCOL.md` from the Orchestra XML, Python sample client, prebuilt practice venue (§7) | Teams need weeks with this, not days. |
| 6 | `bunting score` / `bunting judge` (§3) | Organizer workflow becomes one command; enables published pre-event scoring. |
| 7 | `bunting doctor` + conformance harness (§8) | Buy it before setup day, not during. |
| 8 | Baseline agents and round scenarios (§6, §10) | The market environment and the content. Largely data plus existing policies. |
| 9 | Docs reframing + CI policy rewrite (§9, audit §4.2 step 0) | Gates the Cloudflare decoupling. |
| 10 | Pure deletions: `edge-api`, `temp`, tRPC oracle, crate-mirror submodules, relay (audit §4.5) | Zero-risk shrinkage; fill gaps with it. |
| 11 | Python binding, then C++; engine module splits; `fix-application` extraction | Real work, but FIX carries the competition. Python matters for practice tooling more than for competing. |

Items 1–4 are the difference between an engine and a venue. Items 5–8 are the
difference between an event that runs and an event that runs well.

Note how the priority order shifts from the earlier audit: the language
bindings drop from headline to item 11, because **FIX over TCP is already a
binding for every language a competitor might use**, and the competition runs on
it. The bindings matter for practice tooling, scoring, and embedding — not for
letting teams compete.

---

## 13. Open decisions for the organizer

- **Latency model** (§2). Co-located, gateway-normalized, or discrete matching
  intervals? Blocks the protocol document and shapes what the competition
  actually rewards. Decide first.
- **Market topology.** One shared market for all teams (dramatic, realistic,
  but one bad actor affects everyone) or parallel identical markets per small
  group (isolated, cleanly comparable, less spectacle)? Baseline agents make the
  second option viable.
- **Live scoring or end-of-round scoring.** Live is better theatre and creates
  gaming incentives near the bell; end-of-round is cleaner. Recommendation:
  display live, settle from the replayed archive.
- **Reconnect policy.** If a team's session drops mid-round, do their resting
  orders stay live? This has real strategic consequences and must be in
  `RULES.md` before anyone writes code.
- **FIX codec** — still open from the audit and now on the critical path, since
  it gates the protocol document and the conformance harness. Decide it against
  the QuickFIX-Go interop gate.
- **Remote participation.** Answered largely by §2. If remote teams are allowed,
  discrete matching intervals are effectively required.
