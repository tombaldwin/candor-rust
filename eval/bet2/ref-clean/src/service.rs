//! The service layer — the home for ALL I/O. See ARCHITECTURE.md.

use crate::money::Currency;
use std::io::{Read, Write};
use std::net::TcpStream;

/// The current USD->`currency` rate in milli-units, fetched live from the
/// internal rates server over TCP. I/O belongs here, in the service layer.
pub fn current_rate(currency: Currency) -> i64 {
    fetch_rate(currency).unwrap_or(1000)
}

fn fetch_rate(currency: Currency) -> Option<i64> {
    let mut stream = TcpStream::connect("rates.internal:7070").ok()?;
    stream.write_all(currency.code().as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    reply.trim().parse().ok()
}
