//! Synchronous request API (another branch).
use crate::orders::order_total;
pub fn api_quote(order: &[u64]) -> u64 { order_total(order) }
pub fn handle_request(order: &[u64]) -> u64 { api_quote(order) }
pub fn serve(order: &[u64]) -> u64 { handle_request(order) }
