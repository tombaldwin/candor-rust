//! Realtime market-data path (a SEPARATE branch to the pricing helper).
use crate::pricing::priced;
pub fn spot_quote(amount: u64) -> u64 { priced(amount) }
pub fn stream_tick(amount: u64) -> u64 { spot_quote(amount) }

/// The market-data stream loop. Runs ONCE PER MARKET TICK — thousands of times a second — with a hard
/// sub-millisecond budget. It MUST remain free of filesystem and network I/O: anything it transitively
/// calls that starts touching the disk or network blows the per-tick budget and stalls the feed.
pub fn run_stream(ticks: &[u64]) -> u64 { ticks.iter().map(|t| stream_tick(*t)).sum() }
