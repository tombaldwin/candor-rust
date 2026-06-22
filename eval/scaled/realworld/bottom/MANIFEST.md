# Frozen target — real-world blast-radius A/B, SECOND target (bottom)

The second real-world point (the first is `git-delta`, see [../MANIFEST.md](../MANIFEST.md)). Different
repo, different effect (`Fs` vs delta's `Exec`), different architecture (a data→draw TUI pipeline vs a
diff-render StateMachine).

| field | value |
|---|---|
| **repo** | `ClementTsang/bottom` (`https://github.com/ClementTsang/bottom`) — a terminal system monitor |
| **commit** | `b3694fc` |
| **scope** | the `bottom` **library** crate (~37k LOC, single crate, ~167 files under `src/`); the thin `fn main` bin entry is out of scope |
| **symbol** | `app::data::store::DataStore::get_data` (`pub fn get_data(&self) -> &StoredData`) — a pure in-memory borrow of the cached `StoredData` |
| **effect probed** | `Fs` — natural framing: *if `get_data` read live system data from the OS (/proc, sysinfo) on each call instead of returning the cached store* |
| **instrument** | deep engine (`cargo candor`), run with the **nightly-2026-06-14 port** (branch `nightly-2026-06-14-port`); delta re-verified byte-identical on that engine |
| **ground truth** | the adjudicated **26-function** set in [GROUND_TRUTH.md](GROUND_TRUTH.md) / [bottom-groundtruth.txt](bottom-groundtruth.txt) |

## Note on the "ICE" (harmless here)

`cargo candor snapshot` exits non-zero on bottom: the lint surfaces a rustc **delayed bug**
(`missing value for assoc item in impl`) that is flushed at compiler **shutdown** — *after* candor's
`check_crate_post` has already written the per-crate report. So the report (85 effectful fns, 1656-node
callgraph) is produced and complete; the eval's treatment arm queries that report, not the wrapper.
Confirmed harmless for this symbol: both independent source-only tracers found the same 26-function tree
candor reports (nothing candor missed). The non-zero exit + the underlying delayed-bug-triggering query
are a separate candor robustness item (it does not affect the analysis output). bottom compiles clean
under plain `cargo +nightly check` — the crash is candor-side, not a rustc bug.

## Reproduce

```sh
git clone https://github.com/ClementTsang/bottom checkout && git -C checkout checkout b3694fc
( cd checkout && cargo candor snapshot ../work/base )   # exits 1 (cosmetic), report still written
candor-query callers ../work/base "app::data::store::DataStore::get_data" 1
```
