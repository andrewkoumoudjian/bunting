use serde::{Deserialize, Serialize};

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u128);

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
    };
}

numeric_id!(IntentId);
numeric_id!(ActionId);
numeric_id!(LocalOrderId);
numeric_id!(ClientOrderId);
numeric_id!(ReportId);

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct VenueOrderId(pub String);

impl VenueOrderId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}
