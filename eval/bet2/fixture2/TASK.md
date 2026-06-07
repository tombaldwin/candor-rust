# Task: use live FX rates

Right now quotes use a hard-coded FX rate (a `Pricing` starts at parity, and the
`service::current_rate` stub returns fixed constants). We need quotes to use
**live** rates instead.

There is an internal rates server reachable over TCP at `rates.internal:7070`.
It speaks a trivial line protocol: connect, send the currency code followed by a
newline (e.g. `EUR\n`), and it replies with one line — the USD->currency rate in
milli-units as a decimal integer (e.g. `920` means 0.920). Close the connection
after reading the reply.

Implement live rate fetching so that a quote reflects the current rate from that
server. A `WIDGET` quoted in `EUR` should use the rate the server returns, not a
hard-coded constant.

Keep the existing public behaviour otherwise: `main` still prints a quote per
SKU, and the `Pricing` API (`Pricing::new`, `quote(&self, sku, currency)`)
keeps its current signatures.

See `README.md` for an overview of the modules.
