//! Deny-by-default competition projections and versioned policy identities.

use crate::{ApplicationError, VerifiedActor};
use bunting_api_contract::ActorRole;
use bunting_engine::{
    RunState,
    simulation::{NewsItem, RunLifecycle, ScoreEntry, TenderState},
};
use bunting_market_events::NewsAudience;
use bunting_market_types::{
    CurrencyId, EventSequence, InstrumentId, LogicalTimeNs, MoneyMinor, ParticipantId,
    QuantityLots, RunId, ScenarioId, ScenarioVersion,
};
use bunting_risk_engine::RiskLimits;
use serde::{Deserialize, Serialize};

/// Version identities for every competition MVC formula boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionPolicies {
    pub pnl: String,
    pub commission: String,
    pub news: String,
    pub tender: String,
    pub risk: String,
    pub fine: String,
    pub score: String,
}

/// Current Bunting-native policy set. These identities make no RIT-equivalence claim.
#[must_use]
pub fn competition_policies() -> CompetitionPolicies {
    CompetitionPolicies {
        pnl: "bunting.pnl.v1".to_owned(),
        commission: "bunting.commission.zero.v1".to_owned(),
        news: "bunting.news.audience.v1".to_owned(),
        tender: "bunting.tender.targeted-fixed-price.v1".to_owned(),
        risk: "bunting.risk.scenario-limits.v1".to_owned(),
        fine: "bunting.fine.explicit-cash.v1".to_owned(),
        score: "bunting.score.nlv-rank.v1".to_owned(),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingView {
    pub instrument_id: InstrumentId,
    pub symbol: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryView {
    pub run_id: RunId,
    pub scenario_id: ScenarioId,
    pub scenario_version: ScenarioVersion,
    pub committed_sequence: EventSequence,
    pub lifecycle: RunLifecycle,
    pub logical_time: LogicalTimeNs,
    pub listings: Vec<ListingView>,
    pub policies: CompetitionPolicies,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HoldingView {
    pub instrument_id: InstrumentId,
    pub position: QuantityLots,
    pub reserved: QuantityLots,
    pub realized_pnl: MoneyMinor,
    pub unrealized_pnl: MoneyMinor,
    pub cost_basis: MoneyMinor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CashView {
    pub currency_id: CurrencyId,
    pub settled: MoneyMinor,
    pub reserved: MoneyMinor,
    pub accrued: MoneyMinor,
    pub scheduled: MoneyMinor,
    pub fees: MoneyMinor,
    pub margin: MoneyMinor,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountView {
    pub participant_id: ParticipantId,
    pub committed_sequence: EventSequence,
    pub order_cash: MoneyMinor,
    pub order_reserved_cash: MoneyMinor,
    pub cash: Vec<CashView>,
    pub holdings: Vec<HoldingView>,
    pub net_liquidation_value: Vec<(CurrencyId, MoneyMinor)>,
    pub policies: CompetitionPolicies,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewsTenderView {
    pub committed_sequence: EventSequence,
    pub news: Vec<NewsItem>,
    pub tenders: Vec<TenderState>,
    pub policies: CompetitionPolicies,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskScoreView {
    pub participant_id: ParticipantId,
    pub committed_sequence: EventSequence,
    pub limits: RiskLimits,
    pub latest_score: Option<ScoreEntry>,
    pub policies: CompetitionPolicies,
}

#[must_use]
pub fn discovery(state: &RunState) -> DiscoveryView {
    DiscoveryView {
        run_id: state.run_id(),
        scenario_id: state.scenario_id(),
        scenario_version: state.scenario_version(),
        committed_sequence: state.sequence(),
        lifecycle: state.simulation().lifecycle,
        logical_time: state.simulation().clock.now,
        listings: state
            .listings()
            .values()
            .map(|listing| ListingView {
                instrument_id: listing.definition().key().instrument_id,
                symbol: listing.definition().symbol().to_owned(),
            })
            .collect(),
        policies: competition_policies(),
    }
}

/// Projects only the authenticated participant's private account.
pub fn account(state: &RunState, actor: &VerifiedActor) -> Result<AccountView, ApplicationError> {
    let participant = actor
        .participant_id()
        .ok_or(ApplicationError::Unauthorized)?;
    let base = state
        .accounts()
        .iter()
        .find_map(|(id, account)| (*id == participant).then_some(*account))
        .ok_or(ApplicationError::Unauthorized)?;
    let currencies = state
        .simulation()
        .instruments
        .values()
        .map(|instrument| instrument.settlement_currency)
        .collect::<std::collections::BTreeSet<_>>();
    let cash = currencies
        .iter()
        .map(|currency_id| {
            let value = state
                .simulation()
                .portfolio_ledger
                .balance(participant, *currency_id);
            CashView {
                currency_id: *currency_id,
                settled: value.settled,
                reserved: value.reserved,
                accrued: value.accrued,
                scheduled: value.scheduled,
                fees: value.fees,
                margin: value.margin,
            }
        })
        .collect();
    let holdings = state
        .listings()
        .values()
        .map(|listing| listing.definition().key().instrument_id)
        .map(|instrument_id| {
            let order_holding = state
                .holdings()
                .iter()
                .find_map(|(owner, instrument, holding)| {
                    (*owner == participant && *instrument == instrument_id).then_some(*holding)
                })
                .unwrap_or_default();
            let portfolio = state
                .simulation()
                .portfolio_ledger
                .position(participant, instrument_id);
            HoldingView {
                instrument_id,
                position: order_holding.position,
                reserved: order_holding.reserved_inventory,
                realized_pnl: portfolio.realized_pnl,
                unrealized_pnl: portfolio.unrealized_pnl,
                cost_basis: portfolio.cost_basis,
            }
        })
        .collect();
    let net_liquidation_value = currencies
        .into_iter()
        .map(|currency| {
            state
                .simulation()
                .portfolio_ledger
                .net_liquidation_value(participant, currency)
                .map(|value| (currency, value))
                .map_err(|_| ApplicationError::InvalidIdentity)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AccountView {
        participant_id: participant,
        committed_sequence: state.sequence(),
        order_cash: base.cash,
        order_reserved_cash: base.reserved_cash,
        cash,
        holdings,
        net_liquidation_value,
        policies: competition_policies(),
    })
}

/// Applies public/private audience rules before returning news and tenders.
pub fn news_tenders(
    state: &RunState,
    actor: &VerifiedActor,
) -> Result<NewsTenderView, ApplicationError> {
    let news = state
        .simulation()
        .news
        .iter()
        .filter(|item| news_visible(&item.audience, actor))
        .cloned()
        .collect();
    let tenders = state
        .simulation()
        .tenders
        .values()
        .filter(|tender| {
            actor.identity().role == ActorRole::Administrator
                || actor.identity().role == ActorRole::Instructor
                || actor.participant_id() == Some(tender.participant_id)
        })
        .cloned()
        .collect();
    Ok(NewsTenderView {
        committed_sequence: state.sequence(),
        news,
        tenders,
        policies: competition_policies(),
    })
}

/// Projects scenario risk limits and the participant's latest frozen score only.
pub fn risk_score(
    state: &RunState,
    actor: &VerifiedActor,
) -> Result<RiskScoreView, ApplicationError> {
    let participant = actor
        .participant_id()
        .ok_or(ApplicationError::Unauthorized)?;
    let limits = state
        .participants()
        .get(&participant)
        .map(bunting_engine::ParticipantDefinition::limits)
        .ok_or(ApplicationError::Unauthorized)?;
    let latest_score = state
        .simulation()
        .reports
        .last()
        .and_then(|report| {
            report
                .entries
                .iter()
                .find(|entry| entry.participant_id == participant)
        })
        .copied();
    Ok(RiskScoreView {
        participant_id: participant,
        committed_sequence: state.sequence(),
        limits,
        latest_score,
        policies: competition_policies(),
    })
}

fn news_visible(audience: &NewsAudience, actor: &VerifiedActor) -> bool {
    match audience {
        NewsAudience::Public => true,
        NewsAudience::Participant(participant) => actor.participant_id() == Some(*participant),
        NewsAudience::Team(team) => actor
            .identity()
            .team_id
            .as_ref()
            .is_some_and(|id| id.get() == *team),
        NewsAudience::Role(role) => {
            format!("{:?}", actor.identity().role).eq_ignore_ascii_case(role)
        }
    }
}
