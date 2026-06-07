# Ground truth — orderflow (Net) — the LARGE fixture (batch 3)

**Effect gained:** `Net`. **Edited function:** `pricing::Pricing::quote`.

The natural implementation adds `std::net::TcpStream::connect` (+ write/read) inside `quote` to fetch
the live FX rate. That makes pricing perform network I/O, which propagates transitively to **every
function that computes or displays a price** — the whole app, since `quote` is the one place a
foreign-currency amount is produced. A `quote` that callers assume is a cheap in-memory catalog
lookup now does a network round-trip per SKU, including `report::daily_revenue` (a periodic dashboard
that re-quotes the whole catalog on a tight schedule) and `admin::recompute_prices`.

This is the **scale** fixture: where the small tasks (minicache/geoip/renderer) propagate to 7
functions across 4 files, here the effect reaches **16 non-local functions across 9 files**, through
3–5 layers of call graph (pricing → cart → discount/checkout → service → api/report/admin → main).
Tracing that by hand — the control arm's job — is the realistic failure mode candor targets.

**Propagation set (16 functions, across 9 files)** — verified by applying the canonical edit and
running `cargo candor diff .candor/baseline` (not by hand); candor reported exactly these 16 gaining
`Net` (plus the edited `Pricing::quote` itself, the source, excluded from the set):

- `Pricing::quote_bulk`        (src/pricing.rs — quotes many SKUs)
- `Cart::line_total`           (src/cart.rs)
- `Cart::subtotal`             (src/cart.rs)
- `Cart::total`                (src/cart.rs)
- `Discount::for_cart`         (src/discount.rs)
- `Checkout::review`           (src/checkout.rs)
- `Checkout::place`            (src/checkout.rs)
- `OrderService::quote_one`    (src/service.rs)
- `OrderService::quote_many`   (src/service.rs)
- `OrderService::checkout`     (src/service.rs)
- `api::get_quote`             (src/api.rs)
- `api::list_quotes`           (src/api.rs)
- `api::post_checkout`         (src/api.rs)
- `report::daily_revenue`      (src/report.rs — the periodic dashboard)
- `admin::recompute_prices`    (src/admin.rs — back-office tooling)
- `main`                       (src/main.rs)

The non-local consequence under test: callers in `pricing.rs` (quote_bulk), `cart.rs`, `discount.rs`,
`checkout.rs`, `service.rs`, `api.rs`, `report.rs`, `admin.rs`, `main.rs` — not just `Pricing::quote`
— now perform `Net`.
