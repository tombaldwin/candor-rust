//! Value types for money. Pure data — no I/O, no global state.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
}

impl Currency {
    pub fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
        }
    }
}

/// An integer amount of minor units (cents/pence) in a given currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub fn new(minor: i64, currency: Currency) -> Self {
        Money { minor, currency }
    }

    /// Convert this amount using a rate expressed in milli-units
    /// (rate_milli = 1000 means parity). Pure arithmetic.
    pub fn convert(self, rate_milli: i64, to: Currency) -> Money {
        Money {
            minor: self.minor * rate_milli / 1000,
            currency: to,
        }
    }
}
