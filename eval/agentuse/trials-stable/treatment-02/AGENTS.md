# Project conventions

This is a Rust project. Build with `cargo build`, test with `cargo test`.

## Tooling: candor (effect analysis)

This project has **candor** installed — a tool that maps which functions perform side effects
(filesystem, network, subprocess, …) and **how those effects propagate transitively across the call
graph**. Run it from the project root:

- `./candor audit` — the effect map: every effectful function and what it does.
- `./candor callers <fn>` — which functions call `<fn>` (who would be affected by a change to it).
- `./candor map` — a module → effects overview.
- `./candor diff .candor/baseline` — what effects changed versus a saved baseline.

Use whatever tooling helps you work accurately.
