#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Deterministic, sans-I/O scheduling for authenticated built-in participants.

use bunting_agents::{
    AgentConfig, AgentContext, AgentObservation, BuiltInPolicy, ManagedAgent, ManagedAgentSnapshot,
    NextWake, PolicyKind, PolicySnapshot,
};
use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::VerifiedActor;
use bunting_engine::RunState;
use bunting_market_events::{CommandPayload, EventEnvelope, EventPayload};
use bunting_market_types::{
    CorrelationId, InstrumentId, ListingKey, LogicalTimeNs, ParticipantId, PriceTicks,
    QuantityLots, RunId, VenueId,
};
use quarcc_bunting_adapter::{BuntingCommandContext, BuntingExecutionAdapter};
use quarcc_execution_engine::ids::ActionId;
use quarcc_execution_engine::lifecycle::OrderLifecycle;
use quarcc_execution_engine::{
    ExecutionAction, ExecutionConfig, ExecutionSnapshot, QuarccExecutionEngine,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

mod fundamental;
pub use fundamental::{FundamentalPath, FundamentalWalker, VolatilityRegime};

const AGENT_ID_STRIDE: u128 = 1_000_000;
/// Orders, open or finished, a built-in agent's QUARCC state remembers.
const AGENT_MAX_ORDERS: usize = 256;
/// Reports and intents a built-in agent's QUARCC state remembers by ID.
const AGENT_MAX_SEEN_REPORTS: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAgentConfig {
    pub kind: PolicyKind,
    pub participant_id: ParticipantId,
    pub base_quantity: QuantityLots,
    pub spread_ticks: i64,
    pub inventory_target: QuantityLots,
    pub wake_interval_ns: u64,
    pub seed: u64,
    pub max_intents_per_wake: usize,
    /// The instrument this agent trades; absent means the scheduler's
    /// `instrument_id`. Any other instrument needs an entry in
    /// `RuntimeConfig::other_instruments` and a `venue_id`.
    #[serde(default)]
    pub instrument_id: Option<InstrumentId>,
    /// The venue whose listing this agent trades. Absent means the
    /// instrument's only listing (its orders are ordinary `SubmitOrder`s);
    /// present, its orders are `SubmitOrderAtListing` for that listing.
    #[serde(default)]
    pub venue_id: Option<VenueId>,
}

/// The fundamental of an instrument other than the scheduler's own. Every
/// listing of the instrument shares it; the listings' books stay independent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentFundamental {
    pub instrument_id: InstrumentId,
    pub fundamental_price: PriceTicks,
    /// Its own path (and seed); absent keeps the price constant.
    #[serde(default)]
    pub fundamental_path: Option<FundamentalPath>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub run_id: RunId,
    pub instrument_id: InstrumentId,
    pub fundamental_price: PriceTicks,
    pub remaining_parent_quantity: QuantityLots,
    pub max_actions_per_tick: usize,
    pub agents: Vec<RuntimeAgentConfig>,
    /// How the fundamental moves with the run clock; absent keeps
    /// `fundamental_price` constant.
    #[serde(default)]
    pub fundamental_path: Option<FundamentalPath>,
    /// Fundamentals of the further instruments agents trade.
    #[serde(default)]
    pub other_instruments: Vec<InstrumentFundamental>,
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.run_id.get() == 0
            || self.instrument_id.get() == 0
            || self.fundamental_price.get() <= 1
            || self.remaining_parent_quantity.get() < 0
            || self.max_actions_per_tick == 0
            || self
                .fundamental_path
                .as_ref()
                .is_some_and(|path| !path.is_valid(self.fundamental_price))
        {
            return Err(RuntimeError::InvalidConfig);
        }
        let mut instruments = BTreeSet::from([self.instrument_id]);
        for other in &self.other_instruments {
            if other.instrument_id.get() == 0
                || other.fundamental_price.get() <= 1
                || other
                    .fundamental_path
                    .as_ref()
                    .is_some_and(|path| !path.is_valid(other.fundamental_price))
                || !instruments.insert(other.instrument_id)
            {
                return Err(RuntimeError::InvalidConfig);
            }
        }
        let mut participants = BTreeSet::new();
        for agent in &self.agents {
            let instrument = agent.instrument_id.unwrap_or(self.instrument_id);
            if !instruments.contains(&instrument)
                || (instrument != self.instrument_id && agent.venue_id.is_none())
                || agent.participant_id.get() == 0
                || agent.base_quantity.get() <= 0
                || agent.spread_ticks <= 0
                || agent.wake_interval_ns == 0
                || agent.max_intents_per_wake == 0
                || !participants.insert(agent.participant_id)
            {
                return Err(RuntimeError::InvalidConfig);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeAgentSnapshot {
    pub participant_id: ParticipantId,
    pub managed: ManagedAgentSnapshot<PolicySnapshot>,
    pub adapter: BuntingExecutionAdapter,
    pub next_wake: NextWake,
}

/// One decided action not yet submitted: `agent` indexes the configured
/// agents.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PendingAction {
    pub agent: usize,
    pub action: ExecutionAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeSnapshot {
    pub version: u16,
    pub config: RuntimeConfig,
    pub logical_time: LogicalTimeNs,
    pub previous_trade: PriceTicks,
    pub last_trade: PriceTicks,
    pub agents: Vec<RuntimeAgentSnapshot>,
    /// Last two trade prices of each listing an agent names by venue.
    #[serde(default)]
    pub listing_trades: Vec<ListingTrades>,
    /// Decided actions not yet submitted, in submission order. A runtime
    /// restored mid-tick submits these before deciding anything new.
    #[serde(default)]
    pub pending: Vec<PendingAction>,
    /// Agents retired after an error of their own, in retirement order.
    #[serde(default)]
    pub retired: Vec<ParticipantId>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListingTrades {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
    pub previous_trade: PriceTicks,
    pub last_trade: PriceTicks,
}

/// Snapshot format written by [`DeterministicRuntime::snapshot`].
pub const RUNTIME_SNAPSHOT_VERSION: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidConfig,
    InvalidSnapshot,
    InvalidMarket,
    Agent(String),
    Adapter(String),
    Host(String),
    ActionBoundExceeded,
    ArithmeticOverflow,
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for RuntimeError {}

pub trait RuntimeHost {
    /// Reads the committed run without copying it. `read` must not commit.
    fn read_state<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, RuntimeError>;

    /// Commits through the application's authenticated transaction boundary and
    /// returns this command's committed events. A command whose ID is
    /// already committed (a runtime resumed from a checkpoint re-submitting
    /// its pending actions) must not commit again: the host returns the
    /// recorded events, so each action takes effect exactly once. A host
    /// shared with other participants stamps the command's logical time
    /// from the run clock (ADR 0037): the runtime's own counter only
    /// orders its decisions.
    fn commit(
        &mut self,
        actor: &VerifiedActor,
        command: &bunting_market_events::Command,
    ) -> Result<Vec<EventEnvelope>, RuntimeError>;

    /// Events other participants committed since the host last returned
    /// them, in commit order, never the runtime's own (those come from
    /// [`Self::commit`]). Agents learn of fills on their resting orders, and
    /// of trades, from these. Hosts whose runtime is the only committer
    /// return nothing.
    fn take_committed(&mut self) -> Result<Vec<EventEnvelope>, RuntimeError> {
        Ok(Vec::new())
    }

    /// Durably records the runtime after a tick's decisions and before any
    /// of them is submitted. Resuming from the latest checkpoint and
    /// re-submitting its pending actions (see [`Self::commit`]) reproduces
    /// an uninterrupted run.
    fn checkpoint(&mut self, _snapshot: &RuntimeSnapshot) -> Result<(), RuntimeError> {
        Ok(())
    }

    /// The run's time now, on the run clock (ADR 0037), for a host shared
    /// with other participants: agents wake when run time reaches their
    /// next wake, never ahead of it. `None` (a host whose runtime is the
    /// only committer) wakes the next agent at once.
    fn run_time(&self, _run_id: RunId) -> Result<Option<LogicalTimeNs>, RuntimeError> {
        Ok(None)
    }

    /// Called once when an agent is retired after an error of its own (the
    /// market carries on without it). The host reports it to the operator.
    fn agent_retired(&mut self, _participant_id: ParticipantId, _reason: &str) {}
}

struct ScheduledAgent {
    participant_id: ParticipantId,
    managed: ManagedAgent<BuiltInPolicy>,
    adapter: BuntingExecutionAdapter,
    next_wake: NextWake,
    /// The instrument traded, and the listing when the config names a venue.
    instrument: InstrumentId,
    listing: Option<ListingKey>,
    /// Set when the agent hit an error of its own: it no longer wakes or
    /// hears reports, and only the cancels of its resting orders are sent.
    retired: bool,
}

pub struct DeterministicRuntime {
    config: RuntimeConfig,
    agents: Vec<ScheduledAgent>,
    logical_time: LogicalTimeNs,
    previous_trade: PriceTicks,
    last_trade: PriceTicks,
    pending: VecDeque<(usize, ExecutionAction)>,
    /// Retired agents in the order they were retired.
    retirement_order: Vec<ParticipantId>,
    /// Retirements the host has not yet been told of.
    announce: Vec<(ParticipantId, String)>,
    /// Last two trades of each listing named by an agent's `venue_id`.
    listing_trades: BTreeMap<ListingKey, (PriceTicks, PriceTicks)>,
    /// One per instrument; caches only, rebuilt from the config after a
    /// restart.
    walkers: BTreeMap<InstrumentId, FundamentalWalker>,
}

fn walkers_of(config: &RuntimeConfig) -> BTreeMap<InstrumentId, FundamentalWalker> {
    let mut walkers = BTreeMap::from([(
        config.instrument_id,
        FundamentalWalker::new(config.fundamental_path.clone(), config.fundamental_price),
    )]);
    for other in &config.other_instruments {
        walkers.insert(
            other.instrument_id,
            FundamentalWalker::new(other.fundamental_path.clone(), other.fundamental_price),
        );
    }
    walkers
}

fn start_price(config: &RuntimeConfig, instrument: InstrumentId) -> PriceTicks {
    config
        .other_instruments
        .iter()
        .find(|other| other.instrument_id == instrument)
        .map_or(config.fundamental_price, |other| other.fundamental_price)
}

/// The listings agents name by venue, each starting at its instrument's
/// fundamental.
fn listing_trades_of(config: &RuntimeConfig) -> BTreeMap<ListingKey, (PriceTicks, PriceTicks)> {
    config
        .agents
        .iter()
        .filter_map(|agent| {
            let instrument = agent.instrument_id.unwrap_or(config.instrument_id);
            let start = start_price(config, instrument);
            agent
                .venue_id
                .map(|venue| (ListingKey::new(venue, instrument), (start, start)))
        })
        .collect()
}

impl DeterministicRuntime {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        config.validate()?;
        let agents = config
            .agents
            .iter()
            .map(|agent| scheduled_agent(agent, &config))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            previous_trade: config.fundamental_price,
            last_trade: config.fundamental_price,
            walkers: walkers_of(&config),
            listing_trades: listing_trades_of(&config),
            config,
            agents,
            logical_time: LogicalTimeNs::new(0),
            pending: VecDeque::new(),
            retirement_order: Vec::new(),
            announce: Vec::new(),
        })
    }

    pub fn restore(snapshot: RuntimeSnapshot) -> Result<Self, RuntimeError> {
        if !(1..=RUNTIME_SNAPSHOT_VERSION).contains(&snapshot.version)
            || snapshot.agents.len() != snapshot.config.agents.len()
            || snapshot
                .pending
                .iter()
                .any(|pending| pending.agent >= snapshot.agents.len())
        {
            return Err(RuntimeError::InvalidSnapshot);
        }
        snapshot.config.validate()?;
        let agents = snapshot
            .agents
            .into_iter()
            .zip(&snapshot.config.agents)
            .map(|(saved, configured)| {
                if saved.participant_id != configured.participant_id
                    || saved.managed.policy.config.participant_id != configured.participant_id
                    || saved.managed.policy.config.instrument_id
                        != configured
                            .instrument_id
                            .unwrap_or(snapshot.config.instrument_id)
                {
                    return Err(RuntimeError::InvalidSnapshot);
                }
                let execution =
                    QuarccExecutionEngine::restore(saved.managed.execution).map_err(|error| {
                        RuntimeError::Agent(format!("execution restore: {error:?}"))
                    })?;
                Ok(ScheduledAgent {
                    participant_id: saved.participant_id,
                    managed: ManagedAgent::new(
                        BuiltInPolicy::restore(saved.managed.policy),
                        execution,
                    ),
                    adapter: saved.adapter,
                    next_wake: saved.next_wake,
                    instrument: configured
                        .instrument_id
                        .unwrap_or(snapshot.config.instrument_id),
                    retired: snapshot.retired.contains(&configured.participant_id),
                    listing: configured.venue_id.map(|venue| {
                        ListingKey::new(
                            venue,
                            configured
                                .instrument_id
                                .unwrap_or(snapshot.config.instrument_id),
                        )
                    }),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut listing_trades = listing_trades_of(&snapshot.config);
        for saved in &snapshot.listing_trades {
            if let Some(entry) =
                listing_trades.get_mut(&ListingKey::new(saved.venue_id, saved.instrument_id))
            {
                *entry = (saved.previous_trade, saved.last_trade);
            }
        }
        Ok(Self {
            walkers: walkers_of(&snapshot.config),
            announce: Vec::new(),
            retirement_order: snapshot.retired.clone(),
            listing_trades,
            config: snapshot.config,
            agents,
            logical_time: snapshot.logical_time,
            previous_trade: snapshot.previous_trade,
            last_trade: snapshot.last_trade,
            pending: snapshot
                .pending
                .into_iter()
                .map(|pending| (pending.agent, pending.action))
                .collect(),
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            version: RUNTIME_SNAPSHOT_VERSION,
            config: self.config.clone(),
            logical_time: self.logical_time,
            previous_trade: self.previous_trade,
            last_trade: self.last_trade,
            retired: self.retirement_order.clone(),
            listing_trades: self
                .listing_trades
                .iter()
                .map(|(key, (previous_trade, last_trade))| ListingTrades {
                    venue_id: key.venue_id,
                    instrument_id: key.instrument_id,
                    previous_trade: *previous_trade,
                    last_trade: *last_trade,
                })
                .collect(),
            agents: self
                .agents
                .iter()
                .map(|agent| RuntimeAgentSnapshot {
                    participant_id: agent.participant_id,
                    managed: agent.managed.snapshot(),
                    adapter: agent.adapter.clone(),
                    next_wake: agent.next_wake,
                })
                .collect(),
            pending: self
                .pending
                .iter()
                .map(|(agent, action)| PendingAction {
                    agent: *agent,
                    action: action.clone(),
                })
                .collect(),
        }
    }

    /// The configured agents' participant IDs.
    pub fn participants(&self) -> impl Iterator<Item = ParticipantId> + '_ {
        self.agents.iter().map(|agent| agent.participant_id)
    }

    /// Runs one tick: unless actions decided before a restart are still
    /// pending, takes other participants' commits, wakes due agents and
    /// checkpoints; then submits every pending action and reacts to its
    /// reports.
    pub fn advance<H: RuntimeHost>(&mut self, host: &mut H) -> Result<usize, RuntimeError> {
        if self.pending.is_empty() {
            self.decide(host)?;
        }
        let mut processed = 0_usize;
        while let Some((index, action)) = self.pending.pop_front() {
            if processed >= self.config.max_actions_per_tick {
                return Err(RuntimeError::ActionBoundExceeded);
            }
            // A retired agent only gets its cancels out.
            if self.agents[index].retired && !matches!(action, ExecutionAction::Cancel { .. }) {
                continue;
            }
            processed = processed.saturating_add(1);
            let expected_sequence = host.read_state(self.config.run_id, RunState::sequence)?;
            self.logical_time = LogicalTimeNs::new(self.logical_time.get().saturating_add(1));
            let participant_id = self.agents[index].participant_id;
            let built = self.agents[index].adapter.command_for_action(
                &action,
                &BuntingCommandContext {
                    run_id: self.config.run_id,
                    actor: participant_id,
                    expected_sequence,
                    logical_time: self.logical_time,
                    correlation_id: CorrelationId::new(u128::from(self.logical_time.get())),
                },
            );
            let mut command = match built {
                Ok(command) => command,
                Err(error) => {
                    // The agent's own action does not convert: the agent is
                    // at fault, not the venue.
                    let mut cancels = VecDeque::new();
                    self.retire(index, format!("command: {error:?}"), &mut cancels);
                    self.pending.extend(cancels);
                    continue;
                }
            };
            // An agent that names a venue trades that listing; the order
            // still goes through the same sequencer as any other.
            if let (Some(listing_key), CommandPayload::SubmitOrder(order)) =
                (self.agents[index].listing, &command.payload)
            {
                command.payload = CommandPayload::SubmitOrderAtListing {
                    listing_key,
                    order: order.clone(),
                };
            }
            let actor = built_in_actor(participant_id)?;
            let events = host.commit(&actor, &command)?;
            self.observe_trades(&events);
            let mut reactions = VecDeque::new();
            self.dispatch(&events, &mut reactions)?;
            self.pending.extend(reactions);
        }
        for (participant_id, reason) in std::mem::take(&mut self.announce) {
            host.agent_retired(participant_id, &reason);
        }
        Ok(processed)
    }

    /// Retires an agent after an error of its own. Its resting orders are
    /// cancelled through the same sequencer, queued after anything already
    /// decided: leaving them would strand quotes nobody maintains any more
    /// (a retired provider's GTC quotes would sit at a stale price for the
    /// rest of the run). The IDs continue the agent's own sequence, so the
    /// cancels are deterministic and the host deduplicates them on a replay.
    fn retire(
        &mut self,
        index: usize,
        reason: String,
        pending: &mut VecDeque<(usize, ExecutionAction)>,
    ) {
        if self.agents[index].retired {
            return;
        }
        self.agents[index].retired = true;
        let participant_id = self.agents[index].participant_id;
        self.retirement_order.push(participant_id);
        self.announce.push((participant_id, reason));
        let execution = self.agents[index].managed.snapshot().execution;
        let mut next = execution.next_id;
        for (local, order) in &execution.orders {
            if matches!(
                order.lifecycle,
                OrderLifecycle::PendingSubmit
                    | OrderLifecycle::Live
                    | OrderLifecycle::PartiallyFilled
                    | OrderLifecycle::Quarantined
            ) {
                pending.push_back((
                    index,
                    ExecutionAction::Cancel {
                        action_id: ActionId::new(next),
                        local_order_id: *local,
                        venue_order_id: order.venue_order_id.clone(),
                    },
                ));
                next = next.saturating_add(1);
            }
        }
    }

    /// Takes other participants' commits, wakes due agents into `pending`,
    /// and checkpoints before anything is submitted.
    fn decide<H: RuntimeHost>(&mut self, host: &mut H) -> Result<(), RuntimeError> {
        let Some(next_time) = self
            .agents
            .iter()
            .filter(|agent| !agent.retired)
            .map(|agent| agent.next_wake.logical_time)
            .min()
        else {
            return Ok(());
        };
        let mut pending = VecDeque::new();
        let elsewhere = host.take_committed()?;
        self.observe_trades(&elsewhere);
        self.dispatch(&elsewhere, &mut pending)?;
        let now = match host.run_time(self.config.run_id)? {
            // Nobody is due yet on the run clock: only learn what others did.
            Some(now) if now < next_time => {
                if !elsewhere.is_empty() {
                    self.pending = pending;
                    host.checkpoint(&self.snapshot())?;
                }
                return Ok(());
            }
            Some(now) => now,
            None => next_time,
        };
        self.logical_time =
            LogicalTimeNs::new(now.get().max(self.logical_time.get().saturating_add(1)));
        let now_ns = self.logical_time.get();
        let fundamentals: BTreeMap<InstrumentId, PriceTicks> = self
            .walkers
            .iter_mut()
            .map(|(instrument, walker)| (*instrument, walker.at(now_ns)))
            .collect();
        let due = self
            .agents
            .iter()
            .enumerate()
            .filter_map(|(index, agent)| {
                (!agent.retired && agent.next_wake.logical_time <= self.logical_time)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in due {
            let instrument = self.agents[index].instrument;
            let listing = self.agents[index].listing;
            let fundamental = fundamentals
                .get(&instrument)
                .copied()
                .ok_or(RuntimeError::InvalidConfig)?;
            let observation = host.read_state(self.config.run_id, |state| {
                self.observation(state, instrument, listing, fundamental)
            })??;
            let position = self.agents[index]
                .managed
                .snapshot()
                .execution
                .positions
                .get(&instrument)
                .map_or_else(|| QuantityLots::new(0), |position| position.quantity);
            let woken = self.agents[index].managed.on_wake(
                &AgentContext {
                    logical_time: self.logical_time,
                    current_position: position,
                    remaining_parent_quantity: self.config.remaining_parent_quantity,
                },
                &observation,
            );
            match woken {
                Ok((next_wake, actions)) => {
                    self.agents[index].next_wake = next_wake;
                    pending.extend(actions.into_iter().map(|action| (index, action)));
                }
                Err(error) => self.retire(index, format!("wake: {error:?}"), &mut pending),
            }
        }
        self.pending = pending;
        host.checkpoint(&self.snapshot())
    }

    fn observation(
        &self,
        state: &RunState,
        instrument: InstrumentId,
        listing: Option<ListingKey>,
        fundamental: PriceTicks,
    ) -> Result<AgentObservation, RuntimeError> {
        let key = match listing {
            Some(key) => key,
            None => state
                .listing_key_for_instrument(instrument)
                .map_err(|_| RuntimeError::InvalidMarket)?,
        };
        let (previous_trade, last_trade) = listing
            .and_then(|key| self.listing_trades.get(&key).copied())
            .unwrap_or((self.previous_trade, self.last_trade));
        let (bids, asks) = state
            .visible_levels(key)
            .map_err(|_| RuntimeError::InvalidMarket)?;
        let fallback_bid = fundamental.get().saturating_sub(1).max(1);
        let best_bid = bids.first().map_or(fallback_bid, |(price, _)| price.get());
        let best_ask = asks
            .first()
            .map_or_else(|| best_bid.saturating_add(2), |(price, _)| price.get());
        let quantity = |levels: &bunting_engine::VisibleLevels| -> Result<i64, RuntimeError> {
            Ok(levels.first().map_or(0, |(_, value)| value.get()))
        };
        Ok(AgentObservation {
            best_bid: PriceTicks::new(best_bid),
            best_ask: PriceTicks::new(best_ask.max(best_bid.saturating_add(1))),
            bid_quantity: QuantityLots::new(quantity(&bids)?),
            ask_quantity: QuantityLots::new(quantity(&asks)?),
            last_trade,
            fundamental,
            previous_trade,
            observed_volume: QuantityLots::new(quantity(&bids)?.saturating_add(quantity(&asks)?)),
            stress_bps: 0,
        })
    }

    fn observe_trades(&mut self, events: &[EventEnvelope]) {
        for event in events {
            if let EventPayload::TradeExecuted {
                price,
                instrument_id,
                listing_key,
                ..
            } = event.payload
            {
                if let Some(entry) = listing_key.and_then(|key| self.listing_trades.get_mut(&key)) {
                    *entry = (entry.1, price);
                } else if instrument_id == self.config.instrument_id {
                    self.previous_trade = self.last_trade;
                    self.last_trade = price;
                }
            }
        }
    }

    fn dispatch(
        &mut self,
        events: &[EventEnvelope],
        pending: &mut VecDeque<(usize, ExecutionAction)>,
    ) -> Result<(), RuntimeError> {
        for index in 0..self.agents.len() {
            if self.agents[index].retired {
                continue;
            }
            match self.dispatch_one(index, events, pending) {
                Ok(()) => {}
                Err(error @ (RuntimeError::Agent(_) | RuntimeError::Adapter(_))) => {
                    self.retire(index, error.to_string(), pending);
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn dispatch_one(
        &mut self,
        index: usize,
        events: &[EventEnvelope],
        pending: &mut VecDeque<(usize, ExecutionAction)>,
    ) -> Result<(), RuntimeError> {
        let agent = &mut self.agents[index];
        let reports = agent
            .adapter
            .normalize_committed_events(agent.participant_id, events)
            .map_err(|error| RuntimeError::Adapter(format!("reports: {error:?}")))?;
        for report in reports {
            let actions = agent
                .managed
                .on_private_event(&report)
                .map_err(|error| RuntimeError::Agent(format!("private event: {error:?}")))?;
            pending.extend(actions.into_iter().map(|action| (index, action)));
        }
        Ok(())
    }
}

fn scheduled_agent(
    config: &RuntimeAgentConfig,
    scheduler: &RuntimeConfig,
) -> Result<ScheduledAgent, RuntimeError> {
    let instrument_id = config.instrument_id.unwrap_or(scheduler.instrument_id);
    let policy = BuiltInPolicy::new(AgentConfig {
        kind: config.kind,
        participant_id: config.participant_id,
        instrument_id,
        base_quantity: config.base_quantity,
        spread_ticks: config.spread_ticks,
        inventory_target: config.inventory_target,
        wake_interval_ns: config.wake_interval_ns,
        seed: config.seed,
        max_intents_per_wake: config.max_intents_per_wake,
    });
    // An agent's orders live two wakes, so it never has many; its reports
    // arrive exactly once from the host. Small windows keep the snapshot
    // the runtime checkpoints every tick small.
    let mut snapshot = ExecutionSnapshot::empty(ExecutionConfig {
        max_orders: AGENT_MAX_ORDERS,
        max_seen_reports: AGENT_MAX_SEEN_REPORTS,
        ..ExecutionConfig::default()
    });
    snapshot.next_id = config
        .participant_id
        .get()
        .checked_mul(AGENT_ID_STRIDE)
        .ok_or(RuntimeError::ArithmeticOverflow)?;
    let execution = QuarccExecutionEngine::restore(snapshot)
        .map_err(|error| RuntimeError::Agent(format!("execution: {error:?}")))?;
    Ok(ScheduledAgent {
        participant_id: config.participant_id,
        managed: ManagedAgent::new(policy, execution),
        adapter: BuntingExecutionAdapter::default(),
        next_wake: NextWake {
            logical_time: LogicalTimeNs::new(0),
        },
        instrument: instrument_id,
        retired: false,
        listing: config
            .venue_id
            .map(|venue| ListingKey::new(venue, instrument_id)),
    })
}

fn built_in_actor(participant_id: ParticipantId) -> Result<VerifiedActor, RuntimeError> {
    VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(participant_id.get()),
        role: ActorRole::BuiltInAgent,
        participant_id: Some(UnsignedDecimalString::new(participant_id.get())),
        team_id: None,
    })
    .map_err(|error| RuntimeError::Host(format!("built-in identity: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_engine::{ListingDefinition, ParticipantDefinition, ScenarioDefinition};
    use bunting_market_types::CurrencyId;
    use bunting_market_types::{
        IterationId, ListingKey, MoneyMinor, PriceBounds, ScenarioId, ScenarioVersion, VenueId,
    };
    use bunting_risk_engine::RiskLimits;
    use std::collections::BTreeMap;

    #[derive(Clone)]
    struct MemoryHost {
        state: RunState,
        roles: Vec<ActorRole>,
        /// Events other participants committed, not yet taken.
        elsewhere: Vec<EventEnvelope>,
        /// Every committed command's events, by command ID.
        committed: BTreeMap<bunting_market_types::CommandId, Vec<EventEnvelope>>,
        checkpoints: Vec<RuntimeSnapshot>,
        /// Commits allowed before the host "crashes".
        crash_after: Option<usize>,
    }

    impl RuntimeHost for MemoryHost {
        fn read_state<T>(
            &self,
            run_id: RunId,
            read: impl FnOnce(&RunState) -> T,
        ) -> Result<T, RuntimeError> {
            (self.state.run_id() == run_id)
                .then(|| read(&self.state))
                .ok_or_else(|| RuntimeError::Host("unknown run".to_owned()))
        }

        fn commit(
            &mut self,
            actor: &VerifiedActor,
            command: &bunting_market_events::Command,
        ) -> Result<Vec<EventEnvelope>, RuntimeError> {
            if let Some(recorded) = self.committed.get(&command.command_id) {
                return Ok(recorded.clone());
            }
            match &mut self.crash_after {
                Some(0) => return Err(RuntimeError::Host("crashed".to_owned())),
                Some(remaining) => *remaining -= 1,
                None => {}
            }
            self.roles.push(actor.identity().role);
            let events = self
                .state
                .apply(command)
                .map(|applied| applied.events)
                .map_err(|error| RuntimeError::Host(format!("transition: {error:?}")))?;
            self.committed.insert(command.command_id, events.clone());
            Ok(events)
        }

        fn checkpoint(&mut self, snapshot: &RuntimeSnapshot) -> Result<(), RuntimeError> {
            self.checkpoints.push(snapshot.clone());
            Ok(())
        }

        fn take_committed(&mut self) -> Result<Vec<EventEnvelope>, RuntimeError> {
            Ok(std::mem::take(&mut self.elsewhere))
        }
    }

    /// A team trading with the built-in agent from outside the runtime.
    const TEAM: u128 = 20;

    fn fixture() -> Result<(RuntimeConfig, MemoryHost), RuntimeError> {
        let instrument_id = InstrumentId::new(1);
        let participant_id = ParticipantId::new(10);
        let listing = ListingDefinition::new(
            ListingKey::new(VenueId::new(1), instrument_id),
            "BNT".to_owned(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
                .map_err(|_| RuntimeError::InvalidMarket)?,
        )
        .map_err(|_| RuntimeError::InvalidMarket)?;
        let funded = |id: ParticipantId| {
            ParticipantDefinition::new(
                id,
                true,
                RiskLimits::new(
                    QuantityLots::new(100),
                    QuantityLots::new(1_000),
                    QuantityLots::new(10_000),
                ),
                BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(1_000_000))]),
                BTreeMap::from([(instrument_id, QuantityLots::new(100))]),
            )
        };
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [bunting_engine::InstrumentDefinition::new(
                InstrumentId::new(1),
                "BNT",
                CurrencyId::new(1),
                bunting_engine::InstrumentKind::Equity,
            )
            .with_opening_mark(PriceTicks::new(100))],
            [listing],
            [funded(participant_id), funded(ParticipantId::new(TEAM))],
        )
        .map_err(|_| RuntimeError::InvalidMarket)?;
        let state = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|error| RuntimeError::Host(error.to_string()))?;
        let config = RuntimeConfig {
            run_id: RunId::new(1),
            instrument_id,
            fundamental_price: PriceTicks::new(100),
            remaining_parent_quantity: QuantityLots::new(1_000),
            max_actions_per_tick: 16,
            fundamental_path: None,
            other_instruments: Vec::new(),
            agents: vec![RuntimeAgentConfig {
                kind: PolicyKind::StaticLiquidityProvider,
                participant_id,
                base_quantity: QuantityLots::new(5),
                spread_ticks: 2,
                inventory_target: QuantityLots::new(0),
                wake_interval_ns: 1_000_000_000,
                seed: 42,
                max_intents_per_wake: 4,
                instrument_id: None,
                venue_id: None,
            }],
        };
        Ok((
            config,
            MemoryHost {
                state,
                roles: Vec::new(),
                elsewhere: Vec::new(),
                committed: BTreeMap::new(),
                checkpoints: Vec::new(),
                crash_after: None,
            },
        ))
    }

    #[test]
    fn built_in_agents_commit_as_authenticated_participants() -> Result<(), RuntimeError> {
        let (config, mut host) = fixture()?;
        let mut runtime = DeterministicRuntime::new(config)?;
        assert_eq!(runtime.advance(&mut host)?, 2);
        assert_eq!(host.roles, vec![ActorRole::BuiltInAgent; 2]);
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let (bids, asks) = host
            .state
            .visible_levels(key)
            .map_err(|_| RuntimeError::InvalidMarket)?;
        assert_eq!(bids, vec![(PriceTicks::new(98), QuantityLots::new(5))]);
        assert_eq!(asks, vec![(PriceTicks::new(102), QuantityLots::new(5))]);
        Ok(())
    }

    #[test]
    fn snapshot_restore_preserves_deterministic_schedule() -> Result<(), RuntimeError> {
        let (config, mut host) = fixture()?;
        let mut uninterrupted = DeterministicRuntime::new(config)?;
        uninterrupted.advance(&mut host)?;
        let mut resumed = DeterministicRuntime::restore(uninterrupted.snapshot())?;
        let mut resumed_host = host.clone();
        uninterrupted.advance(&mut host)?;
        resumed.advance(&mut resumed_host)?;
        assert_eq!(
            host.state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?,
            resumed_host
                .state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?
        );
        assert_eq!(uninterrupted.snapshot(), resumed.snapshot());
        Ok(())
    }

    fn walking_config() -> Result<(RuntimeConfig, MemoryHost), RuntimeError> {
        let (mut config, host) = fixture()?;
        config.agents[0].kind = PolicyKind::FundamentalLiquidityProvider;
        config.fundamental_path = Some(FundamentalPath::RandomWalk {
            seed: 9,
            step_ns: 1_000_000_000,
            annual_volatility_bps: 60_000,
            trading_days_per_year: 252,
            run_ns_per_day: 10_000_000_000,
            regimes: vec![VolatilityRegime {
                from_ns: 8_000_000_000,
                annual_volatility_bps: 90_000,
            }],
            min_price_ticks: 20,
            max_price_ticks: 500,
        });
        Ok((config, host))
    }

    /// BNT on venue 1; QRC on venues 1 and 2. One provider per listing.
    fn two_listing_fixture() -> Result<(RuntimeConfig, MemoryHost), RuntimeError> {
        let bounds = || {
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
                .map_err(|_| RuntimeError::InvalidMarket)
        };
        let listing = |venue: u128, instrument: u128, symbol: &str| {
            ListingDefinition::new(
                ListingKey::new(VenueId::new(venue), InstrumentId::new(instrument)),
                symbol.to_owned(),
                bounds()?,
            )
            .map_err(|_| RuntimeError::InvalidMarket)
        };
        let funded = |id: u128| {
            ParticipantDefinition::new(
                ParticipantId::new(id),
                true,
                RiskLimits::new(
                    QuantityLots::new(100),
                    QuantityLots::new(1_000),
                    QuantityLots::new(10_000),
                ),
                BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(10_000_000))]),
                BTreeMap::from([
                    (InstrumentId::new(1), QuantityLots::new(100)),
                    (InstrumentId::new(2), QuantityLots::new(100)),
                ]),
            )
        };
        let instrument = |id: u128, symbol: &str, mark: i64| {
            bunting_engine::InstrumentDefinition::new(
                InstrumentId::new(id),
                symbol,
                CurrencyId::new(1),
                bunting_engine::InstrumentKind::Equity,
            )
            .with_opening_mark(PriceTicks::new(mark))
        };
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [instrument(1, "BNT", 100), instrument(2, "QRC", 50)],
            [
                listing(1, 1, "BNT")?,
                listing(1, 2, "QRC")?,
                listing(2, 2, "QRC")?,
            ],
            [funded(10), funded(11), funded(12)],
        )
        .map_err(|_| RuntimeError::InvalidMarket)?;
        let state = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|error| RuntimeError::Host(error.to_string()))?;
        let agent =
            |participant: u128, instrument: Option<u128>, venue: Option<u128>| RuntimeAgentConfig {
                kind: PolicyKind::FundamentalLiquidityProvider,
                participant_id: ParticipantId::new(participant),
                base_quantity: QuantityLots::new(5),
                spread_ticks: 2,
                inventory_target: QuantityLots::new(0),
                wake_interval_ns: 1_000_000_000,
                seed: u64::try_from(participant).unwrap_or(0),
                max_intents_per_wake: 4,
                instrument_id: instrument.map(InstrumentId::new),
                venue_id: venue.map(VenueId::new),
            };
        let config = RuntimeConfig {
            run_id: RunId::new(1),
            instrument_id: InstrumentId::new(1),
            fundamental_price: PriceTicks::new(100),
            remaining_parent_quantity: QuantityLots::new(1_000),
            max_actions_per_tick: 16,
            fundamental_path: None,
            other_instruments: vec![InstrumentFundamental {
                instrument_id: InstrumentId::new(2),
                fundamental_price: PriceTicks::new(50),
                fundamental_path: None,
            }],
            agents: vec![
                agent(10, None, None),
                agent(11, Some(2), Some(1)),
                agent(12, Some(2), Some(2)),
            ],
        };
        Ok((
            config,
            MemoryHost {
                state,
                roles: Vec::new(),
                elsewhere: Vec::new(),
                committed: BTreeMap::new(),
                checkpoints: Vec::new(),
                crash_after: None,
            },
        ))
    }

    #[test]
    fn agents_on_two_listings_each_quote_their_own_book() -> Result<(), RuntimeError> {
        let (config, mut host) = two_listing_fixture()?;
        let mut runtime = DeterministicRuntime::new(config)?;
        assert_eq!(runtime.advance(&mut host)?, 6);
        let book = |venue: u128, instrument: u128| {
            host.state
                .visible_levels(ListingKey::new(
                    VenueId::new(venue),
                    InstrumentId::new(instrument),
                ))
                .map_err(|_| RuntimeError::InvalidMarket)
        };
        let ticks = |levels: &bunting_engine::VisibleLevels| -> Vec<i64> {
            levels.iter().map(|(price, _)| price.get()).collect()
        };
        // Each book is centred on its own instrument's fundamental, and the
        // two QRC listings hold separate quotes of their own providers.
        let (bnt_bids, bnt_asks) = book(1, 1)?;
        assert_eq!((ticks(&bnt_bids), ticks(&bnt_asks)), (vec![98], vec![102]));
        for venue in [1, 2] {
            let (bids, asks) = book(venue, 2)?;
            assert_eq!((ticks(&bids), ticks(&asks)), (vec![48], vec![52]));
        }
        // A restored runtime continues identically.
        let mut resumed = DeterministicRuntime::restore(runtime.snapshot())?;
        let mut resumed_host = host.clone();
        runtime.advance(&mut host)?;
        resumed.advance(&mut resumed_host)?;
        assert_eq!(runtime.snapshot(), resumed.snapshot());
        assert_eq!(
            host.state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?,
            resumed_host
                .state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?
        );
        Ok(())
    }

    /// A trade claiming far more lots than agent 11's order holds: a report
    /// no venue could send, which the agent's execution state refuses.
    fn impossible_fill(runtime: &DeterministicRuntime) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let order = runtime.snapshot().agents[1]
            .managed
            .execution
            .orders
            .keys()
            .next()
            .copied()
            .ok_or(RuntimeError::InvalidSnapshot)?;
        Ok(vec![EventEnvelope {
            schema_version: 1,
            run_id: RunId::new(1),
            event_id: bunting_market_types::EventId::new(9_000_001),
            sequence: bunting_market_types::EventSequence::new(9_000_001),
            logical_time: LogicalTimeNs::new(0),
            actor: ParticipantId::new(12),
            command_id: bunting_market_types::CommandId::new(9_000_001),
            correlation_id: CorrelationId::new(1),
            causation_sequence: None,
            payload: EventPayload::TradeExecuted {
                instrument_id: InstrumentId::new(2),
                listing_key: Some(ListingKey::new(VenueId::new(1), InstrumentId::new(2))),
                maker_order_id: bunting_market_types::OrderId::new(order.get()),
                taker_order_id: bunting_market_types::OrderId::new(1),
                buyer_id: ParticipantId::new(11),
                seller_id: ParticipantId::new(12),
                price: PriceTicks::new(50),
                quantity: QuantityLots::new(1_000_000),
                buyer_fee: bunting_market_types::MoneyMinor::new(0),
                seller_fee: bunting_market_types::MoneyMinor::new(0),
            },
        }])
    }

    #[test]
    fn an_agent_that_errors_is_retired_and_the_market_carries_on() -> Result<(), RuntimeError> {
        let (config, mut host) = two_listing_fixture()?;
        let mut runtime = DeterministicRuntime::new(config)?;
        runtime.advance(&mut host)?;
        let open = |runtime: &DeterministicRuntime, index: usize| {
            runtime.snapshot().agents[index]
                .managed
                .execution
                .orders
                .values()
                .filter(|order| order.lifecycle.is_open())
                .count()
        };
        assert_eq!(open(&runtime, 1), 2, "agent 11 quotes both sides");
        host.elsewhere = impossible_fill(&runtime)?;
        // The error is the agent's own: the runtime carries on, retires the
        // agent and cancels its resting orders through the sequencer.
        runtime.advance(&mut host)?;
        assert_eq!(runtime.snapshot().retired, vec![ParticipantId::new(11)]);
        let (bids, asks) = host
            .state
            .visible_levels(ListingKey::new(VenueId::new(1), InstrumentId::new(2)))
            .map_err(|_| RuntimeError::InvalidMarket)?;
        assert!(bids.is_empty() && asks.is_empty(), "its quotes were pulled");
        // The others keep quoting for many wakes; the retired one never returns.
        for _ in 0..4 {
            runtime.advance(&mut host)?;
        }
        let book = |venue: u128, instrument: u128| {
            host.state
                .visible_levels(ListingKey::new(
                    VenueId::new(venue),
                    InstrumentId::new(instrument),
                ))
                .map_err(|_| RuntimeError::InvalidMarket)
        };
        assert!(!book(1, 1)?.0.is_empty(), "BNT still quoted");
        assert!(!book(2, 2)?.0.is_empty(), "QRC on venue 2 still quoted");
        assert!(book(1, 2)?.0.is_empty(), "the retired agent stays gone");
        assert_eq!(runtime.snapshot().retired.len(), 1);
        Ok(())
    }

    #[test]
    fn a_retired_agent_survives_a_snapshot_restore() -> Result<(), RuntimeError> {
        let (config, mut host) = two_listing_fixture()?;
        let mut runtime = DeterministicRuntime::new(config)?;
        runtime.advance(&mut host)?;
        host.elsewhere = impossible_fill(&runtime)?;
        runtime.advance(&mut host)?;
        let mut resumed = DeterministicRuntime::restore(runtime.snapshot())?;
        assert_eq!(resumed.snapshot(), runtime.snapshot());
        let mut resumed_host = host.clone();
        for _ in 0..3 {
            runtime.advance(&mut host)?;
            resumed.advance(&mut resumed_host)?;
        }
        assert_eq!(resumed.snapshot(), runtime.snapshot());
        assert_eq!(
            host.state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?,
            resumed_host
                .state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?
        );
        Ok(())
    }

    #[test]
    fn an_agent_on_another_instrument_must_name_a_venue() -> Result<(), RuntimeError> {
        let (mut config, _) = two_listing_fixture()?;
        config.agents[1].venue_id = None;
        assert_eq!(config.validate(), Err(RuntimeError::InvalidConfig));
        Ok(())
    }

    #[test]
    fn a_walking_fundamental_moves_the_quotes_and_survives_a_restart() -> Result<(), RuntimeError> {
        let (config, mut host) = walking_config()?;
        let mut uninterrupted = DeterministicRuntime::new(config)?;
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let mut best_bids = BTreeSet::new();
        let mut resumed = None;
        let mut resumed_host = host.clone();
        for round in 0..12 {
            uninterrupted.advance(&mut host)?;
            if round == 4 {
                let mut copy = DeterministicRuntime::restore(uninterrupted.snapshot())?;
                resumed_host = host.clone();
                copy.advance(&mut resumed_host)?;
                resumed = Some(copy);
                uninterrupted.advance(&mut host)?;
                assert_eq!(
                    uninterrupted.snapshot(),
                    resumed
                        .as_ref()
                        .map(DeterministicRuntime::snapshot)
                        .ok_or(RuntimeError::InvalidSnapshot)?
                );
                continue;
            }
            if let Some(copy) = resumed.as_mut() {
                copy.advance(&mut resumed_host)?;
                assert_eq!(uninterrupted.snapshot(), copy.snapshot());
            }
            let (bids, _) = host
                .state
                .visible_levels(key)
                .map_err(|_| RuntimeError::InvalidMarket)?;
            best_bids.extend(bids.iter().map(|(price, _)| price.get()));
        }
        assert!(best_bids.len() > 1, "quotes never moved: {best_bids:?}");
        assert_eq!(
            host.state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?,
            resumed_host
                .state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)?
        );
        Ok(())
    }

    #[test]
    fn agents_learn_of_fills_caused_by_other_participants() -> Result<(), RuntimeError> {
        use bunting_market_events::{Command, CommandPayload, OrderKind, Side, SubmitOrder};
        use bunting_market_types::{CommandId, OrderId};
        let (config, mut host) = fixture()?;
        let mut runtime = DeterministicRuntime::new(config)?;
        runtime.advance(&mut host)?;
        let position = |runtime: &DeterministicRuntime| {
            runtime.snapshot().agents[0]
                .managed
                .execution
                .positions
                .get(&InstrumentId::new(1))
                .map_or(0, |position| position.quantity.get())
        };
        assert_eq!(position(&runtime), 0);
        // The team lifts the agent's whole 102 offer; the runtime commits
        // nothing itself and learns of the fill only from the host.
        let lift = Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(900),
            correlation_id: CorrelationId::new(900),
            logical_time: LogicalTimeNs::new(10),
            expected_sequence: host.state.sequence(),
            actor: ParticipantId::new(TEAM),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(900),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(TEAM),
                side: Side::Buy,
                quantity: QuantityLots::new(5),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(102),
                },
                anonymous: false,
            }),
        };
        let events = host
            .state
            .apply(&lift)
            .map_err(|error| RuntimeError::Host(format!("{error:?}")))?
            .events;
        assert!(
            events
                .iter()
                .any(|event| matches!(event.payload, EventPayload::TradeExecuted { .. }))
        );
        host.elsewhere = events;
        runtime.advance(&mut host)?;
        assert_eq!(position(&runtime), -5, "the agent knows it sold 5");
        assert_eq!(runtime.snapshot().last_trade, PriceTicks::new(102));
        Ok(())
    }

    #[test]
    fn a_runtime_resumed_mid_tick_commits_each_action_exactly_once() -> Result<(), RuntimeError> {
        let (config, host) = fixture()?;
        let mut uninterrupted = DeterministicRuntime::new(config.clone())?;
        let mut reference = host.clone();
        for _ in 0..3 {
            uninterrupted.advance(&mut reference)?;
        }

        // Tick 2 crashes after the first of its two commits is durable.
        let mut runtime = DeterministicRuntime::new(config)?;
        let mut store = host;
        runtime.advance(&mut store)?;
        store.crash_after = Some(1);
        assert!(runtime.advance(&mut store).is_err());
        drop(runtime);

        // Restart from the last checkpoint: tick 2's decisions, both still
        // pending. The first resolves to its recorded events.
        store.crash_after = None;
        let checkpoint = store
            .checkpoints
            .last()
            .cloned()
            .ok_or(RuntimeError::InvalidSnapshot)?;
        assert_eq!(checkpoint.pending.len(), 2);
        let restored: RuntimeSnapshot = serde_json::from_str(
            &serde_json::to_string(&checkpoint).map_err(|_| RuntimeError::InvalidSnapshot)?,
        )
        .map_err(|_| RuntimeError::InvalidSnapshot)?;
        let mut resumed = DeterministicRuntime::restore(restored)?;
        assert_eq!(resumed.advance(&mut store)?, 2);
        resumed.advance(&mut store)?;

        let hash = |host: &MemoryHost| {
            host.state
                .state_hash()
                .map_err(|_| RuntimeError::InvalidSnapshot)
        };
        assert_eq!(hash(&store)?, hash(&reference)?);
        assert_eq!(store.roles.len(), reference.roles.len(), "no double commit");
        assert_eq!(resumed.snapshot(), uninterrupted.snapshot());
        Ok(())
    }
}
