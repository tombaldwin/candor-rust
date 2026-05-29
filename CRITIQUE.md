# A critical look at candor

Written deliberately against the project's own interest. If you're deciding whether to
use or invest in candor, read this before the README's pitch.

## Prior art — is this novel?

Only partly. The space is occupied:

- **[Cackle / cargo-acl](https://github.com/cackle-rs/cackle)** is the closest neighbour and is
  more mature. It classifies APIs into the *same* classes (`net`, `fs`, `process`), does proper
  **reachability / dead-code elimination**, enforces which dependencies may use which capability,
  and sandboxes build scripts and proc-macros. It targets a sharper problem (supply-chain attacks).
- **[cap-std](https://github.com/bytecodealliance/cap-std)** is the *strong* form of candor's
  capability idea: a capability-oriented `std` where you **cannot open a file without a `Dir`
  token** — enforced by the type system, no ambient authority. candor's `&Fs` tokens only pretend
  to be this.
- **[Rust effects initiative](https://rust-lang.github.io/keyword-generics-initiative/)** is the
  language-level future of effects; `pure fn` has been proposed repeatedly.
- `cargo-geiger` does the same "audit what your code does" move on the `unsafe` axis.

**What candor genuinely adds:** function-level, *transitive*, machine-readable effect inference
over first-party code. Cackle says "crate X can touch the network"; candor says "*function*
`handle_key` transitively touches the network." Nobody else produces that specific artifact cheaply.
That is the part worth keeping.

## The honest weaknesses

1. **Conformance was unsound — and an unsound guarantee is worse than none.** Calls candor cannot
   resolve (dynamic dispatch, fn-pointers, callbacks through `impl Fn`) were silently treated as
   pure. In real Rust — async, trait objects, callbacks — that is a large blind spot, so
   `CANDOR_STRICT` could green-light functions that actually perform I/O. *(Partially fixed — see
   below.)*

2. **The tokens are not capabilities.** `&Fs` does not gate `std::fs`; you can still call it without
   the token and candor only complains afterward. cap-std makes the bad call impossible to write.
   candor's discipline is advisory, enforced by a lint, bypassed by any soundness hole. *(Largely
   addressed two ways: (1) `CANDOR_NO_AMBIENT` (AS-EFF-004) flags any direct reach for ambient
   authority; (2) candor now recognises **cap-std's** capability types (`Dir`/`Pool`/`SystemClock`/…)
   as declarations and its operations as the matching effect — so a project built on cap-std gets
   conformance against *real, unforgeable, compile-enforced* capabilities for free, with candor as
   the visibility layer on top. candor itself still can't make the bad call fail to compile — that's
   cap-std's job — but it no longer needs its own advisory tokens to be the whole story.)*

3. **The classifier is a curated allowlist.** It only knows hard-coded crates; an unrecognised
   effectful crate is a silent false negative. *(Partially fixed: `CANDOR_CONFIG` lets a project add
   its own rules; the built-in list was broadened to raw `std`/`tokio` sockets, HTTP clients
   (`reqwest`/`ureq`/`isahc`), and randomness — Net was previously AWS-only, and the `reqwest` gap
   was caught empirically by the eval (`EVAL.md`) misreporting real Anthropic-API calls as
   network-free. Now covered by unit tests that pin the precision rules. The core list is still
   hand-maintained.)*

4. **Effect granularity is coarse.** `Net` lumps all network; `Fs` doesn't split read vs write.
   Too blunt for real capability security. *(Partially addressed: effect *classes* broadened —
   `Db` and `Ipc` are now distinct from `Net`. The intra-class split (`Fs` read vs write) is still
   deferred — it breaks committed baselines and the `&Fs` token model; see `BACKLOG.md`.)*

5. **Adoption is viral.** Threading tokens to honour the discipline cascades up the whole call graph
   (measured on ebman). High churn for an advisory, partially-sound guarantee. *(Sidestepped:
   `CANDOR_BASELINE` (AS-EFF-005) gives most of the value — catching a function that newly gains an
   effect — with zero token threading and zero rewrite. This, not token migration, is the realistic
   adoption path for an existing codebase.)*

6. **The "for AI agents" thesis is unproven.** candor is a useful effect auditor for *humans*;
   nothing here has been shown to make an AI agent's edits better. The JSON mode is a gesture toward
   it, untested with a real agent. The project conflated three goals (AI legibility, developer
   documentation, capability security) and is middling at all three rather than excellent at one.
   *(Update: a controlled pilot was run — see `EVAL.md`. JSON-only vs source-only on the same
   scoping task: the JSON was ~3× cheaper in tokens, ~8× fewer tool calls, ~6.5× faster. BUT the
   source-only agent was more *correct* — it caught `reqwest` HTTP calls candor silently
   misclassified as network-free, exposing a real classifier gap (since fixed). So: efficiency
   supported; accuracy only as good as the classifier; "do edits improve" still unproven.)*

7. **Nightly fragility.** dylint pins a nightly and uses `rustc_private`; it will break across
   toolchain bumps and is a maintenance tax.

## Status of fixes (this pass)

- **Unresolved calls are now first-class.** Dynamic dispatch, fn-pointers, and `impl Fn` callbacks
  record an `Unknown` effect instead of being dropped. In audit mode you see `Unknown` in the set;
  in conformance mode a function carrying `Unknown` raises **AS-EFF-003** ("effect set not provably
  complete") and cannot be certified. candor no longer lies by omission about calls it can't see.
  - *Residual gap:* statically-dispatched **generic** trait calls (`t.method()` where `t: T: Trait`)
    are still assumed to honour their bound, rather than marked `Unknown` — otherwise every
    `.clone()` / `.to_string()` / iterator adaptor would drown the report. This is a deliberate,
    documented trade, not an oversight.
- **Project-extensible classifier** via `CANDOR_CONFIG` (crate/path → effect rules).
- **JSON reports an `unresolved` flag** per function for machine consumption.
- **Enforcement mode** `CANDOR_NO_AMBIENT` (AS-EFF-004): flags any *direct* reach for ambient
  authority, the cap-std-aligned discipline that has actual teeth (it fires even on token-holders).
- **Opt-in max soundness** `CANDOR_PARANOID`: also marks generic static trait dispatch `Unknown`,
  closing the residual gap for users who accept the noise.

### Call-graph pass (CHA) — seeing through dynamic dispatch

The root limitation behind "unsound conformance" and "can't do reachability" was a call
graph that only contained statically-resolvable calls; `dyn`/generic dispatch fell into
`Unknown`. Class Hierarchy Analysis now resolves calls to **locally-defined** trait
methods to *all* their impls (edges → union of impl effects), seeing through both `dyn`
and generic dispatch soundly. Non-local trait objects (std/deps, whose impl bodies we
can't see) stay honestly `Unknown`.

What this revealed, measured on ebman, is more reassuring than the old "100 `Unknown`
(22%)" headline suggested. After CHA:

- 100 → 92 `Unknown` (CHA resolved the local-trait cases — e.g. the whole LLM
  `explain`/`lint` feature, now correctly `Net`).
- Of the 92, **86 are *additive*** — the function's real effects are captured and
  `Unknown` only flags "also calls something opaque." No effect is lost.
- Only **6 functions (1.3%)** are *purely* `Unknown` — and they're event/detail handlers
  (closure/std-`dyn` dispatchers). That is the true residual blind spot, and it is small
  and well-delimited, not pervasive.

The deep remainder — closures passed as `impl Fn`/`dyn Fn` — needs interprocedural
closure-flow (effects riding function types), which HIR can't give; that's a MIR-level
engine, deliberately not attempted. Note the residue is partly *not even a hole*: an
effectful closure is attributed to the function that *defines* it (lexically), so the
effect usually lands on the concrete caller, not lost.

### Usefulness pass

- **Regression guard** `CANDOR_BASELINE` (AS-EFF-005): diff the current effect report against a
  committed snapshot; fail CI when an existing function gains an effect. Zero-friction adoption — no
  tokens, no rewrite — and the strongest answer to "is this actually useful day-to-day." Baseline
  JSON parsed with serde; loader returns None (never panics) on a missing/garbled file.

### Robustness pass

- **No more ICEs.** `resolve_callee` uses `maybe_typeck_results()` and bails gracefully instead of
  panicking on expressions outside a typechecked body — an effect checker must never abort a build.
- **Broader, tested classifier.** Net now covers raw `std::net`/`tokio::net` sockets (not just the
  AWS SDK), and randomness (`getrandom`/`fastrand`/`rand`) is detected. Unit tests pin the precision
  rules (e.g. `std::net::TcpStream` ⇒ Net, `std::net::SocketAddr` ⇒ not). Regression-checked against
  ebman: identical totals except +3 newly-found raw-socket network functions.

## Deliberately still deferred

- **Finer effect granularity** (Fs read vs write, net-by-host). Net-by-host is not statically
  knowable (the host is runtime data) — claiming it would be dishonest. Fs read/write *is* doable
  but requires expanding the effect vocabulary AND the capability-token types with a sub-typing
  relation (`Fs ⊇ FsRead, FsWrite`) so existing `&Fs` declarations still satisfy them; that ripple
  outweighs the payoff for now. Left undone on purpose rather than half-built.
- **Compile-time enforcement** (vs lint-time). Only a cap-std-style API can make the bad call fail
  to compile; candor stays a checker.

## Recommendations

- **Keep and sharpen the audit; treat conformance as secondary.** The audit is the real
  contribution. If you want true enforcement, build conformance on **cap-std** types rather than
  candor's advisory tokens.
- **For a new project:** prefer cap-std for enforcement and use candor's audit as the per-function
  map cap-std doesn't produce. candor's tokens are not a reason to structure a project a certain way.
- **For an existing project (e.g. ebman):** run the audit and gate CI on the JSON ("did any module
  gain an effect it didn't have last release?") — cheap, high signal. Do **not** migrate to token
  threading: high churn, and the `Unknown`/AS-EFF-003 results will show how much dynamic dispatch
  conformance still can't certify.
- **Either prove the agent angle or drop it** from the framing.

**Sources:** [Cackle](https://github.com/cackle-rs/cackle) ·
[Cackle write-up](https://davidlattimore.github.io/posts/2023/10/09/making-supply-chain-attacks-harder.html) ·
[cap-std](https://github.com/bytecodealliance/cap-std) ·
[Rust effects initiative](https://rust-lang.github.io/keyword-generics-initiative/) ·
[`pure fn` RFC discussion](https://github.com/rust-lang/rfcs/issues/1631) ·
[dylint](https://github.com/trailofbits/dylint)
