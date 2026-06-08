# Publishing the stable crates to crates.io

**Status: `candor-report` / `candor-classify` / `candor-scan` v0.3.0 are published.**
`cargo install candor-scan` works. The steps below are the runbook for future releases.

Goal: make the zero-install path a one-liner — `cargo install candor-scan`. Three crates publish (all
stable, no `rustc_private`); the root `candor` lint stays `publish = false` (it needs the pinned nightly
+ `rustc-dev` to build, so it can't live on crates.io as an installable).

```
candor-report   ← leaf (serde only)
candor-classify ← depends on candor-report
candor-scan     ← depends on candor-report + candor-classify   (the installable binary)
```

These are an **outward action** — they push to a public registry under your account. Run them yourself
when ready; nothing here publishes automatically.

## One-time

```sh
cargo login            # paste a crates.io API token from https://crates.io/me
```

Check the three names are free (publish fails cleanly if taken — you'd rename in the manifests +
inter-crate deps and retry): https://crates.io/crates/candor-scan etc.

## Publish, in dependency order

Each crate must be live on crates.io before the next can resolve it (the `version = "0.3.0"` on the path
deps points at the registry once published). Wait a few seconds between steps for the index to update.

```sh
cargo publish -p candor-report     # 1. leaf — fully self-contained
cargo publish -p candor-classify   # 2. after report is live
cargo publish -p candor-scan       # 3. after classify is live → `cargo install candor-scan` works
```

Each does a clean-room build from the packaged tarball before uploading. `candor-report` already passes
`cargo package -p candor-report` here; the other two can only be verified once their dep is live (until
then `cargo package` reports `no matching package named candor-report/-classify` — expected, not a bug).

Optional pre-flight once report is up:

```sh
cargo publish -p candor-classify --dry-run
```

## After publishing

```sh
cargo install candor-scan
candor-scan --version          # candor-scan 0.3.0
candor-scan .                  # writes .candor/report.<crate>.scan.json
```

Then the README's "install from nothing" path can become the one-liner instead of clone + build.

## Future releases

All four crates share one version (currently `0.3.0`). Bump them together (the path deps carry the
matching `version`), then publish report → classify → scan in the same order. A new `candor-scan`
release that pulls in classifier/scanner changes needs the corresponding `candor-classify` /
`candor-report` versions published first.

## What ships

`cargo package --list` confirms each tarball carries only `src/`, `README.md`, `Cargo.toml`
(`target/`, evals, fixtures, and the nightly lint are excluded). The shared MIT/Apache license applies
(SPDX `license = "MIT OR Apache-2.0"`; the `LICENSE-*` files live at the repo root).
