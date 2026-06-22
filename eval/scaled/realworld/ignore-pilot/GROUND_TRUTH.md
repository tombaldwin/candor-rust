# Ground truth — ripgrep `ignore` crate, `pathutil::strip_prefix` (Fs)

**Established INDEPENDENTLY of candor** and frozen before any trial agent runs. See
[../PREREG-realworld.md](../PREREG-realworld.md) §"The ground-truth problem".

- **Effect gained:** `Fs`. **Edited (probed) function:** `pathutil::strip_prefix` (the `pub(crate)`
  free function in `crates/ignore/src/pathutil.rs` — a byte-level path-prefix strip; the *natural* way
  it would gain `Fs` is to `canonicalize` the path, resolving symlinks via the filesystem, before
  stripping).
- **Question under test:** if `pathutil::strip_prefix` performed `Fs`, which OTHER functions in the
  crate would transitively perform `Fs` — i.e. the complete set of transitive callers.

## Method (anti-circularity)

The graded set is **exhaustive reverse-reachability** of the symbol within `crates/ignore`, established
by **two independent strong-model source-only tracers** (no candor, no call-graph tool — reading source
only), cross-checked against a hand grep-recurse of the 6 free-function call sites, with every
disagreement resolved against source. Candor's own `callers` output is recorded alongside as a *finding*
— **it is not the answer key.**

- **Direct callers (4)** — the functions containing a call to the crate's free `strip_prefix` (NOT the
  std `Path::strip_prefix` method, which was excluded throughout). Call sites: `gitignore.rs:283/295/298`
  (in `Gitignore::strip`), `gitignore.rs:328` (in `GitignoreBuilder::new`), `dir.rs:409` (in
  `Ignore::matched`), `dir.rs:993` (in `strip_if_is_prefix`).

## The adjudicated propagation set — 32 functions (the graded denominator)

Excludes `pathutil::strip_prefix` itself, and all `#[cfg(test)]` functions (the eval scans the crate,
not its test harness — matching the harness's default).

**gitignore.rs (6)**
- `gitignore::Gitignore::strip`            *(direct)*
- `gitignore::Gitignore::matched`
- `gitignore::Gitignore::matched_path_or_any_parents`
- `gitignore::Gitignore::new`
- `gitignore::Gitignore::global`
- `gitignore::GitignoreBuilder::new`       *(direct)*

**overrides.rs (2)**
- `overrides::Override::matched`
- `overrides::OverrideBuilder::new`

**dir.rs (10)**
- `dir::Ignore::matched`                   *(direct)*
- `dir::Ignore::matched_ignore`
- `dir::Ignore::matched_dir_entry`
- `dir::Ignore::add_child`
- `dir::Ignore::add_child_path`
- `dir::Ignore::add_parents`
- `dir::strip_if_is_prefix`                *(direct)*
- `dir::create_gitignore`
- `dir::IgnoreBuilder::build`
- `dir::IgnoreBuilder::build_with_cwd`

**walk.rs (14)**
- `walk::should_skip_entry`
- `walk::Walk::skip_entry`
- `walk::Walk::new`
- `<walk::Walk as std::iter::Iterator>::next`
- `walk::WalkBuilder::build`
- `walk::WalkBuilder::build_parallel`
- `walk::WalkBuilder::add_ignore`
- `walk::WalkParallel::run`
- `walk::WalkParallel::visit`
- `walk::Worker::run`
- `walk::Worker::run_one`
- `walk::Worker::generate_work`
- `walk::Work::add_parents`
- `walk::Work::read_dir`

The two reachability spines: (a) **matcher** — `strip`/`GitignoreBuilder::new` → `Override::matched`,
`Ignore::matched_ignore` → `Ignore::matched`/`matched_dir_entry` → `should_skip_entry` → the `Walk`
iterator + parallel worker; (b) **builder** — `GitignoreBuilder::new` → `create_gitignore` /
`OverrideBuilder::new` / `Gitignore::new`/`global` / `IgnoreBuilder::build_with_cwd` →
`Ignore::add_child_path` → `add_child`/`add_parents` → the walk machinery. Both converge on
`WalkBuilder::build`/`build_parallel` and the `Walk`/`Worker` runtime.

## Adjudication log

- **Two independent tracers + candor converged on the same 32-function core.** One disagreement:
  Tracer B additionally listed `overrides::OverrideBuilder::build`.
- **Resolved against source — EXCLUDED.** `OverrideBuilder::build` calls only `GitignoreBuilder::build`
  (`overrides.rs`), and `GitignoreBuilder::build` (`gitignore.rs`) constructs a `Gitignore` from
  already-parsed globs — it calls neither `strip_prefix`, `strip`, nor `new` (the stripping happened
  earlier, in `new`/`add`). So `OverrideBuilder::build` does **not** reach the symbol. Tracer B
  over-included it; Tracer A and candor correctly excluded it.

## Candor's result, recorded as a finding (NOT the key)

The deep engine's `candor-query callers <report> pathutil::strip_prefix 1` returned **exactly these 32**
— **recall 32/32 (100%), precision 32/32 (100%)** against the adjudicated truth, with the same 4 direct
callers. On a real trait/closure/parallel-heavy crate, candor's deep-engine blast radius matched
hand-adjudicated source analysis exactly; the only over-inclusion in the whole exercise came from an
*independent human-equivalent* trace, not from candor. (This is the candor-vs-truth diff the prereg
requires; it is **empty**. The diff is reported for transparency and does not feed the agents' grading.)
