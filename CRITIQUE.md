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
   candor's discipline is advisory, enforced by a lint, bypassed by any soundness hole. *(Partially
   addressed: `CANDOR_NO_AMBIENT` now flags any direct reach for ambient authority — AS-EFF-004 —
   which is the enforceable, cap-std-aligned discipline. It still can't make the call fail to
   compile the way cap-std does.)*

3. **The classifier is a curated allowlist.** It only knows hard-coded crates; an unrecognised
   effectful crate is a silent false negative. *(Partially fixed: `CANDOR_CONFIG` lets a project add
   its own rules; the built-in list was broadened to raw `std`/`tokio` sockets and randomness (Net
   detection was previously AWS-only — a real hole) and is now covered by unit tests that pin the
   precision rules. The core list is still hand-maintained.)*

4. **Effect granularity is coarse.** `Net` lumps all network; `Fs` doesn't split read vs write.
   Too blunt for real capability security. *(Not fixed.)*

5. **Adoption is viral.** Threading tokens to honour the discipline cascades up the whole call graph
   (measured on ebman). High churn for an advisory, partially-sound guarantee.

6. **The "for AI agents" thesis is unproven.** candor is a useful effect auditor for *humans*;
   nothing here has been shown to make an AI agent's edits better. The JSON mode is a gesture toward
   it, untested with a real agent. The project conflated three goals (AI legibility, developer
   documentation, capability security) and is middling at all three rather than excellent at one.
   *(Update: demonstrated, not yet proven. An agent given only the JSON for an 8k-line codebase
   scoped a cross-cutting refactor — all 66 network sites, the logging gap, and the 18 `unresolved`
   functions needing source review — in ~22k tokens without reading source. That shows the artifact
   is consumable and useful; it is NOT a controlled eval of whether edits improve, which remains the
   honest open question.)*

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
