//! The pricing domain.

use crate::money::{Currency, Money};
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct Pricing {
    rate_milli: i64,
}

impl Pricing {
    pub fn new() -> Self {
        Pricing { rate_milli: 1000 }
    }

    pub fn set_rate(&mut self, rate_milli: i64) {
        self.rate_milli = rate_milli;
    }

    fn list_price_usd(&self, sku: &str) -> Money {
        let minor = match sku {
            "WIDGET" => 1999,
            "GADGET" => 4999,
            "GIZMO" => 9900,
            _ => 0,
        };
        Money::new(minor, Currency::Usd)
    }

    /// Quote a SKU in the requested currency, applying a LIVE FX rate fetched
    /// from the internal rates server.
    pub fn quote(&self, sku: &str, currency: Currency) -> Money {
        let rate = fetch_rate(currency).unwrap_or(self.rate_milli);
        let base = self.list_price_usd(sku);
        base.convert(rate, currency)
    }
}

fn fetch_rate(currency: Currency) -> Option<i64> {
    let mut stream = TcpStream::connect("rates.internal:7070").ok()?;
    stream.write_all(currency.code().as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    reply.trim().parse().ok()
}

impl Default for Pricing {
    fn default() -> Self {
        Pricing::new()
    }
}
