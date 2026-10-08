//! Deny-by-default competition projections and versioned policy identities.

use crate::{ApplicationError, VerifiedActor};
use bunting_api_contract::ActorRole;
use bunting_engine::{
    RunState,
    simulation::{NewsItem, RunLifecycle, ScoreEntry, TenderState},
};
use bunting_market_events::NewsAudience;
use bunting_market_types::{
    CurrencyId, EventSequence, InstrumentId, LogicalTimeNs, MoneyMinor, ParticipantId, PriceTicks,
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
        commission: "bunting.commission.listing-per-lot.v1".to_owned(),
        news: "bunting.news.audience.v1".to_owned(),
        tender: "bunting.tender.house-settled-fixed-price.v1".to_owned(),
        risk: "bunting.risk.scenario-limits.v1".to_owned(),
        fine: "bunting.fine.explicit-cash.v1".to_owned(),
        score: "bunting.score.nlv-last-trade-rank.v2".to_owned(),
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
    pub open_buy: QuantityLots,
    pub open_sell: QuantityLots,
    pub realized_pnl: MoneyMinor,
    pub unrealized_pnl: MoneyMinor,
    pub cost_basis: MoneyMinor,
    pub mark: Option<PriceTicks>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CashView {
    pub currency_id: CurrencyId,
    pub balance: MoneyMinor,
    pub reserved: MoneyMinor,
    pub fees: MoneyMinor,
}

/// One participant's account, projected entirely from the authoritative ledger.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountView {
    pub participant_id: ParticipantId,
    pub committed_sequence: EventSequence,
    pub cash: Vec<CashView>,
    pub holdings: Vec<HoldingView>,
    pub reporting_currency: CurrencyId,
    pub net_liquidation_value: MoneyMinor,
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
    if !state.participants().contains_key(&participant) {
        return Err(ApplicationError::Unauthorized);
    }
    let ledger = state.ledger();
    let cash = ledger
        .cash_balances(participant)
        .map(|(currency_id, balance)| CashView {
            currency_id,
            balance: balance.balance,
            reserved: balance.reserved,
            fees: balance.fees,
        })
        .collect();
    let holdings = state
        .instruments()
        .keys()
        .map(|instrument_id| {
            let position = ledger.position(participant, *instrument_id);
            Ok(HoldingView {
                instrument_id: *instrument_id,
                position: position.quantity,
                reserved: position.reserved,
                open_buy: position.open_buy,
                open_sell: position.open_sell,
                realized_pnl: position.realized_pnl,
                unrealized_pnl: ledger
                    .unrealized_pnl(participant, *instrument_id)
                    .map_err(|_| ApplicationError::InvalidIdentity)?,
                cost_basis: position.cost_basis,
                mark: ledger.mark(*instrument_id),
            })
        })
        .collect::<Result<Vec<_>, ApplicationError>>()?;
    Ok(AccountView {
        participant_id: participant,
        committed_sequence: state.sequence(),
        cash,
        holdings,
        reporting_currency: state.reporting_currency(),
        net_liquidation_value: state
            .net_liquidation_value(participant)
            .map_err(|_| ApplicationError::InvalidIdentity)?,
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
        NewsAudience::Role(role) => actor.identity().role.as_str().eq_ignore_ascii_case(role),
    }
}
