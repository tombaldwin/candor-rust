# candor backlog

Honest priority order within each section. Sources: `CRITIQUE.md`, `EVAL.md`, hands-on findings.

## 2026-09-02 — TWO PRE-EXISTING, PUBLISHED CARDINAL SINS in candor-scan's `#[cfg(test)]` handling —
## MEASURED here, filed not fixed, and the obvious fix for the first is PROVEN UNSOUND

Both are at `origin/main` (3cf055d) as well as at HEAD, so neither belongs to the R105/R106/R107/R119
work. Both were found while checking a reviewer's hypothesis rather than by a sweep, so **the boundary
below is what I measured, not a claim that the class is exhausted.**

### [P0] SIN 1 — a module-level `use` map is built with NO `#[cfg(test)]` filter, so a production scan
### can resolve a call through a TEST MOCK, decided by SOURCE ORDER

`scan_items` (decls.rs, the `for it in items { if let syn::Item::Use(u) = it { collect_use(..) } }`
preamble) and `collect_decls` (same shape, further down) both collect EVERY `Item::Use` unconditionally.
`collect_use` inserts into a `HashMap`, so the LAST spelling of a name wins. The idiomatic mocking pair
is two mutually-exclusive `cfg`s, and which one candor believes is decided by which was typed second.

TWO ARMS, BYTE-IDENTICAL EXCEPT FOR THE ORDER OF TWO `use` LINES. Ground truth EXECUTED — each was
compiled with `cargo run` in a normal (non-test) build and printed `ran=true`, i.e. really spawned
`/usr/bin/true`:

    #[cfg(not(test))] use std::process::Command as Runner;     //  ARM "first": mock import FIRST
    #[cfg(test)]      use crate::mockproc::Runner;             //  ARM "last" : mock import LAST
    pub fn run(p: &str) -> bool { let mut c = Runner::new(p); c.status().map(|s| s.success()).unwrap_or(false) }

    arm "first"  ->  run: ["Exec"]
    arm "last"   ->  run: ABSENT from functions[]        <- silent under-report

Measured identically on `origin/main` and on HEAD. The `crate::mockproc::Runner` mock is pure by
construction, which is what makes this the worst shape of the class: the answer is confidently pure.

**THE ONE-LINE FIX IS UNSOUND — PROTOTYPED, BUILT AND A/B'D, THEN REJECTED.** Adding
`if !include_tests && is_cfg_test(&u.attrs) { continue; }` to both loops does close the fixture (both
arms report `["Exec"]`, order-independent) and 326+77 tests stay green. But over all 1489 registry
crates against HEAD it is **ADDED 0, REMOVED 11, CHANGED 39 (wide, 15 fields) in 7 crates**, and the
removals audited in full contain a real loss: `curve25519-dalek` 4.1.3's `scalar::Scalar::random` loses
`["Rand"]` and the row disappears. Cause is SIN 2 below — `is_cfg_test` fires on
`#[cfg(any(test, feature = "rand_core"))]`, which is a PRODUCTION import whenever that feature is on.
So **SIN 2 must be fixed first**, or fixing SIN 1 trades one silent under-report for another. The rest
of that A/B: `tokio` 1.53.1 loses a fabricated `Log`+`Unknown` across its whole `fs` module (35 rows,
`Fs` correctly retained — this looks like the intended gain), `hickory-resolver` GAINS `Clock` on five
rows, and `cap-primitives`/`ordered-float`/`similar` move on test-only functions.

**THE SAME QUESTION IS ANSWERED FIVE TIMES AND TWO OF THEM DRIFTED** (brief §F1 q3). Sites that DO apply
the filter: `collect_module_glob` and `collect_reexports` (both decls.rs). Sites that do NOT:
`scan_items`, `collect_decls`, and `lang.rs`'s `collect_root_reexports` — which has no `include_tests`
parameter at all, so fixing it needs a signature change. `LocalUseCollector` (a `use` inside a fn body)
is unfiltered too and was not measured.

### [P0] SIN 2 — `is_cfg_test` treats `#[cfg(any(test, …))]` as test-only, so production items under a
### default-on feature vanish from the report entirely

`cfg_meta_requires_test` returns true for a `test` found anywhere inside `any(...)` as well as
`all(...)`. Its own doc says the item "POSITIVELY requires test". For `all(test, X)` that is right; for
`any(test, X)` it is exactly backwards — the item compiles when EITHER holds, so it does not require
test at all. A comment asserting the property the code lacks.

Ground truth EXECUTED (`cargo run` on a default build printed `maybe=true` — a real spawned process):

    [features] default = ["extra"] ; extra = []

    #[cfg(any(test, feature = "extra"))]
    pub fn maybe_spawn(p: &str) -> bool { std::process::Command::new(p).status().map(|s| s.success()).unwrap_or(false) }

    pub fn always_spawn(p: &str) -> bool { /* identical body, no cfg */ }        <- the control

    maybe_spawn  -> ABSENT      always_spawn -> ["Exec"]      (origin/main AND HEAD, identical)

On an ISOLATED crate whose only function is `maybe_spawn`, `functions[]` is `[]`, `analyzed.count` is 0,
and every policy form passes:

    deny Exec  ·  deny Unknown  ·  deny Exec Unknown  ·  deny Exec maybe_spawn  ·  pure maybe_spawn
    -> exit 0, exit 0, exit 0, exit 0, exit 0

`is_cfg_test` gates every item-kind in `scan_items` (free fn, impl, impl method, mod, trait) plus
`declared_item_name`, `lazy_unit_emitted`, `collect_module_glob` and `collect_reexports`, so this is not
a `use`-map issue — the whole item disappears. **102 of the 1489 crate sources in the local registry
contain `#[cfg(any(test, …))]`** (`aho-corasick`, `arrow`, `aws-config`, `axum`, `bstr`,
`curve25519-dalek`, … and candor-scan's own older published versions).

Repro fixtures for both live under this session's scratchpad; each is three files and rebuildable from
the snippets above in under a minute. `include_tests: true` is unaffected by either.

## 2026-08-30 — CARDINAL SIN: a closure/coroutine capturing an effectful Drop by move, boxed as
## `dyn Fn*` and dropped without ever being called, is silently pure — filed, not fixed here

Found during a guard-deletion sweep of `src/lib.rs`'s callback/thread-local/coroutine-capture
machinery (the same area that produced e43eec0 and 3e9848c). Live-reproduced on unmodified HEAD, no
code changes needed to trigger it:

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) { let _ = std::net::TcpStream::connect("10.0.0.2:9"); }
    }
    fn boxed_dyn_fn_scope_exit() {
        let g = Guard;
        let _b: Box<dyn Fn()> = Box::new(move || { let _ = &g; });
    }

`cargo dylint` over this produces zero warnings for `boxed_dyn_fn_scope_exit` — it vanishes from the
report entirely, the identical shape to 3e9848c's `closure_scope_exit` (a `move || {}` captured Guard,
dropped unused) except boxed as a trait object first. The bare (unboxed) case IS caught, correctly —
this is specifically the `Box<dyn Fn*>` path.

**Root cause**, confirmed with a debug probe (not guessed): `mir_spike::local_drop_impls`'s
`TyKind::Dynamic` arm resolves the concrete type behind a `Box<dyn Trait>` by CHA — `tcx.trait_impls_of
(principal)` — and recurses `local_drop_impls` on each impl's self type. For `principal = std::ops::Fn`
this returned 13 non-blanket impls on this fixture, none of them the closure's own type: a closure
satisfies `Fn`/`FnMut`/`FnOnce` through compiler-synthesized dispatch, never a registered `impl Fn for
X` item, so ordinary CHA is structurally blind to it — no amount of widening the trait list fixes this,
the closure is simply not in the set CHA enumerates. The arm's own comment ("most trait objects
(`Box<dyn Error/Any/Fn…>`) have no local impl carrying a Drop, so produce no edge") is wrong for exactly
this case — the CORRECTNESS COMMENT suppressed the measurement that would have falsified it (the review
class from 2026-08-16/29: "when you meet a comment explaining why something is safe, treat it as the
highest-value thing in the file to attack").

**Deliberately not fixed in this session** — this needs new machinery, not a one-line CHA widening, and
a rushed version risks trading this under-report for a fabrication:

- The `Drop` terminator `drop_edges()` walks only carries the STATIC (erased) type of the dropped
  place — by the time a `Box<dyn Fn()>` local reaches its `Drop` terminator in MIR, the concrete closure
  type used to construct it is already gone. There is no CHA-style trick that recovers it from the drop
  site alone.
- **Candidate fix A (bounded, sound, in-function only):** at the *construction* site — an unsizing
  coercion (`CastKind::PointerCoercion(Unsize)` in MIR, or the HIR adjustment) of a local closure/
  coroutine literal into a `dyn Fn*` trait object — record which concrete closure type was assigned to
  which local, and thread that through to the later `Drop` terminator on the SAME local within the SAME
  function body. No flood: it only ever fires for a closure actually coerced into that exact place, so
  it can't leak onto an unrelated `Box<dyn Fn()>` elsewhere (unlike a blanket "assume every closure in
  the crate" CHA-widen, which WOULD flood and is not being proposed).
- **Candidate fix B (charge at construction, rejected):** charge the effect directly to the closure's
  enclosing function at its construction/coercion site, skipping the drop-site tracking entirely. Sound
  within a single function, but wrong across functions: `fn make() -> Box<dyn Fn()> { let g = Guard;
  Box::new(move || { let _ = &g; }) }` constructs the closure in `make`, but a caller that never invokes
  it and simply lets the return value drop is the one that actually RUNS the guard's effect — charging
  `make` either misses the caller's exposure or (if propagated further) double-charges/mis-locates it.
  Not proposed as the fix; recorded so the next attempt doesn't re-discover it costs a design decision.
- A cross-function-sound version of Candidate A (the return-value case above) needs the coercion fact to
  travel through the return type / field / whatever carries the `Box<dyn Fn*>` onward — genuinely a
  small interprocedural extension, not a local one.

This is a THIRD instance of the "guard captured by a closure that never runs" class in the same file (the
other two — bare `move || {}` and the `async`/`async ||` coroutine forms — are now regression-tested,
see `ui-2021/coroutine_drop.rs` and `tests/integration.sh` 9c-iii). Translate the question once B (or A)
lands: swift's `deinit` and java/kotlin's lambda-capturing-a-`Closeable`-never-invoked shape should be
asked the identical question (rule F, corpus brief) — untested here, not investigated this session.

## 2026-08-29 — `is_pure_std_trait`'s 11-trait allowlist: a MEASURED, real, silent gap in a
## DELIBERATE, tested, documented trade-off — filed, not fixed here

`is_pure_std_trait` (`src/lib.rs`) exempts dynamic/generic dispatch over
Display/Debug/Error/ToString/Clone/PartialEq/Eq/PartialOrd/Ord/Hash/Default — WHEN the trait's defining
crate is the genuine sysroot `core`/`std`/`alloc` (now enforced by `is_real_sysroot_frontier`, this
session's fix for the impostor-crate class, above) — from ever reading `Unknown`, on the stated,
measured rationale that flagging all of them floods reports (`ui/trust.rs`'s `format_error` pins this
choice deliberately: a `&dyn std::error::Error` dispatch produces ZERO diagnostic, by design, not by
omission).

**Nothing stops a REAL (non-impostor) external crate's Display/Clone/Hash/… impl from doing arbitrary
I/O** — unlike `Drop::drop`, which Rust forbids calling explicitly (E0040, checked and confirmed sound
2026-08-29), there is no language guarantee these traits are pure. Live-reproduced with an ORDINARY
`path` dependency (no name games at all): a crate `effectlib` with

    impl std::fmt::Display for Sneaky {
        fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            let _ = std::fs::write("/tmp/sneaky-display-poc", b"hit");
            write!(f, "sneaky")
        }
    }

called from the CONSUMER crate through `&dyn Display` (`show`) and a monomorphized generic
`T: Display` (`show_generic`) — both genuinely non-local, non-CHA-resolvable dispatch, the exact
condition `is_pure_std_trait`'s doc comment describes — produced `"functions": []` for both callers:
total silence, and `deny Fs` exited 0 with no warning at all. `SneakyClone`'s effectful `Clone` impl,
called via `.clone()`, was silent the same way.

**Deliberately not fixed in this session.** A blanket removal of the exemption was already tried and
measured too costly (`ui/trust.rs`'s own comment: "found in the wild: `dyn Error` error-formatting
taints whole call trees") — my repro doesn't invalidate that measurement, it only measures the OTHER
side (the cost of staying silent) that the original decision didn't have data for either. The closest
safe fix — a non-gating disclosure (mirroring `invisible`'s "floored to pure, but flagged" convention,
distinct from `Unknown` so it doesn't reopen the flood) for dispatch through one of these 11 traits when
the concrete impl is non-local and unverifiable — would add a new wire-format signal, which is a
cross-engine SPEC decision (candor-spec is owned by a different agent this session; java/ts/swift almost
certainly share the identical Display/toString/equals/hashCode purity assumption and the identical
exposure). Filing rather than guessing at a fix candor-spec hasn't ratified.

Reproduction kept as a real two-crate fixture (path dependency + consumer), not minimised further —
built this session under this machine's scratchpad, `puretrait-fixture/{effectlib,main}`; regenerate from
the `effectlib` snippet above (a consumer crate calling `show`/`show_generic`/`clone_it` over it) if the
scratch dir is gone. Not covered by any existing conformance row or unit test; `ui/trust.rs` only pins the
ACCEPTED-exempt case (`format_error`), never an effectful one, so this class has zero regression coverage
in either direction today.

## ⟨0.29⟩ hardening round, 2026-08-17 — two MEASURED gaps, deliberately not fixed here

Both found by generating fixtures against the std API surface rather than reading the tables. Filed
rather than patched because each turns on a contract question that should be answered once, not guessed.

- **`[P3]` A socket METHOD on a typed receiver is not classified by the SYNTACTIC FLOOR.** Measured, all 14 probed:
  `UdpSocket::{send_to,recv_from,send,recv,peek_from,peek}` and `TcpStream::{read,write,write_all,
  read_to_end,read_to_string,peek,flush,shutdown}` on a parameter typed `&UdpSocket` / `&mut TcpStream`
  report the enclosing function PURE, while the control `TcpStream::connect(dst)` in the same file is
  `Net`. `deny Net` is therefore GREEN over `fn f(s: &UdpSocket, …) { s.send_to(b, dst) }`; candor-java
  charges `Net` on the identical shape. The same is true of `File::write_all` on a `&mut File`, so it is
  not Net-specific — it is the receiver-typing route for std types generally.

  **NOT a cardinal sin, and the reason matters.** `candor-scan` is the STABLE SYNTACTIC backend and says
  so on every run: *"advisory floor — the syntactic backend under-reports; the nightly engine is the sound
  gate"*, and a violating run prints *"a clean run is necessary, not sufficient"*. Under that contract an
  unresolved receiver is a disclosed limitation, not a false all-clear.

  **THE DEEP ENGINE WAS THEN MEASURED, which is what this entry asked for, and it ANSWERS CORRECTLY**
  (`cargo candor` @3f5bb87 on the same fixture):

      udp_send        { Net }      ← candor-scan said PURE; `deny Net` CHARGES it here
      control_connect { Net }
      tcp_write       { Unknown }  ← disclosed as unresolved, NOT silently pure (AS-EFF-003 / deny Unknown)
      file_write      { Unknown }

  So the sound gate is sound, and where it cannot resolve a trait-method receiver it says so rather than
  certifying. **This is therefore a FLOOR-PRECISION item, not a soundness one — P3, not P0.** The value in
  closing it is that `candor-scan` is what runs on stock cargo in CI, so the advisory floor is quieter
  than it needs to be for the most common socket shapes. The classifier is already correct
  (`std::net::UdpSocket::*` → Net, with a pure-accessor denylist), so any fix belongs in receiver typing.

- ~~**`[P3]` A BIND address is published into `hosts`, a DESTINATION surface.**~~ **CLOSED 2026-08-17** — the bind literal is withheld from `hosts` and no hedge is added (an empty surface fails closed on its own, measured four-way); pinned by `the_net_locator_position_and_the_bind_address_rule`. Original filing: `UdpSocket::bind("0.0.0.0:0")`
  puts `0.0.0.0:0` in `hosts`, so `allow Net 0.0.0.0` reads as *"may talk to 0.0.0.0"* when the code binds
  locally and sends anywhere. Measured three-way and there is NO consensus to copy: candor-ts publishes
  nothing for `server.listen(8080, "127.0.0.1")`; candor-java publishes `127.0.0.1` AND marks
  `incomplete: [Net]`. Whether a local bind belongs in a destination surface is a SPEC question (§2
  `hosts`), and it should be settled in the spec before any engine moves — changing one engine to match
  another here would just relocate the divergence.


> **AUDIT 2026-08-05 — this file had not been touched since 2026-07-09 and four of its claims are
> contradicted by the code.** Checked against the repo, not against the file's own prose, which is the
> method the umbrella backlog's audit note prescribes after that one found 8 of 13 headings wrong.
>
>   · **`macro_rules!` body effects — SHIPPED, not deferred.** Listed below as too risky to attempt; it
>     landed as R48 (`94f333c`, hardened in `8585c42`), and the code is at
>     `crates/candor-scan/src/decls.rs:731`. It recovers the pervasive local-logging-wrapper idiom.
>   · **The "naive fixpoint, a worklist rewrite isn't worth it" note — the worklist SHIPPED.**
>     `crates/candor-scan/src/propagate.rs:19,50` is a worklist, seeded per function, with a documented
>     re-enqueue rule. The 0.23.1 performance sweep did it.
>   · **"MCP tool-set divergence (#4), STILL OPEN" — the integration it names NO LONGER EXISTS.**
>     There is no `integrations/mcp/` in the umbrella; the MCP surface is candor-ts's `mcp.mjs`, shipped
>     through the editor integrations. The item is not open, it is void.
>   · **Envelope divergence (`undeclared`) — decided, not drifting.** `crates/candor-scan/src/scan.rs:1743`
>     emits `undeclared: None` deliberately, because `undeclared: []` would CLAIM the pass ran and found
>     nothing. Absent means not computed. That is the spec's own absence rule, applied on purpose.
>
> Entries below still carrying those claims are stale. The rest of the file has not been re-verified
> line by line — treat an unannotated entry here as a claim, and read the code before acting on it.

> **Review note (2026-06-21).** Landed since this file was last swept (some entries below are now stale —
> see annotations): the `unknownWhy` vocabulary harmonised to the canonical 4 kinds + a conformance check
> (PART 10); the dispatch-frontier (`callers --include-unknown`, spec 0.7) across class/protocol engines
> (PART 9); `containment` + the AS-EFF-010 ratchet added to the cross-engine conformance differential
> (PART 11, Java vs candor-query) and to candor-ts; conformance effect coverage extended to 8 (added
> Rand/Db/Log). The big cross-engine SOUNDNESS result of the period (candor-java only): the
> inherited-into-project silent-pure vein class (active-record / repository / modeled-base-subclass) was
> closed in candor-java and CONFIRMED not shared — candor-ts/scan disclose `Unknown` for the same shape
> (their AST/syntactic models never resolve-to-nothing-then-pure). STILL OPEN here: the κ-treadmill
> dep-tree scanning (P2), the CI self-guard nightly ICE (a rustc bug), and the MCP tool-set divergence (#4 — **VOID 2026-08-05: `integrations/mcp/` no longer exists**).

## Direction — where the value actually concentrates (a critical read)

candor's measured, durable value is narrow and worth protecting from scope creep. Concentrate effort
on the two places the evidence is strongest:

1. **Agent edit-time blast-radius feedback** — the diff / `CANDOR_REVIEW` self-review / MCP / Claude
   Code hook. The pre-registered eval's decisive result (full-propagation reporting **7% → 100%**) is
   here. This is the north star (P0); everything that sharpens the *delta back to the agent* wins.
2. **The effect-regression CI ratchet** (AS-EFF-005) and **policy boundaries** (AS-EFF-006). Low
   adoption cost (no token threading), real felt need ("fail the PR that makes a parser open a
   socket"), unambiguous. Under-emphasised relative to its deployability — promote it.

**De-prioritise** the capability/conformance discipline (token threading, `&Fs`/cap-std migration):
elegant, but realistically almost no team rewrites a real codebase for it, so it stays the deep,
optional tier — not a place to spend polish.

**Honest ceiling to keep in view (don't paper over):** correctness is bounded by a *curated
classifier*, so the failure mode is a silent *under*-report ("network-free" when it isn't) — the most
dangerous direction. The mitigation (`Unknown` / coverage-gaps / version-trust) is candor's best
asset; keep widening classifier coverage and never trade the honesty for a cleaner-looking number.
The efficiency claim ("cheaper than reading source") depreciates as models get cheaper/longer-context;
lead with blast-radius, which is a reasoning gap, not a cost gap.

### Non-goal: a "candor score" / cross-codebase grade (decided — do not build)

A single aggregate score (incl. the tempting "resolvability = 1 − unresolved%") was considered and
**rejected.** It conflates *analyzability* with *quality* (penalises DI / plugins / event-driven —
good designs); drifts with the *engine* version, not just the code (this session's resolution work
would move it for unchanged source); is gameable in the harmful direction (de-abstract a seam to
satisfy the analyzer); and collapses the actionable signal (the *list* of unresolved fns) into an
ambiguous digit that cross-codebase comparison would read as "team A writes worse code" when it means
"team A uses Spring." The right primitives are a **ratchet** (don't regress) + a **map** (the
descriptive profile) + opt-in **discipline counts → 0** (conformance/policy) — candor already has all
three. Resolvability is legitimately useful **inward**, as a metric candor tracks about *its own*
coverage release-over-release — not a developer-facing grade.

## P0 — agent coding: make candor change what an agent *does* (the north star)

This is the point of the rest. The bet: candor's value to a coding agent is **verification, not
context**. An agent can read source and infer most effects itself — the A/B eval (`EVAL.md`) showed a
source-only agent matching/beating the report wherever the classifier has a gap, and over-trusting a
blind spot makes it *worse*. What an agent *cannot* cheaply compute is the **transitive effect delta
of its own edit** across the call graph and crate boundaries — exactly the failure mode agents have: a
*local* edit with a *non-local* consequence (add a `reqwest::get` in a helper → twelve callers now
transitively perform `Net`). candor computes that for free. **Lead with the delta, not the dump.**
(Rests on the P1 correctness foundation — a feedback signal is only worth acting on if it's right.)

- [x] **1. Agent-facing effect diff — `cargo candor diff`.** v1 ships: describes the per-function
      delta vs a baseline (`+ worker { +Net }`) *including the transitive blast radius* (a network
      call added in `worker` also shows `+Net` on its caller `main`), flags a new `Unknown`, and has
      `--json` for the agent. v2 ships too: the diff now separates **introduced** (the new effect is
      in the function's own `direct` set — the source) from **inherited** (transitive), with a
      headline `Fs: introduced in Cache::get → inherited by 6 caller(s)`. Remaining polish: the exact
      call-site location (`@ foo.rs:12`) — `explain` (§3) has it; emitting it in the report is the step.
- [x] **2. Close the loop in the agent's edit cycle.** Opt-in `CANDOR_REVIEW=1`: the Stop hook diffs
      the fresh report vs the baseline and, on a newly-introduced effect, feeds the delta *back to the
      agent* (`decision:block` + `additionalContext`) as a self-review checkpoint; `AGENTS.md` §5 tells
      the agent how to respond. Triple loop-guard: a once-per-effect `review-seen` marker,
      `stop_hook_active`, and Claude's 8-block cap; off by default. This is what makes candor *change
      behaviour*, not just inform. (Tested: candor-run exit-11 + stop-hook block/no-block.)
- [x] **3. `explain <fn>` — effect provenance** (the P2 item below). `cargo candor explain <fn>` traces
      the call path to where each effect originates: `main → middle → leaf` with `leaf via
      std::net::TcpStream::connect at main.rs:1`. For scoping (what flows through here before I edit)
      and to answer the diff's "why". Engine records effect *sites* (callee + location) under
      `CANDOR_EXPLAIN`; a BFS finds the nearest source.
- [x] **4. Speed for a tight loop** — done as P0′ §8 (fast `diff`, `watch`, instant queries,
      incremental re-lint). A full re-lint (~minutes on a big crate) is too slow per edit;
      need an incremental path, or at least a diff against the cached report that re-lints only the
      changed crate(s).
- [x] **5. Measure it — don't assume** (pilot; `EVAL.md` Trial 5). Pre-registered with/without eval on
      a non-local-effect trap (`eval/minicache`), blind-judged. Result: **treatment 4/4 vs control 1/4**
      fully identified the transitive blast radius (control 3/4 only gestured "callers/perf"). Honest
      bounds: candor makes non-local propagation *complete & explicit* (the axis it targets), but did
      NOT catch what agents miss entirely — control independently found path-traversal/TTL/error bugs
      candor doesn't. Pilot caveats: N=4/arm, one task, one capable model. **Still to do:** multi-task,
      multi-model study quantifying end-to-end *edit-quality* gains (not just awareness) before scaling.

### P0′ — where to take it next (post-eval reassessment)

The eval reframed the goal. AI agents fail at code in a way candor is positioned for: they **don't
hold the global architecture in their head**, so they put I/O in the wrong layer, break a purity
boundary, or give a function an effect it was never meant to have. candor sees the whole effect graph;
the agent doesn't. Lean into *that* asymmetry — not into restating effects the agent already sees
locally. (The eval also showed the *guard* is the dependable value, that candor misses the
security/correctness bugs that often matter most, and that its value scales with codebase size.)

- [x] **6. Effect policy / architectural invariants — shipped.** `CANDOR_POLICY` / `cargo candor
      policy` enforces a declarative `.candor/policy`: `deny Net Db Fs domain`, `pure parse`,
      `deny Exec`. Each rule checks a function's **transitive** effect set, so it catches a layer
      reaching an effect *through a helper* (`AS-EFF-006`) — the architectural violation an agent
      can't see from a local edit. Tested: parser unit test + integration (transitive violation fires,
      genuinely-pure fn doesn't). Spec'd (AS-EFF-006 in SPEC/SEMANTICS); `examples/candor-policy`.
- [~] **7. Effects → *risk* (argument provenance) — v1 shipped (heuristic).** `CANDOR_TAINT` /
      `cargo candor risk` flags `AS-EFF-007`: an injection-class effect (Fs/Exec/Db/Net/Env/Ipc) whose
      argument *syntactically derives from a function parameter* — `fs::read(format!("/x/{key}"))`,
      `Command::new(name)`. Catches the path-traversal/command-injection class the eval exposed; a
      literal-arg effect is not flagged. **Honest limits (the `~`):** intraprocedural + syntactic — it
      misses flow through struct fields and across functions, and over-flags a validated parameter; it's
      advisory (exit 0), never a gate. Tested (param-derived fires, literal doesn't). **The real frontier
      remains:** interprocedural, field-sensitive data flow (a MIR-level pass) for sound taint.
- [x] **8. Speed — separate the slow analysis (one compile per change) from instant queries.** The
      principle: the analysis only changes when the code does, so compile once off the critical path
      and serve queries from the cached report. **Done:** `cargo candor diff` now reads the kept-fresh
      `.candor/report.*` (when its source-hash matches `.candor/state`, maintained by the Stop hook)
      instead of recompiling — ~30s → **0.26s** in the common case; falls back to a re-lint when stale
      (content-hash, so never wrong). Also **done:** `cargo candor watch` — a background poller that
      re-lints on a real source change and stamps `.candor/state` only on a successful build, keeping
      the report fresh off the critical path so `diff` is instant even without the Stop hook (the
      compile runs concurrently with editing). Also **done:** instant read-only queries served from
      the fresh report (no recompile) — `cargo candor show <fn>` (its effect set, `*`=direct) and
      `cargo candor where <Effect>` (functions performing it, split direct-source vs inheritor),
      both `--json`. This is the net speed *win*: the agent answers "what does X do / what touches
      Net" in one ~0.5s call instead of grepping and tracing source. Also **done:** a forced re-lint
      (explain/policy/risk) is now incremental — `lint_fresh` tries an incremental build and only
      clears `target/dylint` on a pure cache hit (detected via cargo's "Checking"/"Compiling" line),
      so a re-lint recompiles just the changed crate instead of the whole tree.
- [x] **9. Selectivity — surface only the *consequential* propagation.** `cargo candor diff` no longer
      lists every inheritor: it computes, per effect, the **top-level** gainers (those not called by any
      other gainer — the entry point / public API where the effect actually surfaces) from the report's
      `calls` graph, and leads with `Fs: introduced in Cache::get → reaches main (+5 intermediate)`. The
      list shows the source and the top-level endpoints; the in-between plumbing is collapsed to a count.
      `--json` still carries everything. Cuts the noise on a wide blast radius to the functions that
      matter. (Could extend to flag "reaches a policy-forbidden fn" — ties to §6.) Tested (3-hop chain:
      source shown, `main` tagged top-level, `mid` collapsed).

- [x] **10. Realize the speed/cost savings — make the agent *use* the fast queries.** §8 made queries
      instant; this is about the agent reflexively reaching for them instead of grepping/reading.
      **Done:** `cargo candor callers <fn>` — instant reverse-dependency lookup ("who calls this?", the
      most common pre-edit grep), served from a new effect-relevant `calls` field in the report. Also
      **done:** an **MCP server** (`integrations/mcp/candor-mcp.py`, no SDK) exposing the query set
      (`candor_effects`/`where`/`callers`/`diff`) as native tools, so an MCP agent calls candor
      reflexively in one cheap call (CLI is the fallback) — the leverage point converting "candor *can*
      answer fast" into "the agent *skips* reading files". Also **done:** `cargo candor map` — a
      compact module→effects overview (`app { … } (80 fns)`) to front-load understanding at session
      start without grepping. Caveat: keep the tool surface small — over-querying adds round-trips.

**Not worth doing:** more interactive-loop polish (call-site line, prettier output) — the eval says
that's the narrow, modest-value axis. Diminishing returns.

## P1 — correctness (silent wrong answers are the worst failure)

- [x] **candor-scan silent-pured an UNCALIBRATED crate reached ONLY via a MACRO — no blind disclosure**
      (FOUND 2026-06-17 by the non-syscall recall corpus, case log_slog; FIXED same day — `visit_macro` now
      records ANY crate-qualified macro path, so a macro reach into a declared-but-unmodeled dep is DISCLOSED
      blind at attribution (slog::info! → invisible), the same honest Unknown a normal call gets. Verified
      ZERO over-disclosure: candor-query invisible count identical before/after, 4-way conformance unchanged.
      recall corpus now 14/14 honest, no tracked exceptions.) `slog::info!(logger, "m")` reads pure: slog isn't calibrated, AND because the reach is a
      MACRO, `visit_macro` never records it (it only records a macro it can classify), so it never reaches
      the κ-ledger / `invisible` disclosure either — whereas the SAME crate reached via a normal call WOULD
      be disclosed (cf. net_minreq → `invisible`). So it's silent, not honest-Unknown. DISTINCT from the
      builder-chain family (that's a recall miss on CALIBRATED crates; this is a DISCLOSURE miss on
      UNCALIBRATED ones). FIX: `visit_macro` should record/disclose a macro call to a DECLARED-but-unmodeled
      dep as a blind reach (Unknown/invisible), so an uncalibrated macro reads honest-uncertain not pure.

- [x] **candor-scan silent-pured `duct::cmd!(...).run()` — macro-result receiver typing hole** (FOUND by
      the real-world dynamic oracle 2026-06-17; FIXED same day, `scan_builder_entry_effect` in candor-scan —
      over-approximate the duct entry `cmd`/`sh` → Exec for the syntactic engine; shared classifier + deep
      engine unchanged. The generalization below stays open as a watch-item.) Original finding: the
      syntactic scanner can't type the result of the `cmd!` MACRO, so the chained `.run()` doesn't resolve
      to `duct::Expression::run` and its Exec is dropped — the program spawns a process (kernel-confirmed
      via strace) yet reads pure+certain. The deep engine catches it (typed `.run()`, lib.rs:3675), so this
      is scan-only. SAME macro-blindness family as the log-macro bug. FIX is a deliberate cross-engine call:
      (a) over-approximate the SHARED classifier — `duct::cmd`/`sh` entry → Exec (safe-direction, candor's
      never-under-report bias; but degrades the deep engine's builder-discipline precision + breaks its
      `duct::cmd → None` test), OR (b) a SYNTACTIC-MODE macro classifier candor-scan uses for entry macros
      the typed classifier leaves to verbs (keeps the deep engine pristine; more infra — and note scan's
      effect attribution runs through `classify()` at the call-join, so a scan-only path needs threading
      there too). Likely generalizes beyond duct (any builder-macro whose terminal verb carries the effect).

- [x] **candor-scan silent-pured BUILDER-CHAIN verbs whose entry it can't type — `ureq::get(url)….call()`**
      (FOUND by the real-world oracle 2026-06-17; FIXED same day by GENERALIZING `scan_builder_entry_effect`
      into an entry→effect TABLE — duct `cmd`/`sh`→Exec + ureq `get`/`post`/…→Net; the oracle drives new
      rows. Closed duct + ureq together; the table is the systematic "generalize" fix.) Original finding:
      candor-classify puts
      ureq's Net on the `.call()` VERB (lib.rs:348); the syntactic scanner can't type the chain from the
      `ureq::get()` entry, so `.call()` is missed — and because ureq is CALIBRATED it isn't disclosed blind
      → silent-pure (worse than the uncalibrated case, which discloses). This is the SAME family as duct but
      via a FUNCTION entry, not a macro — so it CONFIRMS the family generalizes, and the real fix is broader
      than the duct point-patch: over-approximate builder-chain ENTRIES generally for the syntactic engine
      (extend `scan_builder_entry_effect` — e.g. `ureq::get`/`post`/`request` → Net — and ideally drive it
      from a small data table of "entry → effect where the typed classifier defers to a terminal verb").
      The deep engine catches all these via the typed verb, so it stays precise. The systematic version of
      this is the "generalize the macro-blindness fix" backlog direction.

- [~] **Soundness fuzzer — "never silently under-reports" is now a CI gate, not a hope** (`soundness/`,
      **Bet 1 phase 1** of the improvement roadmap). A hand review found multiple trust-contract
      violations the unit/integration suite missed (`Box<dyn Fn>` callbacks, non-local callbacks,
      `Arc<dyn>` dispatch — candor reported them PURE instead of `Unknown`). The fuzzer generates crates
      that thread a known effect through every call form known to under-report (closures, `dyn`
      dispatch, generic/boxed callbacks, the receiving side) and asserts every reachable function is
      `effect`-or-`Unknown`, never silent-pure. **Verified to have teeth:** reintroducing the historical
      `resolve_callee` `_ => None` hole makes every `recv_boxed` seed fail. **Phase 2 shipped — a
      dynamic oracle** (`soundness/oracle.sh`): RUNS each generated program under `strace`, confirms the
      effect executed (its marker in the trace, filtering runtime startup syscalls), and asserts
      candor's static prediction (`main`'s transitive `inferred`) over-approximates the kernel-observed
      effect — ground truth that trusts nothing about the generator. CI runs 60 construction seeds + 40
      oracle seeds/push (oracle is Linux-only; `oracle_check.py` decision logic unit-validated).
      **Extended:** a **cross-crate variant** (`run_cross.sh` — a lib→bin boundary exercising
      `DefPathHash` cross-crate propagation; teeth-verified by disabling `load_cross_reports`), plus
      **macro forms** (a call inside a `macro_rules!` expansion, and a macro-DEFINED `sink` testing the
      #5 macro-fn-visibility fix; teeth-verified by reverting it). CI now runs 60 construction + 40
      cross + 40 oracle seeds/push. **Now also:** **arbitrary-self-type trait-object forms** (`arc_dyn`
      + a cross-crate `Arc<dyn LibTrait>` variant — #4 teeth: reverting `is_dyn_receiver` fails the
      non-local arc seeds), and **per-function oracle attribution** (`oracle_pf.sh` — instrument the
      chain with `eprintln` entry/exit markers, reconstruct the call STACK at each effect syscall, and
      pin a runtime-proven effect to the EXACT on-stack function, not just `main`). CI runs 60
      construction + 40 cross + 40 whole-program-oracle + 40 per-function-oracle seeds/push.
- [x] **Database clients.** `sqlx`/`rusqlite`/`postgres`/`tokio_postgres`/`diesel`/`redis`/… now
      classified `Db` (execution verbs only, not query building; best-effort, tune via CONFIG).
- [x] **`tokio::net` / `std::os::unix::net` Unix sockets** → `Ipc`, no longer conflated with `Net`.
- [x] **`tokio::process` → `Exec`.** The async mirror of `std::process` (which was covered) — spawning
      a subprocess via `tokio::process::Command`/`Child` was classified pure, a silent under-report of
      subprocess execution (the dangerous direction). Closed to match the existing `tokio::fs`/
      `tokio::net` coverage; `tokio::process` has no pure data types to over-flag. Unit-tested.
- [x] **`rand` over-report fixed — now verb-precise.** Whole-crate `rand` → `Rand` flagged its *pure*
      distribution constructors (`Uniform::new`, `Normal::new`) and deterministic-seed constructors
      (`seed_from_u64`) as effectful. Now matched to the calls that actually consume randomness — the
      entropy sources (`OsRng`/`thread_rng`/`rng`/`from_entropy`) and generation verbs
      (`gen*`/`random*`/`fill*`/`sample*`/`next_u*`). `getrandom`/`fastrand` stay wholesale (effectful
      end-to-end). Removes the false positives without losing the generation coverage. Unit-tested.
- [x] **const/static initializers** now reported (a `static X = effectful()` performs its effect),
      with macro-generated items (e.g. tracing `__CALLSITE` statics) filtered out via
      `span.from_expansion()` — that filter was needed; without it the report flooded.
- [x] **memmap2** → `Fs`.
- [x] **Trait dispatch (dyn + generic) over local traits** — broke through with Class Hierarchy
      Analysis: a call to a locally-defined trait method now adds edges to all impls, so effects
      propagate through `dyn` and generic dispatch (sound over-approximation). On ebman this resolved
      the LLM feature and dropped `Unknown` 100→92, of which only 6 are now *purely* Unknown.
      `CANDOR_PARANOID` remains the opt-in for the residual *non-local* generic-dispatch gap.
- [~] **Closures / callbacks — the statically-resolvable half is closed.** A **named** function
      passed as a value (handed to a combinator — `iter().map(parse)` — `thread::spawn`, stored as a
      callback, registered) now adds a `caller → fn` edge, so its effects propagate even though the
      *call* happens inside unseen library code. Previously dropped (the `Path`-to-`FnDef` value
      wasn't a Call), a silent under-report inconsistent with inline closures (already charged
      lexically). Over-approximates in the safe direction; local targets only; ui-tested. **Residue
      (deferred, the genuinely MIR-hard part):** the *receiving* side — an `impl Fn`/fn-pointer
      **parameter** invoked inside a function — keeps an honest `Unknown`, because pinning its
      concrete target needs interprocedural closure-flow (effects riding in function types), a
      MIR-level pass. An effectful *inline* closure already lands on its lexical owner, so the live
      hole is now just "effect arrives only through an unknown callback parameter." Small, characterized.
- [~] **stdio / println.** Decided *against* for now: `std::io::stdout`/`println!` is pervasive and
      low-signal (especially for TUIs); would add noise without authority-level value. Reconsider if
      a use case appears. `stdin` (real input) could be added later as its own effect.

- [x] **(candor-java) Validate Scala and Groovy on real bytecode. DONE (2026-06-13).** Scanned
      scala-library 2.13.8 + cats (Scala) and the groovy 4.0.28 runtime + groovy-json (Groovy): both
      parse without crashing; the dynamic surface (Scala broad-collection dispatch, Groovy MOP) lands
      in honest Unknown; real effects attributed to genuine sources. Fixed two engine bugs found en
      route — System.getProperty miscounted as Env, and unbounded CHA over deep hierarchies (now the
      §4 ≤12-or-Unknown bound applies to ALL dispatch). App-level cats = all-Unknown, ZERO fabricated
      effects; groovy-json = 9 real Fs. Promoted to the validated tier in the README. Original item:
- [ ] **(candor-java) Validate Scala and Groovy on real bytecode.** The README claims them by
      bytecode-lowering only; the validation pass Kotlin got (real okhttp/ktor/coroutines bytecode,
      dispatcher entry points flagged, no crashes on coroutine state machines) hasn't been run for
      either. Do the same: one real Scala codebase (lambda/`Future`-heavy — closures lower to
      `invokedynamic`; traits/mixins) and one Groovy one (a Gradle plugin is a natural fixture —
      Groovy's MOP/dynamic dispatch should land in `Unknown`, verify it does rather than silently
      passing). Document the result in the README either way. Until this lands, the marketing page
      (candor.poly.io) deliberately lists Scala/Groovy at lowering-confidence only — promote them to
      the validated tier when it does.

## P2 — depth / precision

- **Under-report seam residuals (deferred from the 2026-06-18 under-report round; candor-scan).** The round
  FIXED the FFI-call→Unknown disclosure + Drop-glue edge (shipped 0.5.11). Deferred, honestly:
  (1) the GENERAL "any unresolvable bare call → Unknown" — rejected for now because it FLOODS (measured 80
  false-positive Unknowns on tokio: closure-param invokes, macro-defined locals, cfg-gated fns, re-exports);
  needs a way to distinguish a genuinely-foreign bare call from a syntactically-unresolved-but-pure local one
  (e.g. seed from extern/glob-import provenance) before widening disclosure. (2) **[STALE — SHIPPED as R48 `94f333c`, decls.rs:731]** bare local `macro_rules!`
  body effects (the transcriber token-soup parse is fragile — risks fabrication). (3) Deref/Index/operator
  implicit-coercion edges (no syntactic call node; needs type-directed coercion the syntactic backend lacks).

- [~] **Model the modern async-HTTP / TLS / QUIC stack as Net (+ a few more) — MOSTLY DONE 2026-06-17**
      (`4c6d66a` found it, calibrated same day: hyper/hickory_resolver/h3/quinn/tokio_rustls/native_tls→Net,
      tokio_vsock→Ipc, rustls_native_certs→Fs, rlimit→Env — verb-keyed + crate-gated in classify (num_cpus
      was initially modeled Env but REVERTED to pure: it's a near-pure CPU-topology query, std's equivalent
      thread::available_parallelism is pure, and Env would spray over every thread-pool ctor);
      on oha: Net 30→40 fns, disclosed deps 32→25, fabrication-probe clean. REMAINDER: hyper_util +
      native_tls calls in oha stay disclosed because they're a bare `.request()`/`.connect()` on an UNTYPED
      builder receiver — the syntactic scanner can't form `hyper_util::…::request`, and (unlike ureq::get) the
      entry is a constructor-then-method, so neither the verb rule nor scan_builder_entry_effect catches it.
      That's the candor-scan builder-chain-receiver-typing boundary; honest — disclosed, not silent. The deep
      engine catches all of these via typed resolution.) Original finding: (`soundness/realworld/` method: scan a real corpus, read candor's κ-ledger
      DISCLOSED-dep list, have an independent classifier pin each to effect-or-pure). Run on `oha`: candor
      honestly DISCLOSES (invisible, NOT silent) but doesn't MODEL a stack that is candor's core value (Net):
      **hyper, hyper_util, hickory_resolver (DNS), h3, quinn, h3_quinn, tokio_rustls, native_tls** → Net
      (high-confidence; the lower layer reqwest/ureq build on — the hand-picked recall corpus missed it).
      Also: **tokio_vsock** → Ipc; **rustls_native_certs** → Fs (loads OS trust store); **rlimit** → Env
      (num_cpus considered but left PURE — topology query, not Env). Low-confidence (verify the called API):
      humantime (Clock via now-formatter), clap (Env via args).
      The classifier CORRECTLY left url/kanal/serde_json/bytes/ratatui/http/aws_sign_v4/rand_regex PURE
      (disclosure right). These are honest-but-incomplete (P2 precision), not silent under-reports. Calibrate
      verb-keyed (like reqwest) + add the builder ENTRIES to scan_builder_entry_effect so candor-scan catches
      them too. NOTE: the DEEP engine has the same gap (shared classifier), so this helps both.

### The κ-treadmill endgame: dep-tree scanning (κ → builtins-only frontier)

The curated classifier's structural weakness is that κ tries to know the PACKAGE ECOSYSTEM. The
2026-06-11 ledger work (the receipt now NAMES unlisted deps the code calls — Rust via Cargo.toml,
TS via resolved imports) makes the blind spot visible; the endgame is to make most of it
*derivable*: **scan the dependencies themselves and chain the reports** (the CANDOR_DEPS/hash
mechanism that already exists, spec §2). A dep's effects derive from ITS calls into the builtin
frontier (std/node:/java.*), which is bounded and slow-moving — κ then only has to know builtins +
the FFI floor, and the treadmill becomes a cache-refresh. Pieces: (1) candor-scan already analyzes
unbuilt third-party crates from `~/.cargo/registry/src` — add a `--deps` mode that scans the
lockfile's tree and emits sibling reports; (2) candor-ts: scan node_modules dist JS via --allow-js
(half the ecosystem ships JS — its `require("fs")` calls are exactly the builtin frontier);
(3) JVM: analyze the dep jars on the classpath, not just project classes (it's the same ASM pass).
**SLICE 1 SHIPPED (2026-06-11 late):** candor-scan now emits `hash` (`crate#qual`) and CONSUMES
`CANDOR_DEPS` (files/dirs of sibling reports; unambiguous tail2-then-leaf join; stale-version →
Unknown per §2.1; inherited effects + all four literal surfaces; a chained dep counts as covered in
the κ ledger). End-to-end verified: the ledger names the blind spot → one dep scan closes it →
`settle` inherits Db+tables, `check_rates` Net+host across the crate boundary, on stable. The JVM
ledger and TS interface-CHA also landed same-day. SLICES 2+3 SHIPPED (same night): `--deps` walks Cargo.lock, scans every
registry dependency's unbuilt source in-process (the self-gate's own `deny Exec` forbade the
spawn-yourself shortcut) into `.candor/deps/`, then chains the root scan — MEASURED on pgman:
328/328 deps in 75s one-time (~0.23s/dep, cached thereafter), the κ ledger dropping 12 unlisted →
1 (the path dep, the documented skip), 7 fns gaining effects through the chain. An all-pure dep's
EMPTY report registers as covered (its emptiness is the purity claim — serde_json was briefly
misnamed a blind spot). TS CJS units shipped too (`module.exports`/`exports.foo`/object-literal
exports are units named the way consumers resolve; jsonwebtoken 4→8 fns, sign reads Clock; the
@types/<pkg> join mismatch fixed). REMAINING: JVM classpath-jar scanning + chaining verification;
path/git deps need manual scans (no registry checkout); dist-JS minified bundles unprobed.

### From the critical-assessment pushback — "fundamental" was too strong (these are buildable)

These were dismissed as hard limits; they're really *expensive or risky*, not impossible. The one
genuine floor is undecidability (no sound+complete effect set in general) — and the answer there is
sound over-approximation (`Unknown`), which candor already does.

- [~] **Closure-flow — the `impl Fn` *receiving* side: BOUNDED SLICE SHIPPED; full MIR pass deferred**
      ([docs/closure-flow.md](docs/closure-flow.md)). **Shipped (no MIR needed):** a free fn that
      invokes a callback param defers its `Unknown`; `check_crate_post` resolves it from the concrete
      fns passed at the HOF's call sites — all **named** → edge the HOF to them and drop the redundant
      `Unknown` (effects + host detail propagate *through* the HOF, "CHA for callbacks"); any
      **closure / fn-ptr**, or **never called locally** → the sound `Unknown` stands. Bounded to free
      fns (arg index == param index) and named callbacks (a closure's effects are already captured
      lexically on its definer). Integration-tested (resolved drops Unknown; closure-passed keeps it).
      **Residue (deferred, the genuinely MIR-hard part):** methods (self offset), and removing the
      `Unknown` for closure callbacks (needs un-folding the lexical charging / per-instance MIR). The
      original scoping + the trigger for the full pass are in the note. A function that *invokes* a
      callback parameter keeps an honest `Unknown` (its target isn't pinned at HIR). Scoping it
      surfaced the decisive fact, **measured** not assumed: the effects are **not missed** — an inline
      closure's body is charged lexically, and a named fn passed as a value gets a call edge (the
      passing side, already shipped), so `Net`/`Fs`/etc. land correctly on the definers and their
      callers. The *only* residue is the `Unknown` the HOF (and its callers) carry — **sound**, and in
      the common case **redundant** (the real effect is already captured elsewhere). So this isn't a
      soundness fix; it's *reducing redundant `Unknown` noise* on higher-order call paths. Three
      buildable routes exist (effect-polymorphic signatures / per-monomorphization MIR via the
      `Instance::try_resolve` we already use / closure-flow dataflow), none a different project — but
      the payoff is precision on a safe base. **Recommendation: don't build yet**; the trigger is a
      measurement — if `audit`'s `Unknown` count on real callback-heavy crates traces mostly to HOF
      invokers (not genuine dynamic dispatch / FFI), prototype the per-mono route behind a flag against
      a 3-shape fixture corpus. *(Pushback was right that "needs a MIR engine" framed a road as a wall;
      scoping then showed the road isn't worth walking until the noise is shown to matter.)*
- [x] **Macro-generated effects — narrow the blanket filter. DONE + VALIDATED (2026-06-13).** The
      blanket skip was already narrowed (skip macro-generated consts/statics — where the tracing
      `__CALLSITE` static flood lives — but ANALYZE macro-generated FUNCTIONS). Validated on a real
      macro-heavy crate (tracing + async_trait + serde derive): an `async_trait` method's inner I/O
      shows Fs; a `tracing`-using fn that ALSO does Fs shows both (not hidden); a pure tracing fn shows
      only Log (correct — it IS a log framework); a `#[derive(Serialize)]` struct is pure with no false
      effects. The DefKind heuristic aligns with the noise/real boundary in practice (the flooding
      macros generate statics; the fn-generating ones — async_trait/serde — are real), so the
      ExpnData-identity allowlist below is NOT needed: it would add complexity + re-flood risk for no
      measured benefit. (Validation answered the "tune against a corpus" question: current is sufficient.)
- [ ] **Macro-generated effects — narrow the blanket filter (original framing, superseded above).** Today `span.from_expansion()` skips
      *all* macro output (added because compiler-internal/`tracing __CALLSITE` expansions flooded the
      report). But every expansion carries its macro's identity (`ExpnData`/`DefId`), so we can filter
      **only known-noise macros** and analyze the rest (so an `async_trait`/derive/decl-macro that
      generates effectful code becomes visible). The blocker is *re-flooding risk*, which needs a
      real-codebase corpus to tune — a **validation problem, not an impossibility**. *(Pushback: the
      user-vs-noise distinction is decidable from the expansion's identity.)*
- [x] **Literal `Net` host detail — shipped (engine + report + spec).** Full host-by-runtime-value is
      undecidable, but the **literal** subset is extractable. **Done:** a directly-classified `Net`
      call carrying a string-literal address/URL records the host (`host[:port]`, scheme/path/userinfo
      stripped), propagated on the same call graph as `fs` detail (`propagate` is now generic over the
      element), surfaced as the report's optional `hosts` field and rendered by `show` as
      `Net*(api.example.com)`. Honest by construction: a runtime address yields `Net` with **no** host
      (absent ≠ "no network"); a header value / verb is filtered out (`net_host_literal` requires a
      dotted name or `host:port`). Spec'd in candor-spec §2 (never a completeness claim — `Net` keeps
      the I/O claim; `hosts` only narrows it with what's provably visible). Unit + integration tested.
      **Follow-up:** candor-java parity; more host-bearing APIs (e.g. URL builders) if a real case wants
      it. *(Pushback was right: "not statically knowable" only held for the runtime subset.)*
- [ ] **Smarter generic-dispatch over foreign types** (currently assumed pure to avoid flooding;
      `CANDOR_PARANOID` is the opt-in). A *choice*, not a limit: could assume-pure only when the
      dispatch's trait bounds **exclude** all known-effectful traits, and mark `Unknown` otherwise —
      tightening the default without the paranoid-mode noise. *(Pushback: a tunable tradeoff, not a
      wall.)*

- [x] **Entry-point handling in strict mode.** `main` no longer raises AS-EFF-001 (it's the root
      that legitimately holds the whole capability bundle).
- [ ] **Reachability / dead-code elimination.** CHA + the new named-fn-callback edges made the call
      graph much more complete (edges through local trait-object/generic dispatch, and through fns
      passed by name to combinators/`spawn`/registries), but it's still missing the *unknown-callback*
      `impl Fn`/std-`dyn` edges, so reachability would *still* mislabel some callback-reached code as
      dead. Closer to soundness than before, but not there yet — **deferred** until that residual
      closure-flow gap closes.
- [~] **Finer `Fs` granularity (read vs write).** **Non-breaking refinement shipped:** each report
      entry now carries an optional `fs: ["read"|"write"]`, derived from the verb of every
      directly-classified `Fs` call (`fs::write`→write, `File::open`→read, `fs::copy`→both;
      `OpenOptions::open` left unannotated since its direction is runtime-flag-decided) and propagated
      through the call graph in a separate fixpoint that never touches the effect set. The `Fs` effect
      itself is unchanged, so **no baseline regresses** (verified: the self-guard stays clean) and the
      field is omitted when unknown. `cargo candor show` renders it (`Fs*(write)`, `Fs(read,write)`)
      and `show --json` exposes it. **Still deferred (the breaking part):** splitting `Fs` into
      first-class `FsRead`/`FsWrite` *effects* with a capability-subtyping relation (`Fs ⊇ FsRead,
      FsWrite`) — that needs the vocabulary + token-subtyping work and *does* break committed baselines
      (`Fs`→`FsWrite` reads as a gained effect → spurious AS-EFF-005). Cross-crate `Fs` carries no
      detail (the dependency's report doesn't record it). (Net-by-host is *not* statically knowable —
      won't do.)
- [x] **Cross-crate effect propagation** (CRITIQUE §8 — closed). Each report entry carries a stable
      `DefPathHash`; a dependent crate loads its dependencies' reports keyed by it (surviving
      reexport-shortened paths) and inherits their *already-transitive* effects. Fixed a real consumer
      whose `bin` under-reported the `Db`/`Net`/`Exec` it performs through its `lib`.
- [x] **Devirtualize concrete trait calls** (CRITIQUE §9 — closed). A method call on a concrete
      (non-`dyn`) receiver resolves to its single impl instead of CHA-expanding to *every* impl —
      removing the over-report where a pure `self.applies()` inherited a sibling rule's effect
      (104 fns de-over-reported on gitui, no soundness loss).
- [x] **`cargo candor explain <fn>` — effect provenance.** Traces the call path that gives a function
      each effect: `main` has `Net` *because* `main → middle → leaf`, and `leaf` calls
      `std::net::TcpStream::connect` at `main.rs:1`. Turns an effect *set* into a story you can follow
      to its source. Engine records effect *sites* (callee + span) under `CANDOR_EXPLAIN`; a BFS over
      the call graph finds the nearest source per effect. (Cross-crate and unresolvable calls are
      labelled as such — the path stops at the boundary.) Used by P0 §3.

## P3 — real enforcement

- [x] **Recognise cap-std capability types** in `declared_caps` (and its operations in `classify`):
      a project on cap-std now gets conformance against real, unforgeable capabilities for free,
      with candor as the visibility layer. Validated in `sample-capstd/`. Compile-time enforcement
      stays cap-std's job. (Mapped: Dir→Fs, Pool/TcpStream→Net, SystemClock→Clock, UnixStream→Ipc;
      extend the small `capstd_cap` table for more.)

## P4 — packaging / maintenance

- [x] Distribution: repo is **public** (git is the channel — `--git` / `git clone`, as AGENTS.md
      uses).
- [x] **crates.io distribution — vendored `span_lint`, dropped the only git dep (the old "can't" was
      wrong).** The prior note said crates.io is impossible because `clippy_utils` is a git dependency —
      but candor used clippy_utils for **exactly one function** (`diagnostics::span_lint`, a thin
      wrapper over `LintContext::emit_span_lint`). **Done:** vendored those few lines (minus clippy's
      docs-page link) into `src/lib.rs`, removed the dep. Verified `0` git sources in `Cargo.lock`;
      `cargo package` now packages candor and stops only on `candor-report` not yet being on crates.io
      (routine multi-crate release ordering), with no git-dep blocker. Added a `version` to the
      `candor-report` path dep so the manifest is publish-ready. ui tests confirm byte-identical
      diagnostics. The nightly + rustc-dev toolchain is still needed to *build* (`rustc_private`) — that
      was never the crates.io blocker. *(Pushback was right: I'd called this fundamental; it wasn't.)*
- [x] Nightly fragility (`rustc_private` pins `nightly-2026-04-16`) — the bump process is now a
      step-by-step in `CONTRIBUTING.md` (pick matching nightly+clippy_utils rev, fix rustc_private
      breakage, re-bless ui, re-baseline the self-guard).
- [x] **Automated nightly bump — shipped** (`.github/workflows/nightly-bump.yml`). The pin can't be
      removed while we're a dylint lint, but the *migration* is now a bot: a weekly (+ manual) workflow
      that pins a candidate nightly, runs the full build/test, **re-blesses the ui fixtures** and
      **re-baselines the self-guard**, and opens a PR if green — or fails loudly and files a tracking
      issue if the nightly broke `rustc_private` (the case that needs a human). Vendoring clippy_utils
      already removed the old "pick a matching clippy_utils rev" step, so a bump is now just "pick a
      nightly, fix any rustc_private breakage". *(Pushback item: "expensive maintenance", not a hard
      limit — now a notification.)*
- [x] Test coverage — unit (pure logic) + `ui_test` fixtures with blessed `.stderr` (copied from the
      framework-saved file, since compiletest has no bless) + scripted `tests/integration.sh`
      (AS-EFF modes, cross-crate, version stamping, audit) + `test-receipt.sh` (the bash receipt).
      **23 unit · 5 ui · 15 integration · 10 receipt**, all gated in CI.
- [x] JSON output via serde (correct escaping for any path/loc; replaced the hand-rolled escaper).
- [x] **Report provenance / versioning.** `build.rs` stamps the source commit + toolchain into the
      dylib (a `#[used]` `candor-build-version=` tag), the report envelope, and the calibrated sidecar;
      `cargo-candor` and the receipt read the *true* dylib version (not the source tree's HEAD), so a
      pulled-but-not-rebuilt engine can no longer masquerade as current and mask a stale baseline.
- [x] **v0.2 self-describing report envelope** `{ candor: {version, toolchain}, functions: [...] }`.
      All readers accept the legacy v0.1 bare array during migration (candor-spec §2).
- [x] **`cargo candor audit` at-a-glance profile** — effect tally, unresolvable-call list, coverage
      gaps, broadest-surface functions; `--all` keeps the full per-function lint.
- [x] **`cargo candor audit --coverage` — make the classifier ceiling auditable** (the principled
      partial-fix for "silent under-report via an uncovered dep", the most dangerous gap). The default
      coverage check only warned for crates matching the `candor-suspect` *name* heuristic, so an
      effectful crate whose name doesn't look effectful slipped through silently. `--coverage` now lists
      **every** external crate candor saw called but has no effect rules for — calls into them are
      assumed pure, so any I/O they perform is under-reported. Can't *eliminate* the ceiling (candor
      can't know an unanalyzed crate is effectful), but converts it from silent to visible — candor's
      honesty thesis applied to its own coverage. Fixed a latent false-positive en route: path-matched
      runtimes (`tokio`/`async_std`/`mio`, matched by module path not crate name) were absent from
      `CALIBRATED_CRATES` and would have been mislabeled blind spots; a new `path_crates` field in the
      `calibrated.json` sidecar marks them covered. Default also gained a one-line count hint;
      `candor-suspect` widened (subprocess/dns/serial/socket families). Integration-tested.
- [x] **candor-java: adopt the v0.2 envelope + first tests/CI — done.** It now emits the
      `{ candor: {version, toolchain}, functions }` envelope (git hash baked in at build time via
      `build-info.properties`, readers still accept v0.1 bare arrays), and ships a 26-check `test/
      smoke.sh` behavioural suite (over real bytecode fixtures) gated in `.github/workflows/ci.yml`
      (JDK 21). Reached near-parity with the Rust impl: cross-jar hash/inheritance + version-trust,
      `calls`, the `show/where/callers/map/diff` query layer (`--json`), fs read/write detail,
      lambdas/method-refs, and constructors.
- [x] **Engine-level version-aware cross-crate trust** (candor-spec §2.1 SHOULD): `load_cross_reports`
      now reads each sibling report's `candor.version`; on a mismatch with the running engine it
      downgrades the inherited effects to `Unknown` (can't trust analysis by rules this engine may have
      changed). Legacy v0.1 reports have no version → trusted as before. Tested (mismatch → Unknown,
      match → effects as-is).
- [x] **De-duplicated the coverage `SUSPECT` heuristic** — now a single `candor-suspect` file at the
      clone root, read by both `candor-run.sh` (via `CANDOR_HOME` / its own location) and `cargo-candor`
      (via `CANDOR_DIR`), with a graceful skip if missing. No more two-copy drift.
- [x] **Ported the tooling/query layer from bash+Python to a Rust CLI binary.** The engine (`lib.rs`)
      *must* be Rust (a `rustc_private` dylint lint), but the wrapper — `cargo-candor`'s diff /
      show / where / callers / audit logic — was bash with embedded Python for JSON. That was the
      fast, zero-install choice, but a recurring *glue*-bug source (the sidecar/report glob collision,
      quoting, state-hash matching) with **duplicated logic** (report-reading re-implemented in nearly
      every Python snippet; the `SUSPECT` heuristic copy-pasted).
      **Done:** a Cargo **workspace** now holds the lint plus two no-`rustc_private` crates —
      `crates/candor-report` (the report structs + envelope-or-bare-array parsing, the single source
      of truth the lint and CLI both depend on) and `crates/candor-query` (the read-only
      `audit`/`show`/`where`/`callers`/`map`/`diff` commands, one typed binary over those structs).
      `cargo-candor` dispatches to it and is now **python-free** (606 → 355 lines, 251 lines of inline
      Python deleted). The port was verified **byte-for-byte** against the Python it replaced
      (identical human output for every command; `diff --json` identical content, now deterministically
      ordered instead of Python's hash-order). Thin bash remains only for genuine shell glue
      (orchestrating `cargo dylint`, the fast-path freshness check, `watch`). *(The Stop-hook receipt
      `candor-run.sh` and the MCP stdio server stay in Python — they're hook orchestration / a
      protocol server that already delegates report logic to `cargo-candor`, not duplicated query
      logic; folding them onto `candor-query` is a possible follow-up, not required for DRY.)*

## P5 — research (the thesis)

- [ ] **Beyond code: field transfers (assessed 2026-06-11).** candor's kernel is field-agnostic:
      a closed vocabulary of consequential actions + a resolvable invocation graph + transitive
      propagation + an `Unknown` honesty marker + a deterministic policy gate. The transfer test
      (all four required): (a) the graph is *mechanically* resolvable — once extraction needs an
      LLM, determinism dies and you have a lint, not a gate; (b) the effect vocabulary is small and
      closed; (c) leaves are classifiable at a boundary; (d) change is frequent enough that a gate
      pays. Ranked candidates:
      1. **Agent fleets** (orchestrators → subagents → tools; effects = tool capability classes;
         `tools:` declarations are static, delegation is CHA-shaped, an undeclared MCP tool is
         `Unknown`) — same buyers, spec nearly ports as-is. **Exploration PUBLISHED:
         [candor-agents](https://github.com/tombaldwin/candor-agents) — tested (25 checks +
         a teeth-verified soundness fuzzer), validated on the 36k-star wshobson/agents fleet
         (headline: 182/192 agents run with ambient authority), combined fleet+code mode
         linked via the Exec boundary.**
      2. **IaC / CI-CD** (Terraform modules, Actions workflows; effects = cloud-API classes;
         an unpinned third-party action = `Unknown`; OPA gates the plan, nothing gates the
         composition graph transitively).
      3. **Data lineage / privacy** (dbt/Airflow DAGs; effects = data categories — PII/PHI;
         purpose-limitation as a deny rule; an unparseable stage = `Unknown`, never silently clean).
      4. Access control is the existence proof at org scale (IAM reachability analysis — same math).
      **Database development (assessed 2026-06-11, ranks between fleets and IaC):** the catalog IS
      the graph (pg_depend; proc bodies via libpg_query — the engine's own parser), units =
      functions/procs/views/triggers, the literal surface = per-relation read/write (the
      hosts/cmds/paths analog), dynamic SQL = the reflection analog → Unknown, triggers need
      clinit-style edge synthesis. The queries: "which procs transitively WRITE ledger.entries"
      (append-only as a gate), whatif on migrations, rewire on dropped view edges. Prior-art gap is
      real (lineage tools do pipelines, not the proc call graph + gate + Unknown). **First step SHIPPED
      (2026-06-11, `794ec38` + java `e3e6c55` + spec `9877b43`): the `tables` literal surface +
      `allow Db in <scope> <table>` gate, both engines, conformance-locked.** Remaining: the
      DB-side engine (pg_depend/libpg_query) and --link at the Db boundary ("this handler
      transitively writes payments.ledger, through the app AND the database"). Contracts/SOPs fail test (a) today. The launch landed 2026-06-11; these are now
      eligible, in rank order, as capacity allows.

- [~] Controlled eval of *edit quality* (not just analysis cost) with independent ground truth and
      multiple trials — **the gate on P0** (see P0 §5). The pilot (`EVAL.md`) showed consumability +
      efficiency, that a source-only agent can beat the report where the classifier has a gap, and (Trial
      5) that candor's edit-feedback lifts non-local awareness on one task.
      **Now scaled — first batch shipped:** `eval/scaled/` is a pre-registered, reproducible 3-task ×
      2-arm × 2-trial harness (fixtures with candor-verified ground truth, blind judge, falsification
      clause). Batch 1 (`eval/scaled/RESULTS.md`) found a *completeness* gap but was confounded — it also
      exposed its own metric mis-spec, fixture leakage, and an answer-key contamination bug (all fixed).
      **Batch 2 (`eval/scaled/RESULTS-v2.md`) — clean, pre-registered:** completeness as primary,
      de-leaked fixtures, weaker model (Sonnet), N≈9/arm, contamination fixed and fully re-run. Result:
      **control completeness 7% vs treatment 100%** (binary 0.17 vs 1.00); neither falsification
      condition triggered — candor's edit-feedback takes a realistic agent from naming ~0 of the
      non-local callers to all of them, for ~5% extra tokens, a large lift consistent across 3 tasks.
      **Batch 3 (`eval/scaled/RESULTS-v3.md`) — the untested cell, shipped:** a *large* fixture
      (`tasks-v3/orderflow` — Net propagating to **16** non-local fns across 9 files / 3–5 call-graph
      layers, vs 7/4 in the small tasks) × a **frontier** agent (Opus-class), 2 arms × 3 trials, blind
      frontier judges, pre-registered (fixture + 16-fn denominator committed before the run). Result:
      **control completeness 6% vs treatment 79%** (conservative; ~100% under the lenient blanket
      reading 2 of 3 judges applied), gap 0.73 — neither falsification condition triggered. **Decisive
      new finding:** a frontier model does **not** close the gap — the Opus control still named only the
      one helper it edits past (1/16) and wrote "callers unaffected"; the missing thing is call-graph
      *enumeration*, not capability, which is exactly what candor supplies. The 6%→100% effect is not a
      small-fixture / weak-model artifact. **Remaining:** more tasks/power at this scale, and
      edit-quality-beyond-summary measures (does the agent *act* on the propagation, not just report it).
- [x] Effect-aware PR review — `examples/candor-pr-review.yml`: a workflow that POSTS the per-function
      effect delta vs the baseline as a PR comment + step summary (the *review-time* sibling of P0's
      *edit-time* loop; both powered by `cargo candor diff`). It informs rather than blocks (pair with
      `candor-guard.yml` to also fail); an AI reviewer can consume the same via `diff --json`.
- [x] **Formal semantics** — `candor-spec/SEMANTICS.md`: the effect lattice, call-site resolution
      rules (CLASSIFY/CROSS/DEVIRT/CHA/EXEMPT/UNKNOWN), the transitive least-fixpoint, cross-crate
      composition, the conformance predicates, and the conditional-soundness properties (with the two
      honesty caveats). The implementation was then verified against it clause-by-clause.

## Done (recent, for context)

Unknown/AS-EFF-003 · CANDOR_RULES (né CANDOR_CONFIG) · CANDOR_NO_AMBIENT/AS-EFF-004 · CANDOR_PARANOID ·
CANDOR_BASELINE/AS-EFF-005 · ICE hardening · raw-socket + HTTP + Rand + **Db + Ipc** classification ·
**const/static initializers (macro-filtered)** · **main entry-point exemption** · unit tests ·
`cargo-candor` wrapper · CI + downstream guard workflow · self-guard ·
**CHA: see through dyn/generic dispatch over local traits**.

_Latest pass:_ cross-crate propagation (DefPathHash) · devirtualization · report provenance &
versioning · v0.2 self-describing envelope · `cargo candor audit` at-a-glance profile · formal
`SEMANTICS.md` + a clause-by-clause code↔spec audit · remediated a real consumer's stale baseline.

_Hardening pass:_ Python→Rust query port (`candor-report` + `candor-query`) · Fs read/write detail ·
frictionless `install.sh` + stable `~/.candor` home · dylib/query resolution by newest-mtime (was the
`head -1` glob) · self-guard trusts the baseline's own siblings (no spurious cross-engine downgrade) ·
fixpoint profiled on ripgrep (negligible).

## Known limitations (confirmed by review 2026-05-29; documented, not all worth fixing)

- [~] **declared_caps now peels common wrappers** — a capability behind `Option<&Fs>` / `Vec<&Fs>` /
  `Box` / `Result` / a tuple is recognised (was a false AS-EFF-001). `caps_in_ty` recurses into type
  arguments (bounded depth). **Residual:** a cap behind a user STRUCT FIELD (a `Caps { fs: &Fs }`
  bundle) still isn't seen — that needs recursing into ADT field types, not just generic args.
- [x] **Macro-generated functions are no longer skipped** — the blanket `from_expansion()` filter
  was narrowed to consts/statics (the tracing `__CALLSITE` noise); macro-generated *functions*
  (async_trait/derive/decl-macro) are analyzed and reported, held by the fuzzer's macro forms.
- **const/static initializers don't propagate as callees** — their init effects are reported
  standalone but not inherited by code that references the const (we only follow `Fn`/`AssocFn`
  call edges, and a const reference isn't a Call expr anyway).
- **Baseline key is `def_path_str`** — not guaranteed unique; identical stringly-named items
  (rare) could collide in the guard. Names are the only stable cross-run key, so this is inherent.
- **[STALE — the worklist SHIPPED in the 0.23.1 sweep; see propagate.rs:19,50 and the 2026-08-05 audit at the top]** ~~Effect/fs fixpoint is naive iterate-to-convergence~~, not a worklist (`O(rounds × V × E)` per
  crate). Measured negligible: on ripgrep (52k lines) the largest crate (`rg`: 1179 fns / 3302 edges)
  fixpoints in 0.42 ms, ~0.7 ms total across the workspace — ~0.017% of the run. Per-crate cost is
  dominated by call resolution + rustc, not propagation, so a worklist rewrite isn't worth it.

## Cross-component consistency (from the 2026-06-16 core-component sweep)

> **Status (2026-06-21):** item 1(d) `unknownWhy` vocabulary divergence is **DONE** (harmonised to the
> canonical reflect/native/dispatch/callback + a conformance check, PART 10). Item 3 is **partly done**
> (conformance now covers 8 effects + the dispatch frontier + `containment` PART 11; Clipboard/Ipc and the
> desugared-call generative forms remain). Items 1(a/b/c/e/f) envelope-field divergence (**`undeclared` is a DECISION, scan.rs:1743**) and 4 (MCP tool-set — **VOID**)
> are still **open**.

These four are NOT simple bugs — they are cross-engine consistency / coverage efforts that need a
deliberate design pass (some touch the spec + all 4 engines + conformance + releases). The clear bugs
the same sweep found are already fixed (java serialization soundness 0.5.7; candor-query audit silent-pure
+ whatif empty-target; ci self-gate stale-binary; mcp arg-validation; candor-agents crash + strict gate).

1. **Report ENVELOPE cross-engine field divergence.** A consumer written against one engine breaks on
   another: (a) candor-scan (rust) never emits `declared`/`undeclared`/`overdeclared` (java/swift/ts always
   do, as `[]`); (b) rust OMITS `direct` when a fn has no own-body effects (others emit `[]`), so a
   consumer can't tell "no direct effect" from "untracked"; (c) rust emits exact-DUPLICATE entries
   (tokio: 2 fns appear twice, identical hash/loc/inferred — cfg-gated re-emission; DEDUP-by-fn before
   writing, others don't dup); (d) `unknownWhy` token vocabulary diverges 4 ways (java `dispatch-broad:`/
   `dispatch-broad-ext:`, ts `call:`/`accessor:`, swift `contentsOf:`, rust `callback:` — spec defines
   only reflect:/native:/dispatch:/callback:); (e) java envelope coverage field is `packages:[]` (array)
   vs `package:""` (string) elsewhere; (f) `unresolved` is presence-only on rust/ts vs explicit bool on
   java/swift. The dedup (c) is a clear small fix; the rest needs a SPEC decision (which envelope fields
   are mandatory vs optional, and a single `unknownWhy` vocabulary) then per-engine alignment + a
   conformance check on the envelope shape. candor's whole pitch is cross-language-consistent reports, so
   this matters. NOTE: do NOT blindly add declared/undeclared to rust — confirm whether rust supports the
   declaration-annotation feature at all first (it may be a genuine semantic gap, not a missing field).

2. **candor-scan (syntactic, stable) silent-pure on macro-hidden effects.** A `libc::socket(...)` inside an
   idiomatic `syscall!{...}` declarative-macro wrapper (socket2/mio) comes back PURE — the stable backend
   doesn't expand local `macro_rules!`, so the libc callee is invisible (candor-classify WOULD map it to
   Net). This is the documented SYNTACTIC-FLOOR limitation (absence≠pure; the nightly HIR engine, which
   expands macros, is the sound gate). NOT a candor-scan bug to "fix" by guessing — but consider whether
   candor-scan can raise Unknown when a function body contains an UNEXPANDABLE local-macro invocation it
   can't see through (sound over-approximation), vs the current silent omission, and whether the κ ledger
   discloses it. Verify the disclosure story; don't add noisy Unknown-on-every-macro.

3. **Conformance coverage gaps.** The 4-way suite exercises only 6/10 effects (Fs/Net/Exec/Env/Clock);
   Db/Rand/Log/Clipboard/Ipc have ZERO cross-engine coverage (deliberately scoped out in expected.json —
   they need per-language deps / are structurally asymmetric, so each engine tests them in its own suite).
   And the generative matrix has NO desugared-call forms (custom-iterator for-of, disposal, operators,
   subscripts), so the ~14 implicit-call holes fixed 2026-06-15 are gated ONLY by per-engine fuzzers, not
   conformance — a cross-engine regression of them wouldn't be caught. Worth: add desugared-call fixtures
   to the generative differential, and at least Db/Log (the common ones) to the cross-engine set.

4. **MCP tool-set divergence across engines.** ts exposes 8 tools (impact/where/reachable/path/callers/
   show/map/whatif), rust 5 (effects/where/callers/whatif/diff), java 4 — and the arg key is `fn` (ts) vs
   `function` (python). An agent/skill written against one engine's candor MCP breaks on another, defeating
   the "same prompt everywhere" promise. Align: a canonical tool set + arg key across all three servers
   (the python servers shell out to candor-query which already has impact/reachable/path/show/map, so it's
   mostly wiring), update the schemas + mcp.json.example + READMEs.
