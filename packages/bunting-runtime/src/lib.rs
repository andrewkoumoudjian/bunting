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
use bunting_market_events::{EventEnvelope, EventPayload};
use bunting_market_types::{
    CorrelationId, InstrumentId, LogicalTimeNs, ParticipantId, PriceTicks, QuantityLots, RunId,
};
use quarcc_bunting_adapter::{BuntingCommandContext, BuntingExecutionAdapter};
use quarcc_execution_engine::{
    ExecutionAction, ExecutionConfig, ExecutionSnapshot, QuarccExecutionEngine,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, VecDeque};
use std::fmt;

const AGENT_ID_STRIDE: u128 = 1_000_000;

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
}

impl RuntimeConfig {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.run_id.get() == 0
            || self.instrument_id.get() == 0
            || self.fundamental_price.get() <= 1
            || self.remaining_parent_quantity.get() < 0
            || self.max_actions_per_tick == 0
        {
            return Err(RuntimeError::InvalidConfig);
        }
        let mut participants = BTreeSet::new();
        for agent in &self.agents {
            if agent.participant_id.get() == 0
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeSnapshot {
    pub version: u16,
    pub config: RuntimeConfig,
    pub logical_time: LogicalTimeNs,
    pub previous_trade: PriceTicks,
    pub last_trade: PriceTicks,
    pub agents: Vec<RuntimeAgentSnapshot>,
}

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
    fn state(&self, run_id: RunId) -> Result<RunState, RuntimeError>;

    /// Commits through the application's authenticated transaction boundary and
    /// returns only committed events.
    fn commit(
        &mut self,
        actor: &VerifiedActor,
        command: &bunting_market_events::Command,
    ) -> Result<Vec<EventEnvelope>, RuntimeError>;
}

struct ScheduledAgent {
    participant_id: ParticipantId,
    managed: ManagedAgent<BuiltInPolicy>,
    adapter: BuntingExecutionAdapter,
    next_wake: NextWake,
}

pub struct DeterministicRuntime {
    config: RuntimeConfig,
    agents: Vec<ScheduledAgent>,
    logical_time: LogicalTimeNs,
    previous_trade: PriceTicks,
    last_trade: PriceTicks,
}

impl DeterministicRuntime {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        config.validate()?;
        let agents = config
            .agents
            .iter()
            .map(|agent| scheduled_agent(agent, config.instrument_id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            previous_trade: config.fundamental_price,
            last_trade: config.fundamental_price,
            config,
            agents,
            logical_time: LogicalTimeNs::new(0),
        })
    }

    pub fn restore(snapshot: RuntimeSnapshot) -> Result<Self, RuntimeError> {
        if snapshot.version != 1 || snapshot.agents.len() != snapshot.config.agents.len() {
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
                    || saved.managed.policy.config.instrument_id != snapshot.config.instrument_id
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
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            config: snapshot.config,
            agents,
            logical_time: snapshot.logical_time,
            previous_trade: snapshot.previous_trade,
            last_trade: snapshot.last_trade,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            version: 1,
            config: self.config.clone(),
            logical_time: self.logical_time,
            previous_trade: self.previous_trade,
            last_trade: self.last_trade,
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
        }
    }

    pub fn advance<H: RuntimeHost>(&mut self, host: &mut H) -> Result<usize, RuntimeError> {
        let Some(next_time) = self
            .agents
            .iter()
            .map(|agent| agent.next_wake.logical_time)
            .min()
        else {
            return Ok(0);
        };
        self.logical_time = LogicalTimeNs::new(
            next_time
                .get()
                .max(self.logical_time.get().saturating_add(1)),
        );
        let state = host.state(self.config.run_id)?;
        let observation = self.observation(&state)?;
        let due = self
            .agents
            .iter()
            .enumerate()
            .filter_map(|(index, agent)| {
                (agent.next_wake.logical_time <= self.logical_time).then_some(index)
            })
            .collect::<Vec<_>>();
        let mut pending = VecDeque::new();
        for index in due {
            let position = self.agents[index]
                .managed
                .snapshot()
                .execution
                .positions
                .get(&self.config.instrument_id)
                .map_or_else(|| QuantityLots::new(0), |position| position.quantity);
            let (next_wake, actions) = self.agents[index]
                .managed
                .on_wake(
                    &AgentContext {
                        logical_time: self.logical_time,
                        current_position: position,
                        remaining_parent_quantity: self.config.remaining_parent_quantity,
                    },
                    &observation,
                )
                .map_err(|error| RuntimeError::Agent(format!("wake: {error:?}")))?;
            self.agents[index].next_wake = next_wake;
            pending.extend(actions.into_iter().map(|action| (index, action)));
        }
        let mut processed = 0_usize;
        while let Some((index, action)) = pending.pop_front() {
            if processed >= self.config.max_actions_per_tick {
                return Err(RuntimeError::ActionBoundExceeded);
            }
            processed = processed.saturating_add(1);
            let state = host.state(self.config.run_id)?;
            self.logical_time = LogicalTimeNs::new(self.logical_time.get().saturating_add(1));
            let participant_id = self.agents[index].participant_id;
            let command = self.agents[index]
                .adapter
                .command_for_action(
                    &action,
                    &BuntingCommandContext {
                        run_id: self.config.run_id,
                        actor: participant_id,
                        expected_sequence: state.sequence(),
                        logical_time: self.logical_time,
                        correlation_id: CorrelationId::new(u128::from(self.logical_time.get())),
                    },
                )
                .map_err(|error| RuntimeError::Adapter(format!("command: {error:?}")))?;
            let actor = built_in_actor(participant_id)?;
            let events = host.commit(&actor, &command)?;
            self.observe_trades(&events);
            self.dispatch(&events, &mut pending)?;
        }
        Ok(processed)
    }

    fn observation(&self, state: &RunState) -> Result<AgentObservation, RuntimeError> {
        let key = state
            .listing_key_for_instrument(self.config.instrument_id)
            .map_err(|_| RuntimeError::InvalidMarket)?;
        let (bids, asks) = state
            .visible_levels(key)
            .map_err(|_| RuntimeError::InvalidMarket)?;
        let fallback_bid = self.config.fundamental_price.get().saturating_sub(1).max(1);
        let best_bid = bids
            .first()
            .map(|(price, _)| i64::try_from(*price).map_err(|_| RuntimeError::ArithmeticOverflow))
            .transpose()?
            .unwrap_or(fallback_bid);
        let best_ask = asks
            .first()
            .map(|(price, _)| i64::try_from(*price).map_err(|_| RuntimeError::ArithmeticOverflow))
            .transpose()?
            .unwrap_or_else(|| best_bid.saturating_add(2));
        let quantity = |levels: &Vec<(u128, u64)>| {
            levels.first().map_or(Ok(0), |(_, value)| {
                i64::try_from(*value).map_err(|_| RuntimeError::ArithmeticOverflow)
            })
        };
        Ok(AgentObservation {
            best_bid: PriceTicks::new(best_bid),
            best_ask: PriceTicks::new(best_ask.max(best_bid.saturating_add(1))),
            bid_quantity: QuantityLots::new(quantity(&bids)?),
            ask_quantity: QuantityLots::new(quantity(&asks)?),
            last_trade: self.last_trade,
            fundamental: self.config.fundamental_price,
            previous_trade: self.previous_trade,
            observed_volume: QuantityLots::new(quantity(&bids)?.saturating_add(quantity(&asks)?)),
            stress_bps: 0,
        })
    }

    fn observe_trades(&mut self, events: &[EventEnvelope]) {
        for event in events {
            if let EventPayload::TradeExecuted { price, .. } = event.payload {
                self.previous_trade = self.last_trade;
                self.last_trade = price;
            }
        }
    }

    fn dispatch(
        &mut self,
        events: &[EventEnvelope],
        pending: &mut VecDeque<(usize, ExecutionAction)>,
    ) -> Result<(), RuntimeError> {
        for (index, agent) in self.agents.iter_mut().enumerate() {
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
        }
        Ok(())
    }
}

fn scheduled_agent(
    config: &RuntimeAgentConfig,
    instrument_id: InstrumentId,
) -> Result<ScheduledAgent, RuntimeError> {
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
    let mut snapshot = ExecutionSnapshot::empty(ExecutionConfig::default());
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
    use bunting_market_types::{
        IterationId, ListingKey, MoneyMinor, PriceBounds, ScenarioId, ScenarioVersion, VenueId,
    };
    use bunting_risk_engine::RiskLimits;
    use std::collections::BTreeMap;

    #[derive(Clone)]
    struct MemoryHost {
        state: RunState,
        roles: Vec<ActorRole>,
    }

    impl RuntimeHost for MemoryHost {
        fn state(&self, run_id: RunId) -> Result<RunState, RuntimeError> {
            (self.state.run_id() == run_id)
                .then(|| self.state.clone())
                .ok_or_else(|| RuntimeError::Host("unknown run".to_owned()))
        }

        fn commit(
            &mut self,
            actor: &VerifiedActor,
            command: &bunting_market_events::Command,
        ) -> Result<Vec<EventEnvelope>, RuntimeError> {
            self.roles.push(actor.identity().role);
            let outcome = self
                .state
                .transition(command, None)
                .map_err(|error| RuntimeError::Host(format!("transition: {error:?}")))?;
            self.state = outcome.candidate;
            Ok(outcome.events)
        }
    }

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
        let participant = ParticipantDefinition::new(
            participant_id,
            true,
            RiskLimits {
                max_order_quantity: QuantityLots::new(100),
                max_open_order_quantity: QuantityLots::new(1_000),
                max_absolute_position: QuantityLots::new(10_000),
            },
            MoneyMinor::new(1_000_000),
            BTreeMap::from([(instrument_id, QuantityLots::new(100))]),
        );
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [listing],
            [participant],
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
            agents: vec![RuntimeAgentConfig {
                kind: PolicyKind::StaticLiquidityProvider,
                participant_id,
                base_quantity: QuantityLots::new(5),
                spread_ticks: 2,
                inventory_target: QuantityLots::new(0),
                wake_interval_ns: 1_000_000_000,
                seed: 42,
                max_intents_per_wake: 4,
            }],
        };
        Ok((
            config,
            MemoryHost {
                state,
                roles: Vec::new(),
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
        assert_eq!(bids, vec![(98, 5)]);
        assert_eq!(asks, vec![(102, 5)]);
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
}
