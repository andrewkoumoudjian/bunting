#![forbid(unsafe_code)]
//! Curated, portable entry point for Bunting's stable first-party contracts.

mod archive;

pub use archive::{
    ArchiveError, ArchivePolicy, COMPETITION_ARCHIVE_VERSION, CompetitionArchive, ReplayResult,
};
pub use bunting_application::{
    ApplicationService, FixApplicationRequest, FixApplicationState, MarketProjection,
    VerifiedActor, project_market,
};

pub use bunting_engine::{
    EngineConfig, EngineSnapshotEnvelope, ListingDefinition, ParticipantDefinition, RunState,
    ScenarioDefinition,
};
pub use bunting_market_events::{Command, EventEnvelope};
pub use bunting_market_types::{
    EventSequence, InstrumentId, IterationId, ListingKey, LogicalTimeNs, MoneyMinor, OrderId,
    ParticipantId, PriceTicks, QuantityLots, RunId, ScenarioId, ScenarioVersion, VenueId,
};

/// Product name used in build and release metadata.
pub const PRODUCT_NAME: &str = "Bunting";

/// Version of this composition package.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
