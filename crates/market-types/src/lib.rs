#![forbid(unsafe_code)]
//! Strongly typed, fixed-point primitives shared by the Bunting market kernel.

use core::fmt;

macro_rules! integer_newtype {
    ($name:ident, $inner:ty) => {
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_arithmetic_rejects_overflow() {
        assert_eq!(PriceTicks(i64::MAX).checked_add(PriceTicks(1)), None);
        assert_eq!(EventSequence(0).checked_sub(EventSequence(1)), None);
    }
}
