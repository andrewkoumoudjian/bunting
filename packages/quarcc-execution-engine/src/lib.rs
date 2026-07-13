#![forbid(unsafe_code)]
//! Portable, deterministic participant-side execution and reconciliation.

pub mod capabilities;
pub mod command;
pub mod compatibility;
pub mod config;
pub mod error;
pub mod event;
pub mod ids;
pub mod lifecycle;
pub mod market_data;
pub mod normalized_report;
pub mod order;
pub mod planner;
pub mod positions;
pub mod reconciliation;
pub mod risk;
pub mod snapshot;
pub mod strategy_signal;

pub use capabilities::ExecutionCapabilities;
pub use command::ExecutionIntent;
pub use config::ExecutionConfig;
pub use error::ExecutionError;
pub use event::ExecutionAction;
pub use market_data::MarketObservation;
pub use normalized_report::{NormalizedVenueReport, VenueReportKind};
pub use reconciliation::AuthoritativeVenueSnapshot;
pub use snapshot::ExecutionSnapshot;

use crate::ids::{ActionId, ClientOrderId, LocalOrderId, VenueOrderId};
use crate::lifecycle::OrderLifecycle;
use crate::order::ManagedOrder;
use crate::positions::PositionProjection;
use bunting_market_types::QuantityLots;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionActionBuffer {
    limit: usize,
    actions: Vec<ExecutionAction>,
}

impl ExecutionActionBuffer {
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            actions: Vec::new(),
        }
    }

    /// Appends an action without exceeding the configured bound.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::BufferFull`] when the buffer is at capacity.
    pub fn push(&mut self, action: ExecutionAction) -> Result<(), ExecutionError> {
        if self.actions.len() >= self.limit {
            return Err(ExecutionError::BufferFull { limit: self.limit });
        }
        self.actions.push(action);
        Ok(())
    }

    #[must_use]
    pub fn as_slice(&self) -> &[ExecutionAction] {
        &self.actions
    }

    #[must_use]
    pub fn into_vec(self) -> Vec<ExecutionAction> {
        self.actions
    }

    pub fn clear(&mut self) {
        self.actions.clear();
    }
}

pub trait ExecutionEngine {
    /// Applies a participant intent and emits the venue actions it requires.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid state transitions, risk failures, or a full output buffer.
    fn submit_intent(
        &mut self,
        intent: ExecutionIntent,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError>;

    /// Applies committed public market data to local execution state.
    ///
    /// # Errors
    ///
    /// Returns an error if the observation cannot be applied deterministically.
    fn apply_market_data(
        &mut self,
        observation: &MarketObservation,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError>;

    /// Applies a normalized, committed private venue report.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid transitions, inconsistent identifiers, or overflow.
    fn apply_venue_report(
        &mut self,
        report: &NormalizedVenueReport,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError>;

    /// Reconciles local projections against an authoritative venue snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for inconsistent snapshot state or a full output buffer.
    fn reconcile(
        &mut self,
        snapshot: &AuthoritativeVenueSnapshot,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError>;

    fn snapshot(&self) -> ExecutionSnapshot;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuarccExecutionEngine {
    state: ExecutionSnapshot,
}

impl QuarccExecutionEngine {
    #[must_use]
    pub fn new(config: ExecutionConfig) -> Self {
        Self {
            state: ExecutionSnapshot::empty(config),
        }
    }

    /// Restores a complete deterministic execution snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot version or a bounded collection is invalid.
    pub fn restore(snapshot: ExecutionSnapshot) -> Result<Self, ExecutionError> {
        if snapshot.version != config::EXECUTION_SNAPSHOT_VERSION {
            return Err(ExecutionError::InvalidSnapshotVersion(snapshot.version));
        }
        if snapshot.orders.len() > snapshot.config.max_orders
            || snapshot.seen_reports.len() > snapshot.config.max_seen_reports
            || snapshot.deferred_reports.len() > snapshot.config.max_deferred_reports
        {
            return Err(ExecutionError::BufferFull {
                limit: snapshot.config.max_orders,
            });
        }
        Ok(Self { state: snapshot })
    }

    #[must_use]
    pub const fn capabilities(&self) -> ExecutionCapabilities {
        ExecutionCapabilities {
            submit: true,
            cancel: true,
            replace: true,
            reconcile: true,
            snapshot_restore: true,
            kill_switch: true,
        }
    }

    #[must_use]
    pub fn order(&self, local_order_id: LocalOrderId) -> Option<&ManagedOrder> {
        self.state.orders.get(&local_order_id)
    }

    #[must_use]
    pub fn order_by_client(&self, client_order_id: ClientOrderId) -> Option<&ManagedOrder> {
        self.state
            .client_to_local
            .get(&client_order_id)
            .and_then(|local| self.state.orders.get(local))
    }

    fn next_local_id(&mut self) -> Result<LocalOrderId, ExecutionError> {
        let id = self.state.next_id;
        self.state.next_id = self
            .state
            .next_id
            .checked_add(1)
            .ok_or(ExecutionError::ArithmeticOverflow)?;
        Ok(LocalOrderId::new(id))
    }

    fn next_action_id(&mut self) -> Result<ActionId, ExecutionError> {
        let id = self.state.next_id;
        self.state.next_id = self
            .state
            .next_id
            .checked_add(1)
            .ok_or(ExecutionError::ArithmeticOverflow)?;
        Ok(ActionId::new(id))
    }

    fn resolve_report(&self, report: &NormalizedVenueReport) -> Option<LocalOrderId> {
        report
            .local_order_id
            .filter(|id| self.state.orders.contains_key(id))
            .or_else(|| {
                report
                    .client_order_id
                    .and_then(|id| self.state.client_to_local.get(&id).copied())
            })
            .or_else(|| {
                report
                    .venue_order_id
                    .as_ref()
                    .and_then(|id| self.state.venue_to_local.get(id).copied())
            })
    }

    fn defer_report(&mut self, report: &NormalizedVenueReport) -> Result<(), ExecutionError> {
        if self
            .state
            .deferred_reports
            .iter()
            .any(|deferred| deferred.report_id == report.report_id)
        {
            return Ok(());
        }
        if self.state.deferred_reports.len() >= self.state.config.max_deferred_reports {
            return Err(ExecutionError::BufferFull {
                limit: self.state.config.max_deferred_reports,
            });
        }
        self.state.deferred_reports.push(report.clone());
        Ok(())
    }

    fn retry_deferred(&mut self, output: &mut ExecutionActionBuffer) -> Result<(), ExecutionError> {
        let reports = core::mem::take(&mut self.state.deferred_reports);
        for report in reports {
            self.apply_venue_report(&report, output)?;
        }
        Ok(())
    }

    fn quarantine(
        &mut self,
        local: LocalOrderId,
        report: &NormalizedVenueReport,
    ) -> ExecutionError {
        let from = self.state.orders.get(&local).map_or_else(
            || "unknown".to_owned(),
            |order| format!("{:?}", order.lifecycle),
        );
        if let Some(order) = self.state.orders.get_mut(&local) {
            order.lifecycle = OrderLifecycle::Quarantined;
            order.quarantine_reason = Some(format!("invalid report {:?}", report.kind));
        }
        ExecutionError::InvalidTransition {
            from,
            report: format!("{:?}", report.kind),
        }
    }

    fn bind_venue_id(
        &mut self,
        local: LocalOrderId,
        venue: Option<&VenueOrderId>,
    ) -> Result<(), ExecutionError> {
        let Some(venue) = venue else {
            return Ok(());
        };
        if let Some(existing) = self.state.venue_to_local.get(venue)
            && *existing != local
        {
            return Err(ExecutionError::UnknownVenueOrder(venue.clone()));
        }
        self.state.venue_to_local.insert(venue.clone(), local);
        if let Some(order) = self.state.orders.get_mut(&local) {
            order.venue_order_id = Some(venue.clone());
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_resolved_report(
        &mut self,
        local: LocalOrderId,
        report: &NormalizedVenueReport,
    ) -> Result<(), ExecutionError> {
        self.bind_venue_id(local, report.venue_order_id.as_ref())?;
        let lifecycle = self
            .state
            .orders
            .get(&local)
            .ok_or(ExecutionError::UnknownLocalOrder(local))?
            .lifecycle;

        match &report.kind {
            VenueReportKind::Accepted
                if matches!(
                    lifecycle,
                    OrderLifecycle::PendingSubmit | OrderLifecycle::IntentReceived
                ) =>
            {
                self.state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?
                    .lifecycle = OrderLifecycle::Live;
            }
            VenueReportKind::Accepted
                if matches!(
                    lifecycle,
                    OrderLifecycle::Live
                        | OrderLifecycle::PartiallyFilled
                        | OrderLifecycle::Filled
                        | OrderLifecycle::Cancelled
                ) => {}
            VenueReportKind::Rejected { reason }
                if matches!(
                    lifecycle,
                    OrderLifecycle::PendingSubmit | OrderLifecycle::PendingReplace
                ) =>
            {
                let order = self
                    .state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                order.lifecycle = OrderLifecycle::Rejected;
                order.quarantine_reason = Some(reason.clone());
            }
            VenueReportKind::Fill {
                last_quantity: _,
                cumulative_quantity,
                price,
            } if matches!(
                lifecycle,
                OrderLifecycle::PendingSubmit
                    | OrderLifecycle::Live
                    | OrderLifecycle::PartiallyFilled
                    | OrderLifecycle::PendingCancel
                    | OrderLifecycle::PendingReplace
                    | OrderLifecycle::Filled
            ) =>
            {
                let (instrument, side, previous, total) = {
                    let order = self
                        .state
                        .orders
                        .get(&local)
                        .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                    (
                        order.desired.instrument_id,
                        order.desired.side,
                        order.filled_quantity,
                        order.desired.quantity,
                    )
                };
                if cumulative_quantity.get() < previous.get() {
                    return Ok(());
                }
                if cumulative_quantity.get() > total.get() {
                    return Err(self.quarantine(local, report));
                }
                let delta = cumulative_quantity
                    .checked_sub(previous)
                    .ok_or(ExecutionError::ArithmeticOverflow)?;
                if delta.get() > 0 {
                    self.state
                        .positions
                        .entry(instrument)
                        .or_default()
                        .apply_fill(side, delta, *price)?;
                }
                let order = self
                    .state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                order.filled_quantity = *cumulative_quantity;
                order.average_fill_price = Some(*price);
                order.lifecycle = if *cumulative_quantity == total {
                    OrderLifecycle::Filled
                } else {
                    OrderLifecycle::PartiallyFilled
                };
            }
            VenueReportKind::Cancelled
                if matches!(
                    lifecycle,
                    OrderLifecycle::PendingCancel
                        | OrderLifecycle::Live
                        | OrderLifecycle::PartiallyFilled
                        | OrderLifecycle::PendingReplace
                        | OrderLifecycle::PendingSubmit
                ) =>
            {
                self.state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?
                    .lifecycle = OrderLifecycle::Cancelled;
            }
            VenueReportKind::CancelRejected { .. }
                if lifecycle == OrderLifecycle::PendingCancel =>
            {
                let order = self
                    .state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                order.lifecycle = if order.filled_quantity.get() > 0 {
                    OrderLifecycle::PartiallyFilled
                } else {
                    OrderLifecycle::Live
                };
            }
            VenueReportKind::Replaced if lifecycle == OrderLifecycle::PendingReplace => {
                let order = self
                    .state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                if let Some((quantity, kind)) = order.pending_replace.take() {
                    order.desired.quantity = quantity;
                    order.desired.kind = kind;
                }
                order.lifecycle = if order.filled_quantity.get() > 0 {
                    OrderLifecycle::PartiallyFilled
                } else {
                    OrderLifecycle::Live
                };
            }
            VenueReportKind::Expired if lifecycle.is_open() => {
                self.state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?
                    .lifecycle = OrderLifecycle::Expired;
            }
            _ => return Err(self.quarantine(local, report)),
        }
        Ok(())
    }
}

impl ExecutionEngine for QuarccExecutionEngine {
    #[allow(clippy::too_many_lines)]
    fn submit_intent(
        &mut self,
        intent: ExecutionIntent,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError> {
        let intent_id = intent.intent_id();
        if self.state.processed_intents.contains(&intent_id) {
            return Ok(());
        }
        match intent {
            ExecutionIntent::Submit { order, .. } => {
                if self.state.kill_switch_active {
                    return Err(ExecutionError::KillSwitchActive);
                }
                if self
                    .state
                    .client_to_local
                    .contains_key(&order.client_order_id)
                {
                    return Err(ExecutionError::DuplicateIntent);
                }
                let open = self
                    .state
                    .orders
                    .values()
                    .filter(|managed| managed.lifecycle.is_open())
                    .count();
                risk::validate_submit(
                    &self.state.config,
                    &order,
                    open,
                    self.state.positions.get(&order.instrument_id),
                )?;
                let local = self.next_local_id()?;
                let action = ExecutionAction::Submit {
                    action_id: self.next_action_id()?,
                    local_order_id: local,
                    order: order.clone(),
                };
                output.push(action)?;
                self.state
                    .client_to_local
                    .insert(order.client_order_id, local);
                self.state.orders.insert(
                    local,
                    ManagedOrder {
                        local_order_id: local,
                        desired: order,
                        venue_order_id: None,
                        lifecycle: OrderLifecycle::PendingSubmit,
                        filled_quantity: QuantityLots::new(0),
                        average_fill_price: None,
                        replacement_for: None,
                        pending_replace: None,
                        quarantine_reason: None,
                    },
                );
            }
            ExecutionIntent::Cancel {
                client_order_id, ..
            } => {
                let local = *self
                    .state
                    .client_to_local
                    .get(&client_order_id)
                    .ok_or(ExecutionError::UnknownClientOrder(client_order_id))?;
                let venue = self
                    .state
                    .orders
                    .get(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?
                    .venue_order_id
                    .clone();
                let lifecycle = self.state.orders[&local].lifecycle;
                if !matches!(
                    lifecycle,
                    OrderLifecycle::PendingSubmit
                        | OrderLifecycle::Live
                        | OrderLifecycle::PartiallyFilled
                ) {
                    return Err(ExecutionError::InvalidTransition {
                        from: format!("{lifecycle:?}"),
                        report: "cancel_intent".to_owned(),
                    });
                }
                output.push(ExecutionAction::Cancel {
                    action_id: self.next_action_id()?,
                    local_order_id: local,
                    venue_order_id: venue,
                })?;
                self.state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?
                    .lifecycle = OrderLifecycle::PendingCancel;
            }
            ExecutionIntent::Replace {
                client_order_id,
                quantity,
                kind,
                ..
            } => {
                if quantity.get() <= 0 {
                    return Err(ExecutionError::InvalidQuantity);
                }
                let local = *self
                    .state
                    .client_to_local
                    .get(&client_order_id)
                    .ok_or(ExecutionError::UnknownClientOrder(client_order_id))?;
                let order = self
                    .state
                    .orders
                    .get(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                if !matches!(
                    order.lifecycle,
                    OrderLifecycle::Live | OrderLifecycle::PartiallyFilled
                ) {
                    return Err(ExecutionError::InvalidTransition {
                        from: format!("{:?}", order.lifecycle),
                        report: "replace_intent".to_owned(),
                    });
                }
                let venue = order
                    .venue_order_id
                    .clone()
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                output.push(ExecutionAction::Replace {
                    action_id: self.next_action_id()?,
                    local_order_id: local,
                    venue_order_id: venue,
                    quantity,
                    kind,
                })?;
                let order = self
                    .state
                    .orders
                    .get_mut(&local)
                    .ok_or(ExecutionError::UnknownLocalOrder(local))?;
                order.lifecycle = OrderLifecycle::PendingReplace;
                order.pending_replace = Some((quantity, kind));
            }
            ExecutionIntent::Query { local_order_id, .. } => {
                let venue = self
                    .state
                    .orders
                    .get(&local_order_id)
                    .ok_or(ExecutionError::UnknownLocalOrder(local_order_id))?
                    .venue_order_id
                    .clone();
                output.push(ExecutionAction::QueryOrder {
                    action_id: self.next_action_id()?,
                    local_order_id,
                    venue_order_id: venue,
                })?;
            }
            ExecutionIntent::ActivateKillSwitch { .. } => {
                self.state.kill_switch_active = true;
                let open: Vec<_> = self
                    .state
                    .orders
                    .iter()
                    .filter(|(_, order)| order.lifecycle.is_open())
                    .map(|(id, order)| (*id, order.venue_order_id.clone()))
                    .collect();
                for (local, venue) in open {
                    output.push(ExecutionAction::Cancel {
                        action_id: self.next_action_id()?,
                        local_order_id: local,
                        venue_order_id: venue,
                    })?;
                    if let Some(order) = self.state.orders.get_mut(&local) {
                        order.lifecycle = OrderLifecycle::PendingCancel;
                    }
                }
            }
        }
        self.state.processed_intents.insert(intent_id);
        Ok(())
    }

    fn apply_market_data(
        &mut self,
        observation: &MarketObservation,
        _output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError> {
        self.state
            .market_data
            .insert(observation.instrument_id, observation.clone());
        self.state.last_venue_sequence = self
            .state
            .last_venue_sequence
            .max(observation.committed_sequence);
        Ok(())
    }

    fn apply_venue_report(
        &mut self,
        report: &NormalizedVenueReport,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError> {
        if self.state.seen_reports.contains(&report.report_id) {
            return Ok(());
        }
        if self.state.seen_reports.len() >= self.state.config.max_seen_reports {
            return Err(ExecutionError::BufferFull {
                limit: self.state.config.max_seen_reports,
            });
        }
        let Some(local) = self.resolve_report(report) else {
            return self.defer_report(report);
        };
        self.apply_resolved_report(local, report)?;
        self.state.seen_reports.insert(report.report_id);
        if let Some(sequence) = report.source_sequence {
            self.state.last_venue_sequence = self.state.last_venue_sequence.max(sequence);
        }
        if matches!(
            report.kind,
            VenueReportKind::Accepted | VenueReportKind::Replaced
        ) {
            self.retry_deferred(output)?;
        }
        Ok(())
    }

    fn reconcile(
        &mut self,
        snapshot: &AuthoritativeVenueSnapshot,
        output: &mut ExecutionActionBuffer,
    ) -> Result<(), ExecutionError> {
        if snapshot.open_orders.len() > self.state.config.max_orders {
            return Err(ExecutionError::BufferFull {
                limit: self.state.config.max_orders,
            });
        }
        let mut observed = BTreeSet::new();
        for external in &snapshot.open_orders {
            let client = external
                .client_order_id
                .unwrap_or(external.order.client_order_id);
            let local = if let Some(local) = self.state.client_to_local.get(&client).copied() {
                local
            } else {
                let local = self.next_local_id()?;
                let mut desired = external.order.clone();
                desired.client_order_id = client;
                self.state.client_to_local.insert(client, local);
                self.state.orders.insert(
                    local,
                    ManagedOrder {
                        local_order_id: local,
                        desired,
                        venue_order_id: Some(external.venue_order_id.clone()),
                        lifecycle: OrderLifecycle::ExternallyDiscovered,
                        filled_quantity: external.filled_quantity,
                        average_fill_price: None,
                        replacement_for: None,
                        pending_replace: None,
                        quarantine_reason: None,
                    },
                );
                local
            };
            self.bind_venue_id(local, Some(&external.venue_order_id))?;
            if let Some(order) = self.state.orders.get_mut(&local) {
                order.filled_quantity = external.filled_quantity;
                if !matches!(order.lifecycle, OrderLifecycle::ExternallyDiscovered) {
                    order.lifecycle = if external.filled_quantity.get() > 0 {
                        OrderLifecycle::PartiallyFilled
                    } else {
                        OrderLifecycle::Live
                    };
                }
            }
            observed.insert(local);
        }
        let missing: Vec<_> = self
            .state
            .orders
            .iter()
            .filter(|(id, order)| order.lifecycle.is_open() && !observed.contains(id))
            .map(|(id, order)| (*id, order.venue_order_id.clone()))
            .collect();
        for (local, venue) in missing {
            output.push(ExecutionAction::QueryOrder {
                action_id: self.next_action_id()?,
                local_order_id: local,
                venue_order_id: venue,
            })?;
        }
        self.state.positions.clear();
        for position in &snapshot.positions {
            self.state.positions.insert(
                position.instrument_id,
                PositionProjection {
                    quantity: position.quantity,
                    average_price: position.average_price,
                    realized_pnl: position.realized_pnl,
                },
            );
        }
        self.state.last_venue_sequence = snapshot.committed_sequence;
        self.retry_deferred(output)
    }

    fn snapshot(&self) -> ExecutionSnapshot {
        self.state.clone()
    }
}
