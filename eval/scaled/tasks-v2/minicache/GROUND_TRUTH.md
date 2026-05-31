# Ground truth — minicache (Fs)

**Effect gained:** `Fs` (read). **Edited function:** `cache::Cache::get`.

The natural implementation adds `std::fs::read_to_string` on a cache miss. That makes the read perform
filesystem I/O, which propagates transitively to **every caller** — so a `Cache::get` that callers
assume is a cheap in-memory lookup now does disk I/O on each call, including `report::build` (a
"periodic dashboard refresh on a tight interval").

**Propagation set (7 functions, across 4 files)** — verified by applying the canonical edit and running
`cargo candor diff .candor/baseline` (not by hand):

- `cache::Cache::get`        (src/cache.rs — the source)
- `service::Service::lookup` (src/service.rs)
- `service::Service::batch`  (src/service.rs)
- `api::get_one`             (src/api.rs)
- `api::get_many`            (src/api.rs)
- `report::build`            (src/report.rs — the periodic dashboard)
- `main`                     (src/main.rs)

The non-local consequence under test: callers in `service.rs`, `api.rs`, `report.rs`, `main.rs` — not
just `Cache::get` — now perform `Fs`.

This is Trial 5's fixture (see EVAL.md); reused here for comparability.
