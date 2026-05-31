# Ground truth — geoip (Net)

**Effect gained:** `Net`. **Edited function:** `resolver::Resolver::resolve`.

The natural implementation adds `std::net::TcpStream::connect` (+ read/write) on a cache miss. That
makes resolution perform network I/O, propagating transitively to **every caller** — a `resolve` that
callers assume is a cheap in-memory table lookup now does a network round-trip on each miss, including
`report::summary` (a "periodic dashboard refresh on a tight interval").

**Propagation set (7 functions, across 4 files)** — verified by applying the canonical edit and running
`cargo candor diff .candor/baseline`:

- `resolver::Resolver::resolve` (src/resolver.rs — the source)
- `service::GeoService::locate` (src/service.rs)
- `service::GeoService::batch`  (src/service.rs)
- `api::lookup_one`             (src/api.rs)
- `api::lookup_many`            (src/api.rs)
- `report::summary`             (src/report.rs — the periodic dashboard)
- `main`                        (src/main.rs)

The non-local consequence under test: callers in `service.rs`, `api.rs`, `report.rs`, `main.rs` now
perform `Net`.
