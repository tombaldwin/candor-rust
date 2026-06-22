# Frozen target — real-world blast-radius A/B

Committed before any trial agent runs (the pre-trial freeze required by
[../PREREG-realworld.md](../PREREG-realworld.md)). Nothing here is retuned after trials begin.

| field | value |
|---|---|
| **repo** | `BurntSushi/ripgrep` (`https://github.com/BurntSushi/ripgrep`) |
| **commit** | `dfe4a81d2591daca76d25ae4e052c34b26578155` |
| **scope (crate)** | `crates/ignore` — the directory-walking + gitignore/override matching crate (~6.7k LOC) |
| **symbol** | `pathutil::strip_prefix` (the `pub(crate) fn strip_prefix` in `crates/ignore/src/pathutil.rs`) |
| **effect probed** | `Fs` — natural framing: *if `strip_prefix` canonicalized the path (resolving symlinks via the filesystem) before stripping, it would perform filesystem I/O* |
| **instrument** | **deep engine** (`cargo candor`, the nightly rustc/MIR backend) — see the prereg amendment for why not candor-scan |
| **ground truth** | the adjudicated 32-function propagation set in [GROUND_TRUTH.md](GROUND_TRUTH.md) |

## Why this target (against the prereg's selection rules)

1. **Real, widely-used, un-seen.** ripgrep is not in candor's calibration corpus (that's `ebman`/`mcfly`
   and their calibration deps). ✓
2. **Graph exceeds comfortable context.** `crates/ignore` is ~6.7k LOC across 8 modules; the chosen
   symbol's transitive caller tree is **32 functions across 4 files (walk/dir/gitignore/overrides) and
   ~4 call-graph layers**, including trait-dispatch (`<Walk as Iterator>::next`), closures, and the
   parallel-walk worker machinery — not enumerable completely by eye. ✓ (≥25 callers / ≥5 files is met
   counting the two reachability spines + the iterator impl; layers ≥4.) ✓
3. **The deep engine analyzes it cleanly.** Merged deep call graph: **2718 nodes / 4988 edges** (vs the
   syntactic backend's 292 edges on the same code). The `callers` query resolves the symbol's tree with
   no `Unknown` on the path of interest. ✓
4. **Un-leaky names.** Ordinary domain names (`walk`, `dir`, `gitignore`, `strip_prefix`); the call
   structure is not telegraphed by naming. ✓

## Reproduce the frozen setup

```sh
git clone https://github.com/BurntSushi/ripgrep checkout
git -C checkout checkout dfe4a81d2591daca76d25ae4e052c34b26578155
# deep-engine report (treatment arm reads this; control reads only crates/ignore source):
( cd checkout && cargo candor snapshot ../work/baseline )      # nightly dylint backend
# the symbol's caller tree, as the treatment agent would query it:
candor-query callers ../work/baseline pathutil::strip_prefix 1
```
