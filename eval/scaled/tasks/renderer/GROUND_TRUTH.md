# Ground truth — renderer (Exec)

**Effect gained:** `Exec`. **Edited function:** `engine::Engine::expand`.

The natural implementation adds `std::process::Command` (`sh -c CMD`) for `exec:` tokens. That makes
token expansion spawn subprocesses, propagating transitively to **every caller** — an `expand` that
callers assume is a cheap snippet lookup now runs arbitrary shell commands, including `report::build_all`
(a "periodic rebuild job that assumes rendering is cheap"). (It is also a command-injection surface;
that's a separate concern from the propagation under test here.)

**Propagation set (7 functions, across 4 files)** — verified by applying the canonical edit and running
`cargo candor diff .candor/baseline`:

- `engine::Engine::expand`     (src/engine.rs — the source)
- `page::Page::render_token`   (src/page.rs)
- `page::Page::render`         (src/page.rs)
- `api::render_one`            (src/api.rs)
- `api::render_many`           (src/api.rs)
- `report::build_all`          (src/report.rs — the periodic rebuild)
- `main`                       (src/main.rs)

The non-local consequence under test: callers in `page.rs`, `api.rs`, `report.rs`, `main.rs` now
perform `Exec`.
