# Changelog

All notable changes to candor are recorded here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); candor is pre-1.0, so minor versions may include
behavioural changes (always in the soundness-increasing direction — see the §4 trust contract).

## [0.3.7] — 2026-06-12 (crates: candor-report / candor-classify / candor-scan, lockstep)

### Changed — spec 0.4 (conformance-breaking upgrade, wire-compatible)

- Reports now declare **spec `0.4`** (candor-report's `SPEC_VERSION`). 0.4 upgrades four SHOULDs
  to MUST — §2.1 version-trust at the chain join (missing version = unverifiable = Unknown), the
  §7.14 κ-coverage ledger, universal `hash` emission, and literal surfaces when `allow` rules are
  enforced. This engine already satisfies all four; the bump is the declaration.

### Added — the κ-coverage ledger + report chaining (the curation treadmill's exit; SPEC §7.14 / §2)

- **The κ-coverage ledger:** the receipt names every `Cargo.toml` dependency the code demonstrably
  calls that the classifier knows nothing about — `κ doesn't know N dependencies … effects through
  them are INVISIBLE (not Unknown)`. Per-scan evidence instead of a doc footnote. Exempt: the
  platform frontier, calibrated crates, and crates a chained report covers (an all-pure dep's EMPTY
  report registers as covered — its emptiness is the purity claim).
- **`CANDOR_DEPS` chaining:** reports now carry the §2 join key (`hash: crate#qual`), and an
  unclassified call into a crate a sibling report covers inherits its effects AND literal surfaces
  (unambiguous tail-first join; a report from a different scanner version downgrades to `Unknown`,
  §2.1).
- **`--deps`:** scan the whole Cargo.lock dependency tree (unbuilt registry sources, in-process —
  the self-gate's own `deny Exec` forbids the spawn-yourself shortcut) into `.candor/deps/`
  (one subdirectory per name@version, skip-if-already-scanned), then scan the root chained over
  it. Measured on a real 328-dep app: 75s one-time (~0.23s/dep, cached after), the ledger dropping
  12 unlisted deps → 1 (the path dep), 7 fns gaining real effects. The pre-release review closed
  the trap cluster: the root policy no longer leaks into dep scans via CANDOR_POLICY; `--out` is
  honoured; same-crate version pairs don't overwrite each other; the dep-dir + CANDOR_DEPS
  double-load no longer drops every join as ambiguous; joins use the crate-relative path
  (bare-leaf fallback removed — it fabricated); cargo_deps reads every workspace manifest and the
  `[dependencies.name]` header forms.

### Added — local-trait dispatch (syntactic CHA; SPEC §4 bounded-CHA discipline)

- A dispatch-typed receiver — `&dyn T` / `impl T` / generic `X: T` (inline or where-clause), through
  `Box`/`Rc`/`Arc`/`RefCell`/`Mutex`/`RwLock`, as a param, let, or STRUCT FIELD — resolves to the
  trait's local implementors when narrow (≤12, the cross-engine bound; both sides of the bound are
  unit-tested), so the DI pattern (`self.store.save()` → `PgStore::save`) carries its effects on
  the stable scanner. A local trait declaring the method but with no visible impl (or too many, or
  an ambiguous name) reads honest `Unknown` — the previous SILENT miss, closed.
- Resolution is gated to LOCALLY-DECLARED traits whose declaration carries the called METHOD
  (the pre-release review execution-verified the wider rule fabricating: `impl Iterator for
  RowIter` + `fn f(it: impl Iterator)` charged pure `f` with RowIter's `Db`; a same-named method
  on a non-dispatching bound was the same wrongness). External-trait dispatch stays a documented
  miss; `.clone()` on a bound param neither edges nor floods.

### Added — classifier (candor-classify)

- The **entropy tier**: `SaltString::generate` (argon2/scrypt/pbkdf2/password_hash), bcrypt's
  `hash`/`hash_with_result`, and `rand_core`'s OsRng surface → `Rand` (the TS engine's CTA lesson,
  found by the ledger's first probe).
- **Comma-list FROM extraction**: `SELECT a FROM t1, t2, t3` yields all three tables
  (comma-adjacent continuation; an alias breaks the chain — the fabrication guard for column
  lists). Pinned three-way by the conformance vector battery (20 vectors).

## [0.3.6] — 2026-06-11 (crates: candor-report / candor-classify / candor-scan, lockstep)

### Added — the Db literal surface (`tables` + `allow Db`)

- **`tables`** joins `hosts`/`cmds`/`paths` as the fourth literal-refinement surface (SPEC §2):
  table-position identifiers extracted from SQL string literals at `Db`-classified calls
  (`FROM`/`JOIN`/`INTO` anywhere; statement-leading `UPDATE`/`TRUNCATE`; `TABLE` — extraction is
  conservative in the fabrication direction: non-SQL strings and `FOR UPDATE` locking clauses yield
  nothing). Captured by the nightly lint AND candor-scan, propagated transitively, carried
  cross-crate, rendered by `show` as `Db(table,…)`.
- **`allow Db in <scope> <table>…`** (AS-EFF-008) gates it in both backends: case-insensitive
  qualified-name match, `schema.*` covers a schema, an unqualified allow does NOT cover a qualified
  reach. "Billing may only touch `ledger.*`" is now a deterministic CI rule. Both engines (the JVM
  engine shipped the same change in lockstep); the conformance grammar battery covers the new rule.
  This is also the zero-new-engine first step of the database-development transfer (BACKLOG P5).

## [0.3.5] — 2026-06-11 (candor-scan only)

### Fixed — resolution (under-reports recovered)

- **`-> Self` constructors now type their locals.** `let agent = Agent::new_with_defaults();`
  followed by `agent.run(..)` formed `Self::run` (no local def) and silently dropped the edge —
  found by the PROVE-IT dogfood on `ureq`, where 3 public API entry points were missing from a
  16-function blast radius. `Self` in an impl method's return position now resolves to the impl
  type (also un-defeats the ambiguity check: two same-named `-> Self` ctors on different types no
  longer collide as "Self" == "Self").
- **Tuple-struct fields index by position**, so a newtype-wrapped receiver (`self.0.run()`,
  chained `self.0.0`) resolves like a named field.
- README "Misses" updated to the measured blind-spot list (Deref-coercion receivers and
  generic-parameter fields are the ureq residual: 14/16 found, never fabricated); PROVE-IT.md
  prompt aligned (callgraph naming convention, `#[cfg(test)]` scope).

## [Unreleased] (nightly lint)

### Fixed — soundness

- **Method calls on a returned `impl Trait` (opaque) receiver were silently dropped** (bug #33, found
  by dogfooding the site's primary CTA on the `which` crate: `which()` reported `["Env"]` with
  `unresolved: false` while truly reaching `Fs` through
  `which_all(..).and_then(|mut i| i.next())`). Two halves: `devirtualize` now RETRIES resolution under
  a post-analysis typing env (opaques revealed, as codegen resolves) so the call pins to the concrete
  local impl — `which` now carries `Fs` via a real edge to `<WhichFindIterator as Iterator>::next`;
  and `is_dyn_receiver` reveals a local opaque whose hidden type is a `Box<dyn …>` so the dispatch is
  honestly `Unknown`. Teeth: soundness/gen.py `opaque_iter` + `opaque_dyn` forms.

## [0.3.3] — 2026-06-10 (crates: candor-report / candor-classify / candor-scan, lockstep)

Republish so the crates.io artifacts carry the fixes committed after 0.3.2 (the published 0.3.2 had
diverged from the 0.3.2 source tree). Surfaced by a maximum-effort multi-agent `/code-review`.

### Fixed — precision / correctness

- **`candor-classify`: IPv6-aware policy host matching** — `host_part` now keeps a bracketed
  `[::1]:8080` host and a bare `2001:db8::1` intact instead of truncating at the first `:` (which had
  mangled IPv6 endpoints in `allow Net in <scope> <host>` rules into a useless prefix).
- **`candor-scan`: single-codepoint type idents** — the CamelCase test in `type_from_value_path` uses
  `chars().count()`, not byte `len()`, so a one-character non-ASCII type ident (`struct É;`) still
  counts as a single character (a snake/SCREAMING const still yields `None` — honest under-report).

report is bumped in lockstep (unchanged content) to keep the three crates' shared version and their
inter-crate `version =` dependencies resolvable on crates.io.

## [0.3.4] — 2026-06-11 (candor-scan only)

### Added

- **`--policy <file>` / `CANDOR_POLICY` — the stable policy-gate floor.** The published scanner can now
  enforce a spec-§6.2 policy (`deny`/`pure`/`allow`/`forbid`; AS-EFF-006/008/009) over its own scan and
  fail the build, with zero extra install. Explicitly the **advisory floor**: the syntactic backend
  under-reports, so a clean run is necessary, never sufficient — the nightly engine remains the sound
  gate. Shares the §6.2 parser with the nightly and JVM gates (one grammar, everywhere); an unreadable
  policy file exits 2 loudly rather than silently not enforcing.

## [0.3.3] — 2026-06-10 (candor-scan only)

- Metadata release: repository URL moved to `tombaldwin/candor-rust` (the family umbrella now owns
  `tombaldwin/candor`); includes all 0.3.2-era scanner fixes.

## [0.3.2] — 2026-06-10 (crates: candor-report / candor-classify / candor-scan, lockstep)

The "validated everywhere" release: 18 product fixes found by systematic validation (blackout screens,
report-vs-source A/B audits, query property harnesses, fuzzer extensions) since 0.3.1.

### Fixed — soundness / recall (the dangerous direction)

- **`src/build.rs` modules are scanned** (only the crate-root Cargo build script is skipped) — git2's
  `RepoBuilder` module had vanished entirely, so `Repository::clone` reported no `Net`.
- **Struct-literal bindings infer their type** (`let s = S;` / `let s = S{..};` — previously only
  annotated lets), CamelCase-gated; `Enum::Variant` types as the enum.
- **Classifier tiers added:** libcurl FFI (`curl_easy_perform`/send/recv/upkeep + multi pumps → Net)
  + the `curl` consumer crate rule; libgit2 submodule clone/update → Net; `std::path::Path`/`PathBuf`
  stat family → Fs (gix-dir, a directory walker, had reported zero Fs); DB verb dialects — rusqlite's
  canonical API (`query_row`/`query_map`/`execute_batch`/`prepare_cached`/`open`…) had classified
  PURE for consumers, plus `tokio_postgres::query_typed`, diesel `first`/`load_iter`, sqlx `fetch_many`.
- **Report fields:** `spec` (the contract version — required by SPEC §2.1), `unknownWhy`, `entryPoint`
  now emitted by the report crate (published 0.3.0/0.3.1 artifacts predated them).

### Fixed — precision / correctness

- **Callgraph sidecar completeness (SPEC §2.2):** every analyzed function is a key (uncalled leaves
  were invisible to `whatif`/`callers`, conflating "no callers" with "no such function").
- **Name-query matching ladder:** exact > segment-suffix > substring — a precise partial name
  (`Pricing::quote`) no longer silently widens a blast radius to substring cousins (`quote_bulk`).
- **`map` buckets crate-root free functions into `(root)`** per SPEC §6.1 (was one pseudo-module per
  function on flat crates).
- **`diff` fails loud on a prefix matching no reports** (a typo'd current path previously showed zero
  gains — silently passing a gained-effect gate).
- **The shared `CANDOR_POLICY` parser** (SPEC §6.2) — one canonical implementation for the gate,
  `whatif`, and the new `parsepolicy` dump; `deny Unknown <scope>` now parses everywhere.

### Added

- `PROVE-IT.md` — a self-experiment prompt an adopter's agent runs on their own repo (this release is
  its minimum version: earlier published binaries exhibit the since-fixed resolution bugs above).

## [0.3.0] — 2026-06-08

The "enforce, soundly, at scale" release. candor goes from *describing* effects to **enforcing**
architecture-as-code across a whole workspace, makes "never silently under-reports" a set of
CI-enforced fuzzers instead of a hope, and ships a rigorously-measured demonstration that it changes
the code agents *ship*, not just what they report.

### Added — architecture-as-code policy (`cargo candor policy`)

- **Literal allowlists (AS-EFF-008).** `allow <Effect> [in <scope>] <value>…` constrains *which* values
  an effect may reach, checked against the **transitive** literal surface:
  - `allow Net … <host>` — network host allowlist ("billing may only talk to Stripe"), matched by hostname.
  - `allow Exec … <cmd>` — subprocess command allowlist ("build may only run git"), matched by basename.
  - `allow Fs … <path>` — filesystem path allowlist ("config may only read /etc/app"), matched by prefix.
  A model can't self-check these: the literal is buried in a deep, often cross-crate, callee.
- **Module-layering rules (AS-EFF-009).** `forbid <A> -> <B>` — a function in scope `A` must not
  transitively call into scope `B` (the dependency-direction boundary). Follows dependencies **across
  crates**, including ones laundered through a third crate (via per-crate `layerreach` sidecars written
  during the workspace enforce pass).
- **One-command workspace gate.** `cargo candor policy` now snapshots every crate then enforces with the
  siblings loaded, so cross-crate boundaries (effects, hosts, layering) hold in a single invocation.
  Gates on AS-EFF-006 / 008 / 009.
- **`CANDOR_REPORTS`** — a read-only cross-resolution prefix usable in enforcement modes.

### Added — effect detail in the report

- **`hosts` / `cmds` / `paths`** report fields: the statically-visible literal Net endpoints, subprocess
  commands, and filesystem paths a function reaches (the decidable subset; never a completeness claim).
  Propagated transitively and across crates.

### Added — soundness, now a gate

- **Adversarial soundness fuzzers**, all CI-enforced, all teeth-verified (reverting the relevant fix
  turns them red):
  - construction fuzzer — threads a known effect through every call form (closures, `dyn`, generic /
    boxed callbacks, `Arc<dyn>` arbitrary-self-type, macros);
  - cross-crate variant (lib→bin DefPathHash propagation);
  - dynamic oracle — runs each program under `strace` and asserts candor over-approximates the effects
    the kernel actually observed, plus a per-function attribution variant;
  - **drop fuzzer** — threads the effect through a `Guard`'s `Drop` wrapped in random container forms.
- **Implicit `Drop` edges.** candor now reads MIR `Drop` terminators and follows the dropped type's
  reachable local `Drop::drop` impls — including value-embedded fields **and** std owning containers
  (`Box`/`Vec`/`Rc`/`Arc`/`HashMap`/…). An effectful RAII guard (I/O on scope exit) is no longer
  silently dropped from the effect graph. (Found by the Bet 4 MIR spike; see `eval/bet4/FINDINGS.md`.)

### Added — evidence

- **Pre-registered outcome eval** (`eval/bet2/`): when a task tempts an agent to put I/O in a layer that
  must stay pure, candor took the **shipped** violation rate from ~80% to 0% (Fisher p<0.001). Two prior
  pre-registered nulls (floor effects) are reported honestly alongside.
- **Real-world validation** (`eval/realworld/`): the policy + detail features run on candor's own
  non-fixture code; literal extraction matches `build.rs`'s actual `git`/path I/O exactly.

### Changed

- **Policy scope matching now uses the crate-prefixed path** (`<crate>::<path>`), so a layering/allow/
  deny scope spelled as a **crate name** matches that crate's own functions instead of being a silent
  no-op. Module/type-name scopes are unaffected.
- Classifier: `tokio::process` → `Exec`; async runtimes; `time`/`fs_err`/`tempfile`/`glob`/`duct`/
  `dotenvy`; compiler diagnostic emission → `Log`; `rand` verb-gated.
- **crates.io-ready:** vendored `span_lint`, dropping the only git dependency (`clippy_utils`).
- Nightly pin is now auto-bumped weekly by `.github/workflows/nightly-bump.yml` (opens a reviewed PR).

### Fixed

- Soundness holes: `Box<dyn Fn>` called directly, non-local callbacks, and `dyn` behind a smart pointer
  (`Arc<dyn>`) are no longer reported pure; `parse_dph` ICE on a non-ASCII hash.
- Closure-flow: effects propagate through a named function passed as a callback.
- Tooling robustness: the source-freshness hash, the `settings.json` Stop-hook merge, and report
  discovery moved out of fragile (duplicated, drifted) shell into typed, unit-tested `candor-query`
  subcommands (`state`, `reports`, `merge-hook`). The guard no longer fails open; `install.sh` no longer
  risks clobbering a user's settings.

## [0.2.0]

The agent-facing baseline: per-function transitive effect inference, the v0.2 report envelope,
cross-crate propagation by `DefPathHash`, the `cargo candor` wrapper and `candor-query` CLI, the
CANDOR_STRICT / NO_AMBIENT / BASELINE / POLICY enforcement modes, and the Claude Code integration.
