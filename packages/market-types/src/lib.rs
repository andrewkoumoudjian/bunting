#![forbid(unsafe_code)]
//! Strongly typed, fixed-point primitives shared by the Bunting market kernel.

use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NumericError {
    NonPositiveQuantity,
    InvalidPriceBounds,
    PriceOutOfBounds,
    Overflow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierError {
    Zero,
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

macro_rules! integer_newtype {
    ($name:ident, $inner:ty) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        #[repr(transparent)]
        pub struct $name(pub $inner);

        impl $name {
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> $inner {
                self.0
            }

            pub fn checked_add(self, rhs: Self) -> Option<Self> {
                self.0.checked_add(rhs.0).map(Self)
            }

            pub fn checked_sub(self, rhs: Self) -> Option<Self> {
                self.0.checked_sub(rhs.0).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

integer_newtype!(PriceTicks, i64);
integer_newtype!(QuantityLots, i64);
integer_newtype!(MoneyMinor, i128);
integer_newtype!(LogicalTimeNs, u64);
integer_newtype!(EventSequence, u64);

macro_rules! identifier {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        #[repr(transparent)]
        pub struct $name(u128);

        impl $name {
            #[must_use]
            pub const fn new(value: u128) -> Self {
                Self(value)
            }
            #[must_use]
            pub const fn get(self) -> u128 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

identifier!(RunId);
identifier!(InstrumentId);
identifier!(ParticipantId);
identifier!(OrderId);
identifier!(CommandId);
identifier!(EventId);
identifier!(CorrelationId);
identifier!(VenueId);
identifier!(ScenarioId);
identifier!(ScenarioVersion);
identifier!(IterationId);
identifier!(CurrencyId);
identifier!(FacilityId);
identifier!(TenderId);
identifier!(NegotiationId);
identifier!(NewsId);
identifier!(AgentId);

macro_rules! checked_identifier {
    ($name:ident) => {
        impl $name {
            /// Constructs a non-zero domain identifier at an external boundary.
            ///
            /// # Errors
            /// Returns [`IdentifierError::Zero`] when the identifier is zero.
            pub const fn try_new(value: u128) -> Result<Self, IdentifierError> {
                if value == 0 {
                    Err(IdentifierError::Zero)
                } else {
                    Ok(Self::new(value))
                }
            }
        }
    };
}

checked_identifier!(VenueId);
checked_identifier!(ScenarioId);
checked_identifier!(ScenarioVersion);
checked_identifier!(IterationId);
checked_identifier!(CurrencyId);
checked_identifier!(FacilityId);
checked_identifier!(TenderId);
checked_identifier!(NegotiationId);
checked_identifier!(NewsId);
checked_identifier!(AgentId);

/// Venue-specific identity of one tradable listing.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ListingKey {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
}

impl Serialize for ListingKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}:{}", self.venue_id, self.instrument_id))
    }
}

impl<'de> Deserialize<'de> for ListingKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let (venue, instrument) = value
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("listing key must be venue:instrument"))?;
        let venue_id = venue
            .parse::<u128>()
            .map(VenueId::new)
            .map_err(serde::de::Error::custom)?;
        let instrument_id = instrument
            .parse::<u128>()
            .map(InstrumentId::new)
            .map_err(serde::de::Error::custom)?;
        Ok(Self::new(venue_id, instrument_id))
    }
}

impl ListingKey {
    #[must_use]
    pub const fn new(venue_id: VenueId, instrument_id: InstrumentId) -> Self {
        Self {
            venue_id,
            instrument_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PriceBounds {
    pub min: PriceTicks,
    pub max: PriceTicks,
}

impl PriceBounds {
    /// Creates inclusive positive bounds.
    ///
    /// # Errors
    /// Returns [`NumericError::InvalidPriceBounds`] for non-positive or reversed bounds.
    pub fn new(min: PriceTicks, max: PriceTicks) -> Result<Self, NumericError> {
        if min.get() <= 0 || min > max {
            return Err(NumericError::InvalidPriceBounds);
        }
        Ok(Self { min, max })
    }
    /// Checks that a price lies within the inclusive bounds.
    ///
    /// # Errors
    /// Returns [`NumericError::PriceOutOfBounds`] outside the configured range.
    pub fn validate(self, price: PriceTicks) -> Result<(), NumericError> {
        if price < self.min || price > self.max {
            Err(NumericError::PriceOutOfBounds)
        } else {
            Ok(())
        }
    }
}

impl QuantityLots {
    /// Constructs a strictly positive quantity.
    ///
    /// # Errors
    /// Returns [`NumericError::NonPositiveQuantity`] for zero or negative values.
    pub fn positive(value: i64) -> Result<Self, NumericError> {
        if value <= 0 {
            Err(NumericError::NonPositiveQuantity)
        } else {
            Ok(Self(value))
        }
    }
}

impl MoneyMinor {
    /// Computes exact price-times-quantity money in widened arithmetic.
    ///
    /// # Errors
    /// Returns [`NumericError::Overflow`] when the `i128` product cannot be represented.
    pub fn checked_mul_price_quantity(
        price: PriceTicks,
        quantity: QuantityLots,
    ) -> Result<Self, NumericError> {
        i128::from(price.get())
            .checked_mul(i128::from(quantity.get()))
            .map(Self)
            .ok_or(NumericError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_arithmetic_rejects_overflow() {
        assert_eq!(PriceTicks(i64::MAX).checked_add(PriceTicks(1)), None);
        assert_eq!(EventSequence(0).checked_sub(EventSequence(1)), None);
    }

    #[test]
    fn checked_values_reject_invalid_inputs() {
        assert_eq!(
            QuantityLots::positive(0),
            Err(NumericError::NonPositiveQuantity)
        );
        assert_eq!(
            PriceBounds::new(PriceTicks(10), PriceTicks(5)),
            Err(NumericError::InvalidPriceBounds)
        );
    }
}
