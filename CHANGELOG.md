# Changelog

All notable changes to candor are recorded here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); candor is pre-1.0, so minor versions may include
behavioural changes (always in the soundness-increasing direction — see the §4 trust contract).

**⚠ marks a verdict-affecting change** — a gate/guard/report that was green may read differently
after upgrading; review policies and regenerate baselines with the new build.

## Unreleased

- ⚠ **SOUNDNESS R143 — a MULTI-ARM `macro_rules!` was skipped, and the report said nothing at all.**
  `visit_macro` expands only a genuinely single-arm template, because an invocation matches EXACTLY
  ONE arm and a pre-expansion scan cannot tell which; walking every arm would charge a non-matching
  arm's effect. **That skip is right. The silence was not.** The code called it "an honest
  under-report" and it was not one — the report carried no `Unknown`, no `unknownWhy`, no
  `incomplete` and no `invisible` over a template that really writes. A named limitation that
  answers `Unknown` is a limitation; one that answers NOTHING is a silent under-report with a
  footnote. **A cardinal sin.** Executed ground truth: a function invoking the `write` arm of a
  two-arm macro performs the write and is ENTIRELY ABSENT from `functions[]` on `d54108b`, while a
  single-arm control doing the identical write is charged `['Fs']`.
  An unexpandable multi-arm invocation now discloses `Unknown` with
  `ambiguous:unexpanded multi-arm macro_rules! \`NAME\`` — SPEC §4's `ambiguous:` kind with the
  reserved dot-free detail (no function value is involved and no owner type can be formed), the same
  kind and the same ADD-ONLY direction as the R208 site beside it.
  **The over-charge is real and is measured, not argued.** A macro whose arms are ALL pure still
  discloses, because "the arms I can read are pure" is not a fact about the one that expanded — a
  cut that exempted it would be reading the arms it refused to choose between. **1,509-crate
  registry A/B, wide key**, vs `79d0101`: **ADDED 2,832 / REMOVED 0 / CHANGED 2,288** over 266,437
  rows, **0 functions lost a concrete effect**; 4,238 rows carry the new reason, across **131 of
  1,509 crates**. R143's row predicted this "costs no precision"; it costs about 1.6% of rows, all
  of it disclosed `Unknown` rather than a concrete effect. §E1 reach: `R143DISCLOSE` fires 12,770
  times in 131 crates. The debt register does not move — it contains no multi-arm shape, and its
  equivalence verdict would count `Fs -> Unknown` as a VIOLATION regardless.

- ⚠ **SOUNDNESS R142 — a `macro_rules!` declared INSIDE a function body was invisible to call-edge
  resolution.** `local_macros`, the crate-local `macro_rules!` index `collector.rs::visit_macro`
  expands from, is built by `decls::collect_decls`, which visits FILE/MODULE items only. A template
  declared in a function body was therefore in no index that path could consult, and a bare
  `NAME!(..)` expanding it read silent-pure — **the whole function went ABSENT from `functions[]`**
  with no `Unknown`, no `unknownWhy`, no `incomplete` and no `invisible`. **A cardinal sin**, and a
  DIFFERENT AUTHORITY from R206, which gave the `?`-operand drop-safety veto a body-local reader in
  `lang.rs`; R142's shape has no `?` in it at all.
  **Executed ground truth** on a crate that compiles and really writes: four functions reaching a
  real `std::fs::write` through a body-local template (at body level, as a call argument, in a nested
  block, and one whose name collides with another body's) are ABSENT on a binary built at `d54108b`
  and charged `['Fs']` now, while the crate-level twin and the plain direct call are charged in both.
  **The index is OWNED AND BLOCK-SCOPED, which is the whole fix.** `local_macros` is crate-wide and
  NAME-keyed, so hoisting body-local definitions into it lets one body's template expand in another
  body that declares its own macro of that name. Recording happens as the walk MEETS the definition
  (`visit_item_macro`, which is also what reaches a definition inside re-parsed macro tokens) and
  `visit_block` restores the map on the way out. Both controls were degraded: one shared name-keyed
  map fabricates `['Fs']` on a body that writes nothing AND loses the effect on the body that does;
  deleting the `visit_block` restore fabricates `['Fs']` on a body whose only invocation resolves to
  a crate-level pure macro. Revert-red on three independent neutralisations.
  **Property gates:** `soundness/known_open.tsv` 141 shapes → 99 (42 closed, 0 new), and the
  `?`-position gate's BOTH-PURE pairs 84 → 48. **1,509-crate registry A/B, wide key** (`fn` +
  inferred + unknownWhy + declared + unresolved + incomplete + netClass + invisible + drop-glue
  edges), vs `d54108b`: **ADDED 6 / REMOVED 0 / CHANGED 4** over 270,904 rows, **0 functions lost a
  concrete effect**. Every one audited against original source: rayon's `par_sort_by_cached_key`
  (×3 versions) and combine's `str_uncons_while` invoke a caller-supplied callback inside a
  body-local template and now say `Unknown callback:unresolved call` instead of being absent; h2's
  `Peer::convert_poll_message` (×2) and tower-http's `DefaultMakeSpan::make_span` reach a real
  `tracing::debug!`/`tracing::span!` inside one and now report `Log`; axum's
  `MethodRouter::call_with_state` and rustix's `io_uring_layouts` gain an `invisible` disclosure.
  The two withdrawals are the same function twice: `time-0.3.55`'s `SignedDuration::fmt` loses
  `invisible: ["serde_core"]`, and it was a FALSE disclosure — the crate-wide name-keyed index made
  that body's `item!` expand the UNRELATED `macro_rules! item` in `src/serde/mod.rs`, whose template
  names `serde_core::de::Error`. The body-local definition now shadows it, which is Rust's own rule.
  **§E1 reach:** `R142BODY`, the counter on the DECISION (a template the body itself declares, about
  to be walked), fires **2,192 times across 133 crates**.

- ⚠ **SOUNDNESS R238 — a callback held in a STRUCT FIELD is a callback.** `fn via_field(v: &mut
  Vec<i32>, h: &Holder) { v.retain(h.cb) }` was **ABSENT from `functions[]` entirely** while the
  identical program written with the callback as a PARAMETER reported `['Unknown'] callback:unresolved
  call`. Same HOF, same callback type, one difference — the access path. Executed ground truth on a
  compiled fixture: every arm really invokes the caller-supplied body (42 invocations), and the body
  supplied performed `Fs`. **Gate impact isolated:** over a library that defines nothing effectful and
  only invokes a caller-supplied callback, `deny Unknown` exited **1** with the parameter spelling and
  **0** with the field spelling — a green gate over a library that calls arbitrary caller-supplied
  behaviour. **A cardinal sin.**
  **Mechanism, and why it was TWO holes.** Nothing typed a field ACCESS as callable. (1) Pass A's
  struct-field arm recorded `trait_leaves`, which sees a `Fn*` BOUND (`Box<dyn Fn()>`) but not the
  shapes carrying no trait in their syntax — a bare `fn(&i32) -> bool`, a callable type ALIAS,
  `Option<fn(..)>`, a tuple-struct position. Those reached no index at all. (2) `expr_is_fn_typed`
  answered only for a NAME, so even an already-indexed `Box<dyn Fn>` field handed to an invoking
  adapter matched nothing. Both now ask `is_callable_type` / `resolve_recv_traits` — the authorities
  the PARAMETER path already used, which is exactly why the parameter spelling worked.
  **ADDITIVE, deliberately.** The new index entry is written only where `trait_leaves` found NOTHING,
  and it writes the synthetic `"Fn"` hedge without touching the concrete `fields` entry — R177 is the
  measured reason (that hedge DISPLACING a concrete element type took a row from `["Fs"]` to ABSENT).
  `"Fn"` matches no local trait, so `dispatch_calls_for_trait_method` declines it and no CHA fan-out,
  no `Type::method` edge and no concrete effect can come out of it. The only reachable outcome is
  `Unknown`.
  **THE WIDENING THIS IS NOT.** The naive form — "any call whose callee came from a field" — was built,
  measured and REJECTED in candor-java during R217: **+1,697 rows over 395 jars**. This one is gated on
  the field's DECLARED TYPE being callable, so it fails toward SILENCE (unchanged) whenever the base
  type cannot be resolved, the field is declared in another crate, or the pair is not indexed; and
  toward over-charge only where a leaf-keyed collision puts a callable field's name on a type whose
  same-named field is a plain value AND that value reaches a non-callback argument slot of an invoking
  adapter (`fold`'s seed). That collision is a REAL shape, reproduced in a compiling fixture, and it
  produced **zero rows on the corpus**.
  **1,509-crate registry A/B, WIDE key (every field), vs `ade1dc5`: ADDED 24 / REMOVED 0 / CHANGED 2**
  over 263,575 common rows, 0 errors. **All 26 audited in full against SOURCE, not against candor's own
  report.** The 24 additions are real cardinal-sin instances in published crates: 22 in `rustls-ffi`
  0.15.0/0.15.3 (`let cb = self.callback; cb(..)` over an `unsafe extern "C"` callback the C caller
  supplies — `CallbackReader::read`, `CallbackWriter::write`, `VectoredCallbackWriter::write_vectored`,
  `SessionStoreBroker::retrieve`/`store`, `Verifier::verify_server_cert`, `ClientHelloResolver::resolve`
  and their callers) and 2 in `crossbeam-epoch` 0.9.18/0.9.20 (`Deferred::call`). Zero removals — nothing
  the previous build charged is now certified pure. The 2 changes are reason-channel only, both correct:
  `arrow-array` 58.4.0 `get_stream_last_error` moves `unknownWhy` from `ambiguous:same-name local defs`
  (an artefact of resolving the callback as a free-fn NAME — the crate really has two `get_last_error`
  definitions) to `callback:unresolved call`, and `sea-orm` 2.0.2 `RusqliteConnector::connect` GAINS
  `callback:unresolved call` alongside its existing reason. No `inferred` set lost a member anywhere.
  **§E1 reach counters, and one of them is ZERO — recorded here rather than discovered later.**
  `R238DECL` (the new index entries): **75,376 hits in 199 of 1,509 crates**. `R238FIELD`
  (`expr_is_fn_typed`'s new `Expr::Field` arm): **0 hits in 0 of 1,509 crates** — that half is
  **SAFETY-ONLY over this corpus**, and the A/B says nothing about it. A recall hunt found why:
  real code overwhelmingly writes `struct ForEachConsumer<'f, F> { op: &'f F }` with the `F: Fn(T)`
  bound on the **impl block**, not on the struct (rayon `src/iter/for_each.rs:55`,
  `iter.into_iter().for_each(self.op)`), and Pass A reads only the struct's own generics. **Measured on
  a compiling fixture: the struct-bound form is now charged, the impl-bound form is still silent.**
  Left open deliberately — charging an UNBOUNDED generic field would be the +1,697 shape exactly.
  **Over-charge control, written before the fix and part of the test:** a field holding a callback that
  is never invoked (returned, not called), a non-callable sibling field, a non-callable field in an
  invoking adapter's seed slot, and a pure inline closure through the same HOF — all five gain nothing,
  measured. Revert-red verified by stashing the source change with the test in place (10 arms red).
  Cache schema **rev22 → rev23**: `trait_fields` now records something a rev22 entry does not, so a warm
  entry would serve "this type has no callable field" for a type that has one — the same under-report,
  invisible. `soundness/incremental_equiv.sh`: 120 edits (20 of them struct-field edits), every
  incremental scan byte-identical to a full scan.

- ⚠ **SOUNDNESS R229 (second half) — a macro invoked INSIDE a macro is read too, and the leaf-keyed
  macro LICENCE is gone.** `macro_ctor_leaves` said "this leaf was built inside a macro somewhere, and
  no site table can hold that construction, so keep the shipped leaf-keyed answer" — i.e. the R172 site
  gate CERTIFIED the escape with no evidence, for every leaf any macro in the body constructed. It
  existed because a nested macro's parse produces owned temporaries whose addresses die with the call.
  `ord_nested` now composes a nested reading's ordinals into the enclosing reading's space (FNV-1a over
  `(index, inner ordinal)` with the top bit set, so it can never alias a direct ordinal), and
  `nested_macro_nodes` enumerates the nested invocations in ONE canonical order for both walks — so the
  site half and `mark_escape` agree about a construction one macro deep, and the licence has nothing
  left to cover. It is deleted, and with it a guard the drop family has carried since R172.
  **Property gates:** the R216 debt register goes from **169 shapes to 141** — 28 more closed, 0 new;
  `macro=nested`, `macro=tmpl_stmt` and `macro=std_stmt_tokens` are gone entirely, and with them the
  LAST escape-side class in the register. **Everything still registered on the macro side (131 of the
  141) is RESOLUTION-side**: `body_local`, `blocktok_local`, `repetition`, `unparsed` and `match_arms`
  are ABSENT even with nothing escaping and no `?` in the body (measured), so the collector never
  resolves the construction at all and no change to the escape model can reach them. That is R142/R143/
  R144's subject, in `collector.rs`.
  **1,509-crate registry A/B, WIDE key, vs `7dfe710`:** ADDED 0 / REMOVED 0 / CHANGED 0 over 270,874
  rows. §E1 reach: `R229DECIDE` 1,741 hits / 143 crates, `R229NESTED` 5,794 hits / 43 crates.
  Revert red (3 tests). The SIXTH and last shipped "by-value spine stays exempt" control is converted
  to parity with its direct twin, for the reason the other five were.
  `ord_nested`'s separation claim is PINNED BY A TEST rather than asserted in a comment (§E2): the top
  bit it sets is `usize`'s, not a `u64`'s — `1u64 << 63` truncates to ZERO on a 32-bit target, and the
  separation would have vanished there while every test passed on the 64-bit machine anyone would check
  it on (§L).

- ⚠ **SOUNDNESS R229 — the SITE walk and the ESCAPE walk now read a macro the same way, so a
  macro-borne construction is neither certified pure nor fabricated.** R172's site gate suppresses a
  drop only when EVERY construction of that leaf in the body is one of the escaping sites, and until
  now `lang.rs` had two walks that disagreed about what a macro contains. The site walk could not see
  a `macro_rules!` TEMPLATE, statement-only tokens, or a block inside macro tokens, so
  `out.push(mk_h!("a")); let h = H::try_new(m, "b")?; let _ = out; Ok(h)` had exactly one site — the
  escaping one — and the function was certified PURE while an `H::drop` really ran in that frame
  (**a cardinal sin**; its direct twin `out.push(H::new("a"))` was charged all along). The escape walk
  could not see them either, so `pub fn esc_tmpl() -> H { mk_h!("a") }`, which drops nothing here, was
  **fabricated** a `Drop` edge. R199/R203/R204/R210 each closed one spelling of the first half INSIDE a
  `?` operand — every walk they added is gated on an open `?` and writes only `TryExit::interior` — and
  none could reach either half outside one. `macro_reading` is now the single reading both walks take
  and `macro_reading_ordinals` the single numbering that carries site identity between two independent
  parses of one token stream; region 0's ordinals are unchanged, so every site identity the two walks
  already agreed on is untouched.
  **Executed ground truth** (`mem::forget` on every return, so only in-frame drops count): the four
  `dies_*` cells drop 1 each and were ABSENT for the template / block-token / statement-template
  spellings; the six `esc_*` cells drop 0 and three of them were charged. Every direct twin is
  unchanged in both builds, which is what makes the macro the variable under test. Revert red (5 tests).
  **Property gates:** the R216 debt register drops from **216 shapes to 169** — 47 closed, 0 new — and
  BOTH-PURE pairs from 102 to 90; `soundness/known_open.tsv` is regenerated in this commit.
  **1,509-crate registry A/B, WIDE key** (`fn` + `inferred` + `unknownWhy` + `declared` + `unresolved`
  + `incomplete` + `netClass` + `invisible` + drop-glue call edges), vs `7dfe710`, both arms cold:
  **ADDED 0 / REMOVED 0 / CHANGED 0 over 270,874 rows**, 0 functions lost a concrete effect — and the
  same for each half in isolation (a site-recording-only arm is byte-identical to both), so nothing is
  cancelling. §E1 reach: the DECISION counter `R229DECIDE` (the gate would have suppressed on the sites
  the old walk could see, and does not) fires **695 times in 115 crates**; none of those leaves has a
  local `impl Drop`, which is why the corpus is quiet. `R229MACROSITE` (727,315 / 433 crates) and
  `R229ESCAPE` (707,074 / 60 crates) are WALK counters, not decisions — recorded as such.
  **Four shipped tests were asserting a safety property the code does not have.** Each said a
  macro-borne construction on a `?` operand's by-value spine "stays exempt"; all four passed because
  the construction was INVISIBLE, and all four have a direct twin (`use_h_val(H::new("a"), n)?`) that
  has been charged by the site gate since published 0.34.0. They now assert PARITY WITH THAT TWIN,
  which is the property the macro gate exists for. The residual they share — R172 has no by-value
  argument exemption at all — is a separate defect whose fix WITHDRAWS a charge and needs its own A/B.
  Still open and named in the test: a macro NESTED inside macro tokens (`spine_nested`), and a
  statement template that stores through a macro parameter the caller returns (`esc_stmt_tmpl`,
  over-charged at pre-change parity).

- ⚠ **SOUNDNESS R222 (second half) — several bodies under ONE qualified name are ONE definition and
  resolve to the UNION of their effects, per SPEC §4.** `by_leaf` counted `FnInfo` entries rather than
  distinct definitions, exactly as `by_tail2` did, so a bare call to a `#[cfg]`-twinned function saw
  "two definitions" and answered `Unknown ambiguous:same-name local defs` over arms the engine had
  already analysed. It now deduplicates, resolves to the single definition, and inherits the union of
  its units. **This implements SPEC.md §4's clause of 2026-09-05** — *"'Two same-named local
  definitions' means two DISTINCT definitions. Several bodies under ONE qualified name are one
  definition, and resolve to the UNION of their effects"* — and conformance PART 10's `armsunion` /
  `armsswap` rows go from RED to GREEN with it; they were written red on purpose, before the port.
  `ambiguous:` stays exactly where the clause reserves it: two separately-written definitions competing
  for one bare name.
  **The deciding measurement, and it inverts the R208 analogy this half was withheld under.** A twin
  whose arms carry DIFFERENT effects does not go quiet — it goes to the union, order-independently
  (`Fs` + `Exec`, and the same answer with the arms swapped). So a hedge is replaced by a COMPLETE
  answer rather than withdrawn; R208's refused remedy went the other way, concrete → hedge. Same-effect
  arms cannot tell a union from a PICK, which is why no fixture here or in the conformance suite had
  ever caught the distinction; `two_cfg_arms_of_one_fn_resolve_to_the_union_in_either_source_order` is
  the discriminator, in both source orders, and it is what the revert takes red.
  **A/B over 1,509 crates.io crates against the shipped `cc05b8c`, keyed on EVERY report field, one
  variable: 0 rows ADDED, 300 REMOVED, 2,129 CHANGED on the wide key (1,080 on `inferred` alone),
  173 crates touched; 128 functions gain a concrete effect (Clock 89, Rand 80, Env 27, Fs 10, Ipc 6,
  Log 6, Net 4) and 0 functions lose one.** Reach (§E1), counted in the changed branch itself: the
  dedup collapses 2,059 leaf buckets in 569 of the 1,509 crates. **All 300 removals audited from
  source — 228 distinct functions, NONE of which carried a concrete effect**; 134 held nothing but the
  withdrawn hedge and 94 inherited it, and every newly-resolved target they reach was read in the
  crate's own source rather than in candor's report. Executed ground truth (§E3): the union fixture was
  built and run and wrote 8 real bytes; `deny Fs go` over it goes exit 0 → 1.
  **WHAT IT COSTS, measured rather than asserted safe (SOUNDNESS R227).** 1,167 rows lose the
  `ambiguous:` reason. 961 now point at a target that carries an effect; the rest point at one this
  engine already reported PURE. All 122 such targets were read from source: nearly all are
  arithmetic/conversion/platform-shim twins with no effect in either arm, and **32 rows sit over an arm
  whose effect reaches the machine by a route this engine does not model at all** — inline `asm!` and
  `core::arch` RNG intrinsics (`getrandom`'s `rndr`/`rdrand`), an `extern "Rust"` logger hook
  (`defmt`'s `export::timestamp`), a `link`ed Win32 call (`cmake`'s `fix_build_dir`),
  `ffi::sqlite3_threadsafe`, libc `poll`. In every one of them the target is absent from the report on
  BOTH sides of this change: the silence is at the callee, it pre-exists, and what moved is an
  accidental cover over it — the same cover that was already missing from every QUALIFIED call site
  reaching those callees. R227 predicted this population would be R128/R139 (a macro-declared callee)
  or R123/R140 (a `#[cfg]`'d `use` resolved by source order); **neither appears in it**, and the shape
  that does — unmodelled FFI and inline assembly — is a different open question.
  Live gate this closes: `fastrand`'s `random_seed` is a three-arm `#[cfg]` twin whose `getrandom` arm
  is the crate's real entropy source, so `deny Rand global_rng::Rng` exits **0 before and 1 after** over
  `Rng::new`. Scope matters and the earlier framing overstated it: `deny Rand global_rng` was already
  red on the shipped engine and `deny Unknown` caught the constructor in both arms — what was green is
  the effect-named gate.
  Re-pointed, not deleted: `the_ambiguous_reason_kind_and_its_class_are_pinned` now holds two
  separately-written `helper` definitions in two modules (the shape conformance PART 10 also moved to),
  and `save_bare` asserts `Fs` — the answer its qualified twin `save_twin` already had.
  The kind is not thereby made rare, counted rather than assumed: over the same 1,509 crates
  `ambiguous:` goes **24,226 → 22,826 reason entries in 682 → 632 crates**, so the 58/200
  `deny E Unknown[dispatch]` counterfactual that §4 ⟨0.24⟩ cites still has its subject.
  `soundness/run_q.sh` and `soundness/run_macro.sh` are CLEAN before and after with 40 / 133 known-open
  hits unchanged, so no registered shape closed and `known_open.tsv` is untouched.

- ⚠ **SOUNDNESS R222/R129 — `by_tail2` counted ONE definition twice, so a call that was never
  ambiguous resolved to NOTHING.** The index pushed `f.qual` once per analysed UNIT, and several units
  routinely share one qual: two `#[cfg]` arms of a fn or of its module, ≥2 impls of one trait for one
  type differing only in type params (`jiff`'s `ZonedArithmetic::from` ×3), an `impl Trait for
  `[T]`/`(T,)`/`&T` whose type segment `impl_type_name` cannot form and drops (`half`'s
  `convert_from_f32_slice`, `x11rb`'s `serialize_into`, `diesel`'s `to_sql`), an inherent assoc fn
  beside a trait method of one name, erased generic args (`typenum`'s `private_pow`), and ≥2 different
  traits with a same-named method (`rustix`'s `Uid::fmt` ×6). `resolve_target`'s `len() == 1`
  uniqueness filter then read "ambiguous" over a bucket holding one distinct qual and the edge was
  dropped **with no `Unknown` beside it — absent, nothing to fail closed on.** 11,230 functions were
  in that state; 20,495 of 647,698 analysed units (3.16%) in 884 of 1,509 corpus crates are duplicate
  quals. The buckets are now keyed on DISTINCT definitions.
  **THIS LANDED SECOND, AND THAT ORDER IS MEASURED, NOT STYLISTIC.** The duplicate had been
  accidentally BLOCKING R223's mis-resolution, so deduplicating alone on the shipped `c8aa83c`
  **LOSES an effect**: `rand_core-0.6.4`'s `error::Error::fmt` goes `['Rand']` → ABSENT, because
  `tail2` discards the crate qualifier and `getrandom::Error::from` lands on the local `Error::from`.
  Three fixtures isolate it with ONE variable — the number of local `Error` definitions: with none,
  charged in every arm; with ONE, already lost on shipped `main` (that is R223); with TWO, the
  duplicate was what saved it. Built and run as a corpus A/B: c8aa83c + dedup alone loses `Rand` ×1
  and removes 234 rows; with R223 first, the same dedup loses nothing.
  **A/B over 1,509 crates.io crates against `c8aa83c`, keyed on EVERY field, both commits together:
  1,994 rows ADDED, 5 REMOVED, 7,432 CHANGED, 424 crates touched; 596 functions gain a concrete effect
  (Log 157, Rand 156, Fs 124, Clock 111, Db 104, Env 71, Net 58, Exec 32) and 0 functions lose one.**
  All 5 removals audited from source: each lost an `invisible` disclosure it had INHERITED through an
  edge that existed only because the primary route was fake-ambiguous (curl's `FormError::fmt` edging
  to `Error::fmt`, digest's `D::digest` edging to `DynDigest::finalize`).
  **THE `by_leaf` TWIN WAS HELD BACK HERE AND IS TAKEN IN THE BULLET ABOVE — READ THAT ONE FOR WHAT
  SHIPS.** This paragraph is kept as the record of why it was held, because the reasoning was wrong in
  a specific and instructive way. The same duplicate reaches `by_leaf`, where the bare-call path
  already DISCLOSES (`Unknown ambiguous:same-name local defs`) instead of going silent — so it is not
  the cardinal sin, which is true and is why it needed a ruling rather than a fix. Deduplicating it too
  was built and measured: +125 concrete effects, but **1,400 corpus rows lose that disclosure and 228
  rows vanish from the report entirely**, and it turns off rust's single largest `Unknown` reason
  (8,710 of 19,607 `unknownWhy` entries over a 1,062-report census) for exactly the shape §4 ⟨0.24⟩ was
  read as admitting the kind FOR — cfg-gated alternative definitions. **That last clause is where the
  reasoning failed: §4 admitted the KIND, never that shape, and `same-name local defs` appears nowhere
  in SPEC.md. The binding of `ambiguous:` to `#[cfg]` lived only in this engine's fixture and in
  conformance PART 10's, and SPEC §4 (2026-09-05) settles it the other way.** Calling it "withdrawing
  an existing answer, the move refused for R208" was also wrong on the facts: R208's remedy went
  concrete → hedge, this one goes hedge → the UNION of the arms, which no fixture in the family could
  distinguish from a pick until `armsunion`/`armsswap`.
  Every withdrawal that half would make was checked mechanically first: of the 3,603 ambiguity
  sites in the 59 crates it touches, all 134 rows it withdraws had `distinct = 1` — one definition
  counted twice — while the 2,237 `distinct = 2` and 824 `distinct = 3` sites are genuine and stay.
  **NOT TOUCHED, and neither claim moves:** `all`/`functions[]` is not deduplicated (see the four
  numbered reasons at the `blind_direct` insertion), so
  `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two` still pins both units
  in the report and one violation at the gate. R129's own subject — that a per-qual map is a UNION
  over units that need not agree, printed once per unit — is untouched and still open; its duplicate
  ROW count in fact RISES, 6,981 → 7,371, because more functions are now effectful enough to be
  reported at all. R190(c)'s refusal over two genuinely distinct quals sharing a tail is unchanged.

- ⚠ **SOUNDNESS R223 — a call written into a DEPENDENCY no longer has its effect silenced by a
  same-named LOCAL definition.** `tail2` keys the local index on the last TWO segments, so
  `tokio_postgres::Client::execute` and deadpool-postgres' own `generic_client::Client::execute`
  present the identical key `Client::execute`: the local definition won the lookup, `t == f.qual`
  dropped the edge as a self-reference, and `resolved_local` then suppressed the `Db` the classifier
  had. **The whole `GenericClient` forwarding surface — `execute`, `query`, `query_one`,
  `prepare_typed`, `batch_execute` and their `Transaction` twins — was ABSENT from the report, while
  the same crate correctly charged `Db` on `Manager::create`**, so the silence was aimed precisely at
  the methods a consumer calls. Census over 1,509 crates.io crates, instrumented at the site: **123
  suppression sites, 99 caller functions, 26 crates; Net 72, Fs 24, Db 19, Rand 6, Env 2; 59 of the
  sites left the caller ABSENT.** The dominant shape is the newtype/wrapper idiom forwarding to the
  same-named method of an external type, which is why `fs-err` (a `std::fs` wrapper) and `cap-std`
  are on the list.
  **The guard is right and the KEY was too narrow, so what is withdrawn is the local definition's
  AUTHORITY to speak for the dependency, never the edge.** `resolved_local` is unchanged and the
  local `calls` edge is kept, so the result is a UNION and the change can only ADD a charge. That
  direction was not a preference: the first cut made a dependency-qualified path un-`resolvable` and
  **dropped 161 concrete effects** — rdkafka's `Log` ×30 through a `use rdkafka_sys::types::*` glob
  that rewrites a call on rdkafka's OWN `client::Client`, sqlx-postgres ×12, tungstenite's `Rand`
  through an extension-trait `impl IntoClientRequest for http::Uri`; a second cut still dropped 43.
  Cargo.toml is the authority on what is external (the role `std | core | alloc` already plays), and
  a local definition whose own qual is a **≥3-segment suffix** of the written path is corroborated by
  a segment `tail2` had thrown away, so it keeps its authority — a two-segment crate-root qual does
  not, which is measured: with a plain suffix test, tokio-native-tls'
  `native_tls::TlsConnector::connect` walked straight through and stayed silent.
  **A/B over 1,509 crates.io crates against `c8aa83c`, keyed on EVERY field: 53 rows ADDED, 0
  REMOVED, 45 CHANGED, 15 crates touched; 91 functions gain a concrete effect (Net 39, Db 27, Fs 22,
  Rand 3) and 0 functions lose one.** Gains audited in full from source, not sampled:
  deadpool-postgres 26, tokio 23 (mio-backed socket constructors), ignore 8 (walkdir `metadata`),
  fs-err 14, cap-std 6, tonic 4, postgres 2, p12-keystore 2, ureq, axum, hickory-net, rand_core.
  §E1 reach counters: the branch is reached at **437,268 sites in 1,069 of 1,509 crates**, of which
  4,084 in 286 crates also resolved locally and **76 in 15 crates actually flipped**; the 47 census
  sites that stay suppressed were each read from source and are correct local resolutions (own trait
  impls on `String`/`SocketAddr`, a crate's own `mod rand`, a glob-rewritten local `PgStream`, a bin
  calling its own lib). STATED RESIDUAL: the `CANDOR_DEPS` cross-crate join at the same site is still
  gated on `resolved_local` and is left alone here, because this A/B does not run `--deps`.

- ⚠ **SOUNDNESS R208 — an invocation of a `macro_rules!` name the crate defines TWICE now discloses its
  order-dependence.** `local_macros` is keyed by bare NAME and merges last-writer-wins, so which
  module's template R48 expands depends on FILE ORDER: measured on the same two files swapped
  (`panel-fixes-5/r5` and `r5b`), one order leaves the caller SILENT over an executed drop and the
  other charges it. 324 of 1,504 corpus crates define a `macro_rules!` name more than once (`#[cfg]`
  twins — anyhow, aho-corasick, async-compression …). **The row's own remedy — "record duplicates and
  REFUSE" — is deliberately NOT taken, because it WITHDRAWS: measured on `r5b`,
  `mb::a3_collision_hole` goes `['Fs']` → `['Unknown']`, a concrete effect lost.** So the pick is left
  exactly as it was and only the disclosure is added: an invocation of a twinned name carries
  `Unknown` beside `ambiguous:same-name macro_rules! definitions`, and whatever the winning template
  charged is still charged. The order-dependence stays open — visibly rather than silently. A plain
  sequential redefinition in one file is NOT twinned: Rust's textual shadowing makes the later
  definition the one an invocation below it expands, which is what last-writer-wins already does, so
  the intra-file rule fires only when a `#[cfg]` is involved. Corpus A/B over 1,509 crates.io crates:
  **1,477 functions across 56 crates — 0.23% of 647,698 analysed functions, zero concrete effects
  lost.** Cache schema rev22.

- ⚠ **SOUNDNESS R190(e) — a re-export key the alias index REFUSED to admit now discloses.**
  `reexport_aliases` declines a `<module>::<name>` key three ways: the fan-out cap (more definitions
  under one key than `REEXPORT_FANOUT_MAX`), a key two modules claim, and R169's macro-hidden-contest
  guard. Each decline is right — the index will not guess which definition a name means — and each left
  the calling function resolving to NOTHING, reported pure. Measured on a fixture that COMPILES AND
  RUNS: thirteen mutually-exclusive `#[cfg(target_os = …)]` glob arms make `imp::pick()` ABSENT while
  the identical THREE-arm control reads `['Exec']` over a real spawn — the silence appears at 13 and
  nowhere below it, which is why no hand fixture had found it. Corpus A/B over 1,509 crates.io crates:
  **94 functions across 3 crates — 0.015% of 647,698 analysed functions, zero concrete effects lost.**
- **SOUNDNESS R190(c) is NOT closed, and is now priced rather than assumed.** The QUALIFIED-tail
  spelling of the same refusal (`#[cfg(unix)] mod sys` beside `#[cfg(windows)] mod sys`, caller
  `sys::sz()`) still leaves the caller ABSENT. Hedging every ambiguous 2-segment tail measures **45,472
  functions across 674 of 1,509 crates (7.02% of analysed functions)**, against 0.85% for the whole
  rest of this family; counting distinct quals only brings it to 5.29%, and additionally requiring that
  the written path fail to select one candidate brings it to 4.88% — all inside the 8-25%
  false-uncertainty band that got the hedge-every-untyped-receiver design rejected. The flood is
  dominated by two things that are not this row's subject: primitive-conversion collisions
  (`u32::from` with several `impl From<_> for u32`) and a type name two modules share, where the CALL
  names its module and `tail2` throws that qualifier away. Recorded in `scan.rs` with those numbers so
  the next attempt starts from the mechanism split.

- ⚠ **SOUNDNESS R184 — an alias/nominal LEAF COLLISION no longer deletes every effect edge through the
  colliding type in silence.** `prim_aliases` is a crate-wide set of bare LEAVES, so a non-nominal
  `pub type H = fn(&str)` in one module makes EVERY `H::method` call in the crate skip local
  resolution — including the calls that mean an unrelated `struct H` a module over. The skip itself is
  right and stays: it is what stops sled's `type Inner = [u8; CUTOFF]` inheriting a same-named
  `struct Inner`'s effectful `Default`. What was wrong is that the skipped call left no edge and no
  disclosure: measured, `b::S::go_vec`/`go_idx`/`go_direct` were all ABSENT from `functions[]` while
  the one-variable control `b::T::go_vec` (leaf `K`, colliding with nothing) read `['Fs']` over the
  identical `fs::write`. The skip now discloses `Unknown` with reason
  `ambiguous:type alias and nominal type share a leaf`, and only where it actually cost an edge (the
  tail names a definition this scan read). Corpus A/B over 1,509 crates.io crates: **272 functions
  across 19 crates — 0.04% of 647,698 analysed functions, and zero functions lost a concrete effect.**

- ⚠ **SOUNDNESS R182/R196 — the drop route's two REFUSALS now DISCLOSE instead of certifying the caller
  pure.** `ctor_leaf_from_call_returns` declines when a fn leaf resolves to more than one definition —
  because two definitions disagree about the return type (R182), or because a UNIT-returning twin means
  a leaf-keyed lookup cannot say which callee answered (R196). Declining is right; what followed was
  not: the construction was never noted, the destructor's effects never charged, and the enclosing
  function was omitted from `functions[]` entirely — indistinguishable from a function that really is
  pure. Measured, executed: `let h = two::mk(p)` beside an unrelated `other::mk -> usize` left
  `holds_two` ABSENT while its one-variable control read `['Net']`; `let g = net::open(a)` beside an
  unrelated unit `Ui::open` left `drop_limit` ABSENT while `drop_ctrl` read `['Fs']`. Both now carry
  `Unknown` with an `ambiguous:` reason (SPEC §4).
  **The disclosure is priced, not blanket.** A leaf like `new`/`parse` is ambiguous in almost every
  crate, so the refusal is only disclosed where it actually WITHDREW something a charge could have
  landed on: `rets` now records the withdrawn candidate types (under an `<amb>` sentinel key, outside
  the identifier key space), and `scan.rs` intersects them with `drop_relevant`. Corpus A/B over 1,509
  crates.io crates: **R182 2,190 functions, R196 236 — 0.37% of 647,698 analysed functions between
  them, and ZERO functions lost a concrete effect.** `deny Unknown` users: these add `Unknown`, so a
  gate that was green may now be red — that is the point. Cache schema rev21.

- **SOUNDNESS R214 — `coverage-gate-refresh.yml` no longer conflates two different alarms under
  "regressed."** On its first-ever completed run the job called `e9cdd23` (R173) correctly withdrawing
  a drop-glue over-charge from `async_nats::Context::publish`/`publish_message`/`publish_with_headers`
  a "regression" — 3 of its 5 reported rows — because a checked-in `covered.tsv` row the fresh run no
  longer sees covered can mean either of two things: **(a)** classify() genuinely lost a rule (the
  entry still shows an effect — it reappears in the fresh `open.tsv`) — a real coverage loss, or **(b)**
  the self-scan candidate oracle itself stopped nominating the entry (absent from BOTH fresh manifests
  — its `inferred` effect set went empty or non-CORE, e.g. a precision fix) — not a classify()
  regression at all. `eval/coverage-gate/generate.py --diff-manifests` (new; the workflow now calls it
  instead of a ~50-line bash `comm`/`cut` reimplementation) makes that split explicit: (a) still fails
  the job loudly, (b) is reported prominently (a `::warning::` annotation plus the drift issue body,
  including a flag when a whole crate's entries vanish at once, which is more consistent with that
  crate's self-scan failing outright than with a precision gain) but does not fail it — training the
  weekly reader to distinguish a real regression from the gate's own oracle getting better, rather than
  crying wolf on the next correct fix. Also states, in its own output whenever there is anything to
  report, that the checked-in manifest (generated on whatever machine last committed it, historically
  macOS) and the fresh run (Linux CI) have never been proven to produce the same self-scan output for
  the same source (R212, still open — the same commit measured 3 regressed rows locally and 5 in CI,
  and the host-platform cause could not be ruled in or out) — a diffed row is unconfirmed until it
  reproduces from a same-platform regeneration. `generate.py --selftest` (still pure Python, no cargo,
  no registry, no network) now proves the split discriminates: a constructed real classify() regression
  exits 1, the real 3-row async_nats/R173 instance (read from this repo's own checked-in `covered.tsv`)
  exits 0 while still reporting all 3, and a clean run exits 0 with nothing reported.

- **Two PROPERTY gates for the stable scanner, calibrated by retro-rediscovery: `soundness/run_q.sh`
  (`?`-position) and `soundness/run_macro.sh` (macro equivalence).** Six cardinal-sin regressions were
  introduced and caught during the ⟨0.35⟩ round (SOUNDNESS R187, R194, R199, R203, R204, R210) and the
  1,504-crate wide-key corpus A/B caught **zero** of them — every one measured 0 corpus incidence.
  Each was caught instead by somebody hand-writing the right fixture. These gates check a property over
  GENERATED programs: each pair is two spellings of ONE program emitted from one description —
  `let t = EXPR; t?;` vs `EXPR?;`, a `?` before vs after a construction in a loop body, and a
  construction written directly vs through a single-arm `macro_rules!` — so a spelling that loses an
  effect its equivalent was charged is a silent under-report with no oracle needed. Both spellings of
  every pair are COMPILED and EXECUTED (`examples/gt.rs`) and their in-frame drop counts checked, so no
  verdict rests on comparing two absences. Calibrated against the six pre-fix binaries built from each
  fix commit's parent: **6 of 6 rediscovered, all within 12 random seeds** (R187 seed 1, R194 1, R199 1,
  R203 1, R204 8, R210 12); exhaustively, 54/54/60/54/37/8 distinct violating shapes. The obvious
  formulation — insert a never-erroring `?` and compare against a `?`-free twin — was tried first and
  rediscovered **0 of 6**: it is vacuous on exactly these shapes, because with the value escaping by
  return the `?`-free twin is charged nothing (it is R187's own `collect_loop_noq` control, whose
  expected answer is ABSENT). `soundness/known_open.tsv` records, exhaustively, the 216 shapes at which
  HEAD already fails these properties — a debt register, subtracted so a NEW instance fails and printed
  every run so it is not forgotten. Both gates run in CI. `soundness/README.md` carries the calibration
  table and an explicit statement of what they cannot catch.

- **⚠ SOUNDNESS R150 — CLOSED: `gate --report`'s same-artifact guard converged on the one
  implementation, closing a FAIL-OPEN gap against the scan route.** `same_artifact` (SPEC §3.3.1 — is a
  `--policy`/`--gate-json`/`--report` collision one artifact under two names?) existed as two
  independently-maintained copies: `candor-scan/src/scan.rs`'s had the ⟨0.28⟩ device+inode check and a
  dangling-symlink pre-resolve; `candor-query/src/gate.rs`'s was still a bare `canonicalize()` with
  neither. A policy hard-linked to its own `--gate-json` was refused on the scan route and ACCEPTED on
  the gate route, which then armed its fail-closed placeholder over the hardlink and silently destroyed
  the policy before evaluating it — reproduced against the pre-fix binary: `--policy policy.P
  --gate-json <hardlink to policy.P>` exited 0 pre-fix (via a mangled re-parse of its own placeholder,
  the policy already overwritten) vs. a clean `refusing (exit 2)` with `policy.P` byte-identical
  afterwards post-fix. Both crates already depended on `candor-report`, so the guarded implementation
  now lives there once (`candor_report::same_artifact`) and both call sites delegate to it — one
  authority, not two copies that can drift. `candor-query`'s own test suite now pins the gate call site
  directly (a hardlink case and a dangling-symlink case, `gate.rs`'s own `same_artifact`, not the scan
  crate's), since a test that pinned only one copy is how the drift went unnoticed for a release.
- **SOUNDNESS R112 — CLOSED: `soundness/realworld/run.sh`'s real-crate syscall oracle now honours
  `CARGO_TARGET_DIR` for the scanner it runs, and fails loud rather than silently using the wrong
  binary.** The script already honoured `CARGO_TARGET_DIR` for the *build* (cargo reads the env var
  itself) but hardcoded `$ROOT/target/debug/candor-scan` as the path it went on to *read* — the same
  BUILD-vs-READ split R108 found for this script's own predecessor bug. Measured on this machine (not
  in CI, where the trigger — a container-local `CARGO_TARGET_DIR` cache dir — actually occurs): with
  `CARGO_TARGET_DIR` redirected and a stale binary left sitting at the hardcoded path (a plausible state
  after any build that predates a `CARGO_TARGET_DIR` override), pre-fix code silently ran the stale
  binary with zero warning and exit 0; post-fix code correctly resolves to, and verifies, the freshly
  built binary at the redirected path. A second arm — a build that reports success without producing a
  binary at the resolved path — now fails loud (`exit 1`, naming the missing path and the
  `CARGO_TARGET_DIR` value) instead of silently continuing. Mirrors `pf/run_pf.sh`'s already-correct
  form exactly, so the family's two real-crate oracles answer this one question the same way. The
  fix is confined to the build/path-resolution prologue (`git diff` — 2 lines changed, rest new
  comments); the loop, verdict and exit-code composition below it (including R104's existing "a driver
  that never reached the verdict is `broke`, and `broke > 0` fails the whole run" rule, which already
  makes a genuinely-missing report red rather than a silent pass) are untouched. **Not independently
  re-run end-to-end on Linux+strace for this change** — the local Docker environment used for that in
  past rounds had unrelated storage corruption this session; the CARGO_TARGET_DIR-honouring/fail-loud
  logic itself was verified directly (its literal statements, extracted and run standalone, both before
  and after the fix, under a real `cargo` build), but the full oracle run (including "a real finding
  still fails non-zero") should be watched on the `realworld-oracle.yml` CI gate this file already runs
  under.

## [0.35.0] — 2026-09-03

- **⚠ SOUNDNESS R172 (cardinal sin, a REGRESSION against published 0.34.0) — CLOSED: a `Drop` value
  that dies in a function is no longer silenced by a DIFFERENT value of the same type leaving through
  the return.** `pub fn swap_free(p, q) -> H { let _g = H::new(p); from_handle(q) }` and the
  `Self::mk(q)` spelling beside it read `['Net']` on published 0.34.0 (tag `736fa64`) and were ABSENT
  on the 0.35.0 candidate, because R160 and R165 taught the escape gate to recognise two tail
  spellings it used to ignore. The gate is now keyed on the construction SITE, not the type leaf —
  the construction half of the fix R168 made for parameters. Executed ground truth: one `H::drop` runs
  inside each frame while the returned value is still alive. `H::swap_type` (`H::mk(q)`), a victim of
  the same key before either fix, is charged for the first time. 1,504-crate A/B vs the candidate:
  ADDED 7, REMOVED 0, LOST 0 — and all 7 are the fix unmasking a pre-existing over-approximation
  (x11rb `wrap_reply` and its callers; portable-atomic-util `Arc::get_mut`), not a recovered drop.
- **⚠ SOUNDNESS R173 (fabrication, at corpus scale) — CLOSED: a `?` no longer vetoes the escape of a
  value the function has not constructed yet.** `pub fn order(n) -> Result<Wr,()> { gen(n)?;
  Ok(Self::for_region(n)) }` charged `Wr::drop` to a body that drops nothing on either path (executed:
  0 drops on Ok and on Err). Each `?` now removes only what its own position can reach. The veto is
  pre-existing — the explicit-type spelling is charged on published 0.34.0 too — but R160 made it fire
  on the dominant `x?; Ok(Self::new())` spelling. 1,504-crate A/B: REMOVED 37 rows, 128 effect losses,
  every removal read from source; 39 of the 136 new positive claims R160 introduced disappear,
  including the whole 36-row x11rb wrapper family. Controls that stay charged: a construction BEFORE
  the `?`, and one between two `?`s (executed: 1 drop each on the early-exit path).
- **⚠ SOUNDNESS R187 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R173 above,
  caught by a second fix-lens pass before release) — CLOSED: a `?` inside a LOOP is live for
  everything that loop body builds.** R173 numbers each `?` in PRE-ORDER and reads that as evaluation
  order; inside a loop the two disagree, because the body runs again. `for it in items { let v = it?;
  out.push(H::new(v)); } Ok(out)` — with `while step(&mut n)?`, `loop { let v = it?; .. }` and the
  single-slot variant beside it — read `['Net']` on published 0.34.0 and were ABSENT on the candidate:
  `deny Net collect_loop` and `pure collect_loop` both 1 → 0, with no `Unknown` anywhere. Executed
  ground truth: one `H::drop` runs inside each frame when the error fires on iteration 2. Every `?`
  recorded inside a `for`/`while`/`loop` — the CONDITION included, since it is re-evaluated after the
  body has built its value — now moves to that loop's last position when the loop closes, which can
  only ever veto MORE. Six further silent shapes went with the four: nested loops with the `?` in
  either one, a labelled break, `while let`, `break`/`continue` around the `?`, and a `?` whose
  closure sits in the loop body. Controls: a construction AFTER the loop stays ABSENT (R173's gain,
  executed 0 drops), one BEFORE it stays charged (executed 1), and a `?` inside a CLOSURE is left
  where R173 put it — it exits the closure, not this function. 1,504-crate A/B vs the candidate,
  wide key: ADDED 0, REMOVED 0, effects lost 0, call edges lost 0, one row CHANGED — x11rb
  `Image::put_impl`, whose `while` loop pushes `put_image(..)?` cookies into a `Vec` that really
  drops on the error path, gains that `VoidCookie::drop` edge. The branch fires 13,481 times across
  665 of the 1,504 crates, so the zero-diff is measured over a corpus that reaches it.
- **⚠ SOUNDNESS R194 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R173 above and
  NOT reached by R187, caught by a third fix-lens pass before release) — CLOSED: a `?` is live for
  what its OWN OPERAND built.** R173 numbers each `?` at its PRE-ORDER position, i.e. before its own
  operand is walked, so every construction inside that operand got a HIGHER number than the `?` and
  read as "not built yet" — while evaluating the operand is exactly what built it. Nine shapes read
  `['Fs']` on published 0.34.0 and were ABSENT (or lost their `Fs`) on the candidate:
  `{ out.push(H::new()); gen(n) }?`, the `match`/`if` operands with the construction in an ARM's
  statement, `run_cb(|| { out.push(H::new()); .. })?`,
  `items.into_iter().try_for_each(|it| { it?; out.push(H::new()); Ok(()) })?`,
  `use_h_ref(&H::try_new(n)?)?` (a borrowed temporary that dies on this frame's exit),
  `gen({ out.push(H::new()); n })?`, `poll_once(async { out.push(H::new()); .. })?`, and a `?`
  operand nested inside another `?` operand. (`H::try_new(n)?.check()?` and its `.await?` twin drop in
  this frame too, but published 0.34.0 missed both and the 0.35.0 line already charges them — a gain,
  not part of this regression.) `deny Fs` and `pure` both 1 → 0 on them. Executed ground
  truth: one `H::drop` runs in each frame on the error path (two for the loop-shaped one). Each `?`
  now also vetoes the leaves its operand constructs OFF THE OPERAND'S VALUE SPINE, whatever their
  pre-order number says. THE VALUE-SPINE EXEMPTION IS THE LOAD-BEARING HALF, and it is enumerated
  rather than inferred. A lump post-order number would charge `Ok(Repr::new(s)?)` — on the exit where that `?` fires,
  `Repr::new` returned `Err` and no `Repr` was ever built — which is published's fabrication back
  again over ~280 registry rows. The spine descends only the operand itself, `Paren`/`Group`/`Try`/
  `Await`, a block's TAIL, an `if`/`match` arm's tail, and by-value `Call`/`MethodCall` arguments,
  tuple/array elements and struct fields; everything else stays charged. It deliberately stops at a
  `&` (`use_h_ref(&H::try_new(n)?)?` borrows a temporary that dies here, and differs from its exempt
  by-value twin `use_h_val(H::try_new(n)?)?` by that one character), at a method RECEIVER
  (`H::try_new(n)?.check()?`), and at a closure/`async` BODY and a block's non-tail statements. A
  closure's TAIL needs no exemption: it is already an unconditional escape route re-united after the
  positional filter, which is what keeps async-std's `spawn_blocking(|| File::create(&p)).await?`
  quiet. Fourteen executed spine controls (0 drops in-frame each, including one that drops 1 in the
  CALLEE's frame and two `.await?` chains) pin the direction, and the R173 controls
  `operand_ctrl_value`/`operand_ctrl_straight` are unmoved. Both halves were measured by degrading the
  code, not reasoned about: reverting the veto clause returns all nine shapes to ABSENT, and dropping
  the spine exemption charges twelve of the fourteen controls — one of them (`Ok(mk_h(n)?)`, a
  free-function constructor) beyond even published, which had no such route before R165. The one
  exclusion with NO teeth is stated as such in the test: a method RECEIVER cannot reach the veto at
  all, because `for_each_value_child` never descends one and R172's site gate could not accept it as
  an escaping site, so `H::try_new(n)?.check()?` stays charged with the receiver on the spine or off it. 1,504-crate A/B vs the candidate `70fd624`, wide key (17 fields, not just `inferred`):
  ADDED 0, REMOVED 0, CHANGED 0, effects lost 0, call edges lost 0 — byte-identical over 257,243
  common rows. The branch fires 2,824 times across 427 of the 1,504 crates, so that zero is measured
  over a corpus that reaches it; only 5 of those 2,824 veto a leaf whose `Drop` the crate defines, and
  all 5 were traced (3 absorbed by the unconditional escape route — jobserver `Acquired`, jni
  `TLSAttachGuard`, diesel `CopyToBuffer`; dylint `inject_dummy_dependencies`, the real-world instance
  of the `try_for_each` shape, was already charged identically by all three arms). Published → this
  build reproduces the published → candidate ledger line for line (common 252,097, ADDED 5,146,
  REMOVED 699, LOST 620), so nothing in it is unattributed to the earlier rows, and the set of rows
  that lost a published `X::drop` edge is IDENTICAL to the candidate's (symmetric difference 0), which
  is the control that the 279 construction-after-the-last-`?` removals stay removed.
- **⚠ SOUNDNESS R199 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R173 and reached
  by neither R187 nor R194, caught by a fourth fix-lens pass before release) — CLOSED: a `?` is live for
  what a MACRO in its own operand built.** A macro's contents are not in the body's AST — the token
  stream is re-parsed — so the construction-site machinery has a second, separate recorder for them, and
  that one records the position at the MACRO's own node (inside the operand, hence AFTER the pre-order
  `?`) and never consulted the open `?` exits at all. R173's positional filter therefore read a
  macro-borne construction as "not built yet" and R194's operand-interior term could not see it by
  construction — nor could R194's `R194OPERAND` counter. Six spellings read `['Fs']` on published 0.34.0
  and were ABSENT on both `70fd624` and `7d9a970`: `{ out.extend(vec![H::new()]); gen(n) }?`,
  `{ out = vec![..]; gen(n) }?`, `{ let v = vec![..]; out.extend(v); gen(n) }?`,
  `{ out.append(&mut vec![..]); gen(n) }?`, the same inside a `match` arm's statement, and inside
  `try_for_each(|it| { it?; out.extend(vec![..]); Ok(()) })?`. `deny Fs` and `pure` both 1 → 0 on all
  six. Executed ground truth: one `H::drop` runs in each frame on the error path (two for the
  loop-shaped one). Four further shapes the row did not name are the same defect and are now charged
  too: a `?` inside the macro's own tokens, a macro construction inside a LOOP inside the operand, the
  same leaf built both inside and outside the macro, and a `&`-borrowed temporary built inside a macro
  that IS the operand.
  THE SPINE TEST HAS TO BE TWO STEPS, and the second is not decoration. A macro node CAN sit on the
  operand's value spine — `Ok::<Vec<H>, ()>(vec![H::new()])?` puts it there as a by-value argument — and
  then the value positions INSIDE it are on the spine too. But `idm!(use_h_ref(&H::new(), n))?` is also a
  macro node on the spine, and the `&` inside its tokens borrows a temporary that lives to the end of the
  statement and dies on that `?`'s error exit; the escape walk's own macro parse descends `Reference`, so
  that leaf really is in the escaping set and exempting the macro WHOLE leaves the drop lost. So the same
  spine enumeration is re-run over the parsed tokens and only the value positions inside them are exempt.
  Both halves were measured by degrading the code, not reasoned about, and the whole matrix was read
  rather than the first failing row: neutralising the new veto returns all ELEVEN charged shapes to the
  candidate's ABSENT, and exempting the macro whole recovers ten of them and leaves the `&` shape ABSENT.
  Neither degradation moves any over-charge control, so the second step is a pure recall gain.
  Over-charge controls that stay ABSENT (executed: 0 drops in-frame each): the macro on the spine
  (`Ok::<Vec<H>, ()>(vec![H::new()])?`, a `match` arm's tail, `idm!(H::try_new(n)?)`), a by-value macro
  argument (0 here, 1 in the callee's frame, inherited through the call edge), and the no-`?` twins.
  `m_macro_ctrl` — the same macro straight-line BEFORE the `?` — stays charged in every arm.
  1,504-crate A/B vs the candidate `7d9a970`, wide key (16 fields, not just `inferred`): ADDED 0,
  REMOVED 0, CHANGED 0, effects lost 0, call edges lost or gained 0 — byte-identical over 257,243 common
  rows. The new branch fires 10 times across 9 of the 1,504 crates (mongodb, nkeys, munge_macro,
  ratatui-termina, sea-orm-macros, tester, ureq), so the zero is measured over a corpus that reaches it —
  and all 10 were read from source: every one is a genuine off-spine macro construction in a `?` operand
  (`format!(.. Redact(host) ..)` and `err!(VerifyError, ..)` inside a `map_err` closure,
  `matches!(p, Ok(PemItem::Certificate(_)))` inside a `find` closure), and none of the eight leaves has a
  `Drop` impl in its crate, which is why no row moves. Published → this build reproduces published →
  `7d9a970` LINE FOR LINE, not merely by count: common 252,097, ADDED 5,146, REMOVED 699, LOST 620, with
  identical member sets and identical per-row lost-effect content, and the set of published `X::drop`
  edges the build no longer emits is identical too (symmetric difference 0 over 306 edges in 293 rows) —
  the control that R173's and R194's corpus gains stay gained. A 37-fixture regression battery
  (`panel-fixes`, `panel-fixes-2/3/4`, the four wave fixture sets, rusqlite 0.40.2 and crossterm 0.29.0)
  moves only the six `m_macro_*` cells on a full-report wide diff; the R200/R201/R202 cells and all
  fourteen R194 spine controls are unmoved.
- **⚠ SOUNDNESS R203 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R173 and reached
  by none of R187/R194/R199 — found by R199's own fix agent) — CLOSED: a `?` is live for what a macro in
  its operand built even when this scanner cannot READ that macro.** R199 taught the macro token walk to
  record the open `?` exits. Three constructions are not in those tokens at all, each invisible for a
  different reason: a crate-local `macro_rules!` TEMPLATE (`mk_h!("a")`'s tokens are `"a"` — the
  construction is in the DEFINITION); a token stream that is not an expression list
  (`stmts!(let x = H::new("a"); x)`); and a NESTED macro, or one in STATEMENT position, which the walk
  stops at. A fourth was the block boundary: the child walk stops at `{ .. }`, so
  `idm!({ out.push(H::new("a")); gen(n) })?` was invisible too.
  EACH OF THOSE ALONE IS HARMLESS, and that is the part worth writing down: a leaf with no recorded
  position counts as live from the start and is charged. The sin needs the PAIR — build the leaf
  invisibly inside the `?`'s operand AND visibly again after the `?` — because then the visible site's
  position carries the leaf past R173's filter while nothing ever vetoed it. Eight shapes read `['Fs']`
  on published 0.34.0 and were ABSENT on `70fd624`, `7d9a970` and `75053f1`, with `deny Fs <fn>` going
  1 → 0 on every one; executed ground truth is one `H::drop` in each frame on the error path (two for
  the loop-shaped one). Their direct twin — the same body with the macro spelled out — is charged in
  every build, so the discriminator is exactly the invisibility.
  ASK THE AUTHORITY, DON'T GUESS. The template case is answered by the collector's own R48
  `macro_rules!` index and its SINGLE-ARM rule, so the two paths cannot drift about what `NAME!(..)`
  expands to; the unreadable-tokens case is answered by asking `syn` for a STATEMENT sequence instead of
  treating the macro as empty. The spine exemption is again two steps: a template invoked ON the `?`'s
  value spine has its TAIL exempt and nothing else, which `use_u32(ph!(out, "a"), n)?` — a spine-resident
  template whose statement pushes into a `Vec` this frame owns — is the fixture for. Measured by
  degrading the code: reverting leaves all eight ABSENT, and exempting a spine-resident template WHOLE
  recovers seven and leaves exactly that one ABSENT. Everything here writes only the `?`-interior veto,
  never a construction site and never a position, so it can only ever charge more; the exemptions are
  the enumerated value spine and nothing else.
  Over-charge controls that stay ABSENT (executed: 0 drops in-frame): a template macro and a nested
  macro as by-value arguments on the spine, the `?`-free twin, and a macro that constructs nothing.
  THE MULTI-ARM TRADE, TAKEN DELIBERATELY AND PRICED. A MULTI-ARM `macro_rules!` has every parseable arm
  walked for this veto — and only for this veto: R48's single-arm rule still governs CALL-EDGE
  RESOLUTION, where a non-matching arm would ADD an effect, so there is still one authority on the
  resolution side. Here the term only refuses to certify an escape, so "some arm builds it" is the
  over-approximating answer and the one published 0.34.0 gave. It closes `r_multiarm_hole`
  (`{ out.push(two!(a "a")); gen(n) }?`, executed 1 in-frame drop, charged by published and silent from
  R173 onwards) and it over-charges `o_multiarm_pure`, whose invocation matches the arm that builds
  nothing (executed 0 in-frame drops) — a fabrication against the previous commit, and exactly what
  published already said. Both cells are in the fixture; the 1,504-crate A/B is byte-identical either
  way, so the corpus cannot choose, and the ruling is that a SILENCE against published outranks an
  over-charge AT published parity. One thing this still does not do: statement-only tokens on the spine
  are charged with 0 in-frame drops — an over-charge, at published 0.34.0's parity, invisible to `deny`.
  1,504-crate A/B vs `75053f1`, wide key (16 fields): ADDED 0, REMOVED 0, CHANGED 0, effects lost 0,
  call edges lost or gained 0 — byte-identical over 257,243 common rows. The new branch fires 34 times
  across 16 of the 1,504 crates (jni 0.22.4, mysql_common 0.32.4/0.37.3, thirteen `syn` versions) — 46
  once every arm of a multi-arm template is walked, the extra twelve all being `syn`'s 100-arm `Token!`
  macro used inside a `?` operand — so the zero is measured over a corpus that reaches it, and every
  leaf it reaches (`Ok`, `Err`, jni's `Error`, `syn::token::Macro`) has no `Drop` impl anywhere in its
  crate, which is why no row moves. R199's counter
  is unchanged at 10 hits in 9 crates. Published → this build reproduces published → `7d9a970` LINE FOR
  LINE across all six diff lists (ADDED 5,146, REMOVED 699, LOST 620, GAINED 1,752, lost call edges 503,
  lost `unknownWhy` 2,118), identical member sets and identical per-row content. A 40-fixture regression
  battery moves only the R203 cells on a full-report wide diff; the R200/R201/R202 cells and all fourteen
  R194 spine controls are unmoved.
- **⚠ SOUNDNESS R204 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R203 above,
  caught by a fifth fix-lens pass before release) — CLOSED: a statement-position macro INSIDE a
  `macro_rules!` template, block tokens or statement tokens is now walked exactly like one written in
  the function body.** R203 taught `walk_block` that a `Stmt::Macro` never reaches the `Expr::Macro`
  machinery and routed it through the full interior walk — template lookup, then the expression-list
  parse, then the statement parse. The statement half of the INTERIOR walk kept calling the statement
  parse alone, so one level in, inside anything the veto had to re-parse, both of the other two paths
  were skipped. Four shapes read `['Fs']` on published 0.34.0 (tag `736fa64`) and were ABSENT on
  `5cefa62`/`c22a31d`, `deny Fs` 1 → 0 with no `Unknown`: a template whose body is `push_h!($o, $p);`
  (only the template lookup can see that `H`), a template whose body is
  `println!("{}", H::new($p).p);` (only the expression-list parse can), `idm!({ push_h!(out, "a");
  gen(n) })?` and `stmts!(push_h!(out, "a"); 1u32)`. Executed ground truth: one `H::drop` runs in each
  frame on the error exit, and each body builds the same leaf again after the `?` — that pair is the
  sin, either half alone is harmless. The new call is a strict SUPERSET of the old one: the statement
  parse it replaced passed an EMPTY exemption set, so nothing it reached was ever exempt, and the
  replacement passes an all-`false` spine for the same reason (a statement macro's value is discarded)
  and still ends at the identical statement parse when the tokens are not an expression list. It writes
  `TryExit::interior` only, so over-reach over-charges and cannot silence. Over-charge control that
  stays ABSENT (executed: 0 in-frame drops): a nested macro as a by-value argument ON the `?` operand's
  value spine — the cell that a sloppier version of this fix, flattening the spine everywhere rather
  than for statement macros only, would charge. 1,504-crate A/B vs `c22a31d`, wide key (16 fields):
  ADDED 0, REMOVED 0, CHANGED 0, effects lost or gained 0, call edges lost 0 — byte-identical over
  257,243 common rows, and published → this build reproduces published → `c22a31d` LINE FOR LINE across
  all six diff lists (ADDED 5,146, REMOVED 699, LOST 620, GAINED 1,752, lost call edges 503, lost
  `unknownWhy` 2,118), identical member sets and identical per-row content. The zero is measured over a
  corpus that REACHES the branch: the new call fires 27 times across 7 of the 1,504 crates (four
  `rustix` versions, `ring`, `tokio-util`, `termwiz`) on `assert!`, `prefixed_extern!`,
  `tracing::trace!`, `eprintln!` and `core::assert!`, and no leaf it reaches has a `Drop` impl, which
  is why no row moves. R194's, R199's and R203's own counters are unchanged at 2,821 / 10 / 46. A
  42-fixture regression battery (the ledger's 40 plus round 5's `r5`/`r5b`) moves only those four cells
  on a full-report wide diff; every other dir is identical.
- **⚠ SOUNDNESS R205 (cardinal sin, a REGRESSION against published 0.34.0 in the same family, caught by
  the fifth fix-lens pass) — CLOSED: a crate-local `macro_rules!` template invoked BY PATH is looked up
  by its leaf instead of being skipped.** The `?`-interior veto asks R48's crate-wide index what a
  template CONSTRUCTS, and bailed out on any name containing `::` — while `$crate::helper!(..)` is the
  canonical hygienic spelling for an exported macro calling a helper in its own crate, and
  `strip_dollars` renders it `crate::helper`. So the lookup refused the shape it most needed to read.
  Three cells read `['Fs']` on published 0.34.0 and were ABSENT on `5cefa62`/`c22a31d`, `deny Fs`
  1 → 0: `push_via!` → `$o.push($crate::mk_h!($p))` in statement position, `via!` →
  `$crate::mk_h!($p)` in expression position, and a user-written `crate::mk_h!("a")`. Executed ground
  truth: one `H::drop` per frame on the error exit. The same chain with the inner name spelled BARE is
  charged in every build, so what these rows test is the `::`, not the chaining. SAY WHICH DIRECTION IT
  FAILS IN: a leaf is not proof the macro is local — a `dep::mk_h!` whose leaf collides with a local
  `macro_rules! mk_h` now resolves too and walks the wrong template. That is deliberate and it is the
  safe direction, because this path only ever adds to the `?`-interior veto (a refusal to certify an
  escape) and the branch it replaces added nothing at all, so every outcome of a wrong resolution is an
  over-charge; the corpus measures the cost at zero rows. Over-charge control that stays ABSENT
  (executed: 0 in-frame drops, 1 inside the callee): the same `$crate::`-chained construction as a
  by-value argument ON the `?` operand's value spine. 1,504-crate A/B vs `c22a31d`, wide key: ADDED 0,
  REMOVED 0, CHANGED 0 over 257,243 common rows, and published → this build reproduces published →
  `c22a31d` line for line across all six diff lists. The corpus REACHES the branch: 6 templates walked
  through a path in 3 of the 1,504 crates — `objc2-foundation` (`crate::ns_string!`,
  `crate::__ns_string_inner!`), `windows-core` (`crate::s!`) and `mongocrypt` (`error::internal!`,
  `error::encoding!`) — none of them reaching a leaf with a `Drop` impl, which is why no row moves.
  A fourth crate, `tokio-util`, is the wrong-resolution case measured rather than argued: its own
  `macro_rules! trace` invokes the `tracing` CRATE's `tracing::trace!`, the leaf rule resolves that back
  to the local `trace`, and R48's recursion guard stops it — three reaches, zero walks, zero rows.
- **⚠ SOUNDNESS R206 (cardinal sin, a REGRESSION against published 0.34.0 in the same family, caught by
  the fifth fix-lens pass) — CLOSED: a `macro_rules!` defined INSIDE a function body is now visible to
  the `?`-interior veto.** A body-local definition is a `Stmt::Item`; `decls.rs` builds R48's
  crate-wide `macro_rules!` index from ITEM-level definitions only, and the veto's own walk discarded
  every `Stmt::Item`, so a template written where it is used sat in no index either reader could
  consult. `{ out.push(lm!("a")); gen(n) }?` with the same leaf built again after the `?` read `['Fs']`
  on published 0.34.0 (tag `736fa64`) and was ABSENT on `5cefa62`/`c22a31d`, `deny Fs` 1 → 0 with no
  `Unknown`; executed ground truth is one `H::drop` inside the frame on the error exit. `walk_block`
  now records those definitions into a per-body overlay as the walk passes them (a `macro_rules!` is
  not usable before its definition, so walk order is scope order), and the veto reads the overlay
  ALONGSIDE the crate index.
  BOTH TEMPLATES ARE WALKED, NEVER ONE INSTEAD OF THE OTHER, and the sibling fixture is why. Rust
  shadows a crate-level macro with a body-local one of the same name, so "overlay first, index as
  fallback" reads like the correct model — and it is a NEW cardinal sin, because the overlay is per
  BODY and not per BLOCK: a `macro_rules! same` inside an `if` that has already closed then answers for
  the crate-level `same!` a later block really uses, and a template that constructs is replaced by one
  that does not. Measured, not argued: on the round-5 prototype that cell is published `['Fs']`,
  `c22a31d` `['Fs']`, prototype ABSENT. Walking both is a strict superset of `c22a31d` — every leaf it
  put in the `?`-interior set is still put there — and this term only ever REFUSES to certify an
  escape, so the superset can over-charge and cannot silence. Over-charge control that stays ABSENT
  (executed: 0 drops in the frame, 1 inside the callee): the same body-local construction as a
  by-value argument ON the `?` operand's value spine.
  1,504-crate A/B vs `c22a31d`, wide key (16 fields, cold, same dir list, same host): ADDED 0,
  REMOVED 0, CHANGED 0 over 257,243 common rows — and published → this build reproduces published →
  `c22a31d` line for line across all six diff lists (ADDED 5,146, REMOVED 699, LOST 620, GAINED 1,752,
  lost call edges 503, lost `unknownWhy` 2,118), identical member sets and identical per-row content.
  The zero is measured over a corpus that REACHES the branch: the overlay answers 59 times across 9 of
  the 1,504 crates (`jni` 18, `mysql_async` 10+10, `time` 7, `chrono` 6, `h2` 3+3, `mysql_common`,
  `color-print-proc-macro`), and R204's counter rises 27 → 33 because six more statement-position
  macros are now reached inside those bodies. No leaf any of them constructs has a `Drop` impl, which
  is why no row moves. A 43-fixture regression battery (the ledger's 40, round 5's `r5`/`r5b`, and the
  new body-local fixture) moves only the R204/R205/R206 cells on a full-report wide diff.
  AND THE SAME DEFECT ONE WALK IN, found by widening the audit boundary past its own trigger rather
  than by a second report. `note_opaque_block` — the walk the veto uses over a template body, block
  tokens or statement tokens — discarded `Stmt::Item` for the identical reason, so a `macro_rules!`
  written INSIDE those tokens was still in no index:
  `idm!({ macro_rules! lm { .. } out.push(lm!("a")); gen(n) })?` is published `['Fs']` and ABSENT on
  `c22a31d` and on the body-level fix alone, executed 1 in-frame drop. The same recording arm now runs
  in both walks. It is R203 → R204's relationship one row over, and the row ID for it is the
  coordinator's to assign — this entry deliberately does not invent one. SAY IT AT THE TIME: its
  counter fires **0 times in the 1,504-crate corpus**, so its A/B (ADDED 0, REMOVED 0, CHANGED 0) is
  SAFETY-ONLY evidence, not reach evidence; the reach evidence is the executed fixture and the
  discriminator beside it (the same use site with a body-level definition, which the first half
  closes).
- **⚠ SOUNDNESS R207, repetition half (cardinal sin, a REGRESSION against published 0.34.0 in the same
  family, caught by the fifth fix-lens pass) — CLOSED: a `macro_rules!` template whose body is a
  REPETITION is now read by the `?`-interior veto. The remaining half SHIPS OPEN and is stated below.**
  `$( X ) sep? *` survived `$`-stripping as `( X ) *`, which parses as no statement, so the arm was
  dropped whole and `{{ $( $o.push($crate::H::new($p)); )* }}` — the ordinary way to write a template
  that pushes — was as invisible as an unreadable one. Three shapes read `['Fs']` on published 0.34.0
  (tag `736fa64`) and were ABSENT on `5cefa62`/`c22a31d`, `deny Fs` 1 → 0: a `;`-terminated repetition
  of pushes, a repetition in VALUE position, and a repetition whose body itself contains a `?`.
  Executed ground truth: 2, 1 and 2 in-frame drops on the error exit.
  ONE ITERATION IS AN OVER-APPROXIMATION, WHICH IS WHY IT IS VETO-SIDE ONLY. A `$(..)*` can run zero
  times, so "the body ran once" is a fact about no expansion — acceptable here because everything this
  path produces lands in the `?`-interior set, a REFUSAL to certify that a leaf escaped. R48's
  resolution expands templates to ADD call edges and keeps the unflattened reader; the arm-splitting
  walk is shared, so the two cannot drift about arm COUNT, which is what R48's single-arm rule turns
  on. The boundary is measured, not asserted: routing R48 through the flattened reader as well ADDS
  178 rows and 12 `Unknown` effects over the 1,504-crate corpus (jni-0.22.4 59 and all 12 effects,
  `memchr` 21 per version, `windows-result`, `lexical-write-float`) — the fabrications the boundary
  exists to prevent. A unit test drives both readers over one repetition body so a future leak goes
  red rather than quiet. Over-charge control that stays ABSENT (executed: 0 drops in the frame, 1
  inside the callee): a repetition template's value as a by-value argument ON the `?` operand's value
  spine.
  **STILL OPEN AND SHIPPING OPEN, by the owner's ruling: a macro whose INVOCATION TOKENS parse neither
  as an expression list nor as statements AND whose template is a repetition over those tokens.**
  `kv!("a" => H::new("a"))` (maplit-style) and `pick!(n, 0 => H::new("a"), _ => ..)` are SILENT where
  published 0.34.0 charged them, because it vetoed blanket over any macro it could not read. Measured
  corpus incidence: 0 rows of 1,504 crates. Exposure: 387 functions in 51 crates have a
  both-forms-unreadable macro inside a `?` operand (`rustix` ~45 per version, `jni` 27, `mio` 22,
  `polling` 12, `mongodb` 10, `syn`, `socket2`, `duct`). The blanket alternative — an unreadable macro
  vetoes everything in that `?` — closes them and changes 7 corpus rows, all in jni-0.22.4, all
  fabrications, 6 of them beyond published parity, so it is not the fix. A regression test pins the
  silence so that closing it later cannot happen silently. Same for R208's same-name `macro_rules!`
  twins, where which module's template is read depends on FILE ORDER: 324 of the 1,504 crates define a
  `macro_rules!` name more than once (cfg twins), measured corpus rows 0, and the fix is an index that
  records duplicates and REFUSES rather than picking — a separate mechanism.
  1,504-crate A/B vs `c22a31d`, wide key (16 fields, cold, same dir list, same host): ADDED 0,
  REMOVED 0, CHANGED 0 over 257,243 common rows — and published → this build reproduces published →
  `c22a31d` line for line across all six diff lists, identical member sets and identical per-row
  content. The zero is measured over a corpus that REACHES the branch: `R207FLATARM` counts arms only
  the flattened reader can parse and fires 236 times in 10 of the 1,504 crates (`rustix` 62/57/57,
  `jni` 44, `ring` 5, `xattr`, `trybuild`, `time-macros`, `color-print-proc-macro`, `time`); R203's
  and R206's counters rise 46 → 48 and 59 → 60 on the templates that became readable. No leaf any of
  them constructs has a `Drop` impl, which is why no row moves. The 43-fixture wide battery moves only
  the R204/R205/R206/R207 cells.
- **⚠ SOUNDNESS R174 (fabrication) — CLOSED: the return-index construction route now honours the same
  refusals as the path route.** (a) A `std`/`core`/`alloc`-rooted callee is refused for a reason
  (a std path names no local type, and the drop index is leaf-keyed); R165's fallback keyed the same
  call on its bare LEAF and undid it, so a crate with a local `fn open(..) -> Conn` charged `Conn::drop`
  to every `File::open(p)?` caller. event-listener's `full_fence` read `AtomicUsize::new(0)` as
  `Event::new`; jobserver's `Client::release` read `io::Error::new` as `windows::Handle::new`, on a unix
  fd. (b) R165's comment claimed the return index "already drops any leaf recorded with two different
  return types" — false when the twin returns `()`, which `record_return` skipped: git2's free
  `crate::init()` beside `Repository::init` gave 76 functions per version a phantom `Repository::drop`
  edge, visible in `path`/`callers` while `inferred` stayed quiet. A unit return is now a conflicting
  shape like any other. 1,504-crate A/B: REMOVED 18 rows, 7 effect losses, all audited; 8 of the 18
  (syn, windows) are a precision COST — pure functions losing an `invisible` disclosure and two call
  edges to the new ambiguity, recorded as a loss rather than a gain.
- **⚠ SOUNDNESS R188 (cardinal sin, a REGRESSION against published 0.34.0 introduced by R174(b)
  above, caught by the same second pass) — CLOSED: a unit-returning twin withdraws the DROP route's
  answer, not the binding's type.** R174(b) filed its sentinel under the fn's own LEAF, so the twin
  ambiguated that leaf for every reader of the return index — including `let` typing, where `()` has
  no INHERENT methods, so a body that resolves `c.send(b)` through the index could not have called
  the unit twin. With `net::connect(addr) -> Conn` beside
  an unrelated unit `ui::Ui::connect(&mut self)`, the module-qualified — unambiguous — `let c =
  net::connect(addr); c.send(b)` lost `c`'s type and went `['Net']` → ABSENT with no `Unknown` at
  all: `deny Net go` and `pure go` 1 → 0, and the same through a `use crate::net::connect` import.
  The conflict is still recorded, under its own `<unit><leaf>` key, and read by
  `ctor_leaf_from_call_returns` alone — where git2's phantom `Repository::drop` edge actually came
  from, and that fixture stays fixed (each half of this change has its own red test). 1,504-crate
  A/B vs the candidate, wide key: ADDED 8, REMOVED 0, effects lost 0, call edges lost 0 — the 8 are
  rows published 0.34.0 also has, restored byte-identical (syn ×7 `item::parsing::peek_signature`,
  whose leaf `keyword` has a unit twin one module over; windows-0.56.0 `ID3DInclude_Vtbl::new`, the
  direction R174's own counter could not see) — plus 7 rows that GAIN back a real call edge audited
  from source: diesel's four `AnsiTransactionManager` transaction methods recovering all four
  `Instrumentation::on_connection_event` implementors, itertools `diff_with`, ring
  `KeyPair::from_components_`, tokio `oneshot::Inner::close`. The refusal fires 1,726 times across
  86 crates, so the zero-loss is measured over a corpus that reaches it.
- **⚠ SOUNDNESS R71 (cardinal sin, PUBLISHED in 0.34.0) — CLOSED: a callback stored in a field and
  invoked through an unwrap BINDER is disclosed.** `if let Some(f) = &self.cb { f() }` — the idiomatic
  optional-callback-field shape — reported the invoking function ABSENT from `functions[]`, with no
  `Unknown` and no `unresolved`, while the direct spelling `(self.cb.as_ref().unwrap())()` was honest:
  two spellings of one operation disagreeing, and a scoped `deny` over the invoking function PASSING
  while the effect provably runs. The cause is a two-table split — an `Fn`-family binding lands in
  `trait_vars`, which feeds only the `.method()` dispatch resolver, while the call-SYNTAX resolver
  consults only `fn_typed_vars`, so the binding was live and never consulted at its own invocation
  site. Fixed on `main` in `3cf055d` — which was pushed, never released — and shipping here for the
  first time. (Residual, filed as R177: the same shape where the payload is a `Box<dyn Fn>` reached
  through a `type` ALIAS is still ABSENT.)
- **SOUNDNESS R171 (instrument) — the coverage-gate GENERATOR lost 24 rows in its own renderer** (a
  render-time `.dedup()` merged distinct entry points that share a crate-root alias and an effect set),
  diffed on a key it chooses by a different rule in each manifest, made a `pub fn` inside an inline `mod`
  invisible and an `impl` inside one mis-keyed, and picked a crate's registry version lexicographically
  (R162). Fixed: an `entry` identity column in both manifests with closed accounting that exits non-zero,
  inline-mod module paths, a semver version pick, and `generate.py --selftest` on every push — the
  generator's first test surface. Manifests regenerated in the new format: 1,213 covered and 439 open
  entry points (1,239 / 451 identity-keyed rows); no regressed row is a scanner regression (`1f24fc9`).
- **⚠ SOUNDNESS R160 (cardinal sin, PUBLISHED in every release to date) — CLOSED: a sibling call
  qualified with `Self::` now resolves exactly as `<Type>::` does.** `Self` is bound in the ordinary
  import map for the length of each `impl` block, so the one path-expansion routine answers every
  position; no second matching arm. `rusqlite::Connection::open`, `open_in_memory`,
  `open_in_memory_with_flags` and `open_in_memory_with_flags_and_vfs` were ABSENT from `functions[]`
  and now read `['Db','Unknown']`. (An earlier draft of this entry said `pure <forwarder>` "goes from
  binding nothing to a violation". That is FALSE and is corrected here: a policy SCOPE is matched by
  PREFIX, so `pure Connection::open` already binds `open_with_flags`/`open_with_flags_and_vfs` and
  exits 1 on published 0.34.0. The true example, measured on rusqlite 0.39.0 with `candor-query gate`,
  is a scope that binds nothing there: `deny Db Connection::open_in_memory` reports "policy rule
  matched NO function" on the published build and a violation on this one.) 1,489-crate A/B: 2,621 functions gained an entry, 2 lost one (both audited correct),
  no `inferred` set shrank (`bb4851b`).
- **⚠ SOUNDNESS R161 (cardinal sin, published) — CLOSED: a callback typed through a TYPE ALIAS, or a
  bare `fn` pointer unwrapped out of an `Option` by `if let`/`match`/`.map`, is now disclosed as
  `Unknown`.** `is_callable_type` consults a crate-wide `callable_aliases` index and unwraps
  `Option`/`Result` before asking; a bare fn-pointer payload counts as callable. `rusqlite::
  init_auto_extension` read `[]` and now reads `['Unknown']`. Stated residuals: a double alias and an
  alias-typed RETURN. Cache revision 17 (`ba203cf`).
- **⚠ SOUNDNESS R165 (cardinal sin) — CLOSED: a `Drop` value obtained from a FREE-FUNCTION
  constructor (`fn from_handle(p) -> H { H { .. } }`) is now charged for its drop like one from
  `Type::assoc()`.** The marker reads the crate's own return index — a declared fact, not a name
  guess. The escape side learns the same fact through the same function, so forwarding and
  `mem::forget` controls stay uncharged (`8603f3a`).
- **⚠ SOUNDNESS R166 (cardinal sin) — CLOSED: nine `sqlite3_*` leaves added to the `Db` set after an
  audit of the whole 185-symbol surface**, not the one name that triggered it: the process-global
  auto-extension registry (`auto_extension`, `cancel_`/`reset_auto_extension`,
  `enable_load_extension`) and database-content I/O (`serialize`, `deserialize`, `db_cacheflush`,
  `file_control`, `wal_autocheckpoint`). Per-connection callback installers are deliberately still
  absent and named in the file. Gains in rusqlite, diesel and sqlx-sqlite (`dfc14c9`).
- **⚠ SOUNDNESS R168 (cardinal sin) — CLOSED: a by-value `Drop` parameter's drop is no longer
  suppressed because the function returns a fresh value of the same type leaf.** The parameter route
  has its own emission point that skips the construction-leaf gate only; its value-keyed escape gate
  is untouched. Measured over-charge, stated: 4 x11rb `replace_connection` rows where `mem::forget`
  sits one call away (`fe15158`).
- **⚠ SOUNDNESS R169 (cardinal sin, published) — CLOSED: two `#[cfg]`-gated re-exports of the same
  name are a UNION, not an ambiguity to drop.** `crossterm::terminal::size`, `window_size`,
  `enable_raw_mode`, `disable_raw_mode` and `is_raw_mode_enabled` were ABSENT and now carry their
  platform module's effects. Explicit import beats glob, as in Rust. The union exposed R170 (a module
  whose `pub use` is inside `cfg_if!` cannot contest a key it owns), guarded for the keys the union
  newly admits; the universal guard is open (`c3e1660`).
- **⚠ SOUNDNESS R175 (fabrication introduced by R160, caught before release) — an `impl` NESTED IN A
  METHOD BODY now rebinds `Self` to its own type.** R160 bound `Self` for the length of each FILE-LEVEL
  `impl`; a `fn outer() { struct N; impl N { fn go() { Self::eff() } } }` is walked as part of `outer`'s
  body, where `Self` still named the OUTER type — so `A::outer` was charged `A::eff`'s `Fs` over a
  nested `N::eff` that is pure (published 0.34.0 has no row for it at all). The binding is restored on
  exit, a nested `trait`'s default body gets the same treatment, and a non-nominal `impl Trait for
  &[u8]` UNBINDS rather than inheriting. 1,504-crate A/B against the 0.35.0 candidate: 3 rows removed,
  0 effects lost — all three are serde_with's `impl Visitor for Helper { .. Self::Value::parse(..) }`,
  whose `invisible: ["time_0_3"]` came from expanding `Self` through the OUTER type's import; the
  answer now matches the explicitly-written spelling exactly, which is the invariant. Also removes 8
  fabricated call edges (hashlink's `Default for …Visitor { fn default() { Self::new() } }` was edging
  to `LinkedHashMap::new`). MEASURED AND NOT FIXED: a crate-local `macro_rules!` expanding to a call is
  silent in every position, unchanged in all three arms.
- **⚠ SOUNDNESS R176 (cardinal-sin shape introduced by R169, caught before release) — explicit import
  beats glob WITHIN one configuration, never ACROSS `#[cfg]` arms.** `#[cfg(unix)] pub use unix::*`
  (whose `size` spawns a process) beside `#[cfg(windows)] pub use windows::size` (which reads a file)
  made the caller publish `['Fs']` ALONE: the live platform's `Exec` silenced behind a positive claim
  about the other one, on a function published 0.34.0 has no row for. The two arms never coexist in any
  build, so neither shadows the other and the answer is their union — the same treatment this index
  already gives the `#[cfg_attr(path)]` spelling of the split. `Reexport` records whether its `pub use`
  is `#[cfg]`-gated (cache revision **18**; the rev17 the R161 entry above describes was never applied
  to the schema string, so two builds of one version could share a key — fixed by skipping to 18). The
  narrowing now fails toward the UNION, i.e. toward over-charging, never toward silence; the no-`cfg`
  case is unchanged. The union falls back to the explicit set where it would exceed the re-export
  fan-out cap, so widening an answer can never DROP one — measured, on a 14-definition fixture where it
  did. 1,504-crate A/B: 10 unions across 1 crate (`cap-primitives`' `rustix` platform arms), 0 rows
  removed, 0 effects lost.
- **⚠ SOUNDNESS R177 (cardinal sin, PUBLISHED and still open on the 0.35.0 candidate) — CLOSED: a
  callback stored as `Option<Alias>` where `Alias` is a `type NAME = <callable>` is now disclosed at
  every unwrap binder.** `pub type Cb = Box<dyn Fn()>; cb: Option<Cb>` with `if let Some(c) = &self.cb
  { c() }` was ABSENT from `functions[]` — an affirmative purity claim over a caller-installed
  callback, executed ground truth — while the `.unwrap()()` spelling one token away disclosed
  `Unknown`. R161 closed the bare-fn-pointer payload and listed this one as not established.
  `elem_trait_leaves` now asks `is_callable_type`, the one authority for "is this value invokable",
  instead of being a second implementation that had not been told about aliases; and a per-file
  pre-pass collects a file's `type NAME = <callable>` names to a fixpoint BEFORE the walk that consumes
  them, so declaration order and a same-file alias CHAIN no longer decide the answer (R181's double
  alias and alias-typed return, for a same-file alias). `if let`/`match`/`.map`/`.as_ref()`/`while
  let`/`for`, an `Option<Alias>` parameter and a `let` annotation all disclose; `deny Unknown <fn>`
  goes 0 → 1. Stated residual: a CROSS-FILE alias is invisible to the per-file field index, because
  `FileDecls` is keyed on one file's content. 1,504-crate A/B: 0 effects lost.
- **⚠ SOUNDNESS R139 (cardinal sin, introduced by R119's own fix) — CLOSED: a crate-local
  `macro_rules!` TEMPLATE now counts toward the body-item shadow, so a nested block's item can no
  longer rebind a name the macro expansion uses.** R119 promotes a nested block's item to a
  function-wide shadow only when every occurrence of the name lies inside that block, and answered
  that with `count_ident`, which walks the body. `CallCollector::visit_macro` inline-expands a bare
  `NAME!(..)` from `local_macros` (R48) whose tokens live at FILE level — often in another file — so no
  walk of the body could reach them: the counts came out equal, the shadow was promoted, and the
  `Cmd::new` the expansion injects resolved to the `<body-item>` sentinel. Ground truth EXECUTED (a
  generated script that touches a marker file; the marker exists, so a process really ran):
  `f -> ["Exec"]` on `3cf055d` — which was PUSHED, never RELEASED; the v0.34.0 tag is `736fa64`, six
  commits earlier — **ABSENT at HEAD `5af1a27`**, and `deny Exec`, `deny Exec f`,
  `pure f` and `deny Exec Unknown` all fell exit 1 → exit 0 (`deny Unknown` correctly 0 either way).
  `count_ident` now expands through `collector::macro_template_blocks` — the same helper the collector
  injects with, so the two cannot disagree about what `NAME!` expands to — and falls back to counting a
  template's RAW tokens where that helper is opaque (multi-arm, `$(..)*`), which over-counts and can
  only drop a shadow. The count stays SYMMETRIC, so a macro invoked INSIDE the declaring block is still
  promoted (`macro_rules!` is unhygienic for items and types, so the expansion really does bind there)
  — pinned by a control that goes red under the naive "never promote when a local macro exists" fix.
  Six routes measured against a pre-fix binary, all ABSENT before and `["Exec"]` after: file-level
  macro, a macro in ANOTHER FILE, a template invoking a second macro, an invocation nested in another
  macro's opaque argument tokens, a redefined macro name, and a self-mentioning template.
  A/B over 1490 registry crates, wide key (every field of every row, plus the report-level keys):
  **ADDED 0, REMOVED 0, CHANGED 0 over 252,772 common rows** — and the changed branch was REACHED:
  new `BODYMACRO-REACH` / `BODYMACRO` counters fire on **6 promotions in portable-atomic 1.13.1 and
  1.15.0**, all flipping to DROP, none of which moves a row because `cmpxchg16b` is not a key in that
  file's `use` map (the pre-fix shadow was already a no-op there — no `BODYSHADOW` in either arm).
  **So this A/B is an over-charge control only; the recall evidence is the executed fixture, not the
  corpus** — the corpus contains no instance where the flip could have changed a row.
  `BODYMACRO` had to be a NEW counter: R119's `BODYSCOPE-ADD`/`DROP` compare against the FLATTENED
  collector, which gives the R139 shape the SAME answer, so R119's own corpus A/B could not have seen
  this and its `CHANGED 0` read as quiet.
  **STILL OPEN, measured and NOT fixed here** (separate mechanisms, all pre-existing, all silent, each
  demonstrated by a compiled-and-run fixture that really spawns): a `macro_rules!` defined INSIDE a
  function body is never recorded in `local_macros` at all; a MULTI-ARM macro's template is
  deliberately not expanded (the anti-fabrication trade) and carries no disclosure when it hides an
  effect; a `$(..)*`-repetition template is unparseable and likewise silent; and `include!` text is
  never read.

- **⚠ SOUNDNESS R123 (cardinal sin, PRE-EXISTING and PUBLISHED) — CLOSED: two `cfg`'d `use` lines no
  longer resolve by SOURCE ORDER, so a production scan can no longer resolve through a TEST MOCK.**
  `collect_use` inserts into a `HashMap`, so the last spelling of a name won, and the idiomatic mocking
  pair is two mutually-exclusive `cfg`s — with the mock's import written second, a production scan
  resolved `Runner::new(p).status()` through a mock that is pure by construction and `run` vanished from
  `functions[]`. Ground truth EXECUTED: two crates identical in every byte but the order of those two
  lines, both `cargo run` in a normal build printing `ran=true`. One `use_item_applies`/`collect_item_uses`
  authority now answers at all five sites — `scan_items`, `collect_decls` and `collect_root_reexports`
  were the three unfiltered ones, and the last had no `include_tests` parameter to apply. The fixture is
  closed ORDER-INDEPENDENTLY, which is the assertion the inverted test now pins.
  A/B over 1489 registry crates keyed on every field: **ADDED 0, REMOVED 7, 5 crates moved**; §E1 reach
  310 `use` items filtered across 59 crates. hickory-resolver GAINS a real `Clock` on 6 rows. **The
  tokio `fs` losses this fix was held for are gone**: every `tokio::fs` verb keeps its `Fs`,
  `pure fs::symlink::symlink` stays exit 1, and the `["Log","Unknown"]` that disappears is exactly what
  candor was reading out of `#[cfg(test)] mod mocks` — the removed `fs::asyncify` row records
  `calls: ["fs::mocks::spawn_blocking"]` in its own pre-image. **STILL OPEN, deliberately, and pinned by
  a test that asserts the defect:** a `use` written inside a function BODY is still unfiltered and still
  order-decided (`LocalUseCollector`, `CallCollector::visit_item_use`) — 20 sites in 12 of the 1489
  crates. Cache schema rev15 → rev16.

- **⚠ NEW CARDINAL SIN, FOUND AND FIXED HERE (no row number — the coordinator assigns those):
  `std::os::{unix,windows,wasi}::fs` had NO filesystem rule at all, so `symlink`, `chown`, `lchown`,
  `chroot` and the platform `FileExt` positional I/O all read PURE.** `std::fs::` was the whole
  filesystem prefix rule and the platform-specific half of std's filesystem API simply was not under
  it. Ground truth EXECUTED: a crate whose `pub fn a(o,l) { std::os::unix::fs::symlink(o,l) }` really
  creates a symlink on disk (`cargo run` printed `symlink=true exists_as_symlink=true`) reported
  `functions: []` for it, so `deny Fs` exited 0 over a real filesystem write; `std::fs::hard_link` and
  `std::fs::read_to_string` beside it were correctly `["Fs"]`, which is what makes this a gap and not a
  design choice. Found while tracing why `tokio::fs::symlink` carries no `Fs` of its own. A DENYLIST
  keyed on the TRAIT, like the existing `OpenOptions`/`DirBuilder` carve-outs: `MetadataExt`,
  `DirEntryExt`, `FileTypeExt`, `PermissionsExt`, `OpenOptionsExt` and `DirBuilderExt` stay pure (they
  read or configure data already in hand — charging them would fabricate `Fs` on every `m.uid()`), and
  `FileExt` deliberately does not. A/B over 1489 registry crates keyed on every field: **ADDED 34,
  REMOVED 0, no surviving row lost a value in any field**, and every added row is an fs syscall wrapper
  in cap-std, fs-err, async-fs, async-std, jiff, snapbox and tokio. Revert-tested by neutering the rule.

- **⚠ SOUNDNESS R128 (cardinal sin, PRE-EXISTING and PUBLISHED): a call into a module whose items are
  hidden behind an unexpanded MACRO no longer reads PURE.** `collect_decls` skips item-position macro
  invocations, so a `pub fn` or a `pub(crate) use` declared by `cfg_rt! { .. }` / `foo!();` /
  `include!("gen.rs")` contributes no unit and no re-export edge — and the caller matched nothing
  anywhere and fell out of the resolver silently. Three shapes, each COMPILED AND RUN spawning a real
  process, each leaving the caller ABSENT from `functions[]` before this: a macro body holding a
  `pub(crate) use` re-export (tokio's `cfg_rt! { pub(crate) use crate::runtime::spawn_blocking; }`,
  async-std's `cfg_default! { pub use spawn_blocking::spawn_blocking; }`), a macro body declaring the
  `pub fn` itself, and an item-position `include!`. The second is the worst: the TARGET has no report
  row either, so even blanket `deny Exec` exited 0. Such a call now discloses `Unknown` with
  `unknownWhy: ["ambiguous:module items hidden by an unexpanded macro"]` (§4's existing fifth kind —
  the analyser's own name resolution failed; no new kind is invented here, which would need a SPEC
  clause and a conformance PART first). Real recall gained, ground-truthed from source: nix's
  `fcntl::open` (inside `feature! { .. }`), find-msvc-tools' whole `windows_sys` FFI surface
  (`windows_link::link!` — `RegOpenKeyExW`, `LoadLibraryA`, `CoCreateInstance`), reqwest's
  `into_url::try_uri` (inside `if_hyper! { .. }`), tiff's `bytecast` and moxcms' SIMD conversions.
  **The hedge is charged on EVIDENCE, never on a name heuristic**: the owning module must be one whose
  item list demonstrably could not be read. Measured over a 1489-crate registry corpus, the general
  "any unresolved crate-rooted call → Unknown" rule hits **66,196 sites in 741 of 1489 crates**; this
  one reaches **325 sites in 44 crates**. A/B keyed on EVERY field: **ADDED 193, REMOVED 0, and no
  surviving row lost a value in any field** — purely additive. Blanket `deny Unknown` over all 44
  affected crates: **0 verdicts flipped**. Residual over-charge, measured and stated rather than
  claimed away: ~34 of the 325 hits construct a macro-declared TYPE, which is pure. Cache schema
  rev14 → rev15.

- **⚠ SOUNDNESS R122 (cardinal sin, PRE-EXISTING and PUBLISHED — reproduces on `origin/main` `3cf055d`):
  a production function under `#[cfg(any(test, feature = "x"))]` is no longer erased from the report.**
  `is_cfg_test` recursed into `any` and `all` alike, and `cfg_meta_requires_test`'s own doc claimed the
  item "POSITIVELY requires test" — correct for `all(test, X)`, exactly backwards for `any(test, X)`,
  which compiles into an ORDINARY build whenever X holds. Ground truth EXECUTED: a crate with
  `default = ["extra"]` and `#[cfg(any(test, feature = "extra"))] pub fn prod_under_any(p) {
  Command::new(p).status() }` really spawns the process, and candor reported `functions: []`,
  `analyzed.count: 0`, `excluded: []` — `deny Exec`, `deny Exec Unknown`, `pure prod_under_any` and
  `deny Exec prod_under_any` ALL exited 0 (now 1; `deny Unknown` correctly stays 0). Real victims found
  by the corpus A/B include **`ed25519-dalek 2.2.0`'s public `SigningKey::generate`, absent from the
  report entirely and now `["Rand"]`**, `curve25519-dalek`'s inherent `Scalar::random`, `ring`'s
  `Elem::into_unencoded` (`any(test, not(target_arch = "x86_64"))` — production on this machine),
  `bstr`'s `first_non_ascii_byte_fallback` and `tower-http`'s `is_reserved_dos_name`
  (`any(windows, test)`).
  The rule is now "can this `#[cfg]` hold with `test` FALSE": one Kleene fold (`cfg_fold`) shared with
  `cfg_eval`, differing only in the leaf, because two hand-written copies of one question are what
  drifted. Non-`test` predicates stay UNKNOWN and are therefore SCANNED — `any(test, miri)` and
  `any(test, doc)` are kept deliberately, since guessing them test-only is the silent direction.
  A second, order-dependence half: an unconsumed `= "…"` tail aborted syn's sibling iteration, so
  `all(feature = "std", test)` was read as production while `all(test, feature = "std")` was not.
  `--include-tests` is unaffected (byte-identical over the 20 crates the default mode moves).

  **A/B, all 1489 crates in the local registry cache, HEAD vs HEAD+fix, keyed on the FULL row**
  (`inferred`, `direct`, `unresolved`, `unknownWhy`, `invisible`, `incomplete`, `netClass`, `declared`,
  `calls`, `ambiguous`, `entryPoint`, `contributes`, plus `analyzed`/`excluded`/`resolves`/`coverage`):
  **ADDED 42  REMOVED 29  CHANGED 29** with `loc` in the key; **ADDED-names 40, REMOVED-names 27**
  keyed `(package, fn)`, in 30 crates. `inferred` moved on 1 row (the narrow key sees almost none of
  this). §E1 REACH, measured with a counter in the changed branch, not inferred from the diff:
  **864 divergent `is_cfg_test` verdicts across 77 of the 1489 crates** — this is a recall
  measurement, not a safety-only zero-diff. The instrumented binary's reports are byte-identical to
  the committed build's.

  **All 27 removed names audited in full from SOURCE (not from candor's own report): every one is
  inside a `#[cfg(all(feature = "…", test))]` module** — `combine`'s `std_tests`, `redis`'s
  `entra_id_mock_tests`, `proptest`'s `timeout_tests`, `utf8parse`'s `benches`, `x509-cert`'s `tests`.
  One mechanism, the order-dependence half, and every removal is test code that should never have been
  in a production report. **No effect is lost anywhere in the A/B**: the only values dropped on a
  surviving row are 6 `calls` edges in `bstr`'s `inv_memchr`/`inv_memrchr`, and each is replaced by
  `direct: ["Unknown"]` + `unknownWhy: ["ambiguous:same-name local defs"]` — a resolved edge becoming a
  DISCLOSED ambiguity, fail-closed. `curve25519-dalek`'s `Scalar::random` keeps `["Rand"]`; it only
  changes `loc`, and a previous LOC-keyed reading of that row as a loss was a false alarm.
  Over-charge control: no added row carries a fabricated concrete effect — the additions are `Rand`
  (real: both fns take an RNG and call `fill_bytes`), `Unknown`, `invisible`, or empty. The honest cost
  is items under a cfg flag nobody can resolve — `aws-lc-rs`'s `generate_for_test`
  (`any(test, dev_tests_only)`) and `sharded-slab`'s `Track` (`all(loom, any(test, feature = "loom"))`)
  are now scanned; that is the stated direction of the trade, not an oversight.

  **Prevalence, over the same 1489 crates, definition stated:** an `#[cfg]` is affected when its
  predicate contains an `any(...)` with `test` as a DIRECT member alongside at least one sibling.
  **140 crates (9.4%), 719 attribute sites**; of those, **138 crates / 709 sites** have at least one
  sibling that is not harness-only (`miri`/`doc`/`doctest`/`kani`/`fuzzing`), where "not harness-only"
  means a dependent CAN turn it on, not that it is on by default. Only 2 crates are harness-only.
  Commonest shapes: `any(test, feature="alloc")` 127, `any(test, feature="std")` 126,
  `any(test, feature="derive")` 87, `any(test, miri, not(target_arch="x86_64"))` 24.

- **⚠ SOUNDNESS R101 (cardinal sin): a callback installed through a static CELL is no longer silently
  pure.** `static CB: OnceLock<Box<dyn Fn()>>` + `pub fn install(f) { CB.set(f) }` + `fn fire() { if let
  Some(f) = CB.get() { f() } }` left `fire` ABSENT from `functions[]` entirely while the program
  demonstrably wrote a file — silent on `deny Fs`, `deny Unknown`, `deny Fs Unknown` and scoped
  `deny Fs fire`. The sibling path answering the same question, a fn-typed PARAMETER, was already
  correct; `fire` now converges on its exact answer, `["Unknown"]` with `unknownWhy:
  ["callback:unresolved call"]`. A `static`'s declared type was recorded in no index at all, so nothing
  typed the unwrapped binding; the deferred-init cells (`OnceLock`/`OnceCell`/`LazyLock`/`LazyCell`/
  `Lazy`) were also missing from the element-dispatch peel that already covered `Mutex`/`RefCell`/`Cell`,
  `get`/`get_mut`/`get_or_init` were missing from the element-preserving accessor list, and an
  `unsafe { .. }` scrutinee — which reading a `static mut` REQUIRES — was not peeled. Covered spellings:
  if-let, let-else, match, while-let, module-qualified, `Mutex<Option<Box<dyn Fn>>>`, and `static mut`
  through `unsafe`. The index yields only a synthetic `Fn` leaf, so it can hedge a binding to `Unknown`
  and can never contribute or withdraw a concrete effect. Measured: byte-identical over the 256-crate
  a–c registry slice; over all 1489 registry crates it ADDS 2 rows and removes none — proptest 1.9.0 and
  1.11.0's `scoped_hook_dispatcher`, which invokes an externally-installed panic hook through
  `static mut DEFAULT_HOOK` and was reported pure.

- **⚠ SOUNDNESS R105: a `#[cfg]`-duplicated alias is no longer resolved by SOURCE ORDER.** Two `#[cfg]`
  arms declaring the same qualified name — the ordinary platform/feature shim — used to leave whichever
  arm was written LAST in the alias map. Measured on two crates identical but for arm order: one
  reported `["Fs"]` and failed `deny Fs` at exit 1, the other reported `["Env"]` and PASSED at exit 0,
  with the real `Fs` present nowhere in the document. Every arm is now kept and adjudicated at the CALL
  SITE by the classifier, with the leaf in hand: arms that classify alike charge that one effect (with
  the literal surface withheld and the effect marked `incomplete`, since the arms' literals are
  different claims); arms that classify differently disclose `Unknown` with an `ambiguous:` reason. The
  cross-FILE half is fixed too — `#[cfg]`-conditional `#[path]` puts two files at one module path, which
  the merge previously called impossible. Measured cost on 256 crates: 89 collision call-sites in 5
  crates, **zero** became `Unknown` and zero gained an effect.
- **⚠ SOUNDNESS R119 (cardinal sin, introduced by R106 and never released): a body-local item's shadow is
  now scoped to the BLOCK that declares it, and does not reach the SIGNATURE.** R106's collector walked
  the whole function body, so an item declared in a nested block rebound that name for the entire
  function. On a fixture whose two arms differ only in the nested item's NAME — executed ground truth,
  both spawn `/usr/bin/true` — the `struct Cmd` arm went ABSENT from `functions[]` while the `struct
  Helper` control reported `["Exec"]`. On an isolated single-function crate that silence passed `deny
  Exec`, `deny Exec Unknown`, scoped `deny Exec spawn_it` and `pure spawn_it`, all at exit 0. FIFTEEN
  block-introducing constructs reached it, each measured against a pre-fix binary: a plain `{ }`, `if`,
  `else`, a `match` arm, `if let`, `while let`, `loop`, `for`, `while`, a closure body, an `async` block,
  an `unsafe` block, a `const` initializer, a `static` initializer and a labelled `'a: { }`. A SIXTEENTH
  position is not a nested block at all: a parameter's type resolves where the function is DECLARED, so
  `fn f(c: &mut Cmd) { struct Cmd { .. } .. c.status() .. }` lost its receiver typing and vanished too —
  the signature now keeps the outer map. A nested name is promoted to the function-wide shadow only when
  every occurrence of it in the body already lies inside the one block that declares it; otherwise the
  shadow is dropped, restoring the pre-R106 answer for that name — a possible over-report inside that
  block, never a lost effect. Measured over all 1489 registry crates, 252,396 common rows, 15 fields:
  ADDED 0, REMOVED 0, CHANGED 1 (`time` 0.3.55 regains an `invisible: ["serde_core"]` disclosure that
  `origin/main` also reports). Hits on the changed branch: 89 dropped shadows in 46 crates, 99 names
  newly reached in 44, 11 signature-position names in 6.
- **SOUNDNESS R106: a body-local item now shadows a file-level binding of the same name.** `pub type Cmd
  = std::process::Command;` beside a function body declaring its own `struct Cmd` charged that body
  `Exec` with `cmds: ["true"]` — executed ground truth is one file write and no process — and `deny Exec`
  exited 1. The same hole exists for the plain `use std::process::Command;` spelling and PREDATES the
  alias work; both are closed, at the body, where the shadow belongs.
- **SOUNDNESS R107: three reads inside `visit_local` escaped the R100 self-shadow window.** One was a
  silent under-report: `let (d, n) = (d, n); d.go()` was ABSENT from `functions[]` while the identical
  `let (e, m) = (d, n); e.go()` reported its effect. Two were fabrications on the closure-rebind shape,
  both pre-existing. The second mechanism answering the same ordering question (`shadowed_alias`) is
  gone. The window's "a table added tomorrow is caught" claim was disproved mechanically and is now
  worded as the regression pin it is.

- **⚠ SOUNDNESS R68(1): cross-crate drop-glue now uses the same construction authority as the
  in-crate case (candor-spec ⟨0.34⟩'s R66/R69 fixes), for a CALL, a STRUCT LITERAL and a bare VALUE
  PATH.** Before this, a dependency's effectful `Drop` reached the caller only through a bare
  2-segment value path (`deplib::UnitGuard`) — and only by accident, via code shared with the
  lazy-static forcing route. `deplib::Guard::new(1)` (an assoc-fn call) and `deplib::Guard { n: 1 }`
  (a struct literal) read silent-pure regardless of what the dependency's own chained report said.
  Additive only: a function that already charged the effect still does; a function that constructs
  a cross-crate guard and RETURNS it stays pure, exactly as the in-crate case does. A `CANDOR_DEPS`-
  chained scan may now surface a `Fs`/`Unknown`/etc. that a previous run did not — re-baseline and
  diff, per the note above.

## [0.34.0] — 2026-08-31

- **UPGRADING FROM 0.33.1 — re-baselining is not review.** ⟨0.34⟩ is NON-ADDITIVE and this wave
  corrects the classifier in BOTH directions. After regenerating a baseline, **diff it against the
  old one**: effects this release REMOVES will never trip any gate, because `gains` and the baseline
  guard alarm only on effects appearing. A scoped `deny` that went quiet needs eyes, not a re-run.
  Full note, with the measured per-engine numbers and the loud-vs-quiet split, is in the
  [umbrella changelog](https://github.com/tombaldwin/candor/blob/main/CHANGELOG.md).

- **⟨0.33⟩ era markers on permanent prior-floor literals.** `release-preflight [2]` hunts the prior
  floor to catch a bump-miss, and a rung's own ladder comparisons (`spec_predates(spec, "0.33")`)
  reference 0.33 FOREVER by design — so [2] could never go green by fixing anything. Those sites now
  carry a `⟨0.33⟩` marker saying the literal is the RUNG this code names, not a version that bumps.
  Comment-only; no behaviour change.


- **⚠ Fixed: effectful-`Drop` glue fired on the BINDER, so 16 of 17 executed positions were silent —
  and the TUPLE-STRUCT (newtype) guard, the commonest shape in real Rust, had no route in ANY position.**
  Two rules answered one question and had drifted: an assoc-fn `T::assoc()` CALL walk in `scan.rs`
  (construction-keyed, so sound, but blind to `Guard(f)` — a single-segment `Expr::Call` with no `::` to
  test, whose imported spelling `m::Guard` presents the MODULE as the type) and a `T::<construct>` marker
  emitted only under `Pat::Ident`. So `let _ = Guard{..}`, `_ = …`, a bare statement, a call argument, an
  array/tuple element, `Some(..)`, `&temp`, a `match` scrutinee, `if let`, a tuple destructuring, a method
  receiver, `v.push(..)`, a ternary arm, `.unwrap()` and `.into_iter().next()` all read silent-pure.
  GROUND TRUTH EXECUTED, not inferred: 166 units compiled and run with the destructor appending to a log
  interleaved against per-function call/return markers — 76 of the 99 that genuinely release a guard were
  silent before, 1 after (a generic `T::mk()`, which no syntactic scan can key). Under a scoped
  `deny Fs`, that is caller attribution: the `Type::drop` units are in the report either way, so it
  defeats scoped policies, `path`, `gains` and `fix-gate` rather than a blanket deny. The rule is now
  stated once, at the construction expression, and the binder site is REMOVED rather than left beside it.
  Also closed: PARAMETER-OWNED release (`fn take(g: Guard) {}` runs `Guard::drop` inside `take`, and the
  scan never saw the value built), which construction-keying cannot reach by definition.

- **⚠ THE ESCAPE GATE IS THE LOAD-BEARING HALF, and it now covers the DIRECT route too.** Widening the
  construction route without one multiplies candor-spec SOUNDNESS R49's revert (14 false `Unknown`s on
  flate2, from constructors that CONSTRUCT AND RETURN the owner) over every constructor of every guard
  type. A construction escapes via `return`/the body's tail, an assignment into a field/index/deref, a
  binding whose name is returned, an argument to a method on an escaping receiver, a closure's own
  return, or `mem::forget`/`ManuallyDrop::new`. Keyed by type LEAF, which is strictly more precise than
  the `returns_escapable` signature test it replaces on the direct route — that one skipped EVERY type as
  soon as the fn returned an aggregate. A/B over 256 real crates with an `impl Drop` (31,121 effectful
  functions): **+132 rows, −295**, and the removals are the point. flate2's own thirteen constructor rows
  are refused and its sixteen genuinely-releasing `finish`/`into_inner` rows are gained. 163 more removals
  are other returns-the-drop-type escapes and 109 are `&self` accessors that were being charged off a
  SYNTHETIC `Type::method` edge — an `==` operator overload, a `{}` format hole, a `?`-`From` conversion —
  which the old `tail2`-over-every-call rule could not tell from a construction. `format!("{}", guard)`
  does not drop the guard.

- **Three fabrications the corpus A/B caught and nothing else did** (the suite was green for all three):
  a bare value path resolving to a GLOB-imported std constant (tokio's `use Ordering::*` beside its own
  `struct Acquire` with a tracing `Drop` — every `is_closed`/`is_idle` in `batch_semaphore` inherited the
  future's `Log`); `self: Pin<&mut Self>`, which syn parses as a `Receiver` whose `reference` is `None`,
  so the obvious by-value test read every `poll_read`/`poll_flush` in the ecosystem as consuming its
  receiver; and a `match` ARM PATTERN, because syn 2 represents `Pat::Path` with the very same `ExprPath`
  node an expression uses (isahc's `AsyncBody::len(&self)`, one `match` over three arms, charged the
  agent `Handle`'s `Drop`). Each has a fixture; each fixture was falsified by degrading its own guard.
  `CANDOR_CTOR_DEBUG=1` prints the marker stream — all three were found by reading it, and by reasoning
  about the report diff not at all.

- **⚠ Fixed: candor ICE'd the build on `HashSet::insert` of a local type.** `local_trait_method_by_did`
  built `Instance::try_resolve`'s generic args with `mk_args(&[self_ty])` — one argument, for methods
  that declare more. `Hash::hash<H: Hasher>` carries its own `H`, so the set-`insert` driver edge was
  exactly one argument short, and a short list is not a soft `None`: rustc raises a `span_delayed_bug`
  ("missing value for assoc item in impl") that surfaces at the end of the build as an INTERNAL COMPILER
  ERROR. Eight lines reproduce it, `#[derive(Hash)]` included, and the edge it was hiding (a local
  effectful `Hash`/`Ord` impl reached only through a std container verb) never landed. All seven
  `mk_args` call sites now go through a shared `trait_args_for`, which asks `GenericArgs::for_item` for
  a WELL-FORMED list and REFUSES (returns `None`) rather than invent a const-parameter filler. New
  fixture `ui/std_driver_hash.rs`. The comment above `local_trait_method_for_self` documents a SIBLING
  of this ICE and reads as though the class were handled — a LOCAL element type sails straight through
  the gate it describes, which is why this one was never measured.

  **The first attempt at this fix padded uniformly and caused TEN silent under-reports** (reverted as
  `d073699`, landed here corrected). Padding is not uniform: the TRAIT's parameters CHOOSE THE IMPL,
  the method's own ride along without selecting anything. `PartialEq<Rhs = Self>` arrives with only
  `Self` known, was padded to `<E as PartialEq<()>>::eq`, resolved to nothing, and every driver edge
  through `eq` vanished — `soundness/run.sh 60` went from 60 passed / 0 failed to 50 / 10, every failing
  seed a `vec_contains_eq` reported `pure/omitted` over a real effect. An unknown TRAIT parameter is now
  filled from that parameter's own DECLARED DEFAULT (instantiated against the args already built), and
  refused when it has no default; only the method's own parameters get an inert `()`.
  `ui/std_driver_hash.rs` carries the arm that was missing: a hand-written effectful `PartialEq` under
  both drivers that run `eq` (`Vec::contains` and `HashSet::insert`), with a derived-`PartialEq`
  over-charge control. Every `PartialEq` in that fixture had been DERIVED — pure either way — and a
  comment in it asserted the sibling drivers were "unchanged by the args fix", which is exactly why the
  file could not catch this.
- **⚠ Fixed (silent under-report): a trait method named as a fn VALUE edged to the trait DECLARATION.**
  `<S as T>::run` types as a `FnDef` whose `DefId` is the trait's item, which is local and an `AssocFn`
  — so the old "local + `Fn`/`AssocFn` ⇒ resolvable" test said yes and then edged to a body that does
  not exist. `register(<S as T>::run)` read silently PURE however effectful `S`'s impl was, while the
  identical `<S as T>::run()` CALL one line away resolved correctly. New `fn_value_targets` answers with
  the same authority the call path uses: `Instance::try_resolve` first (which is also what finally
  reaches a LOCAL impl of a NON-LOCAL trait — the old code tested `is_local` on the trait item), then
  CHA over the trait's local impls when `Self` is unpinned, PLUS the trait's own default body when some
  impl takes it (`impl_item_implementor_ids` lists only what an impl DEFINES, so CHA alone silently
  dropped the default's effects — measured, `Fs` reported and `Exec` lost). New fixture
  `ui/fn_value_trait_method.rs`, with both over-charge controls: a pure sibling impl stays pure, and a
  default that no impl can reach is not charged.
- **⚠ Fixed (silent under-report): the `Drop` walker's recursion guard was keyed on the ADT's `DefId`.**
  That made the walk ORDER-DEPENDENT — the first instantiation of a generic ADT claimed the key, so
  every later one returned immediately. Fields are walked in DECLARATION ORDER, so
  `struct S { a: Cellish<u8>, b: Cellish<Guard> }` lost `Guard::drop` while the same struct with its two
  fields SWAPPED was caught, and `Mutex<Guard>` / `RwLock<Guard>` / `RefCell<Guard>` all read
  silent-pure (each reaches its payload through `UnsafeCell<T>`, behind a *different* `UnsafeCell`). The
  memo is now keyed on the whole TYPE, and the recursion bound is a DEPTH rather than a set of ancestor
  `DefId`s: a nested same-ADT type (`Box<Box<Guard>>`, `Option<Option<Guard>>`,
  `Cellish<Cellish<Guard>>`) is its own "ancestor" while being an ordinary finite type, and an ancestor
  set cut all three — the same class, one construct over. `PhantomData` joins the owning-container
  list; it is not another curated name but the language's own declaration of the property that list
  hand-enumerates, and it is what recovers `std::vec::IntoIter<Guard>` and hand-written arenas. New
  fixture `ui/drop_container_reuse.rs` covers all eleven curated container names (nine had never been
  driven by any fixture), the three nested shapes, and four over-charge controls (`PhantomData<&'a T>`,
  `PhantomData<fn() -> T>`, `ManuallyDrop`, and a container over a payload with no destructor).
  MEASURED across eleven corpora (1626 units — 9 real third-party crates plus 2 of candor's own): zero
  fabrications; five tempfile units correctly GAINED `Fs` through the pre-existing trait-object arm the
  `DefId` memo had been cutting short; two clap_builder units correctly LOST a bogus `Unknown`.
- **Fixed: the nightly-bump workflow's ui re-bless step matched nothing.** It grepped for
  `/<base>.stderr`, but compiletest_rs 0.11.2 saves to `<base>.stage-id.stderr` — so the step blessed
  zero files on every run, and the verification `cargo test` below it then failed the job for exactly
  the diagnostic shift it exists to absorb. Fail-closed, but never once useful.
- **Coverage-only, no behavior change: a THIRD guard-deletion sweep, scoped to the one area the first
  two passes explicitly named as not reached — `src/lib.rs`'s ~2000-line callback/thread-local/
  coroutine-capture machinery (rust-deep), which needs the dylint `ui/` harness (`cargo test --lib`)
  rather than plain `cargo test`.** Found TWO more guards on the silent-vs-disclosed boundary with ZERO
  fixture coverage — the exact area that produced `e43eec0`'s and `3e9848c`'s cardinal sins. Every new
  test is confirmed RED with its guard deleted and GREEN at HEAD.
  - **`mir_spike::local_drop_impls`'s `TyKind::Coroutine` and `TyKind::CoroutineClosure` arms** — added
    in `3e9848c` alongside `TyKind::Closure` (a `move || {}` closure capturing an effectful-Drop value,
    dropped without ever being called), but only the `Closure` arm ever got a fixture
    (`tests/integration.sh` 9c-iii, driving `sink()` via a plain closure). The Coroutine form (`async
    move { … }`, a Future dropped without being polled) and CoroutineClosure form (`async move || { …
    }`, dropped without being called) were never independently exercised — a textbook instance of a test
    inheriting the blind spot of the bug report that prompted it (`bin/AGENT-CORPUS-BRIEF.md` A.2:
    "every fixture drove only the crate/shape the original bug happened to involve"). Deleting either
    arm alone (leaving the other two intact) makes exactly its own function vanish from the report —
    independently confirmed for both. New `ui-2021/coroutine_drop.rs` (its own `--edition=2021` UI
    battery, `ui_edition_2021` in `src/lib.rs` — the default `ui_test` compiles at rustc's 2015 default,
    where `async {}`/`async || {}` are a hard parse error) pins both `coroutine_scope_exit` and
    `coroutine_closure_scope_exit`.
  - **`edge_fn_value_reference`'s cast-STAYS-callable branch** (`f as fn()`) — the function's own comment
    claims "a cast that STAYS callable (`f as fn()`) … DOES keep the effect," but no fixture anywhere
    used an explicit `as fn()` cast: the one existing "keeps the edge" case (`callbacks.rs`'s
    `passes_cb`) passes the fn by an implicit coercion at an argument position, which never even visits
    the `ExprKind::Cast` arm of the `cast_away` match. Removing `TyKind::FnPtr` from that arm's "still
    callable" set silently drops the edge for an explicit `f as fn()` cast (the whole existing suite,
    including every other case in `callbacks.rs`, stays green). New `keeps_via_fn_ptr_cast` in
    `ui/callbacks.rs` pins it.
  - **⚠ CARDINAL SIN found, FILED NOT FIXED (needs new machinery, see BACKLOG.md):** a closure/coroutine
    capturing an effectful Drop by move, coerced into `Box<dyn Fn*>` and dropped without ever being
    called, is silently pure — the identical class to the two above, one hop further through a trait
    object. `local_drop_impls`'s `TyKind::Dynamic` arm CHAs the principal trait's registered `impl`
    blocks to find the concrete type behind a `Box<dyn Trait>`, but a closure satisfies `Fn`/`FnMut`/
    `FnOnce` through compiler-synthesized dispatch, never a registered `impl Fn for X` — confirmed with a
    debug probe that `trait_impls_of(Fn)` never contains the closure's own type, so no amount of
    widening the CHA trait list closes this. A sound fix needs to track the concrete type at its
    unsizing-coercion site (construction), not at the (type-erased) `Drop` terminator; two candidate
    designs and why a naive one is unsound are in BACKLOG.md. Not reached further this session.
  - Not reached (bounded scope, reported per `bin/AGENT-CORPUS-BRIEF.md` rule 7): `edge_static_force`/
    `edge_thread_local_force`/`local_key_init_fns`/`FnRefCollector` (spot-attacked — the `NestedFilter::
    All` visitor's descent into nested bodies IS load-bearing and already caught by the existing
    `ui/thread_local_effects.rs` fixture when neutered; a hypothesized "bare inline effect with no named
    helper fn" gap was attacked and found NOT to reproduce — the `thread_local!` macro always synthesizes
    a genuine named `fn` for the deferred init, so this path was never actually exposed); `resolve_callback_sites`/
    `record_callback_flow`'s `DefKind::AssocFn` handling (a method-as-callback-value shape, untested, but
    judged lower priority — its failure mode is a precision loss to `Unknown`, not a silent vanish);
    `is_std_owning_container`'s non-Box/Vec members (`Rc`/`Arc`/`BTreeMap`/`BTreeSet`/`HashMap`/
    `HashSet`/`LinkedList`/`BinaryHeap`) and the general `TyKind::Adt`/`Tuple`/`Array`/`Slice` recursion
    in `local_drop_impls` — outside the callback/thread-local/coroutine-capture scope this pass was given.

- **Coverage-only, no behavior change: a SECOND guard-deletion sweep (`bin/AGENT-CORPUS-BRIEF.md`
  attack C), scoped to what the first pass named as unreached — `candor-scan/src/deps.rs`'s
  `CALIBRATED_*` impostor-exemption guards first (highest churn: five cardinal sins fixed there the
  same day), then `candor-query`'s `completeness.rs`/`containment.rs` — found FIVE more guards on the
  silent-vs-disclosed boundary with ZERO test coverage. Each confirmed by deleting the guard in a
  throwaway worktree and watching the relevant `cargo test` stay fully green; every new test below is
  confirmed RED with its guard deleted and GREEN at HEAD.**
  - **`candor-scan/src/scan.rs`'s coverage-ledger impostor carve-out on THREE of its four calibrated-
    style exemption lists** — `PATH_CALIBRATED_CRATES`, `CALIBRATED_PREFIXES`, `REVIEWED_PURE_CRATES`
    each carry their own `&& !impostor` conjunct, structurally identical to the already-tested
    `CALIBRATED_CRATES` arm (`caca530` and its four follow-on fixes), but every existing impostor
    fixture drives `CALIBRATED_CRATES` alone (via `log`). Deleting all three `!impostor` conjuncts left
    `cargo test --workspace` fully green. New
    `path_calibrated_prefix_and_reviewed_pure_impostors_lose_the_ledger_exemption` drives a `path`-
    dependency impostor through `tokio` (PATH_CALIBRATED_CRATES), `aws_sdk_evilthing`
    (CALIBRATED_PREFIXES) and `toml` (REVIEWED_PURE_CRATES), each with its own honest-bare-version
    over-charge control.
  - **`candor-scan/src/deps.rs`'s `verified_workspace_root`'s SELF-ROOT branch** (`canon_root ==
    canon_dir`) — a non-virtual workspace root resolving its OWN `{ workspace = true }` dependency
    against its OWN `[workspace.dependencies]` table, entitled by BEING the root rather than by being
    one of its own listed members. Every existing fixture for this function drives only the MEMBER arm.
    Deleting the early-return (falling through to the members-only check, which a root fails since
    `workspace_members` lists its members, never itself) left the suite green — and, in the opposite
    direction from a silent under-report, would make a non-virtual root's own honest dependency lose an
    exemption it is entitled to. New
    `a_non_virtual_workspace_root_resolves_workspace_true_against_its_own_table` pins it.
  - **`candor-query/src/completeness.rs`'s `incomplete()` `out_of_scope` arm** — the module's own doc
    comment names this cause as the FIRST one the ⟨0.30⟩/⟨0.32⟩ rung closed on `gate --report` and left
    its advisory siblings behind (`unread_armed` was the SECOND, and already has a regression test).
    No fixture anywhere in the suite writes a non-empty `outOfScope` finding — every one uses `[]` or
    omits the key — so deleting the arm left `cargo test -p candor-query -p candor-scan` fully green.
    Without it, `unverified --strict`/`fix-gate --strict` over a report whose peek found a real denied
    effect outside the scan's scope would answer `{"ok": true}` at exit 0 — the exact historical defect
    this module's header measures. New `advisory_verbs_refuse_over_a_peeked_out_of_scope_finding` pins
    both verbs plus the empty-`outOfScope` over-charge control.
  - **`candor-query/src/containment.rs`'s ratchet-mode `comp.absorb(baseline completeness)` call** — the
    fold that lets an incomplete BASELINE hedge `containment`'s `{"leaks":[],"cleanups":[]}` answer,
    exactly as `diff`/`gains` already fold in their baseline side. Neither existing containment test
    uses anything but two ordinary, complete reports, so deleting the one `absorb(...)` line left the
    crate's tests fully green. New `containment_ratchet_hedges_when_the_baseline_report_is_incomplete`
    (judged-nothing baseline, JSON + human channel) pins it.
  - **Judged genuinely redundant, no test added**: `candor-scan/src/deps.rs`'s `dep_report_files`
    filename filter excluding `*callgraph*.json` from a `--deps` directory walk. Deleting it leaves
    `cargo test --workspace` green on both consumers — `load_dep_reports` already falls through on the
    callgraph sidecar's shape (no `functions` key, not a bare array) via its existing structural check,
    and the §3.3.1 sink-registration use only WIDENS what gets protected by including it, never narrows
    it. A fixture that passed either way would be worse than none.
  - Not reached (bounded scope, reported per `bin/AGENT-CORPUS-BRIEF.md` rule 7): `candor-query/src/
    diff.rs` (read in full — `cmd_gains`/`load_fninfo_loud`/`gain_origin`/`report_build_version` are
    directly unit-tested and `attach_manifest`'s hedge causes are exercised by `tests/cli.rs`; not
    guard-deletion-tested line by line); `src/lib.rs`'s ~2000-line callback/thread-local/coroutine-
    capture machinery (rust-deep; spot-read only — its regression suite is dylint `ui/*.rs` fixtures,
    not `cargo test`, and a proper sweep needs its own budget); `candor-classify::classify()`'s per-
    crate rule table (judged, not attacked: it is a sequence of data-carrying rules rather than
    early-return guards over a shared state machine, and the file's own convention pairs nearly every
    rule with a dedicated "found live" regression test plus a `calibrated_crates_are_live` meta-test —
    a full guard-deletion pass over ~80 calibrated crates' rule arms was judged disproportionate to this
    sweep's budget, not verified clean).

- **Coverage-only, no behavior change: a guard-deletion sweep across `candor-report`/`candor-scan`/
  rust-deep found four guards on the silent-vs-disclosed boundary with ZERO test coverage — each
  confirmed by actually deleting the guard and watching `cargo test --workspace` (plus, for the
  rust-deep case, the `ui` fixture suite) stay fully green.** Method: `bin/AGENT-CORPUS-BRIEF.md`
  attack C. Every new test below is confirmed RED with its guard deleted and GREEN at HEAD.
  - **rust-deep, `is_dyn_receiver`'s OPAQUE-ALIAS arm** (the fix for the historical `which`-crate bug —
    `all_results().and_then(|mut i| i.next())` reading silent-pure through a local `-> impl Iterator`
    hiding a `Box<dyn Iterator>`) shipped with no fixture pinning it. New `ui/opaque_dyn_iterator.rs`
    reproduces the exact failure mode with the arm deleted: the callee (`use_it`) still self-reports
    `Unknown`, but that `Unknown` stops PROPAGATING to its caller, which drops out of the report
    entirely — reads as fully pure two hops from unresolved dispatch. (The sibling `Box`/`Rc`/`Arc`/
    `Pin` unwrap arm was also guard-deletion-tested and found genuinely redundant — `devirtualize`'s
    independent instance-resolution catches every shape tried once the structural check is disabled —
    so it is left as documented defense-in-depth, not given a fixture that couldn't discriminate it.)
  - **`candor-report::write_atomic`'s multiply-linked-target guard** (writes in place rather than
    `rename(2)`, so an operator with two hard-linked names for one verdict file doesn't get a fresh
    document at one name and a stale one at the other) had never been exercised: no test anywhere
    created a hard link. New `write_atomic_updates_a_multiply_linked_target_in_place` (plus the
    single-link control) pins both halves.
  - **`candor-report::resolve_sink_artifact`'s symlink-following loop** — used by `write_atomic` and by
    candor-scan's `same_artifact` §3.3.1 sink-collision guard for the one shape `canonicalize` can't
    resolve on its own (a symlink whose target doesn't exist yet) — had no coverage at all. New
    `resolve_sink_artifact_follows_an_ordinary_symlink` and
    `..._resolves_a_dangling_symlink_to_its_named_target` pin it directly; new
    `same_artifact_catches_a_policy_and_gate_json_collision_through_a_dangling_symlink` (candor-scan)
    pins the real guard this backs — the historical `--policy P --gate-json <other spelling of P>`
    class the guard's own doc comment names.
  - Not reached (bounded scope; reported per `bin/AGENT-CORPUS-BRIEF.md` rule 7): the `candor-classify`
    effect-classification allowlist/denylist functions (`is_net_establishing` et al. — spot-checked,
    already have direct unit tests), the AS-EFF-005 baseline-guard branches in `candor-scan/src/gate.rs`
    (heavily exercised — 97 references in `tests.rs`), and `candor-query`'s `unanswerable_pairs` fail-
    closed branches (spot-checked against `tests/cli.rs` fixtures carrying an absent `netClass`/
    `reasonClass` field — both already discriminating).

- **Coverage-only, no behavior change (rust-scan): `is_callable_type`'s `Rc<fn()>`/`Arc<fn()>` wrapper
  peeling — verified sound, then given the fixture it was missing.** The `defe53d` widening's own doc
  comment and match arm name `Box`/`Rc`/`Arc`/`Symbol<T>` symmetrically, but
  `runtime_resolved_pointer_invocation_is_unresolved` only asserted `Box<fn()>` end to end — `Rc`/`Arc`
  were sound by code inspection only, never independently measured. Verified directly (a real scan of
  `fn run(b: std::rc::Rc<fn()>) { b(); }` / the `Arc` twin reads `Unknown`/`callback:unresolved call`,
  and a never-called binding of either stays pure) before adding both shapes to the existing test's
  positive and over-charge arrays, closing the untested half of an already-correct claim.

- **⚠ CARDINAL SIN, closed (rust-deep): a `path`/`git` dependency named `core`/`alloc`/`std`/
  `proc_macro`/`test` — the sysroot's own five names, not a rename — read silent-pure, both for a
  direct call into it and for `dyn` dispatch through a trait it defines itself under one of
  `is_pure_std_trait`'s 11 exempted names.** `record_resolved_call`'s coverage/`invisible`-floor skip
  and `is_pure_std_trait`'s trait-purity exemption both trusted `cx.tcx.crate_name(krate)`'s STRING
  alone. crates.io blocks publishing under these five names, but a `path`/`git` dependency is not a
  registry crate — Cargo compiles and `--extern`s one under whatever `[package]` name its own manifest
  declares, and two different `CrateNum`s can carry the identical name string. Unlike a manifest
  `package = "…"` RENAME (below) — where the source spells an alias and the real dependency still gets
  its own honest, differently-named identity — this attacker doesn't rename anything; their own crate
  answers to the trusted name directly. Live-reproduced: a `path` dependency named `core` performing a
  real `std::fs::write`, called directly and through its own `dyn Display`-shaped trait, produced
  `"functions": []` (total silence, not even `invisible`) and `deny Fs Unknown` exited 0 with zero
  warnings. The exact class R59/R60/`caca530`/`fda08ad`/`75045f0` already closed for rust-scan's
  `CALIBRATED_CRATES` exemption ("a name is not an identity") — unfixed here because rust-deep's
  crate-name gates were assumed immune BY CONSTRUCTION (rustc resolves real identity, unlike rust-
  scan's syntactic string match), which is true for a renamed dependency and false for this one: the
  string can still lie when TWO real `CrateNum`s share it.
  Fixed by asking rustc's own authority rather than re-deriving one: a new `is_real_sysroot_frontier`
  (`src/lib.rs`) uses `used_crate_source` to check where a crate actually loaded from — the real sysroot
  `std`/`core`/`alloc`/`proc_macro`/`test` always resolve under the active toolchain's OWN sysroot
  directory; a `path`/`git` dependency never does. Gated into all three exemption sites (the trait-
  dispatch purity check, and the coverage/invisible-floor skip's two call sites). An impostor with no
  located source at all (declared but never actually needed) can't be told apart from the genuine
  article either way; that residual fails toward TRUST, unchanged from today and the same posture
  `non_registry_lock_names`'s absent-lockfile arm already takes — never assumed from silence. This also
  costs the exemption for a genuine `-Z build-std` project (std/core/alloc rebuilt from source under the
  project's own `target/`, a real but rare nightly-only configuration) — the SAFE direction, extra
  disclosure on real code, never a new false purity claim.
  CONTROLS, falsified against the pre-fix binary (`tests/integration.sh` §9c-iv): the impostor's direct
  call now discloses `invisible: ["core"]` instead of vanishing; its own-defined `Display` trait's `dyn`
  dispatch now reads `Unknown` instead of the pure-std-trait exemption; both are silent/exempt on the
  pre-fix binary. OVER-CHARGE CONTROLS: `ui/trust.rs`'s `format_error` (a REAL `&dyn std::error::Error`)
  stays exempt, unchanged — the genuine sysroot frontier is untouched; `soundness/run_drop.sh` (60/60)
  and the full `tests/integration.sh` (156/156, up from 154 — two new cases) are unaffected; this repo's
  own dogfood fixtures (`sample/`, `sample-capstd/`, the latter with cap-std's real dependency tree) are
  byte-identical before and after under `cargo dylint`.

- **⚠ SOUNDNESS finding, closed (rust-scan) — NOT a cardinal sin (never silent; see below), but a real
  `deny`-gate coverage loss: a manifest `package = "…"` RENAMED dependency's calls never reached
  `classify()` under the crate's real identity.** `classify()`, `scan_builder_entry_effect()` and
  `is_model_sdk_crate()` were all called with `cr` — the call's syntactic first path segment, which is
  the manifest ALIAS when
  `[dependencies] alias = { package = "real" }` renames a dependency — never `cr_real` (`dep_renames`'s
  resolved identity, already used by every OTHER crate-identity lookup in the same loop: the CANDOR_DEPS
  cross-crate joins). A renamed `reqwest` (`htclient = { package = "reqwest" }`) calling
  `htclient::blocking::get(url)` matched no rule in `classify()`'s table (which is written against
  `reqwest`, not the caller's chosen alias) and fell to the honest uncalibrated-dependency floor: NOT
  silent — `invisible: ["htclient"]` was correctly disclosed — but a `deny Net` gate does not act on
  `invisible`, so the byte-identical unaliased call gated correctly (`deny Net` exit 1) while the renamed
  one exited 0 ("policy ✓"). rust-deep is unaffected by this class: `cx.tcx.crate_name` resolves the
  compiled crate's TRUE identity independent of any local extern alias (verified via `cargo dylint`
  against the identical renamed-reqwest fixture: `inferred: ["Net"]`, unchanged by the rename).
  Fixed by resolving `cr_real` once per call (hoisted to the top of the loop, replacing four separate
  redundant re-derivations) and using it for `classify()`/`scan_builder_entry_effect()`/
  `is_model_sdk_crate()` — and rewriting the call PATH's own leading segment too, not just the crate-name
  argument: several `classify()` rules match a full EXACT path (`path == "git2::Repository::clone"`),
  which a resolved crate_name paired with an alias-prefixed path would still never satisfy.
  CONTROLS, falsified against the pre-fix binary (`crates/candor-scan/src/tests.rs`,
  `renamed_dependency_calls_classify_under_the_real_package_name`): a renamed `reqwest` now classifies
  `Net` exactly like the unaliased call (byte-identical report, minus the fn name), and a renamed `git2`
  still hits the FULL-PATH-keyed `Repository::clone` rule — both silent/uncalibrated-floored on the
  pre-fix binary. OVER-CHARGE CONTROL: this repo's own four real crates
  (`candor-report`/`candor-classify`/`candor-scan`/`candor-query`, none of which rename a dependency)
  are byte-identical before and after, save for `loc` line-number shifts from this fix's own added
  comments.

- **⚠ CARDINAL SIN, closed (rust-scan): a function pointer resolved at RUNTIME and then INVOKED read
  silent-pure.** `is_callable_type` (`lang.rs`) recognised a `fn()`/`impl Fn*`/`dyn Fn*` annotation
  directly, and `expr_is_fn_typed` (`collector.rs`) propagated fn-typed-ness through a rebind — but
  neither recognised the two shapes real FFI code actually uses to hold a dynamically-resolved symbol:
  **(1)** a `let` typed with a NAMED wrapper the annotation-matcher never peeled — `libloading::Symbol<T>`
  (and its `os::unix`/`os::windows` twins), the ordinary return type of `Library::get`; and **(2)**
  `std::mem::transmute::<Src, Dst>(ptr)` into an UNTYPED `let` — `Dst` is the caller's declared target
  type, but it lives in the CALL's turbofish, not a `Pat::Type` annotation, so nothing read it. Both
  reproduced: `deny Exec Unknown` over either shape exited 0 with `ok:true, violations:[]` and no
  disclosure at all, while the SAME code with a `fn()`-typed local (`let f: fn(i32)->i32 =
  transmute(sym)`) or a fused transmute-and-call (`transmute::<_, fn()>(sym)()`) was already correctly
  `Unknown`-disclosed — the machinery existed, these two shapes were simply invisible to it.
  Fixed by widening both sides of the SAME mechanism, not adding a new one: `is_callable_type` now peels
  a `Box`/`Rc`/`Arc`/`Symbol<T>` wrapper and recurses on `T` (closing, as a side effect, the identical
  and previously unnoticed `Box<fn()>`/`Rc<fn()>` hole — nothing peeled a smart pointer around a BARE fn
  pointer before, only around a `dyn Fn*`); and `expr_is_fn_typed` now reads a `transmute::<.., Dst>(..)`
  call's own turbofish target, and a method call's own turbofish (`lib.get::<T>(..)`, transparently
  through a trailing `.unwrap()`/`.expect()`), checking each the same way. Every new rule is a NAME match
  on the leaf segment (`Symbol`, `transmute`), not a type-resolved one — rust-scan is syntactic by
  design and cannot ask rustc whether some unrelated crate's own `Symbol<T>` is the one in scope, the way
  rust-deep already can. That is a stated, accepted gap, not a silent one: the call syntax `sym(..)` only
  compiles at all if the receiver really is callable, so a same-named non-callable type can't reach this
  path in code that builds; and every new rule only ever turns a call into `Unknown`, never fabricates a
  specific effect, so a false hit costs an extra honest disclosure, never a false purity claim.
  `libloading` is deliberately left OUT of `CALIBRATED_CRATES`/`CALIBRATED_BUT_PARTIAL_CRATES`: the gap
  fixed here is call-RESOLUTION (does `sym(..)` reach the honest `Unknown` at all), not effect
  CLASSIFICATION of `Library::new`/`Library::get` themselves — those already surface correctly today as
  an ordinary uncalibrated-dependency `invisible` disclosure, and calibrating them would answer a
  different question than the one this fix closes.
  CONTROLS, each independently falsified against the pre-fix binary (`runtime_resolved_pointer_invocation_is_unresolved`,
  `crates/candor-scan/src/tests.rs`): both silent shapes (`Symbol<T>`-typed local, untyped
  `transmute`-into-`let`) now disclose `Unknown`; a same-shape `libloading::Library::get::<T>(..)`
  turbofish left untyped by the `let` (the sweep's extra find, same mechanism) now discloses too; the two
  already-correct shapes (bare `fn()`-typed local, fused transmute-and-call) are pinned unchanged; and
  the OVER-CHARGE CONTROL — a pointer OBTAINED but never called must stay quiet, since marking a binding
  fn-typed changes nothing unless call syntax is actually used on it — holds for all three new-rule
  shapes. Measured over this repo's own four real crates (`candor-report`/`candor-classify`/
  `candor-scan`/`candor-query`) under `.candor/policy`'s real `deny Net Db Exec Ipc`: reports are
  byte-identical before and after (none of the four uses `libloading`/`transmute`/`Box<fn()>`, so this is
  a necessary-but-not-sufficient check, not a substitute for the seeded fixtures above).
  **Filed, not mine to close: `candor-spec` has zero conformance pinning for rust-deep's own correct
  `fn()`-typed-callback-pointer disclosure** (`grep "callback:fn-pointer" candor-spec` returns nothing) —
  sound today, unguarded against regression tomorrow.

- **⚠ CARDINAL SIN, closed (rust-deep, two forms): a value whose destructor was reached through a
  CLOSURE capture, or through an explicit `drop(x)` on anything that doesn't implement `Drop` itself,
  was silently reported pure.** rust-deep is the only engine of the family with a MIR-derived model of
  implicit `Drop` at all (the `Bet 4` fix, `mir_spike::drop_edges`, already closed the ORIGINAL hole —
  scope-exit dropping a bare guard, or one behind a struct field / tuple / array / `Box`/`Vec`/`Rc`/`Arc`/
  `HashMap` / `Box<dyn Trait>`). Both new holes are in code that walks the SAME question — "what local
  `Drop::drop` impls does dropping a value of this type run?" — by two different, incompletely-shared
  routes.
  **(1) Closures/coroutines.** `local_drop_impls` (the walker `drop_edges` calls) pattern-matches
  `Ty::kind()` and had arms for `Adt`/`Tuple`/`Array`/`Slice`/`Dynamic`, but a closure or `async` block is
  its own `TyKind` (`Closure`/`Coroutine`/`CoroutineClosure`), not an `Adt` — so it fell into the `_ => {}`
  catch-all. A guard captured BY MOVE into a closure that is stored and dropped WITHOUT ever being called
  (`let g = Guard; let _c = move || { let _ = &g; };` — `_c` drops at scope exit, running `Guard::drop`)
  was invisible: the enclosing fn was reported with no effect at all, not even `Unknown`. Fixed by adding
  arms that recurse into `ClosureArgs`/`CoroutineArgs`/`CoroutineClosureArgs::upvar_tys()` exactly like the
  existing arms recurse into fields/elements.
  **(2) Explicit `drop(x)`.** `core::mem::drop(x)` moves `x` into a non-local std body, so it never
  produces a MIR `Drop` terminator in the CALLER — `mem_drop_local_edge` exists specifically to recover
  that edge, but it only ever resolved `<T as Drop>::drop` directly on `T`, i.e. only when `T` ITSELF
  implements `Drop`. It never walked fields or containers, so `drop(Wrapper { g: Guard })` (`Wrapper` has
  no `impl Drop`, only a Drop-carrying FIELD), `drop(Box::new(Guard))`, and `drop(vec![Guard])` were all
  silently pure — while letting the IDENTICAL value fall out of scope without the explicit `drop(..)` call
  was already caught by fix (1)'s sibling machinery. Two hand-rolled paths answering the same question
  must not disagree; fixed by making `mem_drop_local_edge` call the SAME `local_drop_impls` walker
  `drop_edges` uses (now `pub(crate)`), returning every local `Drop::drop` reachable from the argument's
  type instead of at most one.
  Both are realistic, not adversarial-only: capturing a resource guard in a closure that outlives its
  invocation, and `drop(guard)` / `drop(Box::new(x))` / `drop(vec)` for early release, are ordinary Rust.
  MEASURED against the pre-fix binary: a closure-scope-exit fixture and three explicit-`drop()` fixtures
  (struct-field, `Box`, `Vec`) all went from zero disclosure (the function omitted from the report
  entirely — candor's own convention for "judged pure") to correctly carrying the guard's effect, with a
  genuinely pure sibling fn in the same crate staying clean throughout. Over-charge control: byte-identical
  reports, zero diffs, running rust-deep over this repo's own four real crates (331 function entries
  across `candor-report`/`candor-classify`/`candor-scan`/`candor-query`) before and after. Four new
  `tests/integration.sh` cases pin both defects (`explicit_wrapper`/`explicit_box`/`explicit_vec`/
  `closure_scope_exit`), each independently falsified against the pre-fix binary. `soundness/gen_drop.py`
  gained a `closure` form so the existing drop-soundness fuzzer (`soundness/run_drop.sh`) covers this
  shape going forward — pre-fix, 300 fuzzer seeds gave 135 failures, every one naming a `closure`-form
  function; post-fix, 300/300 pass.

- **⚠ CARDINAL SIN, closed: a peek finding was scope-matched against the WRONG ENTITY, silently
  defeating any policy rule scoped to an in-scope CALLER (BACKLOG "a peek finding is scope-matched
  against the wrong entity"; candor-swift's `7378f4f` closed the analogous case for its own, differently
  shaped peek).** The ⟨0.29⟩ peek re-analyses the excluded file set and reports findings under the
  EXCLUDED declaration's own qualified name — correctly, and unchanged by this fix. But the scope test
  ran ONLY against that name, so `deny Net Runner` (scoped to the in-scope caller `Runner`) could never
  match a finding named `EvilDoer::work`, even when `Runner::dispatch(&dyn Doer)` is exactly the
  in-scope code that dynamically dispatches into it. MEASURED against the pre-fix binary: an in-scope
  `trait Doer`, an in-scope `Runner::dispatch(&dyn Doer)` (its one visible `impl Doer` is pure, so CHA
  resolves it confidently — it never sees the excluded conformer at all), and an excluded
  `tests/evil.rs` `impl Doer for EvilDoer` performing `Net`: `deny Net Runner` → exit 0, `outOfScope:
  []`, `excluded: [{class:"non-library-target", peeked:true}]`; the identical tree under an UNSCOPED
  `deny Net` → exit 2, naming `EvilDoer::work` directly. Held constant: same tree, same binary, same
  effect — only the policy's scope string varied.
  Unlike candor-swift's peek (which unions in-scope files into the child's own CHA and needed a
  CHA-union effect-set diff), **rust's peek never sees in-scope files at all** — it re-analyses the
  excluded set in total isolation, so there is no re-analysis to fix. The mechanism instead reuses two
  facts the PRIMARY scan already computes in the course of its own single pass, cross-referenced only
  after both halves have run: (1) `FnInfo::dispatch` — a new, purely SYNTACTIC record, per in-scope
  function, of every `(trait_leaf, method_leaf)` it dispatches on through a local bounded-CHA-eligible
  receiver (`&dyn T`/`impl T`/a field/a loop element/a stringified bound), recorded regardless of the
  impl-count/ambiguity gates that guard the EFFECT edge itself — reachability doesn't depend on either,
  only the edge does, and this field never contributes an effect; and (2) the peek's OWN
  `type_to_traits` (which trait(s) an excluded declaration's owning type implements, learned purely from
  the peek's separate parse of the excluded file), smuggled to the enclosing frame via a same-thread
  side channel (`gate::PEEK_TYPE_TO_TRAITS`, cleared before every peek invocation) the same way
  `while_peeking`'s `IN_PEEK` already is. For each peeked finding, every in-scope function that could
  reach it — the direct dispatcher(s) plus every transitive ANCESTOR via a `rev_calls` BFS
  (`lang::reaching_ancestors`), so a policy scoped several hops above the dispatch site is treated
  exactly as an ordinary propagated effect already is — is added to the scope test. ATTRIBUTION IS
  UNCHANGED: the finding still names the excluded declaration, never a caller; only which RULES are
  considered to have scope-matched it widens. Two CHA-bound dispatch sites share the identical hazard and
  both are covered: the main method-call dispatch route (`visit_expr_method_call`, which is also the
  funnel for field/loop/tuple/factory-return dispatch-typed receivers) and the separate implicit
  STRINGIFICATION dispatch route (`charge_stringify_bound` — `println!("{}", e)` on a dispatch-typed
  `e`, which never calls `.method()` by name at all). Left explicitly unexamined, as a documented
  residual rather than a silent narrowing: the self-typed trait-DEFAULT-body dispatch (`scan.rs`'s
  separate `t_type`-keyed CHA fallback) and the cross-dependency workspace-chaining union (gated off by
  default behind `CANDOR_WORKSPACE_CHAIN`) — both share the same `trait_impls`-bounded shape and the same
  theoretical exposure, but neither was reached by any real corpus evidence and closing them was judged
  a larger, separately-scoped change.
  Five controls, falsified against the pre-fix binary: the scoped defect case now exits 2, naming the
  excluded declaration (never relabeled to the caller); the unscoped control still exits 2, unchanged,
  with no duplicate finding now that two routes (the declaration's own name and the reaching caller) can
  both match; a scope matching neither the declaration nor any reaching caller stays exit 0 on the
  identical tree; a two-hop TRANSITIVE ancestor (`App::main` → `Service::run` → `Runner::dispatch`) is
  reached exactly like an ordinary propagated effect; and an excluded conformer NOTHING in scope ever
  dispatches to dynamically is provably unaffected — both as a dedicated fixture and, more importantly,
  as a byte-identical diff against the pre-fix binary on this repo's own four real crates under its own
  `deny Net Db Exec Ipc` policy (which genuinely exercises the peek on real `tests/` content: 77 and 128
  real `outOfScope` findings on two of them) AND under a synthetic but real scoped policy matching dozens
  of this codebase's actual function names. Four new process tests
  (`peek_scope_attribution_*` in `crates/candor-scan/src/tests.rs`) pin the defect, the transitive case,
  the over-charge control, and the stringify-dispatch site. `FnInfo` gained a new analysis-only field
  (`dispatch`) — NOT part of the public report schema (`candor_report::ReportEntry` is untouched, so an
  ordinary scan's published report is byte-identical) — cached like any other Pass-B result
  (`cache_schema` rev9 → rev10).
  **Owed alongside this fix, not mine to write:** a SPEC clause and a conformance PART for "a peek
  finding's scope test must consider every in-scope function that can reach it via dynamic dispatch, not
  only its own name" — interchange behaviour, four-way, per `[[candor-034]]`'s own "row before port"
  rule. Designed to interact with candor-swift's `dispatch-widened` exclusion class (also unspec'd):
  rust's fix never needed that class (its peek's attribution was already unambiguous — the excluded
  declaration is always the sole source, since the peek never unions anything), so the clause should
  cover the property ("which in-scope names may a peeked finding's scope test match") without assuming
  every engine needs a `dispatch-widened`-shaped fallback to get there.

- **⚠ Third adversarial review (2026-08-29): TWO silent gate-passes in `[workspace]` member
  resolution, both pre-existing and untouched by the same day's TOML-parser move. Fixed by replacing
  the hand-rolled resolver with a real `glob` matcher and a recursive nested-workspace fan-out — the
  same "a hand-rolled scanner missed a spelling twice in a week" argument that moved manifest identity
  parsing onto the real `toml` crate, applied one level up to MEMBER resolution.**
  - **(1) NESTED WORKSPACE: a member that is itself a `[workspace]` root vanished with ZERO
    disclosure.** `scan_target` computed `workspace_members` once and handed each resolved member
    straight to `scan_one` as a plain crate; if that member declared its OWN `[workspace]` table,
    `scan_one`'s nested-package filter (the same "a subdir with its own Cargo.toml is a different
    package" rule the outer fan-out itself relies on) pruned every one of its inner members from the
    walk — not into `excluded`, not into `outOfScope`, no stderr, nothing. Reproduced: an inner member
    reaching `Net` through an impostor `{ workspace = true }` dependency was completely absent from the
    report (not even a package entry), `deny Net` printed "policy ✓" at exit 0. `cargo metadata`
    refuses this layout ("multiple workspace roots found"), but candor-scan's own stated purpose is
    reading source WITHOUT building it — the identical reasoning `verified_workspace_root` already
    applies to the ancestor-coincidence fix below. Fixed by `expand_nested_workspace_member`: every
    resolved member that itself declares `[workspace]` is fanned out recursively (its own root package,
    if any, plus its own resolved members, checked again for the same thing), instead of being scanned
    as one flat crate. A nested workspace with zero resolved members warns and falls back to scanning
    itself as one crate, mirroring the top-level "declares \[workspace\] but no members resolved" arm
    exactly.
  - **(2) MULTI-LEVEL GLOB: `members = ["crates/*/*"]` — an ORDINARY layout, not exotic — resolved to
    ZERO members.** `workspace_members`'s matcher special-cased a bare `*` and a single trailing `/*`;
    anything else fell through `.strip_suffix("/*")` and looked for a LITERAL directory named
    `crates/*`, found none, and returned empty. `cargo metadata` on the same tree reports 3 real
    members. Reproduced: `scan_target` read "no members resolved", fell back to a single-crate scan of
    the root, and `deny Net` exited 0 over a real, unscanned `Net` call three directories down. Fixed by
    routing every `members`/`exclude` entry through the `glob` crate — the SAME crate cargo's own
    workspace resolver expands these fields through, not a third hand-rolled special case — so any glob
    shape (`*`, `prefix/*`, `a/*/b`, …) resolves exactly as cargo resolves it. An entry that is a real
    glob but matches nothing, or a literal path with no manifest, still resolves to nothing (unchanged,
    pinned behaviour — see `workspace_members_expand_globs_and_honour_exclude`); an invalid glob pattern
    is warned rather than silently dropped.
  - **Over-charge control (byte-identical, old binary vs. new, before vs. after):** an ordinary
    single-level `members = ["crates/*"]` workspace, a plain single crate, this repo's OWN real
    workspace via `ci/self-gate.sh`, and a direct `candor-scan .` of this repo's own root — all
    byte-for-byte unchanged. Every `caca530`/`fda08ad`/`75045f0`/`2e0521a` test and control passes
    unchanged (250 candor-scan unit tests + 74 CLI tests).
- **⚠ Second adversarial review (2026-08-29) of the ⟨caca530⟩/⟨fda08ad⟩/⟨75045f0⟩ calibrated-crate
  ledger fix, plus a third finding from the same reviewer round: TWO real defects closed, one measured
  and explicitly ACCEPTED as a stated, safe-direction residual.**
  - **(1) `find_workspace_root` granted a false exemption on ANCESTOR NAME COINCIDENCE, not just
    resolution — the code's own doc comment claimed this "cannot manufacture a false exemption"; that
    was measured false.** Reproduced on a real `rust-lang/cargo` clone: a `vendor/fake-lib/` directory
    that is NOT a declared workspace member (`cargo metadata` run there errors "believes it's in a
    workspace when it's not") declares `log = { workspace = true }` and performs a real, unmodelled
    effectful call. The outer, real workspace ALSO happens to declare an unrelated, genuine, bare-version
    `log` in `[workspace.dependencies]` — pure ANCESTRY, not membership — and `find_workspace_root`'s
    upward walk resolved against it, granting the `CALIBRATED_CRATES` exemption to an impostor the outer
    workspace has nothing to do with: `"functions": []`, exit 0. Fixed by `verified_workspace_root`:
    before trusting an ancestor's `[workspace]` table, confirm the scanned directory IS that ancestor (a
    non-virtual manifest resolving against its own table) OR is one of its own RESOLVED members (real
    glob expansion + `exclude`, via `workspace_members` — not re-derived, called), FAILING TOWARD
    DISCLOSURE on any ambiguity, exactly like the redirect-resolution arm it feeds. The doc comment's
    disproven claim is corrected in place. Over-charge guard: a GENUINE member inheriting a genuine,
    honest `workspace = true` registry dependency from the SAME root stays byte-identical.
  - **(2) A THIRD finding, surfaced while fixing (1) and taking priority over it: `has_workspace_table`/
    `workspace_members` were STILL line-based, matching only the literal string `"[workspace]"` — the
    exact class of missed spelling ⟨75045f0⟩'s own doc block claimed was safe here ("a missed spelling
    here is LOUD, not silent"). MEASURED FALSE.** A workspace declared via TOML dotted keys
    (`workspace.members = […]`, real and `cargo metadata`-resolved, identical to the header form) made
    `has_workspace_table` return `false`: `scan_target` fell through to a single, package-less crate
    scan, `analyzed.count` read 0, zero stderr, and a `--policy "deny Net"` gate over the member's real
    Net call printed "policy ✓" at exit 0 — a silent, gate-passing false all-clear over the WHOLE
    workspace, not a loud fallback. A SIBLING of the same shape: `read_crate_name`'s `[package]`+`name =`
    line pair missed `package.name = "…"` (also valid, dotted). Reproduced live: a workspace root naming
    itself this way had its OWN package silently dropped from the fan-out (`scan_target`'s
    `if read_crate_name(dir).is_some() { dirs.push(dir) }`) — a real `TcpStream::connect` at the root
    vanished completely: absent from `functions`, absent from `excluded`, zero stderr, `deny Net` exit 0.
    Fixed by moving `has_workspace_table`, `workspace_members` and `read_crate_name` onto the same real
    `toml`-crate parsing ⟨75045f0⟩ already uses for the dependency-identity surface, through one shared
    `read_manifest_table` — a dotted key and a header section are the SAME structure to a real parser, so
    there is now one tier, not two. The line-based `toml_scalar`/`toml_string_array` primitives, left with
    no remaining production caller, are deleted rather than kept as cruft; `toml_section` survives (its
    one caller, `parse_features`, closes over local feature names, not an identity/membership surface, so
    a missed spelling there is a feature-gating miss, not a silent purity claim).
  - **(3) MEASURED AND ACCEPTED, NOT FIXED: `blind_direct` and its per-fn siblings (`direct`, `hosts`,
    `cmds`, `paths`, `tables`, `incomplete`, `unknown_why`) are keyed on the bare qualified name, which
    two `#[cfg(...)]` branches of one same-named function share** — real `crates/cargo-util/src/paths.rs`
    shape (a `#[cfg(target_os = "macos")]` body calling an unmodelled FFI crate beside a
    `#[cfg(not(...))]` no-op stub). Reproduced live: BOTH branches' report entries carried
    `"invisible": ["core_foundation"]`, one of them on the branch that provably makes no such call.
    PRE-EXISTING and INDEPENDENT of all three prior commits (`core_foundation` is not a
    `CALIBRATED_CRATES` entry) — ⟨75045f0⟩ only widened which calls reach genuine disclosure, which made
    this always-present, always-over-reporting keying quirk visible in more cases without creating it.
    **The obvious quick fix (deduplicate the report so one qualified name yields one entry) was tried and
    REVERTED**: it silently drops a report entry, which
    `a_qualified_name_carried_by_two_cfg_gated_units_yields_one_violation_not_two` pins as required
    ("the report itself still lists both units, so this is a GATE de-duplication, not a lost entry") —
    that test's own fixture never caught this finding because its two cfg branches happen to perform the
    identical call, so the cross-contamination is invisible whenever the branches agree. **Accepted**
    because: the union is a TRUE, SAFE-DIRECTION statement about the shared name (a syntactic scanner
    cannot know the build target, and `#[cfg(unix)]`/`cfg_if!` arms are deliberately unioned elsewhere in
    this file on exactly that argument) and never weakens a gate — a real violation on either branch
    still fires on the shared name; a correct per-declaration fix needs the DIRECT-write maps
    disambiguated while `calls`/the propagated `*acc` maps stay on bare names (a caller genuinely cannot
    know which cfg branch of a callee will run, so edges must stay name-keyed) — a cross-cutting change to
    this function's core data model and, since `"fn"` is the identity candor-query/gate baselines/
    `whatif`/`callers`/the other three engines' matching convention all key on, a cross-engine SPEC
    question this family's own "write the row before the port" rule reserves for its own design pass, not
    a patch bundled into an unrelated adversarial-review response. STATED, not silently absorbed — a
    characterization test (`cfg_branch_pair_shares_one_invisible_disclosure_stated_residual`) pins the
    current shape so a future change either preserves it deliberately or revisits this ruling explicitly.
  - **Controls, falsified against the pre-fix binary**: (a) the ancestor-coincidence case now discloses
    `invisible: ["log"]`, no fabrication; (b) the dotted-key workspace now fans out to its member and
    catches the Net violation under `--policy`, where it previously read `analyzed.count: 0` / exit 0; (c)
    OVER-CHARGE GUARDS — a genuine workspace member, a header-syntax workspace, and an ordinary
    single crate all stay byte-identical; (d) REGRESSION — every ⟨caca530⟩/⟨fda08ad⟩/⟨75045f0⟩ test
    passes unchanged, and `ci/self-gate.sh` (this repo's own real, header-syntax Cargo workspace) is
    unaffected.
  - Full local verification: `cargo test --workspace` (250 candor-scan unit tests + 74 CLI tests, both up
    from before), both clippy legs (`-D warnings`), `ci/gate-equivalence.sh`, `ci/self-gate.sh`,
    `ci/wrapper-smoke.sh`, `tests/integration.sh` — all green.

- **⚠ `--policy` is now a usage error (exit 2) on ten descriptive verbs that accepted and silently
  dropped it: `show`/`where`/`callers`/`map`/`containment`/`reachable`/`path`/`impact`/`blindspots`/
  `tour`.** BACKLOG "`--policy` accept-and-drop is THREE engines, not one" (candor-java `37c9b10` fixed
  the same defect on the same set first). The shared query grammar (`candor-query/src/grammar.rs`)
  accepts `--policy <file>` on every verb — the SPEC §3.3.1 grammar line requires that — but these ten
  never read the parsed value back: verified at HEAD, byte-identical output with and without `--policy`,
  no diagnostic either way. None of SPEC §3.1's pinned JSON shapes for these ten carries a
  policy-derived field (checked per-verb, not assumed — `blindspots`/`containment` were the two the
  intuition said might want one, and do not), so there is nothing for `--policy` to do here: the same
  shape `gains`/`diff`/`rewire` already carry and already refuse with their own bespoke parsers. Applies
  the identical rule in the ONE place all ten verbs share (`Shape.has_policy` in the shared grammar) via
  a single check, naming `gate --report`/`whatif`/`fix`/`fix-gate`/`unverified` as the policy-relative
  alternatives. `CANDOR_POLICY` is likewise inert on these ten — matching `gains`, the model this fix
  follows, so this is an existing spec-sanctioned gap rather than something the fix introduces.
  **Controls (falsified against the pre-fix binary):** every verb that already threads `--policy`
  (`gate`/`whatif`/`fix`/`fix-gate`/`unverified`/`gains`, plus `diff`/`rewire`'s existing loud rejection)
  is byte-identical pre/post with `--policy` present; every one of the ten newly-rejecting verbs is
  byte-identical to the pre-fix binary when `--policy` is ABSENT (only the `--policy`-present case
  changes, from a silent wrong answer to a loud refusal naming the problem and the remedy); the pre-fix
  binary's WITH-`--policy` output was proven byte-identical to its WITHOUT-`--policy` output on all ten,
  confirming the silent drop actually existed rather than being assumed. `diff` and `rewire` (candor-rust's
  own bespoke parsers) already rejected `--policy` loud before this fix and are unchanged — the twelfth
  verb from the four-engine sweep (`rewire`) needed no rust-side work for that reason.

- **⚠ R59/R60 (SOUNDNESS.md): two independent FFI cardinal sins closed — a local `extern "C"` call
  disclosed NOTHING in rust-deep despite its own callgraph proving it visited the call, and an
  unclassified `libc`/`nix`/`rustix` generic-fd-verb call (`read`/`write`/`close`/...) vanished
  entirely from rust-scan's report instead of disclosing the honest no-classify the crate's own
  comment already claimed.**
  - **R60 (rust-deep, `src/lib.rs`)**: `record_resolved_call` classifies a callee by
    `(crate_name, path)` and floors an unreviewed external crate to `invisible` — both routes are
    gated on the callee being NON-local, so a fn declared in a LOCAL `extern "C" { .. }` block (bindgen's
    shape, `#[link(name="c")]` or bare) fell through both: its crate name is the scanned crate itself
    (never calibrated) and `is_local()` excludes it from the floor-disclosure branch too. `fn run_cmd()
    { unsafe { system(c.as_ptr()); } }` produced `"functions": []` while the SAME run's callgraph
    sidecar read `{"run_cmd":["system"]}` — the HIR walk visited the call and the effect layer attached
    nothing. Fixed by disclosing `Unknown`/`native:extern fn` on `cx.tcx.is_foreign_item(def_id) &&
    def_id.is_local()`, mirroring rust-scan's already-correct `decls.rs` `ForeignMod` handling
    unconditionally rather than routing through the crate-name-keyed `invisible` machinery — the same
    mechanism that already correctly discloses `"invisible": ["libc"]` for an external unclassified
    call has no crate name to hang a LOCAL extern block's disclosure off. New `ui/ffi_extern.rs` UI test
    (red on the pre-fix binary: zero warnings emitted; green after: `Unknown` on the direct call and its
    transitive caller, `#[link(name="c")]` behaves identically to a bare block, a genuinely pure sibling
    fn stays unflagged).
  - **R59 (rust-scan, `candor-classify`/`candor-scan`)**: `libc`/`nix`/`rustix` are in
    `CALIBRATED_CRATES`, whose coverage-ledger exemption (`scan.rs`) reads "classify has rules here" as
    "an unmatched call was reviewed and found pure" — true for most calibrated crates, false for these
    three, whose generic fd verbs (`read`/`write`/`close`/`lseek`/`dup`/`fcntl`/...) `classify()`
    *deliberately* leaves unclassified (an ambiguous fd could be Fs/Net/Ipc; the table's own comment
    calls this "an honest no-classify… beats emitting the WRONG effect"). The blanket exemption made
    that comment false: `fn drain(fd: i32) -> usize { unsafe { libc::read(fd, buf, 64) } }` produced
    `"functions": []` — neither function appeared, not as `Unknown`, not as `invisible`, nothing —
    strictly worse than an uncalibrated dependency's honest blind-spot disclosure. New
    `CALIBRATED_BUT_PARTIAL_CRATES` const (`candor-classify`) carves these three OUT of the coverage
    ledger's `CALIBRATED_CRATES` exemption at its one consumer site; a call `classify()` DOES cover
    (`open`/`socket`/...) never reaches the ledger at all (gated upstream on `classified.is_none()`), so
    their precise effects are untouched — only the genuinely-unclassified calls now join `invisible`,
    matching what rust-deep already did for the identical fixture. No effect is fabricated (never Fs/Net
    — disclosure, not resolution). New `candor-scan` test
    `libc_generic_fd_verb_discloses_invisible_instead_of_vanishing` (red on the pre-fix binary: the bare
    fixture's `"functions"` array was empty; green after, with controls proving a classified call stays
    noise-free and a mixed classified+unclassified fn keeps both).
  - **Corpus quantification** (`~/.cargo/registry`, 1202 cached crates, standalone scans, before/after
    binary diff): 97 crates (8%) differ, every byte of every diff explained by (a) a new
    `invisible`/`coverage.uncovered` entry for `libc`/`nix`/`rustix` or (b) a previously wholly-silent
    report (`"functions": []`) gaining its now-honestly-disclosed entries — zero changes to any
    `inferred`/`direct`/`fs`/`hosts`/`hash`/`calls` field anywhere in the corpus. Spot-checked against
    real source: `serial-unix` (a real serial-port crate) calls `libc::{fcntl,close,read,write}` — 6
    previously-silent calls across 5 functions including a `Drop` impl that closes the fd — now
    correctly disclosed; `termios`'s pre-existing disclosure (25 calls) is untouched. Not a flood: only
    crates that declare `libc`/`nix`/`rustix` as a direct dependency and call their generic fd verbs are
    affected at all.
  - This closes the seam these two fixtures probe; it does not make every calibrated crate's
    completeness claim (`CALIBRATED_CRATES` minus the three carved out here) independently re-verified,
    and it does not touch java/ts/swift's own FFI mechanisms (tracked separately, see SOUNDNESS.md R58/
    R61).

- **⚠ The ~79-crate audit R59's own commit named as open: does every OTHER `CALIBRATED_CRATES` member's
  rule set cover its real effectful surface, or does a verb classify() declines to label fall silently
  into the calibrated-crate purity exemption the same way libc's fd verbs did? Three real instances
  found and fixed, all verified against each crate's real vendored source and a compiling fixture; the
  remaining ~76 were checked (rule read + spot-checked against real source, prioritising the
  fd/handle-like and generic-verb-surface crates the method calls out) and found complete.**
  - **`clap::Arg::env`** (`candor-classify`) called `env::var_os(&name)` directly at builder time
    (clap_builder 4.6.6, builder/arg.rs:2205) — a real `Env` read, independent of and long before
    `Command::get_matches()`'s own already-classified read. classify()'s own comment called the verb
    "too generic to gate safely" and left it unmodeled, the same words used for libc's genuinely
    ambiguous fd verbs — but this arm is already crate-gated on `crate_name == "clap"`, and
    clap_builder has exactly ONE `pub fn` ending `::env` in its whole source, so no ambiguity survives
    the gate. `Arg::new("x").env("MY_VAR")` with no `get_matches` call anywhere produced
    `"functions": []`. Now classified `Env` directly (not carved into
    `CALIBRATED_BUT_PARTIAL_CRATES` — the effect is unambiguous once actually checked).
  - **`console::Term`'s raw `io::{Read,Write}` impl** (`candor-classify`) — `Term::write`/`Term::flush`
    (console 0.15.11, term.rs:622-633) call `self.write_through(buf)`, the SAME primitive
    `Term::write_line` already charges `Ipc` for; `Term::read` (term.rs:650) calls
    `io::stdin().read(buf)` directly. classify()'s own comment already NOTED the trait impl ("no
    `write_str` — `Term` impls `io::Write`") without covering it. A fn calling only
    `term.write_all(..)` (no `write_line`) produced `"functions": []`. Now classified `Ipc`,
    `Term::`-scoped so it cannot spread to console's other types.
  - **`arboard::{Get,Set}::file_list` + `Clear::default`** (`candor-classify`) — `file_list`
    (arboard 3.6.1, lib.rs:205,251) is the same builder-then-terminal shape as the already-covered
    `text`/`image`/`html`, and `Clear::default` (lib.rs:265) is `Clipboard::clear`'s own documented
    alternate entry point (`clear() { self.clear_with().default() }`) — neither shares a leaf with an
    already-matched verb. Not missed for ambiguity: missed because
    `eval/coverage-gate/generate.py`'s generator only triggers on a self-scan `inferred` set containing
    Fs/Net/Db/Exec, and Clipboard isn't in that trigger set — a missing Clipboard verb is structurally
    invisible to the completeness gate regardless of phrasing. A fn taking an already-constructed
    `arboard::Get`/`Set`/`Clear` value (isolating the terminal from the already-classified `get()`/
    `set()` constructors) and calling `file_list`/`default` produced `"functions": []`. Now classified
    `Clipboard`.
  - New `candor-scan` tests `clap_arg_env_reads_env_var_directly_at_builder_time`,
    `console_term_raw_write_and_read_trait_impls_are_the_same_ipc_channel`,
    `arboard_file_list_terminal_is_the_same_clipboard_effect_as_text` (all red on the pre-fix binary:
    `"functions": []`; green after, each with a control proving an unrelated pure sibling method on the
    same crate stays unclassified beside the fix).
  - **Corpus quantification** (`~/.cargo/registry`, the same 1202 cached crates, standalone scans,
    before/after binary diff): **0 crates differ** — none of the 1202 published library crates happen
    to spell `Arg::new(..).env(..)` with no `get_matches` call, call `Term::write`/`read` directly
    (rather than through `write_line`/`read_line`), or hold an `arboard::Get`/`Set`/`Clear` value across
    a function boundary. This is the expected shape, not a red flag: CLI-parsing/tty/clipboard code is
    overwhelmingly written in application binaries, not the library crates this registry mirror caches
    — the fixtures above (not this corpus) are what proves each fix fires.
  - `wild::ArgsOs`/`Args`'s `Iterator::next` impl (Windows only, argsiter.rs:47) calls
    `glob::glob_with` — a real Fs read — but ONLY when the returned iterator is actually driven; the
    dominant real-world idiom (`Command::get_matches_from(wild::args_os())`) hands the un-iterated value
    straight into another crate's function, where the drive happens inside clap's own body, invisible to
    any classify() rule on either side. Filed, not fixed: low reachability, Windows-only, and the
    coverage-gate's own candidate generator structurally cannot see it either (a foreign `Iterator` impl
    is excluded from its candidate list by design, the same reason `walkdir::IntoIter::next` needed a
    hand-written rule rather than a generated one).
  - Crates checked against real vendored source beyond the three fixed and `wild`: `git2`, `ignore`,
    `notify`, `reqwest`/`isahc`/`ureq`/`curl`, the DB family (`rusqlite`/`postgres`/`tokio_postgres`/
    `diesel`/`redis`/`mongodb`/`mysql`/`mysql_async`/`sea_orm`/`deadpool_postgres`), `memmap2`/`fs_err`/
    `async_fs`/`tempfile`/`glob`, the rand family (`rand`/`getrandom`/`fastrand`/`rand_core`/`argon2`/
    `bcrypt`/`scrypt`/`pbkdf2`/`password_hash`), `portable_pty`/`async_process`/`duct`, `dotenvy`/
    `dotenv`, `chrono`/`time`, `tracing`/`log`/`rustc_lint`/`rustc_errors`, `rustls`/`native_tls_crate`/
    `tokio_native_tls`, `etcetera`, `sqlx_core`, `walkdir`/`filetime`/`clircle`, `execute`/`ctrlc`/
    `jiff`/`env_logger`, `dialoguer`, `crossterm`/`ratatui`, `terminal_colorsaurus`/`backoff`/
    `grep_cli`/`lscolors`, `tracing_subscriber`, `elasticsearch`, `tonic`/`rdkafka`/`lapin`/
    `async_nats`/`lettre`/`tungstenite`/`pnet` — all found complete for this class (most already carry
    their own "verified against `<crate> X.Y.Z`" citations from the 2026-08-27 coverage-gate sweep,
    which independently closes the DIFFERENT question of missing Fs/Net/Db/Exec entry points).
    `aws_config`/`aws_sdk_*`/`aws_smithy*`/`cap_*` were reasoned about against their existing rules but
    not re-verified against fresh source this pass.
  - Does not touch `CALIBRATED_BUT_PARTIAL_CRATES`, the coverage-gate ratchet (`open.tsv` stayed empty
    throughout — this class is orthogonal to what it tracks), or rust-deep's still-open, separately
    named question (its `invisible` mechanism is crate-name-keyed everywhere except the R59 seam).

- **The completeness-gate GENERATOR's own trigger set was structurally blind to 6 of the 10 concrete
  effects in the vocabulary — widened Fs/Net/Db/Exec to the full Fs/Net/Db/Exec/Clipboard/Ipc/Env/
  Clock/Rand/Log (`eval/coverage-gate/classify_check`'s `CORE` constant).** The R59-class audit above
  named this precisely: `arboard::{Get,Set}::file_list` was missed "not for ambiguity" but because
  Clipboard was outside the generator's trigger set, so a missing Clipboard verb could never become a
  candidate at all, regardless of how a crate's rule was phrased — true of the other 5 excluded effects
  too, not just Clipboard. Widening surfaced two more classes of finding, isolated with an A/B holding
  the self-scan snapshot and every other variable constant (only the `CORE` list differs between arms):
  - **A real generator bug the widening exposed, not caused**: `covered`/`open` were keyed by the
    GUESSED consumer path string, which is not unique — sibling types/fns across modules (async/
    blocking/wasm variants of the same name is the dominant real shape; 610 such guess-strings measured
    shared across >1 distinct self-scan key in the 74 calibrated crates) can legitimately guess the
    identical crate-root alias. A second entry silently overwrote — and thereby DROPPED — a first, real,
    still-open entry whenever both entries' effects happened to intersect `CORE` at once. Latent since
    the generator's first version (proven: reproduces at the OLD narrow `CORE` too, e.g. `filetime::
    open`'s three platform variants collapsed to one), just far less likely to fire with only 4
    qualifying effects to collide on. Fixed by keying on `self_scan_key` (module+type+fn, guaranteed
    unique) instead of the guess string, with output-line deduplication so two entries that legitimately
    share both a guess AND identical effects still collapse to one printed row.
  - **A false-positive class TRIED and REJECTED**: a `pub fn`/`pub struct` living inside a top-level
    `mod NAME;` with no bare `pub` is not reachable by an external consumer at all (found via
    dialoguer's `mod paging;` and mysql's `mod io;`, both containing real effectful `pub fn`s no
    consumer can ever name). A generator-level fix for this was implemented, measured, and reverted:
    `mod internal; pub use internal::Thing;` (re-exporting OUT of an otherwise-private module) is a
    common, idiomatic pattern, not a rare exception — the check dropped `covered.tsv` 1018 → 680 rows
    (~338 genuinely-reachable, already-verified rows) to catch 2 real ones, the opposite of the
    established "loses recall, never shrinks the hard gate" trade. The two known instances are handled
    individually via `REVIEWED_PURE_ENTRIES` instead (a new category there: "genuinely unreachable", not
    "no effect" — same escape hatch, different reason, documented as such).
  - **One real, unambiguous classify() gap fixed**: `dialoguer::Editor::new`/`::default` call
    `get_default_editor()` (edit.rs:31), reading `env::var_os("VISUAL")` then `("EDITOR")` immediately at
    construction — independent of `Editor::edit`'s already-classified `Exec`, the same "builder-time
    env read, separate from the terminal verb" shape as `clap::Arg::env` above. Verified against
    dialoguer 0.12.0: `get_default_editor` is the only caller of either `env::var_os`, no ambiguity.
  - **Quantified** (holding the self-scan snapshot constant across arms): widening added 177 `covered.tsv`
    rows (already-recognized effects that only now qualify as candidates) and, after the two fixes above,
    252 new `open.tsv` ratchet rows — dominated by `Log` (136) and `Unknown`-paired noise (81), then
    `Clock` (58), `Env` (49), `Rand` (28); zero new `Clipboard` candidates (arboard's three verbs above
    were the only gap that effect had). Sampling confirmed the ratchet's `Log` rows are mostly real but
    low-value (e.g. `jiff::tz::db()`'s one-time `debug!` on its `OnceLock` lazy-init transitively tags
    every timezone-lookup convenience function) — the exact "ubiquitous, low-stakes instrumentation"
    flood risk the original narrow cut was tuned to avoid, now visible as a ratchet backlog rather than
    hidden by never looking. `covered.tsv`'s hard gate (`coverage_gate.rs`) stays green throughout; every
    row already in `covered.tsv`/`open.tsv` before this change is untouched (proven by set-difference
    against the pre-change files, not by inspection). NOT the full ratchet: 252 of ~429 widening-surfaced
    candidates remain untriaged, same "may shrink, must never grow without review" contract as before.
    The trigger set now covers the full concrete effect vocabulary (`Llm` deliberately excluded — it
    always co-occurs with `Net` on the same call, so nothing can trigger on it alone); the two named,
    accepted blind spots are unchanged from before this pass (`Unknown` as a qualifying signal — tried at
    ⟨0.33.0⟩'s coverage-gate sweep and rejected, roughly triples the candidate count for modest recall —
    and a foreign `Iterator::next` impl, the `wild::ArgsOs` shape named above).

- **⟨0.34⟩ ITEM 1: the ⟨0.33⟩ cross-policy remedy now names its ACTUAL cause — message-only, verdict and
  `--gate-json` unchanged.** `gate --report`/`whatif`/`fix`/`fix-gate`/`unverified` name a report whose
  peek was bounded by a deny set narrower than the policy in force as *"this report's peek was bounded by
  the deny set its producing scan held, and that set does not cover N rule(s) of this policy"* — TRUE of
  a ≥⟨0.33⟩ producer that genuinely scanned under a different deny set, but MISLEADING of a report that
  predates ⟨0.33⟩ entirely: such a producer never had a `scannedUnder` key to hold ANY deny set in, so
  "does not cover" reads as "chose a different policy" where the truth is "could not yet record one".
  Both readers now check the report's own envelope `spec` (new `candor_report::report_spec`/
  `spec_predates`, unparseable/absent treated as predating — the same direction `ReportMeta::spec`'s doc
  comment already commits to) and print a second sentence naming the real cause and the remedy ("re-scan
  with a 0.33+ engine under THE SAME policy") whenever every report that contributed to the cause predates
  the rung; a SINGLE ≥⟨0.33⟩ contributor keeps the original sentence, because for that report the
  narrower deny set is real. The version is used ONLY to choose which of two already-true sentences to
  print — SPEC ⟨0.34⟩ explicitly RULED OUT a version floor for the VERDICT (a report's age cannot license
  certification: a 0.32 producer's peek was still bounded by SOME policy nobody here can see, so refusing
  is correct either way). `gate --report`'s own eprintln and `crate::completeness`'s shared writer (the
  other of the two places this engine ever prints the cause, feeding `whatif`/`fix`/`fix-gate`/
  `unverified`) each carry the same spec-driven choice, derived from the identical per-report accounting
  (a rule ends up "old-caused" only when EVERY contributing report predates ⟨0.33⟩); route inventory swept
  every `--report`-driven surface in this engine (CLI verbs, `gate-verdict`, the MCP wrapper, which shells
  out with no logic of its own) and found exactly these two independently-coded texts — `candor-scan`'s
  own `--policy` route structurally cannot raise this cause at all (`scannedUnder` always covers its own
  run's policy by construction), so §3.1 byte-equality against it was never at risk. Two test fixtures
  that combined a pre-⟨0.33⟩ `spec` with a `scannedUnder` key no real engine at that spec could have
  written were corrected to `"0.33"` (a report that WRITES the key cannot predate it) rather than left to
  coincidentally still pass. Controls (falsified against the pre-change binary before the fix, matching
  after): a ≥⟨0.33⟩ report's message is byte-identical to the pre-⟨0.34⟩ text on both routes; a pre-⟨0.33⟩
  report's message names the real cause on both routes and never says "does not cover"; the exit code,
  `ok`, `incomplete` and the full `--gate-json`/`whatif --json` documents are identical between the two
  causes and between the pre- and post-change binaries.

- **⚠ SPEC §2 ⟨0.34⟩'s F2 ruling: `parse_spec_ladder` now strips surrounding ASCII whitespace before
  parsing, so a `spec` value like `" 0.33"` reads as 0.33 rather than unparseable.** `candor_report::
  spec_predates` (added in the ⟨0.34⟩ ITEM 1 fix above) fed `major.parse::<u32>()`/`minor.parse::<u32>()`
  directly off `spec.split_once('.')`, and neither Rust integer parser tolerates surrounding whitespace —
  `" 0.33".parse::<u32>()` on the split major segment errs, so a report whose `spec` carried incidental
  padding (a config template, a hand-edited envelope, anything upstream of the JSON literal) misread as
  unparseable and therefore as predating ⟨0.33⟩: the cross-policy refusal's remedy would wrongly claim
  "this report was produced before ⟨0.33⟩, when a producing scan did not yet record the deny set its peek
  ran under" of a report whose `scannedUnder` key was plainly present in the same document — a false
  diagnosis manufactured by a formatting artifact, exactly the misdiagnosis the ⟨0.34⟩ rung exists to
  retire (candor-spec `conformance/run.sh` PART 80's `ws` cell, MEASURED: candor-java and candor-ts already
  trim; candor-rust and candor-swift did not). Fixed by `spec.trim_ascii()` (ASCII-only, not `trim`'s wider
  Unicode whitespace, matching the family's other spec-ladder lexers and SPEC §3.4's identical ruling for a
  config version token: "a trailing `\r` is whitespace, not part of the version") before the `split_once`.
  Message-only, like the fix above: this rung mints no wire key and moves neither `ok` nor the exit code —
  only which already-true sentence prints. Controls (falsified against the pre-change binary first): `"
  0.33"`, `"0.33 "`, `"\t0.33"`, `" 0.33 "` and a CRLF-wrapped `"\r\n0.33\r\n"` all read as 0.33 post-fix
  (pre-fix, all five wrongly predated); the over-charge control — `"0.32"` (genuinely old) and `"0.9"` (the
  lexicographic-ladder trap, 9 < 33 numerically despite `"9" > "3"` as strings) — still correctly predates
  on both binaries, so the fix does not swallow the real case; absent/garbage (`""`, `"abc"`, whitespace-
  only) still predate, fail-closed, on both binaries. The full `--gate-json` document is byte-identical
  across every case above, both cross-binary (pre-fix vs. post-fix) and cross-case (same shape whichever
  sentence printed) — confirmed by hand-built fixtures run through `gate --report` rather than by unit
  test alone, since the defect's user-visible effect is on the CLI's human-channel message. A new
  `candor-report` unit test, `strips_ascii_whitespace_before_the_ladder_parse`, pins the ladder parse
  itself.

- **⚠ The residual R59/R60 left named, closed: rust-scan's coverage-ledger `CALIBRATED_CRATES`/
  `PATH_CALIBRATED_CRATES`/`CALIBRATED_PREFIXES` exemptions are STRING matches against the call's
  syntactic first path segment, with no check that the crate wearing that name is the actual, reviewed,
  published artifact `classify()`'s rules were written against — the COLLISION half of "a crate name used
  as an identity when it is not one" (BACKLOG "rust-deep's crate-name-keyed `invisible` mechanism,
  everywhere else"). A `path`/`git` dependency can be named anything, including one of the 82
  `CALIBRATED_CRATES` entries: a `path` dependency literally named `log` performing an un-modelled
  effectful call reproduced EXACTLY like R59/R60 — `"functions": []`, total silence — purely because
  `log` is calibrated; the identical call shape under an uncalibrated name (`logimpostor`, the only
  variable changed) correctly disclosed `invisible`.** Note the correction this cost: the original filing
  called the class "a key assumed unique that isn't", which names only the collision half — the ABSENCE
  half (no crate name to key on at all) is R60's actual mechanism above, and an audit briefed on
  uniqueness alone would have walked past it.
  - **Fixed (`candor-scan`)**: new `non_registry_lock_names` (`deps.rs`) reads the scanned dir's
    Cargo.lock and returns every package name CONFIRMED not registry-sourced (a `path`/`git` source, or a
    workspace-local package with no `source` line at all). The coverage-ledger filter (`scan.rs`) strips
    all three CALIBRATED_* exemptions for any name in that set, checked on the Cargo.lock-resolved real
    package name (not a caller-chosen manifest alias). A DENYLIST narrowing, not an allowlist: the
    exemption behaves exactly as before unless Cargo.lock gives POSITIVE evidence of non-registry
    sourcing, so an absent/unreadable lockfile costs nothing.
  - **Rust-deep (`src/lib.rs`) already immune to this direction, verified rather than assumed**: its
    `invisible_direct` disclosure keys on `cx.tcx.crate_name(def_id.krate)` — the compiler's OWN resolved
    identity for the callee's defining crate, not a syntactic guess — and applies NO CALIBRATED_CRATES
    shortcut at all (every unclassified non-local, non-std call is disclosed unconditionally). Confirmed
    a dependency cannot even be compiled under the name `std` alongside the real one (`error: cannot
    resolve a prelude import` — verified live); `core`/`alloc` CAN be colliding-named (verified live,
    both compile), but every consumer of those three names in rust-deep is either gated on the exact
    trait-name allowlist `is_pure_std_trait` already carries (Display/Debug/Error/ToString/Clone/
    PartialEq/Eq/PartialOrd/Ord/Hash/Default — Iterator/Fn*/Drop/io::Write deliberately excluded) or
    disambiguated by an already-existing crate-type suffix on self-identity lookups — narrowing the
    residual to a deliberately-adversarial dependency named `core`/`alloc` ALSO defining a trait under one
    of those 11 names with unresolved dynamic dispatch. Filed as a stated, unfixed, very-low-severity
    residual rather than left unmeasured: real-world accidental collision (unlike `log`, a plausible
    internal-shim name) is implausible here, and narrowing the hot trait-purity path carries its own
    over-charge risk against a threat model this narrow.
  - New `candor-scan` test `crate_name_collision_with_a_calibrated_crate_loses_the_ledger_exemption` (red
    on the pre-fix binary: `"functions": []` for the `log`-named path dependency) with two controls: the
    same name as a genuine registry dependency keeps the exemption unchanged (the over-charge guard), and
    the same impostor with no Cargo.lock present falls back to the pre-fix behavior unchanged — a stated
    residual limit, not a silent one.

- **⚠ The above's own "stated residual" was true of the codebase and false of the OUTPUT: `non_registry_
  lock_names` alone returns empty with no Cargo.lock, so EVERY `CALIBRATED_CRATES`/`PATH_CALIBRATED_CRATES`/
  `CALIBRATED_PREFIXES` exemption reverted to the pre-fix, unconditional, name-only behaviour — with
  nothing in the JSON saying so. Adversarial review (2026-08-28) reproduced it live: the `log`-named `path`
  dependency from the fix above, scanned with no Cargo.lock in the tree, performing a real `Net` call,
  still printed `"functions": []`, exit 0, zero disclosure — and candor-scan exists to scan source WITHOUT
  building it, so a Rust library repo (which routinely does not commit `Cargo.lock`) is not an edge case
  of this residual, it is the residual's target population.**
  - **Fixed by widening the identity check to a source that is NEVER absent**: new `non_registry_manifest_
    names` (`deps.rs`) reads the scanned dir's own Cargo.toml manifest(s) — inline-table (`name = { path =
    "…" }` / `{ git = "…" }`) and header-table (`[dependencies.name]` followed by a bare `path =`/`git =`
    line) forms both — for `path`/`git` source evidence, independent of Cargo.lock. Unioned with
    `non_registry_lock_names` at the one call site (`scan.rs`) that consumes either. No new wire key: the
    reproduced defect and the manifest-detectable cases it covers now disclose through the SAME `invisible`
    / κ-coverage-ledger machinery the lock-based check already used — same treatment, exit 0, no
    `incomplete` escalation, for consistency with how that machinery already treats the Cargo.lock-
    confirmed impostor case one entry up.
  - **Why this closes the realistic case without a lockfile**: you cannot get crates.io to publish a
    second `log` — a name-squatting impostor is attached to a project via `path`/`git`, which Cargo.toml
    must state directly on the dependency declaration (there is no way to depend on a path/git source
    without writing `path =`/`git =` somewhere). A bare-version declaration (`log = "0.4"`) is therefore
    strong evidence of a genuine registry dependency even absent a lockfile.
  - **Verdict, not just disclosure, left unchanged on purpose**: this does NOT force `incomplete`/exit 2.
    Escalating every lockfile-less scan that happens to depend on a calibrated crate would be the exact
    over-charge the brief this fix responds to warned against — the ordinary, honest, unlocked Rust library
    is the common case, not the exception.
  - **STATED, NARROWER RESIDUAL, not silently absorbed**: a bare-version dependency with NO Cargo.lock and
    no `path`/`git` anywhere in the manifest cannot be told apart from a genuine registry crate by anything
    in the scanned tree — a `[patch.*]` table override or `.cargo/config.toml` source-replacement can still
    swap the real artifact out invisibly. Closing the `[patch]` half needs the identical `path=`/`git=`
    check extended to `[patch.*]` tables (not implemented here); closing the `.cargo/config.toml` half
    needs reading outside the scanned tree entirely, which candor-scan does not do anywhere else in the
    codebase. Disclosing THIS residual would require a new wire key (something weaker than `invisible` —
    "exemption applied, identity unverifiable" — since forcing it into `invisible`/`uncovered` for every
    unlocked bare-version calibrated dependency is precisely the over-charge just above) — that is a
    candor-spec decision, not made here.
  - Four new/expanded `candor-scan` tests: the reproduced defect (`path` dep, no lockfile) now disclosing,
    a `git`-sourced equivalent, the header-table manifest form, and the sharpest over-charge guard in the
    file — a bare-version dependency with no lockfile keeps the exemption, proving the fix does not scream
    on every unlocked project.

- **⚠ The above's own enumeration was still short two spellings, both live-reproduced against the shipped
  binary before this fix, and one of them turned out NOT to be the exemption bug the brief describing it
  assumed.**
  - **(1) Workspace inheritance.** A member declaring `log = { workspace = true }` while the REAL
    `log = { path = "../evil-log" }` sits in the WORKSPACE ROOT's `[workspace.dependencies]` reproduced the
    exact silent drop (`"functions": []`, exit 0) — `non_registry_manifest_names` walked the scanned
    member's own directory only and never the workspace root, so a real `Net`-performing impostor named
    `log` kept the `CALIBRATED_CRATES` exemption with no Cargo.lock involved at all. Isolation control:
    renaming the SAME shape to a non-calibrated name (`logimpostor`) correctly disclosed `invisible` on the
    pre-fix binary — the only variable was the name, not the shape.
  - **(2) Dotted-key TOML — and the premise about it was wrong.** `log.path = "../evil-log"` inside a flat
    `[dependencies]` table is valid TOML (`cargo metadata` resolves it as a path dependency); the
    line-based `cargo_toml_deps` had never modelled this PRODUCTION at all (only the inline-table and
    header-table spellings), so it parsed the whole `log.path` token as one dependency NAME. That is a
    defect one layer BELOW `non_registry_manifest_names`: it corrupted `cargo_toml_deps`'s base
    "declared dependency" set itself, so `deps.contains("log")` was FALSE and the call skipped the κ ledger
    entirely — reproduced silent (`"functions": []`) on BOTH a calibrated name (`log`) and a
    non-calibrated one (`logimpostor`), which a calibrated-exemption bypass could never do. Fixing only
    `non_registry_manifest_names` would have left this reproduced defect completely unaffected, because the
    call never reached the CALIBRATED_* check to begin with. The brief describing this class named the
    mechanism as the exemption; measured, the mechanism for this spelling was one level upstream of it.
  - **Both are closed by the same structural fix, not two patches**: `cargo_toml_deps` and
    `non_registry_manifest_names` now parse Cargo.toml with a REAL TOML parser (the `toml` crate, added as
    a direct `candor-scan` dependency — already resolved in this workspace's own `Cargo.lock` at
    `1.1.2+spec-1.1.0` as a transitive dependency of the sibling `candor` lint crate via `dylint_internal`,
    so this adds a graph edge, not a new version to vet) instead of enumerating surface spellings line by
    line. Argued in `deps.rs`'s module doc: inline tables, header-table sections and dotted keys are three
    surface spellings of ONE TOML structure (a nested table under the dependency's key), so a line scanner
    that branches per spelling is a spelling ALLOWLIST wearing a parser's clothes — measured TWICE in one
    week to have missed a spelling ([[candor-denylist-over-allowlist]] applies one level up from the
    `CALIBRATED_*` lists it usually names). A real parser needs exactly one check (`Value::as_table` +
    `contains_key("path"/"git")`) instead of an ever-growing branch list, closing the entire class rather
    than the two instances measured. New `find_workspace_root` (walks UP from the scanned directory to the
    nearest ancestor declaring `[workspace]` — bounded by the filesystem's own depth) resolves a
    `{ workspace = true }` entry against that root's `[workspace.dependencies]` table under the same key.
    **FAIL TOWARD DISCLOSURE, not toward trust**, when that resolution cannot be completed for any
    reason — no root found, root unreadable/unparseable, or the root simply silent on that key — because
    `workspace = true` carries ZERO source evidence of its own (unlike a bare version, which cannot exist
    without a real registry entry backing it); trusting an unresolved redirect is exactly how defect (1)
    passed silently. The REST of the manifest-reading surface (`toml_section`/`toml_scalar`/
    `toml_string_array`/`has_workspace_table`/`workspace_members`/`read_crate_name` — package-name reading
    and workspace-member glob expansion) is deliberately UNCHANGED and stays line-based: a missed spelling
    there is loud (an explicit "no members resolved" warning, or a filename fallback), never a silent
    purity claim, so the identity-verification-grade fix does not belong there and widening the rewrite to
    cover it would have been scope creep against this fix's own trigger.
  - **Controls, each isolating exactly one variable, ALL FALSIFIED AGAINST THE PRE-FIX BINARY**:
    (a) OVER-CHARGE GUARD — an honest workspace member (`log = { workspace = true }` where the root's
    `[workspace.dependencies].log` is a genuine bare version) stays byte-identical (`"functions": []`), so
    the fix does not turn ordinary workspace inheritance into noise; (b) OVER-CHARGE GUARD — a dotted-key
    dependency with a genuine bare version (`log.version = "0.4"`) also stays byte-identical, confirming
    the `cargo_toml_deps` rewrite does not over-charge the common case either; (c) FAIL-TOWARD-DISCLOSURE —
    a member declaring `{ workspace = true }` with NO discoverable workspace root at all (a partial
    checkout) now discloses `invisible`, where the pre-fix code (which never attempted cross-file
    resolution) also happened to disclose here only because the exemption path was never reached — this
    control exists so a future change to the lookup cannot silently regress it into a trust default;
    (d) both reproduced defects — the workspace-inheritance impostor and the dotted-key impostor, calibrated
    name — now disclose `invisible: ["log"]` with `coverage.uncovered` counting the call, matching the
    shape every other κ-ledger disclosure in this file already carries; (e) REGRESSION — `caca530`'s and
    `fda08ad`'s own tests (`crate_name_collision_with_a_calibrated_crate_loses_the_ledger_exemption` and its
    header-table sibling) pass unchanged against the rewrite, including their own over-charge and
    no-lockfile controls.
  - **STATED RESIDUAL, unchanged from the note above and not widened by this fix**: a `[patch.*]` table
    override or `.cargo/config.toml` source-replacement still cannot be seen from manifest text alone.

- **⚠ BACKLOG B4: `eval/coverage-gate`'s hard gate asserted PRESENCE, not AGREEMENT — a rule narrowed to a
  still-non-`None` but WRONG effect passed silently.** `coverage_gate.rs` asserted only
  `classify(krate, path).is_some()`. Reproduced live before touching anything: changing async_nats's
  `connect`/`publish`/`subscribe`/`request`/`flush` from `Some("Net")` to `Some("Log")` left `cargo test -p
  candor-classify --test coverage_gate` green — a `deny Net` policy would wave through code opening a NATS
  connection, exactly the class this gate exists to stop.
  - **The fix is bigger than a comparison operator.** `covered.tsv`'s existing third column
    (`effects`) is NOT classify()'s own answer — it is the self-scan oracle's independent, full
    `inferred` set for that entry point (real call-graph reachability over the crate's own source), and
    the two legitimately disagree by design: `async_nats::connect`'s self-scan set is `Fs,Log,Net,Rand,
    Unknown` (auth touches a creds file, connecting logs, etc.), so `Log` was ALREADY a member of it —
    a naive "classify() result must be a member of column 3" fix would have MISSED the reviewer's exact
    mutation. It also breaks the other way: `async_nats::Consumer::request_batch`'s self-scan set is
    `Log` alone while classify() correctly returns `Net` for it, so membership-testing column 3 would
    flag today's genuinely-correct row as a false regression. classify() returns one label per call site;
    `covered.tsv` recorded a different oracle's superset. Fixed by adding a fourth column, `classified_as`
    — the actual `classify(crate, consumer_path)` return value at generation time — and asserting EXACT
    equality against it (`classify_check` now captures this alongside the matched guess;
    `coverage_gate.rs` parses 4 columns and compares). The existing 1014 rows were migrated in place
    (crate/path/self_scan_effects columns byte-unchanged; `classified_as` computed by calling today's
    `classify()`, which agrees with generation time on every row — HEAD has not touched `classify()`
    since the commit that generated the checked-in manifest).
  - **All three controls, falsified against the pre-fix gate:** (1) the defect case — Net->Log on
    `async_nats::connect`/`ConnectOptions::connect`/`Consumer::request_batch`/`Context::request` PASSED
    pre-fix, FAILS post-fix, naming the recorded vs. actual effect for each of the 4 affected rows
    (`connect_with_options` is a separate `if` block untouched by the mutation and correctly unaffected).
    (2) over-charge control — the unmodified tree still passes all 1014 rows post-fix. (3) a second,
    opposite-shape mutation — widening `tracing_subscriber::fmt::try_init` from `Some("Log")` to
    `Some("Net")` — also FAILS post-fix, proving the check is symmetric, not one-directional.
  - **Sweep for the same shape found no other instance in this repo.** Checked every other manifest/CSV-
    driven regression gate (`ci/gate-equivalence.sh`, `ci/self-gate.sh`, `eval/scaled`, `soundness/
    confirmatory`) and every other `is_some()`-based assertion in `candor-classify`'s own test module.
    `calibrated_crates_are_live` and `ci/self-gate.sh`'s denylist membership check are presence-checks BY
    DESIGN (crate-rule liveness / denylist inclusion, not equality against a per-row recorded expected
    value) — a different shape, not this defect.
  - **Residual, NOT folded into this fix, reported not fixed (out of this fix's scope):** the local
    `entries.json`/`~/.cargo/registry` cache this machine already had on disk is independently stale
    relative to the checked-in manifest (crate version drift — e.g. `ureq`'s API surface moved) —
    regenerating `covered.tsv` from that cache via `generate.py`/`classify_check` as-is would silently mix
    ~76 unrelated candidate-set changes into this fix. The migration above instead computed `classified_as`
    directly from the checked-in rows against current `classify()`, leaving the candidate set untouched.
    The weekly `coverage-gate-refresh` workflow is the correct place to reconcile that drift against a
    FRESH `cargo fetch`, not a hand-edit here.

- **`dd90fae` (nested-workspace vanish / multi-level glob) shipped with no test that would notice its own
  deletion** — the only one of that day's ten commits without one; every sibling was swept and confirmed
  to have a genuine, discriminating test (below). Two `candor-scan` CLI tests added, each proven RED
  against a worktree with only that fix reverted and GREEN at HEAD:
  - `nested_workspace_member_is_fanned_out_and_deny_net_catches_the_inner_call` — an outer `[workspace]`
    whose one member is itself a `[workspace]` root with no `[package]` of its own, whose inner member
    performs a real `Net` call. Reverting `expand_nested_workspace_member`/its call site in `scan_target`
    (scan.rs) reproduces the exact gap `dd90fae` describes: exit 0, `policy ✓`, `analyzed.count: 0`, the
    inner member absent from the fan-out with no `excluded`/`outOfScope` entry.
  - `multi_level_glob_workspace_members_resolve_and_deny_net_catches_the_violation` — `members =
    ["crates/*/*"]` over an ordinary `crates/{a/x,b/y,c/z}` layout, one real `Net` call, checked against
    `cargo metadata`'s own resolution as ground truth (asserted 3 members, matching the fixture).
    Reverting `expand_member_glob`/its call site in `workspace_members` (deps.rs) reproduces the exact
    gap: zero members resolved, fallback to a single-crate scan of the root, exit 0 over three real,
    unscanned crates.
  - **Sweep of the day's other nine commits** (`caca530`, `fda08ad`, `75045f0`, `2e0521a`, `27f4beb`,
    `3e9848c`, `79546f3`, `e4bc419`, `7401af9`): each commit's own production change was reverted in
    isolation (test additions kept in place) and its dedicated test(s) checked. Every one went RED on the
    revert — a clean sweep, confirming the reviewer's claim that every sibling had a test, not just an
    absence of counter-evidence. (`2e0521a`'s and `27f4beb`'s over-charge/residual controls correctly
    stayed GREEN, as designed — they pin behavior the fix must NOT change, not the fix itself.)
  - **Controls**: all 254 candor-scan unit tests + 76 CLI tests (74 existing + 2 new) green; both clippy
    legs (stable, -D warnings; pinned nightly whole-workspace, -D warnings) clean; `ci/gate-equivalence.sh`,
    `ci/self-gate.sh`, `ci/wrapper-smoke.sh`, `tests/integration.sh` all green; runtime unaffected (full
    `candor-scan` CLI suite in ~0.5s).
  - **Reported, not fixed (candor-swift, a different repo/owner)**: swift's `7a89dbc` test
    `testSiblingCallIntoAHOFStillGetsJudged` cannot discriminate its fix from its absence — both its
    callers land on `Unknown` regardless of whether the guard is present. The shape that WOULD
    discriminate: one tracked caller (explicit receiver) plus one untracked sibling caller into the same
    HOF — with the guard deleted, the untracked caller should vanish from the report entirely (not just
    read `Unknown`), which is the observable a correct test needs to assert against.

## [0.33.1] — 2026-08-27

- **`ci.yml`'s `stable-crates-macos` job gains a `timeout-minutes` — the last job in the family
  without one.** Found by `release-preflight [7b]` on this cut, which is the first cut since [7b] was
  fixed to read JOBS rather than the FILE: the file-level question passed here because the
  `build-and-test` job beside it declares a deadline, so the macOS lane inherited GitHub's 6-hour
  default and a hang there would have read as a slow job while blocking `[10]`'s CI-green gate for an
  afternoon. That is verbatim the failure `build-and-test`'s own timeout comment records — "fixing the
  two that had failed and not their siblings is the habit this repo keeps measuring" — one sibling
  further on. Set to 20m against a measured 32-51s across five runs (533 tests; the `target` cache
  makes the build the small part), sized to the work rather than to a round number.

- **⚠ First triage pass over the completeness-gate ratchet (`eval/coverage-gate/open.tsv`): 148 of the
  251 rows closed — 96 real rules added to `covered.tsv` (git2 37, sea_orm 24, rusqlite 16, lettre 16,
  tonic 3) plus 52 rows removed as GENERATOR false positives, none reachable by any external consumer.**
  Worked the priority families the gate's own header names (connection/constructor/handshake/open-create)
  crate by crate against real vendored source, each rule proven with a compiling consumer fixture and an
  A/B against the pre-fix binary (never just by reading the table).
  - **git2 and rusqlite share ONE root cause, found twice**: their `crate_name == "..."` branches in
    `classify()` return unconditionally, so a real consumer's call NEVER reaches this file's own
    `sqlite3_*`/`git_*` FFI-leaf tables lower down — those tables only ever fired when SELF-scanning the
    crate's own internals, where the call resolves to the FFI crate's name instead. Every one of git2's
    `Repository::{open,init,init_bare,init_opts,open_bare,open_ext,open_from_env,discover,discover_path,
    checkout_head,checkout_index,checkout_tree,commit,reference,tag,blob_path}`, `Config::{open,
    open_default,add_file}`, `Index::*`, `Odb::*`, `PackBuilder::write`, `Reference::{delete,set_target}`,
    `TreeBuilder::write` read PURE — a `deny Fs` over a fixture calling `git2::Repository::open` +
    `git2::Cred::credential_helper` (a real `sh -c "<helper> get"` subprocess spawn for
    `credential.helper` auth, real Exec) passed at exit 0 before this fix, exit 1 after. `Remote::list`/
    `RemoteConnection::list` (real `git_remote_ls`) were ALSO missing — the crate's own `::ls` suffix
    matches no method that exists in git2 0.20. rusqlite's online-backup (`Backup::{new,new_with_names,
    step,run_to_completion}`, `Connection::{backup,restore}`) and incremental-BLOB positional I/O
    (`Blob::{read_at,read_at_exact,raw_read_at,raw_read_at_exact,write_at,write_all_at}`) were the same
    shape, plus the loadable-extension entry points (`Connection::{from_handle,from_handle_owned,
    extension_init2}`, `init_auto_extension`) that wrap a caller-supplied raw handle into a live
    connection. tonic's client-only rule missed the SERVER half: `transport::server::Router::{serve,
    serve_with_shutdown}` bind a real listening socket via `TcpIncoming::new` — a `deny Net` on a gRPC
    server missed its own listen call.
  - **sea_orm's transaction and pagination families** were never reached by the existing `::exec`/
    `::execute` allowlist: `DatabaseConnection::{transaction,transaction_with_config}`,
    `DatabaseTransaction::{commit,rollback}`, the three `SqlxXxxPoolConnection::{begin,ping,transaction}`
    trios, `Paginator::{fetch,fetch_and_next,into_stream,num_pages,num_items_and_pages}`, and
    `{Insert,Inserter,TryInsert}::exec_with_returning_{keys,many}` all read pure. FQN-exact throughout
    (not a bare `::ping`/`::begin`/`::transaction` suffix) because `MockDatabaseConnection`/
    `ProxyDatabaseConnection` share every one of those verb names while performing no provable real I/O —
    a blanket suffix would have fabricated Db on both. **Bonus, found only by building the reachability
    fixture** (self-scan's own pass never flagged these — it doesn't track an enum match arm's payload as
    a typed receiver): `DatabaseConnection::{ping,begin,begin_with_config}` dispatch through the identical
    shape and were also silently pure.
  - **lettre's TLS-setup family**: `TlsParameters::{new,new_rustls}` and
    `TlsParametersBuilder::{build,build_rustls}` (the latter calls `rustls_native_certs::
    load_native_certs()`, a real OS-trust-store read) were unreached by the `send`-only rule, along with
    every transport constructor that builds TLS through them (`{Smtp,AsyncSmtp}Transport::{from_url,
    relay,starttls_relay}`), `FileTransport::read` (`std::fs::read` directly), and the sealed `Executor`
    trait's `AsyncStd1Executor::{connect,fs_read,fs_write}` — the last three are `#[doc(hidden)]` in
    lettre's own source, a narrower and less-visited surface than the rest, but genuinely reachable (both
    the trait and the type are `pub`, re-exported at the crate root).
  - **THE 52 REMOVED ROWS are a distinct, second finding**: `eval/coverage-gate/generate.py`'s visibility
    check for a `pub fn` never distinguished `pub(crate)`/`pub(super)`/`pub(in ...)` from a bare `pub` —
    it already excluded `pub(crate)` STRUCTS (`restricted_types()`, from the diesel `RawConnection`
    fix) but never functions, so `sea_orm::DatabaseTransaction::{begin,run}`, every one of ureq's
    `connect`/`connect_host`/`connect_http`/`connect_https` (all `pub(crate) fn`), and 36 others across
    17 crates were carried as open "gaps" no external consumer can ever compile a call to. Fixed the
    generator (`is_bare_pub`, `eval/coverage-gate/generate.py`) so a future regeneration won't
    reintroduce them. A SECOND, related shape surfaced by hand rather than by that fix: rusqlite's
    `InnerConnection`/`RawStatement` and lettre's `NetworkStream` are bare `pub struct`s declared inside a
    PRIVATE `mod` and never re-exported — `restricted_types()` only reads a type's own visibility
    keyword, not its enclosing module chain, so these still read as public entries; both left unclassified
    rather than given a dead rule. Two crates' real spellings also turned out to differ from the ratchet's
    recorded one for the same reason in reverse: rusqlite's `Backup`/`Blob`/`init_auto_extension` and
    lettre's `TlsParameters`/`TlsParametersBuilder`/`AsyncSmtpConnection`/`AsyncNetworkStream` are not
    re-exported at the crate root, so the short spelling `generate.py` guesses and records is not what a
    real consumer's source contains — both the recorded short form and the real module-qualified one are
    now classified, proven by a fixture written against the real, compiling long form.
  - **103 rows remain** (largest: tempfile 13, sqlx_core 13, ureq 10, isahc 8, ignore 8), left in the
    ratchet rather than guessed. The cross-crate `Unknown` gap this gate cannot see through (diesel's
    `establish`, sync `mysql::Conn::new`, `sea_orm::connect_proxy`, `tokio_postgres::connect_raw`,
    tungstenite's transport-generic handshake) is unchanged by this pass — none of the fixes above
    crossed it.

- **A completeness GATE for `CALIBRATED_CRATES`, closing the generator behind the ten silent
  under-reports below, not another instance of it.** The coverage ledger (`scan.rs:2458-2479`) is
  deliberately crate-level: once a crate is calibrated, an unmatched path is a claim of reviewed purity
  with no `coverage.uncovered` disclosure — so an incomplete verb table in a calibrated crate was silent
  in the dangerous direction, and nothing enforced that the table was complete. `crates/candor-classify/
  tests/coverage_gate.rs` (riding the existing `cargo test --workspace` step, no new CI surface) asserts,
  for 669 checked-in `(crate, consumer-facing path)` pairs across 74 of the 82 calibrated crates, that
  `classify()` still recognizes each one as effectful — a REGRESSION gate (removing a rule makes it fail,
  naming the exact entry; verified by reverting and restoring `ignore::Walk::new`'s rule). The 669 come
  from a differential, not a hand list: `eval/coverage-gate/generate.py` self-scans each crate's OWN real
  vendored source with `candor-scan` (a ground truth independent of whether the entry point itself has a
  top-level rule — proven by self-scanning `ignore` against the classify.rs commit BEFORE `Walk::new` had
  one; it still reports `Fs`, via real local call-graph propagation through `.build()`), and keeps every
  candidate where self-scan found a real Fs/Net/Db/Exec reach that `classify()` also already recognizes.
  A second, SEPARATE list (`eval/coverage-gate/open.tsv`, 251 rows across 39 crates — git2, sea_orm and
  rusqlite dominate) is everything self-scan found effectful that no rule recognizes yet: a ratchet, not
  a hard gate (a hard gate over all 82 crates today would be unlandable in one pass), refreshed weekly
  against LIVE crates.io by `.github/workflows/coverage-gate-refresh.yml` — the only place this needs
  network; the per-push gate reads only the checked-in manifest. `REVIEWED_PURE_ENTRIES` (beside
  `REVIEWED_PURE_CRATES` in candor-classify) is the escape hatch for an `open.tsv` row read and confirmed
  pure. Measured against the pre-19ce144 classify.rs, this gate's differential would have caught 5 of the
  ten incident groups below outright (`ignore`, both `git2::Submodule` sites, `mongodb::with_options`,
  `mysql_async::Conn::new`, and all five rusqlite sites) by self-scan alone, with no per-crate hand tuning.
  It misses the other five (diesel's `establish`, `mysql::Conn::new`, `sea_orm::connect_proxy`,
  `tokio_postgres::connect_raw`, most of tungstenite): each reaches its effect by crossing into a
  DIFFERENT, uncalibrated external crate, which self-scan can only report `Unknown` for, not a concrete
  effect — broadening the trigger to include `Unknown` was measured and rejected (987 candidates -> 2423,
  260 uncovered -> 1323: a flood for a modest recall gain). Excludes `tokio_tcp`/`tokio_udp`/`async_net`
  (classified blanket, immune to a verb-list gap by construction), `rustc_lint`/`rustc_errors`
  (compiler-internal, not on crates.io), and `libc`/`nix`/`rustix` (exhaustive syscall-name tables, a
  different audit shape than a wrapper crate's verb allowlist) — each a different reason, not one blanket
  "too hard".

- **⚠ `ignore::Walk::new`/`Walk::from_iter` reported ZERO effects — `deny Fs` passed at exit 0 over
  code that walks the filesystem.** `Walk::new(path)` is `WalkBuilder::new(path).build()` in ignore's
  own source (walk.rs:1128-1146) — the crate's own top-level doc example (`for entry in
  Walk::new(path)`) — but the rule keyed the `Fs` charge on `WalkBuilder::build`/`build_parallel`/
  `WalkParallel::run`/`add_ignore` only, and `Walk::new`/`Walk::from_iter` matched none of them.
  `analyzed.count` proved the function was read and classified — a classification miss, not an
  unread file — and it hid well: an unscoped `deny Fs` still caught the sibling `WalkBuilder` form in
  the same file, so only a scoped policy exposed it. Fixed by adding `ignore::Walk::new`/
  `ignore::Walk::from_iter` as two more FQN-exact constructors, the same plain-`Expr::Call` construction
  site as `WalkBuilder::build` — no receiver typing needed, so it is robust regardless of how the
  returned iterator is later consumed. Controls: `WalkBuilder::new(root).build()` is unmoved, a
  same-named `Walk::new` from an unrelated crate stays pure (crate-gated), and a tree with no `ignore`
  usage is byte-identical (982 vendored crates' own source trees + 15 hand-built consumer fixtures,
  zero diffs outside the two known sites).

- **`diesel::Connection::establish` — diesel's OWN name for `::connect`, and its single most common
  entry point (`SqliteConnection::establish(url)` in every diesel quickstart) — read pure with NO
  `coverage.uncovered` disclosure either.** Trace: `establish` (connection/mod.rs:243) is implemented
  by really opening the backend handle (`sqlite/connection/mod.rs:230`, `pg/connection/mod.rs:176`,
  `mysql/connection/mod.rs:158`), but shared no verb spelling with the `::connect`/build-vs-execute
  VERBS list diesel shares with sqlx. The missing disclosure is not a second, independent defect in
  the coverage LEDGER — that ledger is deliberately CRATE-level (`coverage.uncovered` names dependency
  packages candor has no rules for at all; `fs_extra`/`ssh2`/`native_tls`/`csv`/`tar`/`xz2` disclosed
  correctly in the same corpus round precisely because none of them is a `CALIBRATED_CRATES` entry).
  Once a crate IS calibrated, an unmatched path is a claim of reviewed purity by design — which is
  exactly why an incomplete verb table in a calibrated crate is silent in the dangerous direction, same
  as `ignore::Walk::new`. Fixed by adding `::establish` to the shared VERBS list.

- **THE SWEEP — a systematic audit of all 82 `CALIBRATED_CRATES` for the same shape (a documented
  public entry point reaching an already-modelled effect through a spelling missing from its verb
  allowlist), each verdict checked against the real vendored crate source, not the table. Found and
  fixed NINE more live sites**, all proven red→green on hand-built consumer fixtures and unit-tested
  with a no-fabrication control alongside each fix:
  - `rusqlite::Connection::open_in_memory_with_flags`/`open_with_flags_and_vfs`/
    `open_in_memory_with_flags_and_vfs` (3 more `open*` constructors beyond the three the old exact-
    suffix list matched — `"open_in_memory_with_flags".ends_with("::open_with_flags")` is false, the
    suffix must be the literal tail) and `Connection::blob_open`/`Blob::reopen` (the documented
    incremental-BLOB-I/O API, calling `sqlite3_blob_open`/`_reopen` directly — leaves already in this
    file's own FFI table, just never wired to rusqlite's safe wrapper). Now matched on the
    `Connection::open` PREFIX plus the two blob methods, scoped so a same-crate different-type
    `open`-prefixed method (the private `pragma::Sql::open_brace`) cannot be swept in.
  - `git2::Submodule::clone`/`Submodule::update` — call `raw::git_submodule_clone`/`_update` directly,
    the exact leaves already in this file's FFI-tier NET table, but only when a caller names the raw
    leaf itself; git2's documented submodule-init idiom (`sub.clone(opts)`/`sub.update(init, opts)`)
    calls the safe wrapper, which had no rule. FQN-exact, mirroring the `Repository::clone` fix's own
    discipline (a bare `::update`/`::clone` substring would sweep in git2's many pure `update_*`
    setters and the derive-`Clone` dup on every other git2 type). **Reachability caveat, found by
    testing the fixture, not by reading the table**: `Submodule` values come only from
    `Repository::find_submodule`, an external method absent from candor-scan's generic constructor-name
    list — so `update` fires only with an explicit `let sub: Submodule = ..` annotation, not the
    idiomatic `repo.find_submodule(name)?.update(..)` chain, and `clone` does not fire via ANY method
    call at all: candor-scan blanket-excludes `.clone()` from typed receiver resolution everywhere (a
    deliberate anti-fabrication guard against `Arc`/`Rc::clone` false-positiving through the
    smart-pointer deref-peel). Both rules are kept — correct and harmless — but `Submodule::clone`
    specifically is live only via a UFCS call (`Submodule::clone(&mut sub, opts)`), the same
    "kept, not dead, for the narrower case" shape as `walkdir::IntoIter::next`.
  - `mongodb::Client::with_options` (async + sync) — `with_uri_str` is `ClientOptions::parse(uri)
    .await?; Client::with_options(options)` one call down; a caller who already holds parsed
    `ClientOptions` uses `with_options` directly.
  - `mysql::Conn::new`/`mysql_async::Conn::new` — each crate's own primary connection constructor
    (`connect_stream()?; connect()?` directly in the sync crate), not merely a pool helper; scoped to
    the `Conn::` segment since a bare `::new` would sweep in `Opts`/`PoolConstraints`/`TxOpts`'s own
    pure constructors in the same two crates.
  - `sea_orm::Database::connect_proxy` — the `proxy`-feature sibling of `Database::connect` for a
    caller-supplied `ProxyDatabaseTrait` backend.
  - `tokio_postgres::Config::connect_raw` — the same protocol handshake as `Config::connect`, over a
    caller-supplied stream instead of one it dials itself.
  - `tungstenite::client`/`client_with_config`/`client_tls`/`client_tls_with_config`/`accept`/
    `accept_with_config`/`accept_hdr`/`accept_hdr_with_config`/`connect_with_config` — the
    stream-first client AND server WebSocket-upgrade handshake functions (`{Client,Server}Handshake
    ::start(stream, ..).handshake()`), tungstenite's documented way to run over a caller-managed
    TCP/TLS/mio stream (the dominant real-world shape, since tungstenite itself is sync/transport-
    agnostic) — an allowlist keyed only on the dial-it-yourself `connect` spelling missed all nine.
  - `aws_config::load_from_env` — the crate's own "convenience wrapper" (its doc comment's words)
    around `from_env().load().await`, the already-modelled effect one call down;
    `"load_from_env".ends_with("::load")` is false.

  All nine (bar the noted git2 caveat) verified with a hand-built minimal consumer fixture going from
  0 effectful functions before the fix to the expected charge after, checked against the real crate
  source at the cited line, never the classify.rs table alone. Zero new false positives: the full
  982-crate vendored registry snapshot scans byte-identical before/after (these crates' own
  implementations don't happen to call the fixed sites), and 15 of 17 hand-built consumer fixtures
  using unrelated calibrated crates (`c_async_std`/`c_csv`/`c_curl`/`c_fs_extra`/`c_globset`/
  `c_hyper`/`c_memmap2`/`c_native_tls`/`c_redis`/`c_serde_json`/`c_sqlx`/`c_ssh2`/`c_tar`/`c_tokio`/
  `c_xz2`) are byte-identical too — only `c_ignore` and `c_diesel` differ, by exactly the recovered
  effect and nothing else.

- **⚠ `walkdir::WalkDir` traversal reported ZERO effects on every idiomatic usage — `deny Fs` passed
  at exit 0 over code that walks the filesystem.** The classify.rs rule keyed the `Fs` charge on a
  typed `IntoIter::next` receiver, but candor-scan's receiver-typing (`ctor_type`/`resolve_recv_type`)
  hard-blocks the `.into_iter()` verb everywhere (a guard against fabricating onto a DIFFERENT std
  type, e.g. `Vec::into_iter()` → `std::vec::IntoIter`, with no per-crate exception for a SAME-crate
  return like `walkdir::IntoIter`) — so no idiomatic chain (`for e in WalkDir::new(p)`,
  `.into_iter().count()`, the crate's own README `.filter_map(|e| e.ok())` form, or an untyped
  `let it = ..into_iter(); it.next()`) ever reached a typed `IntoIter` receiver, and `candor-query
  blindspots` reported "every call resolved." Fixed by charging at `WalkDir::new` (construction),
  mirroring the already-modeled `ignore::WalkBuilder::build`/`glob::glob` — an ordinary `Expr::Call`
  needing no receiver typing at all. The `IntoIter::next`/`DirEntry::metadata` rule is kept (not
  removed as dead code): it still fires for the narrower explicit-type-annotation case a receiver
  blocklist doesn't gate. Audited the other 81 calibrated crates for the same shape (a same-crate
  iterator reached only through the `iter`/`into_iter`/`drain` blocklist) — walkdir was the only one
  keyed on an iteration method rather than construction, confirmed by two independent checks, so the
  blocklist itself was left untouched rather than narrowed blind. Controls: a std `Vec::into_iter()`
  stays exactly as pure as before, `ignore`/`glob` are unmoved, and 11 of 14 real corpus crates with
  no walkdir usage (duct, flate2, git2, mio, rayon, reqwest, rusqlite, sysinfo, tempfile, ureq, which)
  scan byte-identical to the pre-fix binary. Over the 3 corpus crates that DO call `WalkDir` (notify,
  walkdir itself, zip), exactly one new legitimate `Fs` surfaced — `notify`'s own
  `poll::data::WatchData::scan_all_path_data`, a real published crate silently missing `Fs` on a
  `WalkDir::new(root).follow_links(true).max_depth(..)` scan — and zero false positives.

- **⚠ `git2::Repository::clone`/`clone_recurse`/`RepoBuilder::clone` — libgit2's actual network clone,
  and arguably git2's most common entry point — reported ZERO effects and passed `deny Net` at exit 0.**
  A corpus round found it on published 0.33.0. The classifier's own comment named the trap and then fell
  into it: the git2 rule matched remote verbs precisely and deliberately left bare `::clone` unmatched so
  it wouldn't over-charge `Remote::clone` (the derived `Clone`-trait dup of a `Remote` handle, genuinely
  pure) — but that same bare-`::clone` exclusion also swallowed `Repository::clone`, the thing the
  comment explicitly said it did NOT mean to exclude. Fixed with an FQN-exact carve-out (the same
  technique already used for `reqwest::get`/`reqwest::blocking::get`), so it catches the real network
  clone without re-widening `::clone` and undoing the fix it sits beside: `Remote::clone` stays pure,
  every already-charged git2 remote verb (`fetch`/`push`/`download`/`connect`/`connect_auth`/`ls`/
  `upload`) is unmoved, and a tree with no git2 usage scans byte-identical to before.

- **⚠ Second (closing) triage pass over the completeness-gate ratchet: all remaining 103 rows resolved —
  `eval/coverage-gate/open.tsv` is now EMPTY.** 74 real rules added to `covered.tsv` (tempfile 10,
  sqlx_core 9, ureq 8, isahc 8, ignore 8, grep_cli 5, async_nats 4, dotenv 3, dotenvy 3, mongodb 3,
  mysql_async 2, native_tls_crate 2, and one each for reqwest/crossterm/rustls/dialoguer/duct/jiff/
  notify/portable_pty/tokio_postgres), 5 entries added to `REVIEWED_PURE_ENTRIES` (curl's own documented
  `Multi::timeout` exception, `execute::command`/`shell`'s own documented build-not-spawn exception,
  `elasticsearch::Response::content_type` — a pure accessor over an already-received response — and
  `rusqlite::Context::get_connection`, whose non-effect this file already argued for by name but never
  formally closed), and 24 rows removed as generator false positives across FOUR distinct shapes: (1)
  private modules with no re-export (`tempfile::{create,create_named,reopen}`, `mysql::{MyTcpBuilder::
  connect,Stream::connect_tcp,Stream::make_secure}`, `mysql_async::PathOrBuf::read`,
  `sqlx_core::RustlsSocket::*`, `memmap2::file_len`, `dotenv(y)::{Finder::find,find}`,
  `crossterm::tty_fd`, `reqwest::PoolClient::send_request`); (2) `impl Trait for Container<ForeignType>`
  stripping generics down to a bogus type name (`ureq::Arc::connect`, `tungstenite::TcpStream::
  set_nodelay`); (3) a `pub(super)` fn the generator's visibility check should have excluded but a stale
  registry snapshot recorded as bare `pub` (`diesel::StatementUse::step`); (4) NEW — a proc-macro DSL
  fn name self-scan found in the macro's literal input but that never survives into the compiled public
  API (`mongodb::{CreateDataKey,Encrypt}::execute`: the `#[action_impl]` macro consumes an `async fn
  execute` and re-emits it as `IntoFuture::into_future`, so no consumer can ever spell `.execute()` —
  proven by a fixture where that exact call fails to compile). Every new/moved rule backed by a real
  vendored-source read and a compiling consumer fixture (two throwaway fixture crates, ~20 crates,
  pinned to the exact vendored versions the ratchet's rows were generated from); `ignore`'s and
  `mongodb`'s CSFLE rules needed a SECOND fixture-driven correction after the first draft used the
  generator's crate-root-alias guess instead of the real re-export path (`ignore::gitignore::Gitignore::
  new`, not bare `ignore::Gitignore::new`) — the fixture caught it, a source read alone had not.
  Also fixed IN PASSING, found while reading the redis rule for an unrelated reason: `path.contains(
  "::get_connection")` is a substring match, so it also fabricated `Db` on `Client::get_connection_info`
  — a pure accessor over an already-stored field, no round-trip. And: `classify_check` now actually
  consults `REVIEWED_PURE_ENTRIES` (it previously only existed for `coverage_gate.rs`'s regression check
  on `covered.tsv`; a `REVIEWED_PURE_ENTRIES` addition would have kept reappearing in every regenerated
  `open.tsv` otherwise). The cross-crate `Unknown` gap (an effect reached only by crossing into a
  DIFFERENT uncalibrated crate) is unchanged by this pass and remains open — `mongodb`'s `CreateDataKey`/
  `Encrypt` above is a related but DIFFERENT gap: not a crate boundary, an `IntoFuture` desugaring the
  scanner would need to resolve to see the same effect its own macro-body self-scan already found.

## [0.33.0] — 2026-08-26

- **MIGRATION — ⟨0.33⟩ IS NOT ADDITIVE, and the cost is measured, not estimated.** If you gate a
  **STORED** report that a pre-0.33 engine produced — committed to a repo, cached between CI jobs, or
  published by a dependency and gated downstream — expect exit 2. Measured over **32 real third-party
  projects, 67 reports, 402 report×policy pairs, all four engines**, published **0.32.1** binaries as
  the producer against **0.33** HEAD as the consumer: **202 of the 265 pairs that pass today — 76.2% —
  flip to exit 2** with the policy unchanged. It is deterministic rather than statistical: a report
  carrying any `peeked: true` class refuses **202 of 202**, a report carrying none passes **63 of 63**,
  and **26 of the 32 projects** have at least one.

  **THE REMEDY: re-scan with a 0.33 engine under the SAME policy the gate applies** — not merely *a*
  policy, which is the loose reading this rung exists to close. It discharges the cost in full:
  **265 of 265** pairs green again, no residual tax and nothing to suppress. A pipeline that scans and
  gates in ONE run under ONE policy is **unaffected** — producer and consumer are the same run, so
  `P ⊆ P` holds by construction. Nor is legitimate narrowing over-charged: **62 pairs** whose
  producer's deny set genuinely covers the gate's took **0 refusals**, and over the full cross-policy
  sweep of **918 gates**, **529 refuse correctly and none fails open**.

  **The operators this hits are the ones who followed ⟨0.32⟩'s own remedy** — *scan with the policy* —
  because that is exactly what puts a `peeked: true` class into a report. They migrated one rung ago
  and are being asked to migrate again, for a hole that remedy did not close. The wording was the
  defect and the wording is the fix. It fails **CLOSED**.

- **`receipt` (TSV) carried NO completeness reader at all — SOUNDNESS.md R55, closed.** The same defect
  `diff` had (R54), reproduced on the same fixture: over a report declaring an unread `excluded` class,
  every `fns`/`effects`/`unresolved`/`calibrated`/`encountered` line answered as if the report were
  whole, exit 0, no caveat anywhere.

  `f3bedac` left this open because TSV has no envelope to hedge inside and none of three candidates read
  as clearly right in the abstract (a leading `#` comment, an extra column, or stderr alone). The tie is
  broken by this format's ONE real consumer, `candor-run.sh`, read rather than guessed: it parses with
  `while IFS=$'\t' read -r k v; do case "$k" in fns) …; esac; done` over stdout captured with
  `2>/dev/null`. Measured against that parser: an extra COLUMN on an existing row corrupts the row's
  value (the loop's `read -r k v` glues any third field onto the last variable, so `effects` came back
  holding an embedded tab); stderr alone never reaches the loop at all (the exact failure this rung
  exists to close, reintroduced as the fix). A NEW `key<TAB>value` ROW — `incomplete\ttrue`, appended
  only when the report is incomplete — matches neither failure mode: the `case` has no arm for
  `incomplete`, so it falls through untouched, exactly as a bare `#` line would, but a NAMED row is
  self-documenting and matches the format's own convention (extending the consumer's case statement with
  `incomplete) …` is a one-line change; a comment has nothing to switch on). The full explanation — which
  class, which report — cannot safely live in one TSV cell, so it goes to stderr ADDITIONALLY to the
  stdout flag (never the sole channel, which is what made candidate three unsound).

  Byte-identical on an intact report: the five pre-⟨R55⟩ lines are unchanged, in order, and stderr stays
  empty — proven against the pre-fix binary (`cmp`, matching MD5) rather than by a shallow "no caveat
  key" check.

- **⚠ candor-spec conformance PART 70: `whatif`'s ⟨0.33⟩ cross-policy cause never fired.** `whatif` read
  the bare, unarmed completeness manifest and never called `arm_unread`/`arm_unasked_rules` — the same
  union `unverified` and `fix`/`fix-gate` already apply to their own parsed policy. The ⟨0.32⟩ unread-class
  cause still worked (`must_hedge` reads `unread` directly, unarmed), which is why that cell was already
  green; but ⟨0.33⟩'s cross-policy cause is populated ONLY by `arm_unasked_rules`, so a report whose peek
  was bounded by a narrower `deny` set than the policy `whatif` is asked to check answered as an ordinary
  verdict — `ok` PRESENT (`false`) — where SPEC §2 ⟨0.33⟩ requires `incomplete: true` with `ok` OMITTED.
  Now armed on the SAME `ParsedPolicy` `whatif` already loads, only when a policy was actually given (a
  no-policy run has no deny set to compare `scannedUnder` against, mirroring `diff`'s reasoning for
  holding no policy at all). PART 70's rust row: `outOfScope=OK unread-class=OK cross-policy=OK`,
  controls `violating=OK clean=OK` in both polarities.

- **`diff` carried NO completeness reader at all — SOUNDNESS.md R54, closed.** `diff` answered
  `{baseline_version, engine_version, changes: []}` with no caveat on either channel over a report
  declaring an unread `excluded` class, exit 0 unchanged. MEASURED at HEAD before this fix, on a report
  whose `excluded` names one class with `peeked: false`:

  ```text
  diff cur.json base.json --json   {"baseline_version":…,"engine_version":…,"changes":[…]}   exit 0   no caveat
  ```

  `diff` reads TWO locators that fail in OPPOSITE directions: an unread unit in the CURRENT tree can hide
  a real gain from `changes`, while one in the BASELINE tree can make a longstanding effect read as newly
  gained (or a lost one read as never-had). A bare `incomplete: true` cannot say which side was partial,
  so this reuses `gains`' own PREFIXED shape (`incomplete` + `baselineIncomplete`, now factored into
  `completeness::BaselineCompletenessFields`) rather than a fourth spelling. Both channels now carry the
  hedge, disclosed per side (`CURRENT`/`BASELINE`, never combined into one sentence), and the human
  channel withdraws its determined negative ("no effect changes") under a hedge rather than leaving it
  standing beside the note. `diff` is descriptive (no `ok`, no exit-code obligation), so neither the
  gain-ratchet output nor the exit moves. Healthy output is byte-identical to the pre-fix binary on both
  channels, over a real gain and an empty-changes fixture, diffed rather than assumed.

  ⟨0.33⟩'s cross-policy cause cannot fire on this verb — `diff` never parses a `--policy`, so the
  consumer deny set is always empty and vacuously a subset of any `scannedUnder`, verified rather than
  assumed. `receipt` (SOUNDNESS.md R55) was left open at this commit as a TSV surface with no established
  caveat shape to port — see the entry above this one for the shape it landed on.

- **⚠ ⟨0.33⟩ `scannedUnder`: a report now records the deny set its peek was BOUNDED BY, and
  `gate --report` refuses when that set does not cover the policy being applied.** SPEC §2 ⟨0.33⟩,
  ported from the candor-java reference. `excluded[].peeked: true` is true only relative to the
  PRODUCER's deny set — ⟨0.29⟩ bounds the peek to effects that policy DENIES, so a class read under
  `deny Net` says nothing about `Exec` in those same files. Until this rung the report never recorded
  the question, so a consumer gating with a DIFFERENT deny set got a definite `outOfScope: []` answer to
  a question nobody asked — and it survives every ⟨0.32⟩ control, because the class really WAS read.

  PRODUCER (`candor-scan --policy`): the envelope now carries `scannedUnder: { "deny": [ … ] }` under
  exactly `outOfScope`'s emission rule — present iff a policy was configured AND honoured, absent
  otherwise (no policy, or one this engine refused) — holding the deny/`pure` rules in their canonical
  EXPANDED form (`candor_classify::policy::canonical_deny_set`), recorded from the very list the peek
  matched with, before either short-circuit that would otherwise leave a "policy stood and denied
  nothing" run looking like "nothing was asked".

  CONSUMER (`candor-query gate --report`): refuses at exit 2 with `ok: false, incomplete: true` when
  this gate's own expanded deny set is not a SUBSET of a peeked report's `scannedUnder.deny`, naming the
  rules that went unasked and pointing the remedy at THE SAME policy — not merely *a* policy. **A REPORT
  THIS ENGINE WROTE WITHOUT `scannedUnder` NOW FAILS CLOSED wherever a class comes back `peeked: true`**
  — the same non-additive shape ⟨0.32⟩ took, with an exact remedy: re-scan under the gate's own policy.
  `scannedUnder` itself is read STRICTLY: a non-object, or a `deny` that is not an array of strings,
  impeaches the whole document rather than being coerced to "the producer held these rules".

  Both over-charge carve-outs are STRUCTURAL rather than spelled twice: an empty consumer rule set is a
  subset of everything (so a policy with no `deny`/`pure` rule, or a verb carrying no policy at all,
  never fires), and a report with no `peeked: true` class never fires either (analysed code's effect
  sets are policy-independent; only the peek was ever bounded). `unverified --strict` and `fix-gate
  --strict` gained the identical cause through the shared `ReportCompleteness`/`arm_unasked_rules`
  helper, so an advisory verb cannot be less pessimistic than the gate over the same bytes. §3.1 route
  equality holds by construction and needs no new anchor: on `scan --policy P` the producer and consumer
  are one run, so the recorded set IS `P`, `P ⊆ P` holds, and the scan route's own `--gate-json` verdict
  is byte-unchanged. Conformance PART 69.

- **The README/AGENTS spec-claim gate now reads two spellings it was structurally blind to.** Its claim
  grammar was `spec` + one to FOUR of `[-: "]`, which cannot reach a version behind an ALIGNED envelope
  column (`"spec":    "0.32"` — six separators, the padding that had already defeated a hand sweep at
  0.30) nor one behind a markdown link (`[candor-spec](…) 0.32`). Both were live in shipped documents in
  this family while every gate read clean over them. The grammar is now `spec` + one to EIGHT of
  `[-: "*)\]]`, the expected value is still DERIVED from `candor_report::SPEC_VERSION`, and the
  discriminating control carries both new spellings — so a silent narrowing reddens rather than going
  quiet. Measured: with the old grammar, a README carrying `"spec":    "0.31"` and `[candor-spec](…)
  0.30` passed this test.

## [0.32.1] — 2026-08-25

- Build version → 0.32.1 (crates and `Cargo.lock`); no analyzer change.

- **Family build bump — this engine is unchanged.** The floor stays 0.32, and every report byte, verdict
  and exit code is identical to 0.32.0; nothing here is ⚠. What the patch carries is candor-java's, whose
  v0.32.0 NATIVE binaries were never published — `native.yml`'s parity gate failed the build after the
  image reported `0 functions` over a tree the jar found 210 in, which is exactly what that gate is for.
  Reaching the rebuilt binaries means moving `ENGINE_PIN`, and `ENGINE_PIN` is ONE value for the whole
  family: `candor update` uses it for the java tag, for `cargo install --version "$ENGINE_PIN"`, for
  `npx candor-ts@$ENGINE_PIN` and for the swift tag alike. So the four crates are republished at 0.32.1
  to keep that pin resolvable on the cargo route. `candor-report`, `candor-classify`, `candor-scan` and
  `candor-query` are byte-for-byte the 0.32.0 sources with a version number moved.

## [0.32.0] — 2026-08-25

- **`callers`, `impact` and `path` carried NO completeness reader at all — the SILENT half of the same
  class, closed in the three engines that ship them.** ⟨0.28⟩ widened SPEC §2's re-disclosure MUST to
  *"any verb whose output could be read as a NEGATIVE FINDING about the code — a verdict, an empty
  result set, or a zero count"*, enumerated six verbs, and skipped these three; the ⟨0.32⟩ unread-class
  cause then made the gap fire on nearly every no-policy report. MEASURED at HEAD on a three-function
  crate with one `tests/` dir (`excluded: [{class: "non-library-target", peeked: false}]`):

  ```
  callers wrapper --json   {"of":[…],"direct":["top"],"transitive":["top"]}   exit 0   no caveat
  impact  wrapper --json   {"fn":…,"affectedCount":1,"affected":["top"],…}    exit 0   no caveat
  path    top Fs  --json   {"fn":…,"effect":"Fs","path":[…3 steps…]}          exit 0   no caveat
  ```

  — and nothing on the human channel either, while `where` and `reachable` read the same bytes through
  the same module and hedged. A user asks who calls a function, gets an answer, and is never told that
  part of the codebase went unread. An empty `direct` reads as *nothing calls this*, an
  `affectedCount: 0` as *safe to edit*, an empty `path` as *this function does not reach that effect*.

  Where `show`/`map` OVER-hedged (the caveat replaced the data, ruled the other way in the entry below),
  these three UNDER-hedged. **The remedy is the mechanism already in the file, not a fourth spelling:**
  all three documents have a FIXED key set at their root, so nothing nests and no reserved-key collision
  can arise — they take `write_json` on the machine channel and `print_note` on the human one, exactly
  as `reachable` does. Both of `callers`' answering functions and all three of `path`'s emit sites are
  covered, including the two that answer an EMPTY chain and `callers`' `{}` fallback arm, which is the
  strongest determined negative the format has.

  **The boundary does not move.** The trigger is `must_hedge()` — a disclosure — and `incomplete()`
  with every exit computed from it is untouched, so `gate`/`gate --report` and
  `unverified`/`fix-gate`/`whatif`/`fix` under `--strict` still REFUSE over these same bytes (⟨0.24⟩'s
  "never LESS sensitive than the gate"; conformance PARTs 62 and 67). Healthy output is byte-identical,
  measured by diffing all three verbs on both channels against the pre-change binary over a report whose
  excluded class is `peeked: true`.

- **`show` and `map` returned the WARNING INSTEAD OF THE ANSWER — the ⟨0.32⟩ descriptive hedge, ruled
  the other way and closed four-way.** ⟨0.28⟩ Rung A tells a verb *whose pinned shape cannot carry the
  caveat* to emit the CAVEAT DOCUMENT **instead of** its result document, and yesterday's rung armed
  that substitution on the unread-exclusion-class cause. MEASURED on a two-function crate with one
  `tests/` dir (`excluded: [{class: "non-library-target", peeked: false}]`), and reproduced in every
  engine that ships these verbs:

  ```
  show <fn> --json   ->  {"incomplete": true}      exit 0    the rows are GONE
  map       --json   ->  {"incomplete": true}      exit 0    the map is GONE
  ```

  That is approximately every no-policy scan of a crate with `tests/`, `benches/`, `examples/` or a
  `build.rs`, and of a single-file TS scan with unparsed siblings — and through candor-ts's MCP tools
  `candor_show`/`candor_map` handed the same document to an AGENT, on the edit-time channel that cannot
  ask a follow-up question.

  **RETURN THE DATA AND THE WARNING; DO NOT REPLACE THE DATA WITH THE WARNING.** Rung A's wording was
  written when the trigger was a manifest a scan had FAILED to produce — there was little result to
  lose. It is wrong once the trigger is ordinary: `show` and `map` are DESCRIPTIVE, they certify
  nothing, so there is no claim for a pessimism rule to protect and withholding the answer buys no
  soundness. The same codebase already answers this way one verb over — `gains --json` keeps its gained
  set and adds `incomplete: true` beside it — and the descriptive verbs are now consistent with it.

  **THE BOUNDARY IS WHETHER THE VERB ANSWERS `ok`, AND IT IS NOW STATED IN A COMMENT IN ALL FOUR
  ENGINES.** `gate`/`gate --report`, and `unverified`/`fix-gate`/`whatif`/`fix` under `--strict`, answer
  `ok`: they still REFUSE over these same bytes (exit 2, `ok` false on the gate and OMITTED on the
  advisory siblings), which ⟨0.24⟩ requires and conformance PARTs 62 and 67 pin. Getting the boundary
  wrong in that direction re-opens the cardinal sin, so the controls for it were written FIRST and
  confirmed green before anything moved.

  **THE SHAPE: THE RESULT NESTS, THE CAVEAT SITS AT THE ROOT.** `show` hedging is
  `{"functions": [ … ], "incomplete": true, …}`; `map` hedging is `{"modules": { … }, "incomplete":
  true, …}`. Every property Rung A cited for its own shape still holds — healthy output is untouched,
  the root type change stays LOUD (a consumer doing `for (const x of doc)` over `show` still gets a
  TypeError, not a silent zero-iteration loop), and no reserved-key convention is needed. Nesting
  `map`'s USER NAMESPACE one level down *removes* the collision the ruling deferred rather than
  re-opening it: a module literally named `incomplete` is a key of `modules`, the boolean is a key of
  the root, and neither can displace the other. Relative to the shape it replaces the change is purely
  ADDITIVE: a consumer that already handles today's hedge sees one more key.

  **CONTROLS, WRITTEN FIRST AND BOTH DIRECTIONS ASSERTED**, because the safe-looking empty value passes
  a presence check while deleting the feature: the certifying verbs still refuse over an unread class
  (exit 2, `ok` withheld); a report with nothing unread produces a document BYTE-IDENTICAL to the
  pre-change binary's on both verbs, measured by diff; the hedge still appears when it should; and the
  restored answer is asserted by ROW COUNT and NAME, not by key presence.

  GATES: cargo test --workspace (all green, +1 control row, the Rung A row re-aimed);
  cargo clippy --all-targets -D warnings clean. Conformance was NOT run — candor-spec is under
  concurrent edit; PART 40's oracle scores an object carrying a live disclosure key as a hedge and is
  unaffected, PART 5's guard fires on a hedging FIXTURE and is unaffected.

- **`tour` said "nothing hidden" over a class the scan never opened — the ⟨0.32⟩ descriptive hedge,
  ruled and closed four-way.** Over a report whose `excluded` names a class with `peeked: false`,
  `tour` printed *"candor: nothing hidden — every effect sits where its name says it should"* at exit 0
  here, in candor-ts and in candor-swift, while candor-java hedged and named the class. **candor-java
  was right, and the ruling is now stated in a comment in all four engines so it is not re-litigated.**

  ```
  tour --report <report with excluded[].peeked:false>
    rust/ts/swift   candor: nothing hidden — every effect sits where its name says it should.   exit 0
    java            ⚠ INCOMPLETE — … 1 exclusion class(es) the scan did NOT READ …              exit 0
  ```

  **IT IS A DISCLOSURE, NOT A VERDICT.** `tour` answers no `ok` and carries no exit-code obligation, so
  ⟨0.24⟩'s advisory-verb pessimism MUST does not reach it — which is why the arm is on `must_hedge` and
  NOT on `incomplete()`, and why the exit code is unchanged. What reaches it is §2 ⟨0.28⟩, which widens
  the re-disclosure MUST to *"any verb whose output could be read as a negative finding about the code —
  a verdict, an empty result set, or a zero count"*, and §3.1 ⟨0.18⟩, which already forbids **that exact
  sentence** over a ≥⅓-Unknown graph. An unread exclusion class is the same ignorance by another route,
  and the ⅓ threshold structurally cannot see it: an unread unit contributes no entry, so it moves
  neither the numerator nor the denominator.

  **THE ARGUMENT THAT KEPT IT OUT WAS THE WRONG WAY ROUND**, and it was written down in this file as a
  ruling: *"the descriptive verbs carry no policy, so there is no question whose answer could depend on
  the unread code."* The condition ⟨0.32⟩ states is the QUESTION IN FORCE, and a verb with no policy is
  not asking a NARROWER question than `deny Exec` — it is asking the widest one there is, the whole
  effect surface. `arm_unread` still CLEARS the list for a policy that denies nothing, so an
  `allow`/`forbid`/`only`-only run is untouched, and the `--strict` exit codes are untouched.

  Trigger = the gate's, minus the policy condition: `peeked == false`, no `judgedElsewhere`, `count`
  ignored (measured — all four gates refuse over `count: 0` and certify over `judgedElsewhere: true`).
  The remedy answers the noise objection the old ruling raised: scan with the policy, the peek reads the
  class, `peeked` turns true, the hedge goes away. The machine channel raises `incomplete: true` and
  mints **no** new key — `unread` stays the ADVISORY route's wire spelling, which this engine is still
  the only one to publish. Over-charge controls in-tree, both directions: a peeked class and a
  `judgedElsewhere: true` class each get the unhedged answer and a byte-identical document.

- ⚠ **A verdict row could not say which unit it was about (⟨0.32⟩). NOT ADDITIVE — verdict documents
  change shape: violation rows gain a `hash` key, so bytes differ from any pre-⟨0.32⟩ verdict and a
  baseline taken against one must be regenerated.** No arrangement of the existing keys satisfies the
  MUST, so the field is new; everything that could stay additive did (every pre-existing key keeps its
  name, position and value, and `hash` is omitted when the producer has no identity to give). SPEC §2: *"a verdict row MUST carry
  enough identity for a consumer to tell two units apart… and the sort key MUST include that identity."*
  MEASURED on a two-member workspace whose members both define `go()` and both spawn `curl`, under
  `deny Exec` — two BYTE-IDENTICAL rows:

  ```json
  { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" },
  { "rule": "AS-EFF-006", "fn": "go", "effects": ["Exec"], "detail": "`go` performs { Exec } …" }
  ```

  No hash, no package, no loc. A reader cannot tell two broken members from one listed twice, and a
  consumer that fingerprints on name alone — candor's own SARIF action did — hides one finding behind
  the other. Names are not unique even within one report: an inherent method and a trait implementation
  of the same name emit two entries sharing `fn`.

  Every verdict row (AS-EFF-005/006/008/009/011) now carries **`hash`** — §2.2's join key, `package#fn`
  — BESIDE `fn` and never instead of it, because the name is what a policy scope matches and what a
  human reads. `hash` rather than `package` or `loc` because §2.2 already binds a consumer to join a
  verdict row back to its report entry by hash; a row that omits it forces exactly the name join that
  clause forbids. It is also now part of the SORT KEY: `(rule, detail)` ties on twin rows, and §3.3.1
  makes the document's order part of the byte-equality between `scan --policy` and `gate --report`,
  which accumulate in different orders — identity in the row without identity in the key is half a fix.

  MEASURED after, over `~/.cargo/registry` (265 crates × `deny Exec`/`deny Fs` = 530 pairs, each gating
  the report its own policy-carrying scan produced): **530/530 byte-equal between the two routes**, exit
  codes agreeing on every pair, and **1313 of 1313 violation rows carrying identity** (82 pairs emit a
  multi-row verdict).

  **Wire note:** this is a NEW KEY on the violation record, so a verdict document is no longer
  byte-identical to a pre-⟨0.32⟩ one — unavoidable, since the MUST is that the row carry identity. Every
  pre-existing key keeps its name, position and value; the field is omitted when the producer has no
  identity to give (a hand-authored report with no `hash`), because absent is *cannot answer* and never
  a fabricated id.

- **⚠ `fix-gate --strict` and `unverified --strict` certified what the gate had just started refusing
  (⟨0.32⟩).** SPEC §3.2 binds an advisory verb to be LESS certain than the gate over the same bytes and
  never MORE, naming `unverified`, `fix-gate` *"and any later sibling"*. The unread-class rule landed on
  `gate --report` in the entry above and stopped there, so the moment that route began refusing, this
  opened underneath it:

  ```text
    gate --report N --policy P            exit 2   {"ok": false, "incomplete": true}
    fix-gate   --report N --policy P -s   exit 0   {"ok": true, "remedies": []}
    unverified --report N --policy P -s   exit 0   {"ok": true, "unverified": []}
  ```

  `--strict` is how CI consumes both verbs, and the documents beside them are the agent channel — the
  one that cannot ask a follow-up question. MEASURED over `~/.cargo/registry` (265 crates ×
  `deny Exec`/`deny Fs` = 530 pairs, each gating a report its own no-policy scan produced): **318 pairs
  had `gate --report` at exit 2 while `fix-gate --strict` answered 0**, and 78 of those were
  `unverified --strict` printing `{"ok": true, "unverified": []}` at exit 0 over the same bytes. After
  the fix: 0 pairs where the gate refuses and either verb does not.

  `unverified`'s answer had looked right for the wrong reason. Over a report whose functions carry
  `Unknown` it exits 1 on the HOLES it found and reads as a refusal; over the same tree with no hole in
  it, it certified. A non-zero exit reached by a different finding is not this relation being satisfied,
  which is why the pinned row's fixture has every finding set empty.

  The classes the producing scan never opened now ride `ReportCompleteness` — read off the SAME key and
  through the SAME reader `gate --report` uses, with a corrupt `excluded` riding the existing
  fail-closed `unreadable` arm. They are COLLECTED by the loader and ARMED by the verb, once, against
  this run's rule set, so the caveat, the document and the exit read one value. The descriptive verbs
  (`whatif`, `map`, `where`, `blindspots`, `tour`, `containment`) never arm it: they carry no policy, so
  there is no question whose answer could depend on the unread code, and an unread class rides almost
  every report a bare scan writes — a hedge on every run trains its reader to skip it.

  Fixed beside it: the `⚠ INCOMPLETE` prose built its sentence from the three MANIFEST rows alone while
  `incomplete()` had counted the two SCOPE causes since ⟨0.30⟩, so a note whose only cause is unread or
  out-of-scope code read *"declare 0 unit(s) candor could not analyze"* — a hedge that names no cause.
  Both causes now get a clause and a per-item line.

  **Upgrade note:** same as the entry below, one verb over. `fix-gate --strict` and `unverified --strict`
  over a report produced WITHOUT a policy now exit 2 whenever the tree has a build script, tests, benches
  or examples the scan did not read — 376 of the 530 pairs above. The repair is the same one flag:
  produce the report with the policy. A policy carrying no `deny`/`pure` rule is unaffected, and a
  fully-peeked report still certifies on all four routes.

- **⚠ `gate --report` certified a deny rule over code the producing scan never read (fail-open, ⟨0.32⟩).**
  A scan run WITHOUT a policy writes a report whose `outOfScope` key is absent — nothing was asked — but
  whose `excluded[].peeked` is `false` on every class for that same reason. The unread-class rule was
  gated on `outOfScope` being present, so it was skipped in exactly the case it exists for, and the gate
  route answered `policy ✓`, exit 0, `ok: true`, with no disclosure, over classes nobody had opened.

  MEASURED over `~/.cargo/registry` (265 crates × `deny Exec`/`deny Net`/`deny Fs` = 795 pairs):
  **90 pairs went exit 2 on `candor-scan <crate> --policy P` and exit 0 through
  `candor-query gate --report <no-policy report> --policy P`** — including `anyhow`, whose build script
  spawns `rustc`, named by the scan route and invisible to the other. All 90 now refuse on both routes;
  the 265-crate cross-check finds **0 crates refusing with nothing unread and 0 exiting 0 with an unread
  class**, and the 24 crates carrying a real in-scope violation still exit 1 (a violation dominates).

  `peeked: false` genuinely has two causes — "opened it and failed" and "never asked" — and from a report
  they are indistinguishable, because they leave the identical hole. Which one it is does not decide the
  verdict; the QUESTION does: only a `deny`/`pure` rule's answer depends on code outside the scan's scope,
  so the condition is now applied once, to this run's rule set, on both routes. An `allow`-only,
  `forbid`-only or `only`-only policy is unaffected, and a report with no exclusions, with every class
  peeked, or with the class carved out as `judgedElsewhere`, still certifies.

  Fixed beside it, same class of split: candor-scan recorded unread classes into its `--gate-json`
  document from `outOfScope.is_some()` while deciding its EXIT from `peek_attempted`. Those are different
  predicates, and on a policy with no deny rule the document came out `"ok": false, "incomplete": true`
  **at exit 0** — visible only to a machine reading the JSON, which is the only consumer that matters
  there. One predicate now feeds both halves. `excluded` also joins the strictly-read §2 keys: present-
  but-unparseable is a refusal naming the key, never coerced to "this scan excluded nothing".

  **Upgrade note:** gating a report produced without a policy now refuses (exit 2) whenever the tree has
  a build script, tests, benches or examples the scan did not read — 504 of the 795 pairs above. The
  repair is one flag: produce the report with the policy (`candor-scan <dir> --out r --policy P`), and
  the class flips to `peeked: true` with a definite answer either way.

- **⚠ A call through a SUBMODULE-level re-export resolved to nothing.** The oldest shape in Rust's module
  system — `mod imp { mod platform; pub use self::platform::*; }` with callers writing `imp::doit()` —
  reported no effect at all. The intra-crate call graph keys a qualified call on its last two segments, so
  the definition (`platform::doit`) and the call (`imp::doit`) never met, and the crate-root re-export
  machinery covers only the root file. MEASURED silent on every spelling: file-per-module and inline `mod`,
  glob and named, one hop and two.

  Real instance: `tempfile`'s `src/file/imp/mod.rs` is that shape, so `NamedTempFile::new`, `new_in`,
  `with_prefix{,_in}`, `with_suffix{,_in}` and `Builder::tempfile{,_in}` — eight entry points behind every
  temp file the crate makes — did not reach the `Fs` in `file::imp::unix::create_named`. They do now.

  Each module's `pub use` edges are collected and folded into an alias index, consulted ONLY where the
  primary index holds nothing for a call's tail, so no resolution that worked before can be displaced or
  made ambiguous. `#[cfg_attr(unix, path = "unix.rs")] mod platform;` is followed to every branch's file
  (the scanner analyses all of them; `cfg_if` arms are already unioned the same way), and Rust 2018 uniform
  paths (`pub use unix::*;`) resolve only against a `mod` the module actually declares. A private `use`
  exports nothing. A name reaching a module by two independent routes, or a tail that two different modules
  would answer differently, is refused rather than guessed — `dir::imp` and `file::imp` are both `imp` in a
  two-segment tail, and the first cut of this fix charged `tempfile`'s `dir::create` with the file side's
  temp-name `Env`+`Rand`.

  Corpus A/B, 265 crates × {`deny Exec`, `deny Fs`}: zero exit-code flips, zero effects and zero call edges
  LOST, 195 functions gained one (Fs +11, Clock +46, Log +6, Env +4, Unknown +118). Hand-traced to a real
  effect site in ten crates.

- **⚠ ⟨0.32⟩ The invocation object and the option-builder, told apart.** Three measured residuals of the
  std I/O-handle receiver routing, two over-charges and one silent under-report.

  A submodule's OWN type was charged from the FILE's import: with `use std::process::Command;` at the top
  of a file, `mod mine { pub struct Command; pub fn run(c: &Command) { c.spawn(); } }` reported `Exec` for
  a `spawn` that does nothing. Rust gives an inline module its own namespace and no enclosing `use` reaches
  into it, so an inline `mod` no longer inherits a name it declares for itself.

  A pure read-back's RESULT was charged: `c.get_program()` is carved out of `Exec`, but the carve-out only
  survived as far as the next `.` — `c.get_program().to_str()`, `c.get_args().len()` and
  `for .. in c.get_envs()` all walked back to the `Command` and answered `Exec`.

  `OpenOptions` was wrong in BOTH directions at once. `o.open(p)` on a received `&OpenOptions` opened a file
  and reported NOTHING (the same silent false all-clear as the `Command` parameter — `tempfile`'s
  `create_named`, which opens every temp file it makes, certified pure), while a setter-only
  `OpenOptions::new().read(true)` with no `open` anywhere reported `Fs`. SPEC §1 ⟨0.32⟩ names the boundary:
  an invocation object is `Exec` from construction, an option-builder for another effect stays PURE because
  its resource arrives at the terminal verb. `OpenOptions`/`DirBuilder` now route as handles behind
  type-keyed setter carve-outs, so only `open`/`create` charge. Keyed on the TYPE, not the leaf: `create`
  sets a flag on `OpenOptions` and makes a directory on `DirBuilder`.

  `OpenOptions::open` also claims NO read/write direction — the direction was set by the builder chain,
  which the classifier cannot read, and §2 forbids the partial claim. `File::open` still reads.

- **⟨0.32⟩ A refusal now records itself beside the reports it would have written.** Scan a tree green,
  change it so it now violates, then refuse for any reason: `gate --report <tree>` answered `policy ✓` at
  exit 0 off the previous run's bytes, because a run given no `--out` writes to its default prefix and
  §3.3.1's arming rules only cover a prefix the operator NAMED.

  Arming the default prefix is not the fix and this engine has the scar — a run that died in argv parsing
  once replaced a COMMITTED report in this repository. So the refusal is written BESIDE the reports at
  `<prefix>.refused.json` and overwrites nothing; `gate --report` consults it and declines to certify;
  a run that completes its write phase removes it.

  Because it destroys nothing it is written during argv parsing, the earliest moment the prefix is known
  — which is what covers the argv-death case arming cannot reach. The first version latched the prefix
  downstream and the marker was absent on exactly that case.

- **⚠ The report hand-back restored its own placeholder, leaving a permanent exit 2.** Reachable in three
  ordinary steps and MEASURED on a two-member workspace: scan both members; delete one and let any
  refusal happen (an unknown flag will do), which correctly arms the orphan; then scan the remaining
  member successfully. That third run COMPLETES at exit 0 and the orphan still holds the placeholder,
  because the run's own arming had saved the *previous* run's placeholder as the orphan's "previous
  bytes" and the hand-back dutifully put it back. `gate --report <prefix>` then refuses at exit 2 off
  that leftover for ever, until somebody deletes the file by hand.

  That is the exact state the hand-back exists to prevent, reached by running it twice — the same
  failure its own note records, wearing the fix.

  A placeholder is not a previous run's report; it is this machinery's marker saying a run did not
  finish. It is now recorded as "no previous report", and the hand-back REMOVES it rather than restoring
  it. Removing a placeholder is sound where removing a report never is: §3.3.1 forbids deleting a report
  because a consumer reading absence as "nothing to report" fails open, and a placeholder makes no claim
  about code at all — here its absence is also the truth, since the member is gone. Both directions are
  pinned, because the second one is the whole difference: a real orphaned report is still handed back
  byte-for-byte.

- **The docs drift gate could not see the JSON spelling of the spec version, and the ⟨0.32⟩ sweep proved
  it.** `repo_docs_carry_the_family_attribution_and_spec_floor` asserts `README contains "spec 0.32"` — a
  POSITIVE existence check, satisfied by one correct prose mention and structurally blind to a second,
  stale claim in the same file. So the bump rewrote the prose everywhere and left README's `--gate-json`
  example printing `# → { "spec": "0.31", … }`, which is the literal shape a reader copies into a CI
  assertion. It survived the bump, the doc sweep and CI.

  The gate is now UNIVERSAL as well as positive: every `spec <X.Y>` claim in README.md and AGENTS.md —
  prose, hyphenated, or `"spec": "X.Y"` — must equal `SPEC_VERSION`, DERIVED from the constant and never
  a literal. Historical markers are exempted by the family's `(spec X.Y, informative)` form rather than
  by a list of tolerated old versions, and a control asserts the exemption discriminates, so a broken one
  cannot quietly turn the check into a no-op. This is candor-swift's `AgentsDocDriftTests` ported, not a
  third convention — swift was clean here for exactly that reason. AGENTS.md's one attributive `spec-0.7`
  marker moved to the annotated form with it.

## [0.31.0] — 2026-08-20

- **⚠ The unevaluable-target refusal handed a PREVIOUS run's green report back.** With `--out` naming a
  prefix that already held a green report, the refusal armed its fail-closed placeholder correctly — and
  then `scan_target` granted the hand-back licence anyway, so the stale green was restored byte-for-byte
  and `candor-query gate --report` certified it at exit 0. A false green produced by the rung whose
  purpose is turning that green red, and §3.3.1 ⟨0.28⟩ is explicit: a refusal writes the fail-closed
  report to every prefix named.

  The cause is stated in the latch's own comment — "scan_one's report write precedes every return it
  has" — which ⟨0.31⟩ falsified by adding a return above the write phase. The route did not die, so it
  never lost the licence the way an early exit would. A run that refuses before writing now withdraws it
  explicitly.

- **The refusal document named the wrong cause.** `--gate-json` on this path reported "the gate config
  did not load (exit 2)" — affirmatively false, since the config loaded and the TARGET is what could not
  be read, sending the reader to debug the wrong file. ⟨0.24⟩ pins that field as a string naming the
  cause, and this family rates a false disclosure worse than a missing one. It now names the target and
  carries the remedy.

- **One scan run is one thread, and now the compiler says so.** `GATE_VIOLATIONS` is a thread-local
  while nine sibling accumulators are process-globals — the asymmetry is deliberate (`cargo test` runs on
  parallel threads and a process-global violation list let tests contaminate each other), but it made the
  `[workspace]` member loop load-bearing in a way nothing about the loop said. Parallelise it and each
  worker accumulates into its own list, so cross-member violations are silently lost and the symptom is a
  wrong exit code — one member's certain violation vanishing behind another's "could not evaluate", with
  no panic and no failing fixture. Three sequential loops in this family were parallelised for speed in
  one night.

  `scan_one` takes a `RunToken` that is neither `Send` nor `Sync`, so a parallel iterator over the
  members captures it and does not compile, landing the author on the note that explains why. The token
  is required by `record_gate_violations` and `holds_violation` themselves rather than only at an outer
  boundary, so the proof sits at the write and the read — no caller can reach the accumulator from a
  thread that did not carry it in. It is a forcing function rather than a proof: what it converts is a
  silent runtime under-report into a compile error at the line that would cause it.

- **⚠ §3.1 route equality broke on ORDER, not content.** Found by the corpus round on **ripgrep under
  `deny Fs`**: `scan --policy` and `gate --report` both exit 1 and both carry the same 16 `outOfScope`
  findings, byte-identical entry for entry — but `examples::walk::main` sits at the front on one route and
  the back on the other, so the documents are unequal. §3.1 is byte equality, so the order of that list is
  part of the contract, and the two routes cannot be relied on to build it the same way: the scan route
  accumulates across cargo workspace members as it scans them, while `gate --report` reads one report per
  package in the order the locator expands.

  This is the hardest shape a route-equality break can take. Both documents are correct, complete and
  equally readable; nothing is missing and nothing is over-claimed. No assertion about content can see it.

  Sorted in the one verdict writer both routes go through, beside the `violations` sort that was already
  there for exactly this reason. Pinned by a test that renders the same findings in two orders — and that
  also asserts they are still all PRESENT, since collapsing the list to nothing would satisfy
  order-independence while deleting the disclosure.

- **⚠ The ⟨0.30⟩ peek no longer feeds `netPartners`.** MEASURED on a crate whose only mention of the
  declared partner was in `build.rs`: the `--gate-json` verdict said `netPartners:
  [{hosts:["partner.example"]}]` while the report it had just written said `null`. Both halves of that are
  the failure the first net-partner attempt was reverted for — `gate --report` reads the report and can
  only ever answer `null`, so the two routes diverge, and the disclosure claims an ambient config moved a
  classification the gate never made.

  The peek re-enters the scanner with `policy: None`, which discharges the policy-derived accumulators.
  `netPartners` is not one: it comes from the participating hosts plus the discovered config, and the peek
  walks the same target. **Target-derived keys are the ones `policy: None` does not cover** — and the same
  defect hit `analyzed` two rungs earlier, which reported 276 against the report's 129.

  That first fix was a guard at the one call site, and the class came back anyway: `netPartners` was
  written months later by someone with no reason to think about a peek. So the suppression is central now
  — the recursive call runs inside `while_peeking`, and every gate accumulator returns early while it is
  set, making the default safe instead of correct-when-remembered. Nothing is lost, because the peek
  RETURNS a report body that the outer frame reads; that is how `outOfScope` has always worked, and it is
  the architecture this enforces. The peek is a source of data, never a writer of verdict state.

- **Two clippy lints CI's newer stable catches and the pinned nightly does not.** The repo pins
  `nightly-2026-06-14`, so a bare `cargo clippy` runs June's lints while CI also runs July's `+stable`.
  `verify-local.sh` runs both legs now.

- **⟨0.31⟩ `netPartners` is now emitted — the ambient config that moved a verdict is named in it.** Under
  `deny Net[unknown-host]`, a call to `partner.example` exits 1; adding `net-partner partner.example` to an
  ambient `.candor/config` exits 0 with `ok: true`, and nothing named the file, its path, or the host. The
  report envelope carries `netPartners: { config, hosts }` — which config declared partners and which of
  them **participated** — and both `scan --policy` and `gate --report` put the list of those records in the
  verdict. Verified byte-equal across the two routes.

  The verdict writer gains `gate_verdict_json_v31`; the older wrappers pass an empty list, so every verdict
  without ambient partner vocabulary stays byte-identical. `gate --report` **copies** the producer's record
  rather than recomputing it — that route has no target to anchor `net-partner` at, and re-classifying
  through the consumer's own config would make a verdict depend on the reader's working directory, which
  is the constraint that reverted the first attempt at this disclosure. Additive: no declaration, or a
  declaration that never matched, carries the key nowhere.

- **`partner_for` — one matcher for the partner decision, ahead of the ⟨0.31⟩ `netPartners` port.** A pure
  extraction, no behaviour change: `net_dest_class` now calls it rather than repeating the match inline.
  It exists because the reverted 2026-08-17 `net-partner` disclosure re-implemented that match against a
  normaliser that KEEPS the port, so an observed `partner.example:443` never equalled a declared
  `partner.example` and the disclosure was silently empty on every real run while the verdicts it reported
  on had flipped. With one function and two callers, a differently-normalised disclosure is unwritable
  rather than merely discouraged. The rest of the port — accumulating what participated, and recording
  `{config, hosts}` in the envelope for both routes to copy — is open work; see candor/BACKLOG.md.

- **⚠ ⟨0.31⟩ AN UNEVALUABLE TARGET IS A REFUSAL, NOT A CLEAN SCAN — verdict-affecting.** A target that
  exists but holds no `.rs` this engine can read now exits **2** with no report, where it answered
  `policy ✓` at exit 0. That was a permanent green for a typo'd CI path, a module that moved, or an
  unbuilt tree — and this engine already refused a target that does not EXIST for exactly that stated
  reason. candor-ts, candor-swift and candor-java all refused this shape already; ⟨0.31⟩ writes the cause
  into §3.3 (the fourth) and brings this engine into line.

  **Bounded three ways.** *Per-invocation, never per-member*: a workspace with one live member and one
  scaffolded one stays green, and the empty member still publishes its ⟨0.24⟩ count-0 report. *After the
  ⟨0.30⟩ peek*: if an excluded file holds a denied effect, that finding is the answer — reported, with
  `outOfScope`, exit 2 through the scope cause. *Before any envelope*: §3.1's byte-equality binds any
  report a scan produced, so the refusal writes none.

  It supersedes ⟨0.24⟩'s judged-nothing ruling for **the scan route's own target only** — a report handed
  to `gate --report`, or chained as a dependency, stays verdict-preserving.

  Two earlier attempts at this fix were reverted, and the regression guard that caught both is now a
  test: keying on the gate's analyzed accumulator made a NORMAL crate exit 2, and keying on `paths` read
  a HashMap of Fs path literals that SHADOWS the walk's file list 450 lines below it — same identifier,
  two meanings. The count is captured at the walk now, under its own name.

- **The ⟨0.30⟩ gate state's sequential assumption is pinned by a test.** `GATE_VIOLATIONS` is a
  thread-local that accumulates across `scan_one` calls, which is correct only while workspace members are
  scanned sequentially on one thread. That was documented beside the declaration and pinned by nothing.
  If the loop is ever parallelised, each worker gets its own thread-local and cross-member accumulation
  stops **silently — as a wrong exit code**, one member's certain violation lost behind another's "could
  not evaluate". The test reads the loop region and names the remedy (thread the state through
  `scan_one` first). Calibrated: it fails when `par_iter` is injected into that region.

- **CI: the pinned dylint binaries are cached — 150s of a 613s job, measured.** `cargo install
  cargo-dylint` + `dylint-link` ran on every push to rebuild two binaries pinned to an exact version
  that changes a few times a year. The version now lives in ONE place (the job's `env`), because
  spelling it in both the cache key and the install command is a trap: bump the command without the key
  and the cache serves the OLD binary under the new number, silently linting with a version nobody
  chose. A following step refuses to continue if `cargo-dylint --version` disagrees with the pin, so a
  stale or partial cache fails loudly rather than quietly.

## [0.30.0] — 2026-08-19

- **Every CI workflow now declares `timeout-minutes`.** Two hung for 3h45m with no output and were given
  a deadline; the four siblings were not, and `ci.yml` then hung for 54 minutes against an ~11-minute
  normal runtime. Fixing the workflows that failed and not the ones beside them is the habit this repo
  keeps measuring in its own analysis.

- Moved the `GATE_VIOLATIONS` doc comment onto the static itself: making the accumulator `thread_local!`
  left it attached to a MACRO invocation, which rustdoc does not document and clippy rejects under
  `-D warnings`. `cargo test` does not run clippy, so CI was again the first thing that could see it.

- **CI: the realworld-oracle jobs have a deadline.** Both hung for 3h45m with no log output on a commit
  whose only change was deleting a duplicate clippy attribute; their normal runtime is ~5 minutes.
  Neither declared `timeout-minutes`, so GitHub's 6-hour default applied and a stuck runner would have
  blocked the release gate for most of a working day while looking like a slow job. Now 30 minutes.
- Dropped a duplicate `#[allow(clippy::too_many_arguments)]` the ⟨0.30⟩ signature change introduced;
  `cargo test` does not run clippy, so CI's `-D warnings` was the first thing that could see it.

- **Spec floor 0.30.** The declaration this build emits as `candor.spec` moves with the family; see
  candor-spec's changelog for the rung.

### ⚠ ⟨0.30⟩ VERDICT-AFFECTING — a gate that was GREEN can now exit 2

**What changed.** When a policy is configured, candor "peeks": it reads the files the scan itself
excluded (test files, build scripts, archives under the root, files outside the build's program) and
reports any that perform an effect the policy DENIES. Until now that block was disclosure only — ⟨0.29⟩
required the exit code to stay exactly what it would have been without it. **It no longer does.** A
non-empty `outOfScope` now makes the verdict `ok: false`, `incomplete: true`, **exit 2**.

**Why.** The ⟨0.29⟩ rule assumed the peek surfaces UNCERTAINTY, which a gate may reasonably decline to
act on. Measured on published 0.29.1, it does not: it resolves a CONCRETE denied effect and names the
function. Under `deny Net`, `axios` had **37 functions the engine had concluded perform Net** — printed,
per function — and still exited 0 with `policy ✓`. (`axios` ships 5 real `.ts` files, every one a type
test, against 160 `.js` implementation files.) Also measured: `node-fetch` 15, `ky` 9, `execa` 9, `zx` 3,
`ofetch` 1. An engine that concludes a function performs the denied effect, prints that conclusion, and
then certifies the tree is committing the cardinal sin holding its own evidence.

**Exit 2, not exit 1.** These functions are never reported as `violations` and never appear in
`functions`: the gate did not JUDGE them, so claiming a violation would be false in the other direction.
Exit 2 says *I could not see enough of this tree to answer*, reusing the `{ok:false, incomplete:true}`
vocabulary ⟨0.21⟩ already defines. A real violation (exit 1) still dominates.

**What does NOT change.** The block is bounded to effects your policy DENIES, so the trigger is never
"you excluded something" but "you excluded something that does the thing you forbade". Across 27 real
packages this flips 6 and leaves 14 green — every one of those with an empty peek, because the scan read
them in full. **Measured more broadly since:** across 37 real projects and 4 realistic policies
(`deny Net`/`Exec`/`Fs`, `pure`), **16 flip at least one gate**; of 96 gates green under 0.29.1, **31 now
exit 2**. Verified by reading the named code, **29 of those 31 are genuine** — serde's `build.rs` running
`rustc`, clap's completion tests spawning shells, alamofire launching `/usr/bin/leaks`, axios's 160 unread
`.js` files. **Jar and class-directory targets are unaffected**: java flipped 0 of 14, because there is
nothing under such a root to peek. A present-and-empty `outOfScope` stays exit 0. A report produced with NO policy has no
`outOfScope` key at all, and gating it stays exit 0, so pre-⟨0.30⟩ reports are unaffected on contact.

**If this turns your gate red.** Read the `⚠` lines: each names a function, its file, and the effect.
The verdict document carries the same list under `outOfScope` for machine consumers. Then one of:

- **Bring the files into the scan** so the gate judges them properly — `--include-tests` (rust) or
  `--allow-js` (ts). Expect the truth rather than a pass: axios under `--allow-js` exits 1 with 27
  genuine violations. **There is no flag for every class**: nothing brings a rust `build.rs`, a swift
  harness target, or a **ts test file** into scope — candor-ts filters test paths unconditionally and
  has no `--include-tests` — so those need one of the options below.
- **Scope the rule** so it does not reach the excluded code (`deny Exec src`) — measured working on rust
  and swift for exactly the build-script and test-target cases above.
- **Address the effect**, which is the point of the gate.

**There is no opt-out.** No flag, environment variable or config key restores the ⟨0.29⟩ behaviour. The
rung is a decision about what a green gate means, and a halfway setting would be a second meaning. If you
are not ready, the escape is the engine pin — do not upgrade yet.

**A known over-charge, stated rather than discovered.** A write to a stream or file descriptor reached
through a helper can be charged `Net`, because `tty.WriteStream` extends `net.Socket`. It accounts for 2
of the 31 measured flips (execa under `deny Net`). It predates this rung — ⟨0.30⟩ only makes it
verdict-bearing — and it is a classifier fix with its own risk, so it is being made separately rather
than folded into a release.

**The finding states what candor CONCLUDED, not what is true.** The reason string reads *"candor's
analysis reaches this effect"*, not *"the effect is real"*. For 29 of the 31 measured flips the stronger
wording would have been accurate; for the 2 above it would not, and this family rates a FALSE disclosure
worse than a missing one. A finding that asserts ground truth makes a claim the analysis cannot support.
No verdict changes, and the `did NOT judge` phrase PART 48 pins is untouched.

### Fixed — found by an adversarial review of the rung above, before release

- **A corrupt `outOfScope` key failed OPEN.** A present-but-malformed key was coerced to nothing, so
  `gate --report` answered exit 0 / `ok: true` over a report whose peek had resolved a denied effect —
  the exact fail-open coercion the strict read exists to prevent. It now refuses (exit 2), naming the key.
- **The peek used the GATE'S OWN matcher.** It had flattened the policy into a set of effect NAMES,
  which discards `Net[known-partner]`-style destination classes and rule scopes, and reads `pure` — a
  deny rule with an empty effect list meaning *every effect except Unknown* — as denying NOTHING. So the
  strictest policy silently disarmed the rung (exit 0 where `deny Exec` exits 2 on the same tree), while
  a class-filtered rule fired on hosts it does not deny. Both directions are gone.
- **`unverified --strict` and `fix-gate --strict` follow the gate.** They answered clean at exit 0 over a
  report `gate --report` refuses at 2, which breaks the standing rule that an advisory verb must never be
  less sensitive to incompleteness than the gate over the same bytes.
- **The finding text no longer contradicts the verdict** — it said "the verdict does not account for it"
  directly above the line saying the verdict is incomplete because of it.

- **§3.1 byte-equality held only when the scan was otherwise clean.** The peek's findings were recorded
  after the parse-failure early return, so a scan with BOTH an unparseable file and a peek finding wrote
  a `--gate-json` verdict missing `outOfScope` while `gate --report` over the same report emitted it. The
  recorder is now armed before any exit can return.

## [0.29.1] — 2026-08-18

- **⚠ A workspace root that is ALSO a member was scanned TWICE.** `members = ["sub", "."]` is legal and
  real — bollard v0.16.1 ships it — and `workspace_members` dedupes STRINGS, so `.` survives as
  `<root>/.`: a different string, the same directory as the root pushed beside it. Two symptoms, both
  over-claims: `record_gate_analyzed` fired twice, so the `--gate-json` verdict said `analyzed.count
  856` where its own three reports summed to **592** (breaking SPEC §3.1 — `gate --report` only ever
  sees the reports, so the routes could not agree); and `--json` emitted the same package twice in its
  array. The report FILES were unharmed, the second write being identical, which is why nothing else
  noticed. Deduped by CANONICAL path. Found by the corpus round's new §3.1 oracle over THIRD-PARTY
  trees — the in-repo gate-equivalence fixtures cannot reach it, because candor's own workspace does
  not list its root as a member.

## [0.29.0] — 2026-08-17

- **⚠ ⟨0.29⟩ CI FIX — the peek's nested scan was counted into the verdict it may not change.** The peek
  re-enters `scan_one` over the EXCLUDED files to answer `outOfScope`; `record_gate_analyzed` accumulates
  (`+= count`) into a process-global, so the peek's units landed in the `--gate-json` verdict — while the
  peek writes no report, so `gate --report` could never reach the same number. MEASURED on
  `crates/candor-query`: the scan route said `analyzed.count 276`, the report it had just written said
  129, and `ci/gate-equivalence.sh` failed **20 of 54** §3.1 byte-equality rows. The scan route was wrong
  twice over: `analyzed.count` is IN the verdict document, so inflating it IS the verdict change the peek
  promises not to make; and the count is the ⟨0.21⟩ completeness manifest, so it told a consumer 276 units
  were judged when 129 were — the over-claim direction. `unanalyzed` is suppressed on the same branch and
  is the sharper half: an excluded file that failed to parse would otherwise have pushed the run to
  `incomplete: true` / exit 2, the peek turning a deliberate exclusion into a failed gate. Pinned in-tree
  (`the_peek_does_not_inflate_the_gate_verdicts_analyzed_count`) as well as in CI, with the control that
  `analyzed.count` stays non-zero — an equality satisfied by counting nothing is the manifest deleted
  rather than corrected. Self-review of the fix moved its guard onto `peeking`, the local every
  other peek branch already reads — two spellings of one condition being the shape that goes green quietly. **candor-ts, candor-swift and candor-java were probed and are clean**: java's
  peek runs on its own thread-local context by construction.

- **⟨0.29⟩ REVIEW FIX — the positional-literal rung lost the locator for two Net verbs.** Replacing
  `first_str_lit` ("the first literal ANYWHERE") with `positional_str_lit(args, 0)` as the UNIVERSAL
  default was right for `Fs`/`Db`/`Exec`, whose locator is argument 0 — but `is_net_establishing` already
  listed two verbs whose locator is not: `reqwest::Client::request(Method, url)` and
  `UdpSocket::send_to(buf, addr)`. **MEASURED FOR `request` ONLY, and the first version of this entry
  overclaimed by listing both** — `send_to` is a METHOD on a receiver this stable backend does not type
  (`s.send_to(…)` is not classified at all here, the disclosed floor gap), so its arg-1 literal is still
  not captured and the fix reaches it only in the deep engine. `send_to` stays in the table because the
  position is right and the deep engine can use it; the claim about what was measured is now accurate.
  MEASURED: `c.request(Method::GET, "https://api.example.com/v1")`
  published NO `hosts` and could not be certified, while `c.get(<same url>)` certified normally. The
  direction was SAFE (an uncaptured locator fails closed), which is why only a review found it: the gate
  stayed sound and quietly stopped being USABLE. `is_net_host_arg1` now names those two verbs, gated on
  the VERB — never a blanket "try the next argument", which would be the literal-anywhere hazard again.
  candor-ts made its resolver verb-aware in the same rung; this is that discipline arriving one engine late.
- **⟨0.29⟩ REVIEW FIX — the bind hedge was an over-charge built on an unmeasured assumption.** The first
  fix withheld the bind literal from `hosts` AND marked the surface `incomplete` unconditionally, assuming
  an EMPTY host surface would otherwise certify. **Measured, that assumption is false**: a `Net` function
  with no captured host fails closed on its own ("performs Net with no visible literal", four-way). The
  hedge bought no soundness and cost real precision — an ordinary client (`bind("0.0.0.0:0")` beside a
  visible `connect("api.example.com:443")`) could not be certified by `allow Net api.example.com`. Now the
  literal is simply withheld: the original false all-clear stays closed (`bind` + `send_to(runtime)` →
  empty surface → exit 1), a pure listener still fails closed, and **a client whose destination is visible
  to this backend certifies** — measured on the `bind` + `connect(<literal>)` shape.

  **WHAT THIS DOES NOT FIX, corrected after a release panel measured it: a UDP send still cannot be
  certified under `allow Net` on the stable backend.** An earlier draft of this entry said "that is every
  UDP client", which is false. `s.send_to(buf, "203.0.113.9:53")` is a METHOD on a receiver this syntactic
  backend does not type, so the call is not classified `Net` at all and its literal destination is never
  captured — the restored argument-1 rule cannot reach it. The run fails closed and prints the advisory-floor
  banner, and `cargo candor` (the sound gate) charges it; but an operator who adds that address to their
  allowlist will not turn the build green, so the honest statement is that UDP sends are outside what this
  backend can certify today. Tracked as the receiver-typing floor gap in BACKLOG.md.
- **⟨0.29⟩ `forbid`/`only` stop at the SCAN BOUNDARY, and now say so.** Both are matched over the call
  graph; a chained dependency contributes EFFECTS, not EDGES, so a function calling into a dep has an
  empty adjacency and the crossing is invisible to them. MEASURED with a dep chained:
  `only model -> util` answered `policy ✓` over a call into the dependency while a LOCAL unpermitted scope
  in the same run fired AS-EFF-011 — the rule was armed; the boundary was the gap. **Worse for `only`**,
  which asserts A reaches the listed scopes AND NOTHING ELSE — a completeness claim — and exists precisely
  because `forbid` fails open: a package that calls a third-party library is not a leaf, and the gate
  called it one. Disclosed on the advisory channel beside the verdict, the ⟨0.29⟩ `outOfScope` posture:
  say what was not judged, leave the exit code alone. Making the rules cross would need dep-report EDGES
  and would force operators to enumerate third-party scopes in an `only` list — the enumeration-that-rots
  that form was designed to escape. Silent when no dep is chained, and when the policy carries no name rule.
- **⟨0.29⟩ a malformed `net-partner` line was kept as a junk host instead of being disclosed.** The
  grammar is `net-partner <host>`; the `=` spelling an operator reaches for by habit
  (`net-partner = partner.example`) parsed as the HOST `"= partner.example"`, entered the partner set, and
  matched nothing for the rest of the run. **The direction is SAFE** — the gate stays armed, so nothing is
  certified that should not be — which is exactly why it sat unnoticed in ALL FOUR engines: the operator
  believes a partner is declared, the verdict disagrees, and no line connects the two. ⟨0.28⟩ gave POLICY
  files an `ignored` block for this shape; config files had no equivalent anywhere. Now warns and skips,
  which is the contract candor-java's own config doc already claimed for *"every other malformed line in
  this file"*. A well-formed line stays silent.
- **⟨0.29⟩ ⚠ a LOCAL BIND literal certified a REMOTE destination.** `UdpSocket::bind("0.0.0.0:0")` put
  `0.0.0.0:0` into `hosts` — the destination surface `allow Net` gates on — and, because a literal had
  been captured, nothing marked the surface incomplete. MEASURED: `bind("0.0.0.0:0")` followed by
  `send_to(b"secrets", dst)` with a runtime `dst` answered `policy ✓` at exit 0 under `allow Net 0.0.0.0`.
  A local listen address certifying a send nobody can see — the masking evasion AS-EFF-008 exists to
  close, through a verb whose literal is not a destination at all. `is_net_binding` now marks the surface
  incomplete for `bind`/`listen`/`accept`, whether or not a literal was captured, because a server that
  binds and accepts talks to whoever connects: its destination set is not statically knowable even in
  principle. The address is still PUBLISHED (an operator can see what the service listens on), matching
  candor-java, which already published and hedged. A real destination literal still certifies.
- **⟨0.29⟩ `gate --report`'s `allow` refusal stated a premise this rung made false.** The message said the
  AS-EFF-008 surface-completeness marker *"does not ride the report wire"*. It rides now: `incomplete` is
  published per function and declared in `resolves`. MEASURED — reports carry
  `resolves: ["fs","incomplete"]` and `incomplete: ["Fs"]` on the masked unit, and the verb still refuses.
  **Refusing remains correct** (a producer that does not declare the marker cannot be gated on, and
  answering per-report would make one engine evaluate where its siblings refuse, splitting the verb), so
  only the stated reason changes and exit 2 is unchanged. SPEC §6.2 had already corrected this same
  wording for itself in ⟨0.24⟩ — *"This clause first said the marker 'does not ride the wire', flatly.
  That is FALSE…"* — while the engines kept shipping the flat version. A user-facing message describing a
  limitation the code has since removed is worse than a stale code comment: operators read this one.
- **⟨0.29⟩ ⚠ a two-path `Fs` op with a literal in BOTH positions published only the first — and called
  the surface COMPLETE.** `std::fs::copy("/tmp/lit", "/tmp/dst")` under `allow Fs /tmp/lit` answered
  `policy ✓` at exit 0 while writing `/tmp/dst`. A false all-clear assembled from two correct-looking
  halves: the right completeness verdict attached to half a surface. candor-java and candor-swift
  published both all along. `Call.path_lit2` now carries the second position and `paths` gets both.
  Found by GENERATING a case per `std::fs` path-taking leaf × each literal position (52 cases) rather
  than by re-reading the fix — the previous entry's fix was written and reviewed without noticing this.
  Conformance PART 51 gained the `twoLit` row, calibrated by reverting.
- **⟨0.29⟩ ⚠ an `Fs` path literal came from ANYWHERE in the call, not the path POSITION.** MEASURED:
  ``std::fs::write(user_path, "/tmp/lit")`` published `paths: ["/tmp/lit"]` — the BYTES BEING WRITTEN — so `allow Fs /tmp/lit`
  answered `policy ✓` at exit 0 over a write to a runtime-controlled destination, where candor-java and
  candor-swift fail closed on identical code. The operator's own allow-rule is the mechanism of the false
  all-clear, which is worse than having no rule: the report names a path nothing was written to. The
  discipline already existed in this family twice — candor-ts states it at the `Exec` head and its own comment says the rule was "generalized from Exec to Net" — and stopped there. Reading
  the locator POSITION now, and a multi-path operation (`copy`, `rename`, `link`) is a complete surface
  only when EVERY position is a literal; one literal beside one runtime path marks `incomplete: ["Fs"]`.
  SPEC §2, conformance PART 51, whose `okLit` control asserts a fully-literal write still certifies —
  giving up the surface would pass every other assertion and answer nothing. Found by the pre-release panel.
- **⟨0.29⟩ ⚠ `excluded[].peeked` claimed a read the peek had not finished.** The rung already made
  the flag an OUTCOME rather than a lookup on the exclusion class, and stopped one level short. The peek
  reuses this engine's own entry point, so it produces its own ⟨0.21⟩ `unanalyzed` manifest — and
  the parent read only `functions` and discarded it. An excluded file that FAILED TO PARSE inside the peek therefore published
  `peeked: true` beside `outOfScope: []`, byte-identical to a clean peek, on ALL FOUR engines. The
  ⟨0.26⟩ partial-manifest rule failing inside the rung built to enforce it, in the same field, twice.
  The claim is withdrawn PER CLASS — a parse failure is a fact about one file — and an unread file that
  cannot be attributed to a class withdraws the claim for all of them. SPEC §2, conformance PART 52,
  calibrated in both directions: reverting the fix fails shape A, and publishing the SAFE value
  unconditionally fails the control that notices the feature has been deleted.
- **⟨0.29⟩ ⚠ `outOfScope` was published over a policy this engine REFUSES.** SPEC §2 already said
  the key must be ABSENT there — "the peek is a producer reading the policy, and it may not certify
  relative to a gate that evaluated nothing" — and the clause shipped WITH the rung while only
  candor-java implemented it. The harm is the key, not the finding: `outOfScope: []` beside an exit 2
  reads *a policy was configured, I looked at what it denies, and there is nothing*, when the look was
  taken against rules no route would honour and the denied set searched was the parser's SALVAGE of an
  unhonourable file — the silent rewriting the refusal exists to prevent, one layer down. Conformance
  PART 53, whose two controls stop an engine passing by never emitting the key (which deletes the peek)
  and by collapsing present-and-empty into absent (⟨0.27⟩ asked-and-clear vs ⟨0.26⟩ cannot-answer).
- **⟨0.29⟩ ⚠ `only` violations carry their OWN code, `AS-EFF-011`** — not `forbid`'s `AS-EFF-009`. A rule
  code is the handle a CI suppression, a dashboard link and an alert filter key on, and the two forms are
  opposite constructs: must-not-reach versus must-be-on-the-list, with opposite remedies. **The decisive
  argument is timing.** Before this rung an `AS-EFF-009` suppression meant exactly *"I have accepted a
  `forbid` crossing"*; shipping `only` under it would make every existing suppression silently begin
  muting a class of violation its author never accepted — a fail-open change to an operator's config, made
  by us and invisible to them, which is the argument `only` itself is built on turned on the tool. Free to
  fix before release, breaking after it. Pinned by PART 49, which asserts both halves: 011 present AND 009
  absent, since a row checking only the first would pass on an engine emitting both.
- **⚠ ⟨0.29⟩ `only`'s PERMITTED scopes were matched with the fail-OPEN matcher.** `scope_matches` makes
  the last segment a PREFIX of its name-segment, so `util` matches `utilities`. For `deny`/`pure`/`forbid`
  that widening is FAIL-CLOSED — a scope matching more forbids more. For the `to` list of an `only` rule it
  is the exact inverse. MEASURED: `only model -> util` let `model::go` reach `utilities_untrusted::exfil`
  at `policy ✓`, while `forbid model -> util` charged AS-EFF-009 on the identical reach — the matcher that
  keeps every other rule kind safe silently widening the one form whose entire purpose is to fail safe.
  Permitted scopes now match by EXACT segment run (`scope_matches_permitted`); the `from` side keeps the
  prefix rule, since matching more there CONSTRAINS more. Found by review, four-way.
- **⚠ ⟨0.29⟩ `excluded[].peeked` was a property of the CLASS, not of whether a peek ran.** It was a
  constant, so a scan with no policy — or one whose peek produced nothing readable — published
  `peeked: true` beside an absent or empty `outOfScope`, which reads as "I looked and found nothing" about
  files nobody opened. That is the ⟨0.26⟩ partial-manifest failure inside the rung built to prevent it: the
  flag exists precisely so `[]` cannot overclaim, and a lookup table cannot do that job. It is an OUTCOME
  now. Four-way; found by review.
- **⟨0.29⟩ `parsepolicy` publishes `only`.** The §6.2 grammar WITNESS omitted the new rule kind while
  candor-java emitted it, so the verb whose whole purpose is letting a consumer diff what an engine made of
  a policy could not show the difference. Conformance PART 4's battery gained `only` lines and its
  comparison a fourth key — it read three and stopped, which is why it stayed green over the divergence.
- **⟨0.29⟩ `resolves` now declares `incomplete`** (SPEC §2.1). An absent `incomplete` is overloaded
  between "this producer does not compute undetermined locators" and "it computed them and found none" —
  exactly the ambiguity `resolves` was built for, one field over from the `fs` case that motivated it. A
  producer that computes the surface declares it; one that does not MUST NOT, since listing it would turn
  "unimplemented" into a false "nothing undetermined". Pinned by conformance PART 50, which checks the
  declaration BEFORE reading any absence as meaningful.
- **⟨0.29⟩ `only <A> -> <B> [<C> …]` — the PERMISSION form (SPEC §6.2, AS-EFF-009).** A function in scope
  `A` may reach `A` itself and the listed scopes, and nothing else. **`forbid` FAILS OPEN — a dependency
  you forgot to prohibit is silently permitted — so "this package is a leaf" could only be spelled as an
  enumeration of what it must not reach, a list that does not cover a package added tomorrow and says
  nothing about it.** That is the allowlist hazard this project refuses everywhere in the analysis, living
  in the policy language instead. `only` fails SAFE: the dependency you forgot to permit is a loud
  violation on the day it appears. Found by pointing candor's own architecture gate at candor, where the
  natural `forbid io.poly.candor.model -> io.poly.candor` self-fires at 58 violations.
  - **The walk STOPS at a permitted scope** — a permitted callee's own dependencies are governed by the
    rules about IT. Descending past it would demand the transitive closure of everything you permit, which
    is the same enumeration-that-rots one level down. `A -> A` is implicit; `from` IS descended through.
  - **Zero-match is measured on `from`, not on either endpoint the way `forbid` counts.** A `forbid`'s
    subject is the pair; an `only`'s subject is the scope it makes a promise about, so a rule whose
    destinations all resolve while its `from` names nothing has bound nothing — and that is precisely the
    typo that leaves an operator believing a leaf is protected.
  - **Refused on report routes**, exit 2, like `forbid` and for a stricter reason: `forbid` asks whether
    one named crossing is present, `only` asks whether EVERY reached scope is on a list, so a report that
    omits a crossing turns a green into a claim of COMPLETENESS. All three report verbs refuse through the
    one shared helper.
  - **The NIGHTLY engine refuses it (exit 2) rather than running green.** Its layering pass accumulates
    only the scopes some rule NAMES, so it can answer "does A reach B" and cannot answer "is everything A
    reaches on this list". Silently ignoring the rule would be a green gate over an unenforced permission —
    the exact fail-open shape the form exists to remove.
- **⟨0.29⟩ `peeked` on every `excluded` entry.** An empty `outOfScope` says "I read the excluded files
  and none held an effect this policy denies", and it may make that claim only about the classes it
  actually read. This engine answers `true` throughout — its peek is one walk with the selection INVERTED,
  so the two file sets are exact complements by construction — but the flag is not decoration and it is
  not a constant across the family: **candor-java cannot read a `.java` that was never compiled** (it
  reads bytecode), and **candor-swift does not read `.build/`**. Without the field their `[]` would
  certify files nobody opened, which is the ⟨0.26⟩ partial-manifest failure exactly — a partial answer
  being worse than an absent one. Added while porting this rung to those two arms.
- **⟨0.29⟩ the report now declares WHAT THE SCAN CHOSE NOT TO OPEN** — the missing denominator.
  `analyzed.count` is a numerator, and the file-selection decisions that produced it appeared nowhere, so
  a consumer could not tell whether the answer was to the question they asked. The new `excluded` block
  carries one entry per class (`build-script`, `non-library-target`, `test-module`, `build-output`) with a
  count and the engine's own reason.
  **Every one of these exclusions is deliberate and was already documented in a code comment — which is
  exactly why none of them was measured.** `deny Exec` over a crate whose `build.rs` runs `curl | sh` is
  green today, on a file that runs on every `cargo build` whether or not anyone calls the library. The
  exclusion is right for "what does this library do when I call it" and wrong for "what does building this
  crate do to my machine"; the operator chose neither, and the choice was not in the artifact.
  The block is recorded AT THE POINT EACH EXCLUSION IS DECIDED, not derived afterwards — a second walk
  could disagree with the first. It is emitted even when EMPTY (⟨0.27⟩: zero-match is a positive
  statement; ⟨0.26⟩: an absent key must mean "cannot answer"), and by BOTH report writers, for the reason
  the neighbouring `resolves` comment already gives.
  Two rows, one of them the control that an exclusion-free crate still emits the empty list. The reason
  STRING is asserted, not the key's presence.
- **⟨0.29⟩ …and the PEEK, which is the half that makes it actionable.** candor now READS the files it
  excluded and says so when they hold an effect the policy DENIES:

  ```
  candor-scan: ⚠ build::main performs Exec — OUTSIDE this scan's scope (build-script),
                 so the gate did NOT judge it.
               build.rs
               The verdict below does not account for it. A build script runs on every `cargo build`.
  candor-scan: policy ✓
  exit 0
  ```

  **The verdict does not move** — exit code unchanged, `violations` untouched, the function absent from
  `functions`. A file the gate declined to judge must not decide an exit code; the finding rides the
  report as `outOfScope`, its own kind.

  It is a RECURSIVE `scan_one` with the file selection inverted, not a hand-written second pass, and that
  is the design constraint rather than a convenience: a bespoke walk over `build.rs` would be a SECOND
  OPINION, and a drifted second opinion reported as a warning is worse than no warning — the reader
  cannot tell a real finding from two code paths disagreeing. Reusing the entry point makes "same
  classifier, different file set" true by construction. One flag, one walk, so the two file sets are
  exact complements and no file can fall between them; a peek never peeks again.

  **Three bounds, each a way this would otherwise become noise, and each pinned:** `deny Exec` finds it;
  `deny Net` over the same tree says nothing; no policy says nothing and OMITS the key rather than
  emitting an empty list, because nothing was asked and `[]` would be a claim.

  The bound test needed a second fixture. The first execs `curl http://…`, which the classifier reads as
  Net as well as Exec — so the `deny Net` row reported two findings and looked like a broken bound when
  the bound was correct and the fixture could not test it. An argument-free `ls` isolates Exec.

- **⟨0.29⟩ `unverified` and `fix-gate` certified over a policy the gate had refused.** SPEC §3.1's
  answerability MUST binds every verb reading a §2 report, and the `forbid`/`allow` refusal lived INLINE
  in `gate --report`. `unanswerable_pairs` walks `deny` rules only, so a `forbid`-only policy left the
  refusal set empty and these verbs printed *"no deny/pure boundary crossings in this report ✓"* and
  *"every function in a pure/deny layer is PROVABLY clean ✓"* at exit 0 — a green relative to a gate that
  evaluated nothing. Measured four-way: candor-java disclosed and withheld `ok`; rust, ts and swift did
  not. Extracted as `gate::whole_policy_refusals` and shared, so the fourth caller inherits it.
  `--strict` now reaches the could-not-evaluate 2 on all four engines. Pinned by conformance PART 47.
- `unverified` counts RULES and FUNCTIONS separately. The whole-policy kinds are unanswerable over the
  report rather than at a function, so they carry no function name — and the old renderer printed a bare
  `` `` `` and folded them into a count of "function(s)". A refusal rendered as an empty identifier reads
  as a bug in the tool, and the reader stops believing the block.

## [0.28.2] — 2026-08-15

_A cardinal-sin fix. 0.28.1's body-less-declaration pass reopened, in two shapes, the hole it was
written to close — both found by a max-effort review of that patch, both live on npm and crates.io
until this release. The spec floor is unchanged at 0.28._

- **⚠ The self-gate printed OK over a crate it had not judged — a false all-clear, and the FOURTH
  engine, missed when the other three's fix was described as covering "all three".** It compared the
  report's `functions` against the denylist and read neither `unanalyzed` nor `analyzed.count`. Over a
  crate whose sources fail to parse that asks "did anything we ANALYSED reach a denied effect", gets
  "no", and reports clean — while the engine had disclosed the gap plainly (`analyzed.count: 0`, a
  populated `unanalyzed`). The ⟨0.21⟩ completeness manifest is now checked FIRST and exits 2 naming the
  files; falsified both ways. Note the review's stated cause — "never captures the scan exit code" — was
  wrong: `candor-scan` exits 0 on unparseable source, so the exit code was never the signal. The exit
  code is captured too, but the vacuous check was the defect.

- **`ls | grep` in the report lookup replaced with a glob loop** — an unmatched glob stays literal, so
  the `-e` guard is what distinguishes "no report" from "a file named callgraph".

- **Version-aligned only, no functional change.** The cardinal-sin fix this release carries is in
  candor-ts; `release-preflight` [4] requires every engine's build version to agree, so this arm
  moves with the family. The spec floor is unchanged at 0.28.

## [0.28.1] — 2026-08-15

_Post-release review fixes. 0.28.0 shipped, then a high-effort review of that work found
defects in it — three of them a defect of the same class as the fix that introduced them. The
spec floor is UNCHANGED at 0.28: no contract moved, so this is a build-version patch._

- **Review follow-ups on the refusal-document derivation.** It read the version from **candor-scan**,
  but the verdict that replaces that document at the same sink is written by **candor-query** — two
  independent newest-by-mtime binary searches, so the armed refusal and the real verdict could declare
  different spec versions. Same misdeclaration class, one step sideways. Now derived from candor-query.
  The engine call is also CACHED before first use: `refusal_doc … > "$sink"` truncates the sink and then
  runs the body, so forking inside it widened the 0-byte window from a shell builtin to a process exec —
  in the tool whose ⟨0.28⟩ rung is "a sink is armed before every exit". And `ci/wrapper-smoke.sh` now
  asserts BOTH branches (the derived value, and the omitted key when no engine is resolvable); the key
  was asserted by nothing, and the same change had stripped `spec` from these fixtures, so a fresh clone
  with no engine built would have written every refusal without it and stayed green.

## [0.28.0] — 2026-08-14

- **`cargo-candor` no longer hardcodes the spec version into its refusal document.** `refusal_doc()`
  wrote a literal floor, so a bump left the wrapper stamping the OLD contract onto every refusal it
  writes while the engines declared the new one. It now derives from the installed engine, and OMITS the
  key when the engine cannot be asked — `refusal_doc` runs on GUARD-UNAVAILABLE, so it must not depend on
  running one, and §2.1 reads an absent `spec` as predating the field rather than asserting a version.
  Found by release-preflight, not by a test.
- **`ci/wrapper-smoke.sh` placeholder verdicts drop their `spec` key** — those fixtures assert that a
  stale document gets REPLACED, so the version was scenery that had to be bumped every floor.

- **Two verdict-spec assertions now DERIVE the floor** from `candor_report::SPEC_VERSION` instead of
  comparing one literal to another. They could only ever fail on a floor bump — the moment they are least
  informative — and they put this crate on the edit list for every rung. The `candor-report` canary stays
  a literal on purpose: its job is to notice the constant changed, so deriving it would make it vacuous.

- **⟨0.28⟩ the third row is not the first row: `noManifest`** (SPEC §2, *"AND THE THIRD ROW IS NOT THE
  FIRST ROW — measured, two engines report it as one"*). §2's ⟨0.24⟩ table has THREE rows, and this
  engine filed the third under the first's name. MEASURED on the release build over
  `{"candor":…,"functions":[]}` with **no `analyzed` key at all** (a pre-⟨0.21⟩ producer): `where`,
  `blindspots`, `map`, `reachable`, `unverified`, `fix-gate` and `gains` all emitted
  `judgedNothing: ["<path>"]`, and the prose said the report *"say[s] they JUDGED NOTHING
  (`analyzed.count: 0`)"*. **The report declares nothing.** The hedge was the right DIRECTION — row 3's
  own instruction is *no manifest, no claim* — but the disclosure was FALSE, and this family rates a
  false disclosure worse than a missing one (§3.4's `net-partner` finding: an engine reported "ignoring
  unknown config key" *while honouring it*). It was also a hole in ⟨0.28⟩'s own pin, which defines
  `judgedNothing` as *reports declaring `analyzed.count: 0`*: a row-3 report is not one, so the key
  meant two things and lost the distinction the table exists to draw. The REPAIRS differ — row 1 wants a
  scan that reaches a conclusion, row 3 wants a producer that emits a manifest at all.

  Row 3 now carries its own SPEC-pinned key, `noManifest: ["<report path>", …]`, on every document the
  caveat rides (`CompletenessFields`, the Rung A caveat document for `show`/`map`, the advisory verbs,
  and `gains` on both sides — `noManifest` / `baselineNoManifest`), with its own sentence on the human
  channel and its own clause in `fix-gate`'s withheld-`✓` reason list. It raises `incomplete` like its
  siblings, is omitted when empty, and — like `judgedNothing` — reaches `must_hedge()` and **not**
  `incomplete()`, so no exit code moves.

  **THE SPLIT ADDS A PREDICATE, IT DOES NOT INVERT ONE.** `candor_report::report_judged_nothing` is not
  only a disclosure predicate: candor-scan's chained join (`DepIndex::judged_nothing_pkgs` → the κ
  ledger's coverage exemption) and `gate --report` read it to decide COVERAGE, and an absent manifest
  must keep granting NONE — that is row 3's own instruction. Making it answer `false` for a
  manifest-less report to fix the LABEL would have turned every pre-⟨0.21⟩ report into a covered one: a
  silent under-report introduced by a disclosure fix. So a second, disclosure-only
  `report_has_no_manifest` chooses the KEY for a hedge that was already happening, and a test asserts
  the coverage predicate is unmoved (the mutant that inverts it fails there and in candor-scan's own
  shape table). The gate's stderr note is untouched: it already named both conditions honestly
  (*"`analyzed.count` is 0, or absent with no entries"*).

  **BOTH CONTROLS ARE ASSERTED.** Row 1 (`analyzed.count: 0`) keeps `judgedNothing` and never becomes
  `noManifest` — the split goes both ways or it is a rename. Row 2 (`count: 7`, `functions: []`) is a
  legitimate all-pure claim §2 rule 3 requires a consumer to BELIEVE and MUST NOT hedge; a fix that
  hedges all three rows has disabled the feature rather than implemented the rule (over 1997 JVM
  dependency jars, a predicate keyed on `functions` being empty withdraws 104 real claims to catch 6).
  A manifest-less report that LISTS functions keeps its standing too. Measured before/after across ten
  verb invocations per row: every row-1, row-2, manifest-less-with-entries and intact-report block
  **byte-identical**, only the row-3 block changed.

- **⟨0.28⟩ `cargo-candor`'s gate sink stops deleting what it was pointed at, and carries the
  fail-closed document on every exit-2 cause** (SPEC §3.3.1 (3) input exemption; §3.3 ⟨0.8⟩/⟨0.24⟩ a
  document at the sink on EVERY exit-2; §3.2 ⟨0.28⟩ sinks in a broken argv are still sinks). The
  `policy`/`guard --gate-json` route is not one of the seven binaries conformance PART 43 drives, and
  measured on 2026-08-12 it had the whole family of the day's defects: the up-front `rm -f` DELETED
  whatever the sink named — `guard .candor/base --gate-json .candor/base.app.Executable.json` destroyed
  the baseline member and then reported "no baseline found" (its own act), the `.candor-version`
  provenance sidecar and the policy file identically; a usage error (unknown flag, valueless
  `--gate-json`) exited 2 with a PREVIOUS run's green verdict still at the sink, in both argv orders;
  every post-parse exit-2 cause wrote NOTHING; and the stream form put 0 bytes on stdout. Now: the
  input exemption is asked FIRST against the baseline locator's expansion (`<prefix>.*.json` +
  `<prefix>.candor-version`), the policy and the config — refused having written nothing; the file
  sink is then ARMED with the `{ok:false, refused:true}` refusal document (usage errors deferred past
  arming, so a sink named anywhere in a broken argv still ends fail-closed); every exit-2 leaves that
  document, the stream form prints it, and a config-load refusal — which exits before the verb can arm
  — writes it too, under the same exemption. 21 new `ci/wrapper-smoke.sh` rows assert BYTES at the
  artifacts (10 fail at the parent commit, each naming the destruction); the existing real-verdict
  rows pin that a completed gate still replaces the armed document.

- **⟨0.28⟩ `unverified`/`fix-gate` ANSWER a judged-nothing report at exit 0 with the pinned caveat,
  instead of refusing at exit 2** (SPEC §2 ⟨0.24⟩, *"A DISCLOSURE, NOT AN EXIT CODE"*). Both verbs
  guarded on `entries.is_empty()`, which conflated two causes SPEC rules in opposite directions: over
  a ⟨0.21⟩ Row-1 report (`functions: []`, `analyzed.count: 0` — the standard post-failure artifact)
  they exited 2 with *"no report … scan the crate first"*, claiming they got LESS far than
  `gate --report` (exit 0) on identical bytes — the outlier posture on the rung this engine's own
  `e1a341f` defined, with java/ts/swift all answering at exit 0. Both now load through
  `load_entries_loud` (no-report and net-corrupt stay loud exit-2 refusals) and answer with
  `incomplete: true` + `judgedNothing` (the array of report paths) on both channels. Two adjacent
  channel-consistency repairs rode along: `unverified`'s prose all-clear branch hard-coded
  `incomplete = true` into its strict exit, so prose `--strict` exited 2 where `--json --strict`
  exited 0 over the same judged-nothing bytes (measured); and both verbs' INCOMPLETE notes closed
  with a fixed *"`gate --report` exits 2 over these bytes"*, which is false of the count-0 cause —
  they now take `gate_line()`, byte-identical on the `unanalyzed` arm. Output over intact,
  unanalyzed-declaring, and unreadable-sibling reports is byte-identical to before on every channel.

- **⟨0.28⟩ `candor-query gate`: the input guard covers what the `--report` locator EXPANDS to** (SPEC
  §3.3.1 (3), *"AND AN INPUT LOCATOR NAMES A SET — COMPARE THE EXPANSION, NEVER THE TOKEN"*). The gate
  verb's pre-pass compared `--gate-json` against the raw `--report` token while `load_gate_report` reads
  the token's expansion, so `gate --report r --policy P --gate-json r.<crate>.scan.json` destroyed the
  operator's report at exit 2 — measured, with the diagnostic blaming the report ("failed to parse —
  corrupt input") for the corruption the run inflicted — and the no-`--report` discovery spelling
  destroyed the discovered `.candor/` report identically. The §2.2 sidecars are covered too:
  `--gate-json <the callgraph>` wrote a REAL verdict over the pair's other half at exit 1, a success.
  `gate_report_input_files` enumerates the exact files by the run's own resolution (`resolve_locator` →
  `glob_reports`, or `discover_report_prefix`), kept adjacent to `load_gate_report` so guard and loader
  cannot drift; `<stem>.gate.json` stays a permitted sink (the beside-the-report verdict layout — `gate`
  is skipped from `SIDECAR_KINDS` in the walk), pinned by the control test. The scan CLI closed the same
  vein through `run_inputs` earlier in the ⟨0.28⟩ arc; this is the QUERY route's spelling.

- **⚠ A stray second positional turned a red gate GREEN.** `dir = a.clone()` ran on every bare token, so
  the LAST positional silently won: `candor-scan A B` scanned B and said nothing about A. Measured —
  `candor-scan A --policy 'deny Fs'` exits 1, `candor-scan A B --policy 'deny Fs'` exits **0** with
  `functions: []` and `analyzed.count 1` — the count describes the OTHER tree, which was read; A is absent from the document
  entirely, which under ⟨0.21⟩ is a purity claim about it. And
  a shell glob matching two paths (or an empty `$EXTRA` in `candor-scan "$DIR" "$EXTRA"`) makes it
  permanent. Now exit 2 through the gate sink. It had been reasoned about and worked AROUND rather than
  rejected: the sink pre-pass takes the last positional *to mirror this loop*, with a comment explaining
  that taking the first "checked the wrong pair" when there were two. The right answer to two targets was
  never to pick one.
- **⚠ `--gate-json -` was left EMPTY by the `gate --report` route on an unreadable config.** java and
  swift wrote the refusal on the same input. The cause sits a crate down: `discover_config` is SHARED and
  exits below every gate sink — its own comment records this cause being closed once before, for the EXIT
  CODE only, which is the half that is not the machine channel. A registered refusal sink now lets the
  shared loader reach whichever sink the process armed, and the `.candor/config` shape check is hoisted
  above it so the new writer cannot land on an input.
- **⟨0.28⟩ a repeated `--gate-json` is refused, and every path named gets the refusal** (SPEC §3.3.1).
  Before this, the last sink won and the first was left exactly as found — so a previous run's
  `{"ok": true}` survived a gate that FIRED. Two spellings of one path stay ONE sink; a sink that is an
  input is still refused having written nothing.
- **The mostly-Unknown note no longer guesses at a build it never saw.** `tour --report R` reads someone
  else's report, so "missing project config" was a guess about a build it did not run; it now points at
  the reasons the report actually carries. The scan note names the κ ledger it already prints.
- **Property-based tests for the §6.2 policy parser** (`just props` — the recipe lives in the umbrella justfile, runs in seconds with no engine build, and is part of `just check`. The family's three
  generative fuzzers all generate CODE and check effect propagation; none generates a POLICY, which is
  where the fail-open defects have lived. Three properties — every line honoured or disclosed, lines do
  not interfere (across all three line endings), a typo in an `Unknown[…]` filter is always fatal — each
  verified to FAIL against the matching broken behaviour, with shrinking to the minimal input.
- **rust-analyzer works in this repo again**, and the file that claimed to fix it is gone. The cause was
  never `linkedProjects`: `~/.cargo/bin/rust-analyzer` is a rustup SHIM, and neither the pinned nightly
  nor the default stable had the component, so every request died with `Unknown binary` — a missing
  component wearing a crash's clothes. Both toolchains have it and `rust-toolchain` now lists it.
## [0.27.0] — 2026-08-07






- **…and the guard now enumerates a dep directory exactly as the LOADER does.** The first repair
  registered the directory's files with a FLAT read beside a RECURSIVE loader walk, so a report one
  level down stayed unguarded — and for `--deps`, which writes one subdirectory per `name@version`, the
  nested layout is the ORDINARY one. A guard that enumerates differently from the loader guards a
  different set of files. One enumeration now serves both.
- **⚠ A `--gate-json` sink INSIDE a `deps` DIRECTORY destroyed the operator's dep report.** `deps`
  accepts a directory — `--workspace` writes `.candor/deps/` and hands that back, so it is the common
  spelling — and the loader walks it and reads each report inside. The §3.3.1 sink-over-input guard
  registered only the DIRECTORY, which never equals a file within it, so `--gate-json <depdir>/lib.json`
  was unguarded: arming destroyed the report, the run chained the wreckage and exited 0 with `ok: true`
  written over the input. All four engines. The FILE spelling of this channel had been guarded for a
  release; the directory spelling had not, and no row posed it. Now pinned by conformance PART 36 (b14),
  which asserts both the refusal AND that nothing was written.
- **A root `cargo build --release` does not build the engines**, and the note now says so where the
  workspace members are declared. This file is a package that also declares a workspace, so a root build
  makes the dylint lint. A release binary sat eight days stale through a review because of it, and a
  reviewer measuring that binary reported one defect already fixed and one that did not exist.
- **A configured dep that cannot be READ now refuses, not just a missing one.** SPEC §2 binds "does not
  exist OR CANNOT BE READ" in one sentence and this engine implemented only the first clause: a dep path
  that resolved to a file which then failed to open, or held malformed JSON, was SKIPPED at exit 0 — so
  the caller of that dep serialised `inferred: []`, the ⟨0.21⟩ purity claim the refusal exists to
  prevent, reached by a different door. java and swift already refused on both halves, making the family
  2-v-2 on a MUST. Conformance PART 35 gained rows (d) and (e); it had been testing one clause of a
  disjunction under a title naming both.

- **Scan outputs no longer ride the published crate.** All four crates shipped tracked
  `.candor/report.*.json` inside their tarballs — stamped `spec 0.23`, four rungs stale, referenced by
  nothing, and immutable once on crates.io. Removed, with a Cargo `exclude` so it cannot recur.
- **⟨0.27⟩ The three verdict-document cells (SPEC §3.1/§4, conformance PART 36).** (1) The composed
  document (a certain AS-EFF-005 beside a refused policy) no longer carries `refused`/`reason` beside
  `violations` — the refusal document's discriminator must not ride a verdict — and now discloses the
  refusal as `unevaluated`, one entry per rule of the refused policy (this engine listed NONE; an
  unreadable policy gets one whole-file entry). (2) The stream sink: every pre-verdict exit-2 site
  (unknown flag, valueless `--out`/`--policy`, an unreadable config, `CANDOR_CONFIG` set-but-missing)
  now routes through `gate::exit2_refused`, so `--gate-json -` carries the refusal document instead of
  an empty stream — and a file sink gets the specific reason in place of the armed placeholder.
  (3) `zeroMatch`: the §4 zero-match list now rides the verdict document on BOTH routes (scan +
  `gate --report`), code-point sorted and deduplicated; it was stderr-only.

- **⚠ A certain violation did not dominate a refusal — this engine was alone, 3-v-1.** SPEC §3.1 states
  the order in EXIT CODES: *"violation (1) > refusal (2) > incomplete (2) … Exit 1 is not merely
  fail-closed here, it is CERTAIN, and strictly more informative: it names the violation."* An AS-EFF-005
  baseline regression beside a typo'd policy token exited 2 here and 1 in java, ts and swift. The narrow
  reading a test pinned — "precedence binds the VERDICT, not the policy gate" — was also inconsistent
  with this engine's own incomplete-analysis arm fifty lines away, which already lets a regression
  dominate; refusal and incomplete sit at the SAME rank. Both refusal causes (unhonourable token,
  unreadable file) now yield exit 1 when a violation is held, and the verdict carries BOTH halves —
  keyed on the refusal being RECORDED rather than on the exit code, which is what had silently dropped
  the `refused` marker once the exit became 1.
- **⚠ A configured dep that cannot be read now refuses (exit 2)** — see the spec ruling. Skipping it
  continued the run and its callers serialised `inferred: []`, a ⟨0.21⟩ purity claim published in the
  REPORT about code the scan never saw, while the coverage note travelled only on stderr.


- **CI-only: clippy 1.97's `collapsible_if` on the new gate pre-pass.** Local clippy is 1.96 and does not
  raise it, so `cargo build` and `cargo clippy` were both green here while CI failed — the nested
  `if let … { if … }` forms are flattened with `filter` rather than the lint's suggested LET-CHAIN, which
  this crate's MSRV cannot use. Behaviour verified unchanged afterwards: control gates at exit 1, and
  sinks naming the `--policy`, the `--report` and the config-declared policy all refuse at exit 2 with
  the file intact.

- **The sink guard now shares the loader's parse.** `config_inputs` and `discover_config_file` are
  extracted from `load_candor_config`, so the guard asks the question the loader answers instead of
  re-deriving it. The hand-written copy anchored an out-of-tree `CANDOR_CONFIG`'s relative values one
  level too high and split `deps` on `:` alone, and each divergence was a file it failed to protect.
- **⚠ `deps` was split on `:` alone in the loader too**, so a space-separated list resolved as one
  unresolvable token — every dep after the first was neither chained nor guarded. Now the §3.4 separator
  set (whitespace, `:`, `,`).
- **⚠ The pre-pass took the FIRST positional as the target and the parse loop takes the LAST**, so with
  two positionals the guard discovered a different tree's config than the run reads.
- **⚠ `candor-query gate` did not enumerate the config-declared policy**, so the checked-in form — the
  one a CI job has — was destroyed at exit 0 while the `--policy` form refused.

- **⚠ The collision guard keyed on the FLAG, so a policy from `.candor/config` was invisible to it** —
  the checked-in form, i.e. the one CI actually uses. `--gate-json <that policy>` destroyed it and exited
  0 with `"ok": true`, in all four engines, after the flag-based rows were already green. It now
  enumerates every channel a policy can arrive through, reading the config leniently so it cannot
  pre-empt the real load's refusal.
- **⚠ `candor-query gate --report R --gate-json R` destroyed the report it was asked to judge**, then
  blamed the report ("no `functions` array") rather than the collision. §3.3.1 names a report being read
  as an input. Caught by the new conformance rows, not by hand.
- **An unreadable config was silently "no config" on the QUERY route.** `read_to_string(..).ok()` dropped
  whatever it declared — a policy, a baseline, an engine pin, an `unknown-alias` vocabulary — so
  `gate --report R --policy P` with a broken `CANDOR_CONFIG` exited 1 here and 2 in java and ts. The scan
  route already refused; §3.4's posture does not vary by verb.

- **The `--policy`/`--gate-json` collision guard compared strings, and ran after a write.** Three
  defects in one place: `--policy /w/P --gate-json ./P` from `/w` is one file and the guard's
  path-component comparison missed it (policy destroyed, exit 0, `"ok": true`); the nonexistent-target
  refusal ran BEFORE the check and it WRITES, so `candor-scan /nope --policy P --gate-json P` destroyed
  `P` via the very refusal that exists to keep a red gate red; and `CANDOR_POLICY`/`CANDOR_CONFIG` and
  `.candor/config` were not covered at all. Now a pre-pass ahead of every write, resolving artifacts.
- **Arming moved ahead of the unknown-flag exit**, which SPEC §3.3 names as a broken-gate-config exit-2
  cause that must leave a refusal; it previously exited with the previous run's green intact.
- **`candor-query gate` never armed.** Every enumerated exit wrote a refusal, but a panic, an OOM, a CI
  timeout or a `kill -9` left yesterday's document — enumerating exits is the approach that keeps
  missing one. Verified by killing a run mid-flight and finding the refusal.
- **`--out --policy P` swallowed the gate.** `--out` took the next token unconditionally, so `--policy`
  became the prefix, the displaced bare `P` became the scan target, and the run went GATELESS at exit 0
  while writing a report named `--policy.*`. That is the identical defect this file already recorded as
  fixed for `--gate-json`; the dash-check is now on both.


- **A nonexistent scan target exited 0 with `ok: true`** — a typo'd path in CI was a permanent green.
- **`--policy P --gate-json P` destroyed the policy and turned a red gate green**, because the verdict is
  written before the policy is read. Refused.
- **Pin refusals now reach a `--gate-json -` stream**, which arming (files only) could not cover.

- **A refusal must never leave the last run's green: `--gate-json` is now armed FAIL-CLOSED at run
  start.** With a mismatched or unreadable `engine` pin this engine exited 2 and left the PREVIOUS run's
  verdict document on disk, so a CI wrapper reading the artifact rather than the exit code reported a
  **pass over a run that refused** — from the release's flagship guard. Arming at the start makes it a
  class fix rather than a branch fix: every exit path leaves a refusal unless the run got far enough to
  replace it. candor-java's `armGateJson` is the model.

- **A bare `engine <impl>` still split the family five ways.** `engine swift` — an operator forgetting
  the version on a qualified line — was skipped by candor-java and treated by the other four as a
  WILDCARD pin whose version is the literal `swift`, so it exited 2 in every engine that is *not* swift:
  one typo, a family-wide outage, on the exact property PART 33 exists to pin. The cause was arm ORDER —
  arity was tested before ownership, so the one-token case was claimed by the wildcard arm before anyone
  asked whose line it was. **A known qualifier now decides ownership first**, per §3.4's "whatever
  follows it" — and nothing following it is a case of that too.

- **A baseline DECLARED in `.candor/config` but missing is now exit 2, not a green pass.** An adopter
  review measured this as the second-likeliest first-commit mistake (`.candor/` committed, the baseline
  not) and found every engine printing a note and exiting **0** — the gate quietly not gating. The split
  is by SOURCE, because the same absence means two different things: `CANDOR_BASELINE` is set
  unconditionally by the adopt workflow, so a path that is not there means "the ratchet is not adopted
  yet" and stays a note; a checked-in `baseline` line DECLARES that this repo has one, so an absent file
  was deleted or never committed. Verified four-way: config-declared → 2, env-named → 0.


- **Panel review: the pin grammar disagreed across engines on a shared config.** Three confirmed
  divergences, each a case conformance PART 33 had not thought of, all now fixed and pinned there:
  a junked line qualified for ANOTHER implementation (`engine swift 0.99.0 junk`) killed this engine's
  own run — SPEC §3.4 now rules the skip WHOLE-LINE, because a malformed line naming another engine is
  that engine's problem and it refuses on it, while refusing everywhere turns one typo into a
  family-wide outage; `vv0.27.0` was accepted as a version by engines that stripped every leading `v`;
  and a CRLF config broke a MATCHING pin where `\r` was not treated as whitespace.
- **The zero-match disclosure was missing on the `gate --report` route.** java and swift disclosed
  there, this engine did not — so on the supply-chain gate, the surface a consumer points at a report
  someone else produced, a typo'd layer name was still scored as satisfied in silence. SPEC §4's MUST
  carries no route qualifier.


- **⟨0.27⟩ SPEC §3.4 `engine` — the engine↔baseline coupling, enforced here too.** A build that is not
  the pinned one FAILS with exit 2 (UNEVALUABLE, never 1 — a machine consumer must not read "I could not
  trust this result" as "your code broke a rule"). Two of the five verdicts deliberately do NOT change
  the exit code: an absent pin (the key is opt-in by construction) and one this build cannot check,
  which is §3.1's unanswerable-condition rule — disclosed, never scored, *including* as satisfied. An
  unreadable pin (`engine latest`) exits 2 rather than being skipped: this is the one place §6.2's
  warn-and-skip inverts, because skipping a PIN hands the operator a guard they believe is on. A pin
  qualified for another implementation is ignored — one config serves the family, which versions as a
  ladder. Pinned four-way by conformance **PART 33**.


### SPEC §2 `fs` — the field existed and nothing ever wrote to it

`pub fs: Vec<String>` has been in the wire model for a long time, and the construction site read
`fs: Vec::new()`. Hardcoded empty. Never populated.

That is WORSE than not having the field. An absent field says "this producer does not track kinds"; a
present-but-always-empty one says "kind undetermined" on every function forever — a claim the engine was
in no position to make, dressed in the schema that implies support. And §2's own omit-rather-than-guess
rule is precisely what made it invisible: every empty answer looked legitimate.

Found while pinning `fs` in conformance, after the other three engines gained it. The state was not the
two-of-four it looked like: java emitted, swift and ts had nothing, and rust had a dead field.

`fs_kind(path)` now refines an Fs the classifier already PROVED with the direction its verb implies, on the
same contract as the other three engines: `["read"]`, `["write"]`, `["read","write"]`, or `[]` when the
verb does not say. Kinds PROPAGATE over call edges (a caller that transitively only writes is a writer), with a
`"?"` poison travelling alongside: any contributing `Fs` with no determined kind suppresses the whole
field, because `["write"]` there would claim "writes but never reads" about a function that may do both.
Matches candor-java's `FS_UNKNOWN` discipline. (The first version of this was direct-only — corrected
before release when conformance PART 31 showed the four engines disagreeing.)

Measured: `std::fs::copy` → `["read","write"]`, `read_to_string` → `["read"]`, `write` → `["write"]`, a
function that merely REACHES a writer → `["write"]` (the refinement PROPAGATES, as the paragraph above
says and conformance PART 31 pins; an earlier draft of this row said "omitted", describing the
direct-only version that was corrected before release and contradicting its own entry), `OpenOptions`
(direction lives in the builder chain, not
the terminal verb) → omitted.

Tests assert what the classifier refuses to say as much as what it says; both halves probed by breaking
them. 245 tests green.

## [0.26.0] — 2026-08-04 ⟨spec 0.26⟩

### ⚠ κ breadth: `tracing_subscriber` — Log + Env, and the filing's `Fs` is wrong

Third of the ledger-mined batch, verified against 0.3.23:

  · **Log** — `fmt/fmt_layer.rs:749` defaults `make_writer: io::stdout`, so the fmt INIT terminals
    (`fmt::init`, `try_init`, `SubscriberBuilder::init`) install a subscriber that writes program output.
  · **Env** — `fmt/mod.rs:1219` reads `RUST_LOG` on the `init()` path, `fmt_layer.rs` reads `NO_COLOR`,
    and `filter/env/builder.rs:189,203` read `env::var(self.env_var_name())`. So `EnvFilter`'s from-env
    constructors are Env.

**NOT Fs.** The filing said "Log/Fs"; the only `std::fs` in the crate is `impl MakeWriter for
std::fs::File` — the crate ACCEPTING a caller-supplied File, never opening one. The caller's
`File::create` is already classified on the caller, so charging Fs here would double-count. Same shape as
the `serde_json::from_reader` caveat one crate over, and the second time in this batch that a filed
effect did not survive reading the source.

The builders (`fmt()`, `layer()`, `with_target`, `EnvFilter::new`) DESCRIBE a subscriber and stay pure.

    BEFORE  setup/filtered/build -> invisible:[tracing_subscriber];  uncovered = [(tracing_subscriber, 3)]
    AFTER   setup -> Log,  filtered -> Env,  build -> pure (omitted);  uncovered = []

### κ breadth: `REVIEWED_PURE_CRATES` — serde_json, serde_yml, toml, regex, sha2

The other half of the ledger-mined batch, and it needed a NEW mechanism rather than a new list entry.
`CALIBRATED_CRATES` means "classify has effect rules here", and `calibrated_crates_are_live` fails any
entry no rule matches — "a dead entry would silently suppress a real coverage warning". A genuinely pure
crate has no rule to be live, so it cannot go there, and inventing a rule to get it in would be worse
than the noise.

So: a separate list the κ ledger consults, `classify` never does. **It manufactures purity claims** — a
crate here stops being disclosed and starts being believed — so every entry was checked against its
source in the local cargo registry for `std::{fs,net,process,env}` and stdio use. Every apparent hit was
a DOC COMMENT (serde_json's ``/// [`File`]: std::fs::File``, serde_yml's `///  io::stdout()`).

**`color_eyre` was on the same filing and is deliberately NOT here**: it is absent from this machine's
registry, so it could not be checked, and an unverifiable entry is exactly what the list forbids.

The caveat worth stating: `serde_json::from_reader`/`to_writer` DO move bytes — through a handle the
CALLER had to obtain, and obtaining it (`File::open`, `TcpStream::connect`) is already classified on the
caller. The crate performs no syscall of its own, so charging it would double-count.

Two guards, because the list is a claim: `reviewed_pure_and_calibrated_are_disjoint` (a crate cannot be
both rule-covered and effect-free — the ledger ORs the lists, so a contradiction would resolve silently
to "covered"), and `reviewed_pure_crates_classify_as_pure` (if someone later adds a rule for one of
these, the claim is dead and the entry must be re-read rather than silently outvoted).

    BEFORE  digest/matches/parse -> invisible:[sha2|regex|serde_json]
            coverage.uncovered = [(regex, 3), (serde_json, 1), (sha2, 1)]
    AFTER   all three omitted as pure;  coverage.uncovered = []
    CONTROL readfile -> ['Fs'] in BOTH arms — a real effect is untouched

### ratatui `widgets::canvas` is carved out — the `::draw` tail was FABRICATING

Review of the same-day rule above. `ratatui::widgets::canvas::Context::draw(&shape)` sets `self.dirty`
and paints into an in-memory `Painter` — no terminal, no writer, provably pure — but it ends in `::draw`
and the tails were charging it `Ipc`. MEASURED as a live fabrication (`plot(ctx) -> [Ipc]`), and it is a
HOT path: a TUI drawing charts or maps calls it per shape, per frame.

Carved out as a DENYLIST (`::canvas::` returns pure) rather than narrowing the tails to `Terminal::`,
per the family rule: an allowlist silently under-reports whatever it forgot, and the write surface here
is Terminal AND the backends, so pinning to `Terminal::` would have dropped a direct
`CrosstermBackend::flush`. Both directions are now pinned by tests.

### ⚠ κ breadth: `crossterm` + `ratatui` are CALIBRATED — the tty is an `Ipc` channel

Ledger-mined, per the standing practice of batching by call-count rather than speculating: the
2026-07-14 four-ecosystem sweep put **ratatui at 3,345 disclosed calls across three real repos**, the
single loudest source of blind-spot noise.

**The backlog said "mark ratatui reviewed-pure". Reading ratatui-0.29.0 REFUTES that**, and marking the
crate pure would have claimed purity over the one API that writes: `terminal/terminal.rs`'s
`draw`/`try_draw`/`flush`/`clear`/`autoresize`/`hide_cursor`/`show_cursor` all end in a backend flush,
and `backend/` writes to the terminal. So the split follows the source, not the filing — the render
surface (widgets, layout, buffer, style, text) is genuinely pure and is now COVERED rather than
disclosed; the Terminal verbs are `Ipc`.

`Ipc` because the tty is a user dialogue channel — the ruling `dialoguer`, `console` and
`terminal_colorsaurus` already carry here. Not a new effect class for a new crate.

crossterm likewise, verified against 0.28.1: `command.rs`'s `execute`/`queue` end in `self.flush()?`,
`event::read`/`poll` read tty input, `enable_raw_mode`/`disable_raw_mode`/`size`/`window_size` talk to
the device. Its Command VALUE types (`Print`, `MoveTo`, `SetForegroundColor`) describe an action rather
than performing one and stay pure.

**`size()` and `window_size()` are classified deliberately**, though they only read: once a crate is
CALIBRATED an unmatched path is a PURITY CLAIM rather than a disclosed blind spot, so a tty ioctl left
to fall through would be claimed pure. Calibrating a crate raises the cost of an omission, which is the
part of this work to be careful about.

⚠ VERDICT-AFFECTING, in the sound direction: three effects that were `invisible` are now DETERMINED, so
a `deny Ipc` policy fires where it previously could not. Measured on a fixture:

    BEFORE  build_ui, render -> invisible:[ratatui];  setup, wait -> invisible:[crossterm]
            coverage.uncovered = [(ratatui, 4), (crossterm, 2)]
    AFTER   build_ui -> pure (omitted);  render, setup, wait -> Ipc
            coverage.uncovered = []


### ⟨0.26⟩ THE HIERARCHY SIDECAR'S KEY SET IS ITS MANIFEST — consumer half

SPEC §2.2 `4cae735`. `is_subtype_of` is now the two-valued face of a three-valued `subtype_of`:
`hier.get(t)` returning `None` used to skip the frame in silence, so "indexed, no supertypes" and "never
analysed" both answered `false` — a positive claim about a type nobody analysed, which drops a reacher
from the `callers --include-unknown` frontier with no diagnostic. Unanswerable now collapses to TRUE at
the call site: over-list, never drop. A POSITIVE still DOMINATES an unknown branch.

This engine is CONSUMER-ONLY — candor-scan writes no hierarchy sidecar — so every hierarchy it walks came
from candor-java or candor-ts. The producer's completeness is never this engine's to assume, which is what
makes the tri-state load-bearing here.

MEASURED: with a sidecar missing one key on the path, the frontier answered `[]`; with NO sidecar at all
it answered correctly. **Partial information was worse than none.** Pinned by conformance PART 30 (P6),
verified to catch both by restoring the silent `continue` and by a stub that never answers No.

## [0.25.0] — 2026-08-02

⟨spec 0.25⟩ **Floor bump only — no behaviour change in this engine.** SPEC §2 chaining rule 1 now states
that an ambiguous join key is UNIONED rather than dropped; this engine already implemented the union
(conformance PARTs 25/26 pin it four-way), so 0.25 records the contract catching up with the code. See
candor-spec/CHANGELOG.md for the measurement and the reversal note.


### ⟨0.24⟩ "PROVABLY clean" over a report declaring source candor could not read

SPEC §3.2 `ec1a441`/`93cef40`/`142740a`. `0075987` ruled the omit-`ok` shape for `whatif` and this engine
implemented it **for `whatif`, in `whatif`'s own file** — `unverified.rs` and `fix.rs` contained zero
occurrences of `incomplete`. MEASURED on a report declaring one `unanalyzed` unit, **no `Unknown` holes at
all**, and a `deny Net app` nothing violates:

```text
  gate --report        exit 2   ok:false, incomplete:true + manifest   ← correct
  unverified --strict  exit 0   {"ok": true, "unverified": []}
                       stdout   "every function in a pure/deny layer is PROVABLY clean … ✓"
  fix-gate  --strict   exit 0   {"ok": true, "remedies": []}
                       stdout   "no deny/pure boundary crossings in this report ✓"
```

`unverified`, `fix-gate` and `fix` now read the ⟨0.21⟩ manifest through the SAME file set and the SAME
reader `gate --report` uses, emit `incomplete: true` + the `unanalyzed` manifest, **OMIT `ok`** (never
`ok: false` — on an advisory verb that asserts a finding the analysis did not make), and exit 2 under
`--strict`. **On every channel**: the prose `✓` is the prose `ok: true`, and `unverified` had a second
sentence of the same kind — *"The gate still PASSES"* — which is not merely unhedged but false, since the
gate exits 2 over those bytes. `fix` carries the disclosure with its exit code unchanged (it answers no
`ok`, matching its own gate-refusal branch and candor-ts).

`142740a` also folds in the WITHHELD-RULE trigger, where this engine emitted `ok: false`: where a rule was
withheld no hole was FOUND, the question was declined, so `false` asserts the finding that did not happen
— the same shape and the same answer as the incompleteness trigger, ruled a day apart. And "the same
bytes" means the same report SET: MEASURED here as a null result, since both routes go through one
`glob_reports`, but pinned by a row so a later split cannot reintroduce candor-java's defect silently.

The findings still ship in every case — a partial answer that says it is partial beats a refusal.

13-mutant audit, all killed. The one that mattered: **keeping the `✓` on `unverified`'s all-clear branch
SURVIVED** the first draft of the row, because every existing fixture pairs incompleteness with something
else to find and the verb never reached its own all-clear line. A/B on 40 real report corpora × 7 policies
× 8 invocations (2240 runs): **one** difference, and it is the intended `142740a` change on a legacy
`hosts`-only report. Those 40 corpora declare zero `unanalyzed` units between them, so they prove no loss
and nothing else — the incompleteness path is exercised by hand-written fixtures.

### ⟨0.24⟩ a chained dependency report that judged NOTHING must not read as full coverage

A report carrying `functions: []` and `analyzed.count: 0` bought a consumer **more confidence than not
chaining the package at all**. Its caller dropped out of `functions` — under ⟨0.21⟩ a positive **purity
claim** — with no `invisible` on the entry, no `coverage.uncovered` in the envelope, no verdict caveat and
no line on stderr, while the same scan with `CANDOR_DEPS` unset disclosed all four. Conformance PART 26
found the same door in all four engines and printed `rust INDISTINGUISHABLE — the engine is not reading
analyzed.count`.

**The harm stated precisely, because the loose form sends you after the wrong symptom.** The empty report
carries no effects, so the count-0 arm cannot itself *trip* a gate — it and the unchained arm both exit 0
on `deny Fs`. What it DELETES is the **disclosure**; the gate flip exists only against the *trusted* arm.
So this fix restores the disclosure channel and deliberately does not manufacture a verdict: asserting an
effect the consumer has no evidence for is the mirror sin.

    unchained   entry -> invisible: ['deplib'], coverage.uncovered: [deplib]   exit 0
    trusted     entry -> ['Fs']                                                exit 1
    count: 0    pre   entry ABSENT from `functions`, no coverage, no advisory  exit 0
    count: 0    post  entry -> invisible: ['deplib'], coverage.uncovered       exit 0  + a named stderr line

**WHERE THE RULE LIVES.** A third conjunct on the COVERED set — `deps.rs`'s one `cover` closure and the κ
ledger's one `covered` predicate — beside the §2.1 staleness gate and the ⟨0.21⟩ incompleteness one.
Coverage is the single mechanism that turns a report's SILENCE into a purity claim, and this rung is the
third answer to *"may this silence speak?"*, after staleness and incompleteness. Not the gate: a gate
reads its verdict off a coverage decision already made, so the rule there would have to be repeated per
verb and would leave the report, the κ ledger and every other consumer of the same silence untouched.
Same placement candor-java (`Loader.loadCrossDeps`) and candor-swift (`Deps.swift`) reached independently.
It needed no new plumbing: withhold coverage and the existing `invisible` / `coverage.uncovered` /
`--gate-json` block all fall out.

**Coverage is anchored FOUR times here** — the envelope `package`, the JVM-shape `packages[]`, the
filename fallback and each entry's `hash` prefix — and all four funnel through one closure, so there is
one place to gate. That matters more for this rung than the last: a count-0 report reaches the entry loop
with **no entries**, so its `hash` anchor never fires and gating that one alone would have been exactly
the no-op java measured. `coverage_has_exactly_one_anchor_and_exactly_one_consumer` now enumerates four
writes and three consumers out of the source.

**THE SECOND ROW OF SPEC §2's TABLE IS A CONTROL, NOT A FOOTNOTE, and it is why this is not a one-liner.**
`functions: []` is equally the shape of a legitimately all-pure dependency, whose empty report §2 chaining
rule 3 requires a consumer to BELIEVE. `analyzed.count` is the only thing on the wire that separates them,
so the predicate is keyed on **that integer** and never on the emptiness of `functions` — `functions`
enters for exactly one row, the manifest-less one. Measured over 1997 JVM dependency jars: a fix keyed on
emptiness would have withdrawn **104 real claims to catch 6**.

**BOTH CONTROLS, MUTATION-VERIFIED IN THREE DIRECTIONS.**

- `count: 0` → not covered, and the consumer's answer is asserted **EQUAL to the unchained arm's** report
  rather than against a literal, because "exactly as if it had not been chained" is what §2 states.
  Reverting the predicate fails 5 tests, every one on a FLOOR assertion.
- `count: n>0` → **UNCHANGED**: covered, believed all-pure, exit 0, no hedge, no advisory. Keying on
  emptiness instead fails 4 tests, every one on a CONTROL assertion — **while the count-0 rows stay
  green**, which is what a fix that had "worked" looks like from the floor arm alone.
- Removing the anchor branch, or the ledger conjunct, each fails 4 — including the structural test, which
  is the one that would otherwise let the two halves drift apart.

**SPEC §2's THIRD ROW retires a pre-⟨0.21⟩ affordance, recorded rather than smuggled.** A manifest-less
empty report DID buy coverage, and `kappa_ledger_honors_an_empty_chained_report_as_coverage` pinned exactly
that. Its subject — the envelope `package` field carrying coverage independent of the filename — is
unchanged; it is RE-POINTED at a manifest-bearing fixture, with the count-0 and manifest-less forms added
beside it as their own rows, which is what the spec clause asks for.

**CONSERVATIVE ON CONFLICT, and a deliberate divergence from candor-swift.** A crate chained once as judged
and once as judged-nothing keeps the hedge here; swift subtracts so the real report wins. Swift's argument
is good (a count-0 report makes no claim in either direction), but rust's index DROPS a key two dep entries
disagree under, so granting coverage on one report's authority can make the very key that mattered resolve
to nothing and read confidently pure — the same reasoning `63bbe87` recorded for fresh-vs-stale. The cost
of being wrong this way is one extra hedge; the other way it is a false all-clear.

**⟨0.24⟩ THE SAME RULE BINDS `gate --report` (SPEC §3.1)** — "the obligation is on the reading, not on the
route by which the report arrived". Measured: the verb printed `policy ✓` and exited 0 over a count-0
report, byte-identical to the legitimately-all-pure case. It now **REFUSES (exit 2) and writes no verdict**
— §3.3's exit-2 cause (a), the gate could not be evaluated at all, so an `ok` either way would be a guess.
(Cause (b)'s machine-legible `incomplete` verdict is keyed to `unanalyzed`, a NAMED list of source the
producer could not read; a count-0 report names nothing.) It lands beside the verb's three existing
refusals, which refuse for the same reason. The predicate itself lives in `candor-report`, the one crate
both routes depend on — a rule written twice is a rule that can drift between its routes.

**BLAST RADIUS, real chained trees.** 17 of candor-rust's own 173 dep reports (9.8%) say `count: 0`, 27 of
ebman's 409 (6.6%), 20 of pgman's 270 (7.4%) — every one a macro-only, platform-link-stub, data-blob or
re-export-only crate (`cfg_if`, `windows_*_msvc`, `icu_*_data`, `stable_deref_trait`, `pin_utils`,
`static_assertions`). None is manifest-less. Against them stand 16 / 49 / 39 legitimately all-pure reports
(9.2% / 12.0% / 14.4%) that an emptiness-keyed fix would have hedged. On candor-rust's own chained scan the
REPORTS are **identical before and after** — none of the 17 is a crate this code demonstrably calls — so
the whole live effect is one new stderr advisory naming all 17. A rare-facade catch, not a rule that turns
a dep tree uncovered.

**PART 26**, `rust/empty_zero`: 64 live cells move from `A` (ABSENT — a silent purity claim) to `h`
(HEDGED_LOSS, the correct §2.1 shape), leaving **0 failing cells**, and

    rust     SEPARATED on 64/80 cells — the engine distinguishes them

where it read `INDISTINGUISHABLE — the engine is not reading analyzed.count`. The `rust/empty_zero` waiver
in candor-spec's ratchet baseline is now fully STALE (the harness says so: "every cell now passes") and
should be DELETED rather than narrowed — that is a candor-spec edit and is left alone here.

### ⟨0.24⟩ `candor-query gate --report <locator> --policy <file>` — apply a policy to an EXISTING report

SPEC §3.1 ⟨0.24⟩ makes this a MUST and rust did not have it: conformance PART 27's R6 row printed
`NOSURF` for this engine, `NOSURF` does not fail the run, and the 0.24 CHANGELOG entry in candor-spec had
to be publicly corrected to "pinned 2-of-4" because of it. It is the supply-chain verb — gating a
dependency's PUBLISHED report is the operation an adopter actually wants and could not previously express
without re-analysing code they do not have.

**The reason it is a MUST rather than a convenience is that it makes the code-implements-spec direction
testable at all.** `candor-scan --policy` recomputes the effect set from source, so the classifier is
always in the loop; `whatif` reports only what a hypothetical INTRODUCES. So the gate had never been
reachable as a function of a GIVEN signature, and a defect in the GATE and a defect in the CLASSIFIER were
indistinguishable from any test that could be written here.

**THE SEAM.** The §6.2 matching moved into `candor_classify::gate::gate(&ParsedPolicy, &GateInput)` — one
copy, two routes in. `candor-scan --policy`'s `policy_violations` is now a thin adapter that builds a
`GateInput` from the classifier's fixpoints; `candor-query gate --report` builds one from a written report
and nothing else. `net_classes_of` moved with it, because the report FIELD and the gate FILTER have to be
the same set.

**A NEW READER WAS NEEDED, and that is not an accident of this codebase.** Every existing loader is built
to ENRICH — the `.callgraph.json` sidecar, the type hierarchy, chained deps — and this verb has to read
strictly LESS than any of them, which is not a subset reachable by passing a flag. (candor-swift reported
the same thing from its own tree.) `load_entries` alone would not do either: it returns the `functions`
array and drops the §2 envelope the verdict is written from.

**THE MUST NOT: an ABSENT entry is absent.** No callgraph sidecar, no chained dep, no `.candor/config`
`deps` key, no re-classification of `hosts`/`netClass` through this machine's `net-partner` list. Proved
with all three back-fill channels open at once over an entry that is not in the report — `deny Fs` exits
0 — beside the negative control that exits 1 when the same effect is written INTO the report. Both arms
mutation-verified; without the control, "did not back-fill" and "never evaluated" are the same green.

**ANSWERABILITY: a rule whose evidence the wire does not carry is REFUSED (exit 2), never evaluated** —
each of the three is fail-OPEN if approximated. `forbid A -> B` and `allow <E> …` are refused
whole-policy (enforcing the answerable half and exiting 0 is gateless-green); a class-scoped `deny` whose
scoping datum is an absent optional field is refused per (rule, function), so a scoped rule whose own
matches carry their evidence still evaluates. The refusal is MINIMAL: because the class set only GROWS and
`Reject` is upward-closed, a scoped rule whose determinable classes are already non-empty is ANSWERED —
including the ⟨0.24⟩ CONTRIBUTES counterexample (a reasonless DIRECT `Unknown` under
`deny E Unknown[unresolved]`), which contributes its class from the entry alone and therefore fires.

**EQUIVALENCE IS THE ACCEPTANCE TEST AND IT IS BYTE-LEVEL** — `analyzed.count`, `reasonClass`, `netClass`
and the coverage advisory included. Measured over **90 rows**: 30 policies × three corpora (ebman, pgman,
and this workspace's own five members), 55 of them with violations. `ci/gate-equivalence.sh` keeps 48 of
those rows as a standing CI gate plus a 49th for the arm no in-tree crate can reach — a scan whose own
analysis was INCOMPLETE, where both routes must exit 2 and write the same ⟨0.21⟩ `incomplete:true`
verdict — and it FAILS when no policy in its matrix fires: byte-equal empty verdicts prove nothing.

### fix — the scan gate double-counted a violation on two `#[cfg]`-gated units sharing one name

FOUND BY the equivalence obligation above, on 15 of the first 90 rows. `#[cfg(unix)] fn f` beside
`#[cfg(not(unix))] fn f` puts the qualified name in the gate's function list TWICE while `inferred` holds
ONE merged signature — so the gate emitted two byte-identical `GateViolation` records, an inflated
`N policy violation(s)` count, and a `--gate-json` document a consumer reads as two findings. The report
route cannot reproduce it (a report is keyed by name), which is exactly how it surfaced. The gate now
answers once per (rule, function); the report is unchanged and still lists both units.

### model cross-check — the engine now agrees with `reference/policy_model.py` directly

The verb's other purpose. 2,949,120 rows: 30 policies (`deny e`, `deny e Unknown[C]`, `pure`) over all
**98,304 REACHABLE** signatures of the (S, D) lattice, fed to the shipped binary as one report and
compared against the model's `Reject`. **Zero disagreements.** The domain is `reachable_lattice()`, not
`full_lattice()`: every engine co-emits `Llm` with `Net`, so `Llm ∈ S ∧ Net ∉ S` names 32,768 points none
can produce, and the unrestricted run reports them as 40 phantom `deny Net` families — which is the
negative control proving the differential discriminates at all.

### usage ⟨0.24⟩ — a `--class` value that cannot be honoured is now REFUSED, not quietly narrowed

SPEC §6.2's value grammar, which conformance PART 27 found unimplemented in **all four** engines rather
than divergent between them — the suite's only `engine: "*"` waiver, and the last thing holding the floor
below 0.24. `--class <c>[,<c>…]` takes ONE comma-separated list of the six reason classes plus the two
aliases `dynamic` and `*`. Two things that used to exit 0 now exit 2, on **both** verbs that take the
flag (`unverified --class` and `blindspots --class`):

- **an unrecognised token.** `--class dyanmic` used to print `--class ignores unknown reason-class …`
  and carry on with whatever was left of the list — for a single-token list, an EMPTY filter. It now
  names the token and lists the accepted set.
- **a repeated `--class`.** It was last-wins, silently discarding the first list. A second occurrence is
  a usage error, not a union — and not last-wins either. Both silent readings answer a different question
  than the line on screen.

**Why this is not the policy side's drop-with-a-warning, since the asymmetry looks like an inconsistency
until you write it down.** A token dropped out of `deny E Unknown[reflect,dyanmic]` leaves the WIDER rule
standing: the mistake surfaces as a gate that over-fires, and someone comes to look. The same token
dropped out of `--class` leaves a NARROWER filter, and a narrower filter on `unverified` comes back as a
SMALLER NUMBER — which is indistinguishable from a real all-clear, in the one verb whose entire job is to
say "green, but not provably so". That is the fail-open the surrounding §6.2 clause exists to close, and
it is why the query side refuses what the policy side approximates.

The token rule lives in ONE place (`parse_class_filter`, now `Result`-returning) so the two verbs cannot
drift, and `*` is evaluated after the whole list is walked — `--class *,dyanmic` still reports the typo
rather than short-circuiting past it. The refusal emits **no answer document at all**; a narrower result
one exit code away from a refusal is the same fail-open wearing a different hat. Nothing changes for
well-formed input: the tests pin the unfiltered baseline and each filter's exact selection, on both verbs.

### soundness — three silent under-reports, all three found by ONE SPELLING never being tried

Found by conformance PART 24 (split-invariance) on its first run, then each re-derived from a
hand-written fixture so none rests on the generator. The common shape: **the answer was already under
the right key and the consumer's join had a branch that did not fire.** No report-format change was
needed for any of the three — the third time in a row this vein has come out that way.

1. **A chained dependency's lazy static was charged only through a PATH-QUALIFIED read.**
   `deplib::CFG.len()` → `['Env']`; `use deplib::CFG; CFG.len()` → silent-pure. Deref vs method call made
   no difference — the `use` did, because the import leaves a ONE-segment path behind and the branch
   required two. Conformance PART 19's rust fixture uses the qualified spelling. Fixed by consulting the
   file- **and body-level** `use` maps, and by asking for the dependency's own MODULE-qualified key
   (`<lazy>::cfg::MODC`) as well as the crate-root one — a dep static declared inside a module was
   unreachable under *either* spelling.
2. **A chained dependency FACTORY call with no intermediate binding read silent-pure.**
   `let c = deplib::build(); c.fetch()` resolved; `deplib::build().fetch()` was silent, because the
   provenance that drives the disclosure is only ever written by a `let`. A hole in a shipped guard, not
   an un-attempted precision gap: it broke that guard's own ruling that a key which could not be formed
   must never read pure. The unbound receiver now takes the same route, including through `?`, `.await`
   and `&` — and `visit_local` peels those too, so eliding the binding cannot change the answer either way.
3. **A lazy static read from OUTSIDE its module was not charged at all — single tree, no boundary.**
   `m::inside()` was charged; `fn outside() { let _ = *m::INNER; }` read pure.

**(3) is a regression introduced by `5447eba`, and the measurement is unambiguous.** That commit moved
the module path INSIDE the `<lazy>::` prefix so two same-named statics stop merging — which made the
WRITER module-qualified while the READER still built `<lazy>::<its own module>::NAME`. Three-way on one
fixture (a crate-root lazy static read from a submodule, the ordinary `use crate::ROOT_CFG;` shape):

| | `root_read` (same module) | 4 cross-module spellings |
|---|---|---|
| before `5447eba` (`c0a142c`) | `Fs` | **`Fs`** |
| at `5df4af1` | `Fs` | **PURE** |
| after this change | `Fs` | `Fs` |

A fabrication fix that introduced a cardinal sin, and much wider than the fixture that caught it: at
`5df4af1` *any* crate-root lazy static read from *any* submodule read pure. The identity property
`5447eba` bought is kept — its own test still passes, and the control that two modules' same-named
`CFG`s do not cross still holds — because the reader now takes the module the SPELLING names and keeps
its own-module key beside it, both filtered by `resolve_target`'s uniqueness rule.

**The mirror control caught a live fabrication in the first cut of fix (1)**, which is the reason it was
written: the five typed side-tables that answer "is this name shadowed by a local?" only hold bindings
whose type was RECOVERABLE, so `use deplib::C; … let C = "aa"; C.len()` charged the dependency's `Env`
to a local string. Harmless while only a qualified path could force (a shadow is spelled bare); live the
moment the bare spelling was added. `bound_idents` now collects every ident in a binding position,
typed or not. Mutation-verified in both directions, along with the module-discrimination control (pin
the derived module and the reader of the pure `b::CFG` picks up `a::CFG`'s `Fs`), the local-module
control (held independently by TWO guards — measured by removing each), and the per-static keying
control (make the dependency's pure lazy static effectful and its reader lights up).

**A/B, before/after, each binary producing its own dependency tree:**

| target | fns | gains, `Unknown` only | gains, CONCRETE effect | losses |
|---|---|---|---|---|
| ebman `--deps` | 546 → 550 | 95 | **0** | **0** |
| pgman `--deps` | 200 → 205 | 21 | **0** | **0** |
| candor-rust workspace `--deps` (5 members) | 223 → 230 | 32 | **0** | **0** |
| tb-tui-common | 9 → 9 | 0 | **0** | **0** |

Every gain on real code is (2)'s disclosure, never a concrete effect: functions carrying a direct
`dispatch:untyped cross-package receiver` go 18 → 52 on ebman, 2 → 18 on pgman, 0 → 31 on candor-rust.
**That is a large number and it is stated as one** — 95 of 550 ebman functions newly carry `Unknown`,
about 2.9× the shipped bound-spelling arm's own footprint.

**Every one of them is on a CHAINED dependency, and that was measured rather than argued.** The rung's
THIRD conjunct — the dependency must be chained, because for an unchained one the κ ledger already says
`invisible: [pkg]` and a second hedge is pure false uncertainty — lives on the shared CONSUMPTION path
in `scan.rs`, and this change is emission-side only, so the new arm inherits it. Instrumented at the
marker, *before* the gate:

| | markers | CHAINED | UNCHAINED (suppressed) |
|---|---|---|---|
| ebman alone, chained over its 410-crate dep tree | 53 → **141** | 32 → 73 | 21 → **68** |
| the whole dep-tree walk | 53 → **22,131** | 32 → 73 | 21 → **22,058** |

The conjunct suppresses **99.7%** of the new arm's markers, and the crate heads it suppresses are
exactly what it was written for: `std` (52), `String` (7), and local modules (`app`, `crate`, `eb_cli`,
`project`, `cost_cache`, …) — not one of them a real dependency. End-to-end attribution of the 95:
newly-direct functions with no chained marker **0**; backed only by an unchained marker **0**; of the 28
functions whose only untyped marker was unchained, **0** gained anything. The responsible crates are
chrono 21, serde_yml 7, futures 2, tokio 2, toml 1, tracing_subscriber 1, and all eight crates carrying
a chained marker have substantial reports (chrono 191 fns / 45 published return types; serde_yml 337/24;
tokio 818/114). The number is real and the disclosures are honest.

**The lever on it is DETERMINATION, not suppression** — the ⟨0.24⟩ ordering, and it applies to both
spellings identically so it cannot disturb the parity above. Of ebman's 73 chained markers, 10 resolve
today, 6 miss only because the dependency publishes its key MODULE-qualified where the consumer forms it
from the written path, and 57 are genuinely absent from the published surface. **37 of those 57 are
chrono's `Utc::now`, and the reason it is unpublished is a spurious collision**: chrono declares
`pub fn now() -> DateTime<Utc>` TWICE under mutually exclusive `#[cfg]`s (native and wasm32), the scan
walks both cfg arms by design, and the return index's never-guess-between-two-same-named-defs rule drops
the entry — even though **both candidates name the same return type**, so there is nothing to guess
between. Publishing a collision whose candidates AGREE would recover a little over half of ebman's
chained untyped markers. Recorded as a lead, not taken here.

The lazy halves gained nothing on real code, which the diff alone cannot distinguish from never firing,
so the precondition was instrumented instead: the `use`-spelling dependency marker is emitted 28,938 /
19,579 / 6,955 times across the three dependency trees, and the cross-module read fires 16 times on
ebman's (aws-config's `DEFAULT_PARTITION_RESOLVER`, declared in `endpoint_lib` and read from
`config::endpoint`) and once on pgman's (tracing-subscriber's `FILTERING`). Real shapes; those
particular initializers are pure, which is per-static keying working rather than the fix not landing.

Conformance PART 24's ratchet baseline is now EMPTY for rust (both entries deleted — a waiver that
outlives its defect masks the defect's return), and PARTs 18–24 are green four-way.

### soundness — `unverified --class` under-reported holes, and under-reported MORE the more you narrowed

`unverified` names the functions a `pure` / `deny E` layer PASSES without proving anything — the verb
whose whole job is "this gate is green but not provably so". Its `--class <c,…>` filter dropped holes it
was built to surface. Two independent faults, both live, both fixed here (fixing either alone is worse
than fixing neither — see below):

1. **It read the DIRECT-only field.** `unknownWhy` is direct-only by design (SPEC §4: a reason names an
   unresolvable site in the function's *own* body), so a function whose `Unknown` is purely INHERITED
   from a callee carries no reason of its own. The predicate `unknownWhy ∩ filter ≠ ∅` therefore excluded
   every inherited hole from *every* filter, including one naming the class the callee recorded. The
   `deny E Unknown[class]` gate has always resolved this transitively (`reason_class_acc`); the
   disclosure explaining that gate did not.
2. **It failed OPEN under absence.** An entry with an empty reason set matched no filter at all — not
   even `unresolved`. §6.2 says such a function CONTRIBUTES `unresolved`; it now does, per entry, into
   the direct map so the class propagates to callers like any other.

Measured with the §6.2 diagnostic — `--class dynamic` is an alias naming every genuine class, so it must
exclude NOTHING; a filtered count below the unfiltered one is the defect and the gap is its size:

| target | policy | before | after |
|---|---|---|---|
| candor-rust (chained `--deps`) | `deny Exec` | 54 → **26** (−52%) | 54 → 54 |
| candor-rust (chained `--deps`) | self-gate `deny Net Db Exec Ipc` | 54 → **26** (−52%) | 54 → 54 |
| candor-scan (own sources) | `deny Exec` | 7 → **1** (−86%) | 7 → 7 |
| candor-scan (own sources) | `deny Net Db Exec Ipc` | 7 → **1** (−86%) | 7 → 7 |
| ebman | `deny Exec` | 94 → **23** (−76%) | 94 → 94 |
| ebman | `deny Net Db Exec Ipc` | 74 → **22** (−70%) | 74 → 74 |
| pgman | `deny Exec` | 43 → **21** (−51%) | 43 → 43 |
| pgman | `deny Net Db Exec Ipc` | 36 → **19** (−47%) | 36 → 36 |

All eight target × policy rows converge exactly after the fix, matching the shape candor-swift measured
(387 → 230 and 51 → 16 before; both exact after).

Fault 1 was the bulk of it: 101 of 124 `Unknown`-bearing entries on ebman and 37 of 60 on pgman carry no
direct reason at all.

**The filter still DISCRIMINATES** — the control a blanket "keep everything" would fail. Post-fix on the
same rows, `--class unresolved` selects 0, `--class native` 0, `--class reflect` 0, while
`indirect`/`dispatch` partition the totals (ebman 64/37 of 94).

**Fixing only fault 2 would have been worse than fixing neither.** Contributing `unresolved` on the
absence of a reason set fabricates a class for an inherited `Unknown` that the callee classified
perfectly well — a fail-open traded for its mirror. The contribution is therefore gated on `direct ∋
Unknown` with nothing named (the unit INTRODUCED the hole and did not say why), which rust's reports
carry verbatim (§2 `direct`). Both directions are pinned by mutation: dropping the gate turns exactly one
control red ("an inherited, CLASSIFIED hole is not `unresolved`") and nothing else.

`blindspots --class` is deliberately UNCHANGED. It is the *source* view (§3.1) and excludes a unit whose
`Unknown` is purely inherited, so every entry it filters carries a direct reason by construction and the
direct-only read is CORRECT there — resolving transitively would pull in exactly the units the verb is
defined to exclude. Measured to confirm rather than assumed: unfiltered vs `--class dynamic` is 0→0,
1→1, 23→23, 23→23, 26→26 across the five targets. A shared code path is not a shared defect. Checked
too, since §4 forbids it: no rust report can put a reasonless entry into `sources` (the loop skips
`unknownWhy`-empty entries outright), and across 17,306 entries in 173 dependency reports plus this
repo's own five, **zero** entries carry a direct `Unknown` with no reason recorded beside it.

The `propagate_str` fixpoint moved to `candor-classify` and the §6.2 match rule became
`policy::reason_class_matches`, so the gate and the disclosure that explains it now resolve over one
implementation of the reach — two fixpoints that can drift apart is its own defect. Dependency reports
re-scan byte-identical.

### disclosure — the report glob claimed a SIDECAR, then reported its own mistake as data loss

Every query run against a prefix with a `<prefix>.<pkg>.hierarchy.json` sidecar beside it printed:

```
candor: report …/r.app.hierarchy.json failed to parse — its functions are OMITTED from this
query (corrupt or mid-write); re-run the scan
```

The sidecar was not corrupt, nothing was omitted, the results were correct, and the scan the user was
told to re-run was fine. `report_files` discriminated reports from sidecars by SEGMENT COUNT alone —
`<base>.<crate>.<type>.json`, exactly two segments — which excludes this engine's own sidecars (all
three-segment: `<base>.<crate>.<type>.callgraph.json`) but not a two-segment one from another
producer. SPEC §2.2 lets each engine pair a sidecar to its OWN report stem, so `<base>.<pkg>.hierarchy.json`
is a legitimate name that lands exactly on the `<crate>.<type>` shape. Two globs inside one binary
disagreed about the same file: `load_hierarchy` read it as a sidecar while `report_files` claimed it as
a report.

Measured blast radius — 10 verbs (`show`, `where`, `map`, `path`, `impact`, `reachable`, `blindspots`,
`tour`, `containment`, `callers --include-unknown`) plus `audit`, `receipt`, `diff`, `gains`, and the
lint's own cross-crate sibling loader. Both `.hierarchy.json` and `.callgraph.json` trigger it.

**It was not purely cosmetic.** Three consequences, each measured against a control run with the
sidecar removed:

- an **effect-free crate was refused outright**. The bogus parse failure set the `hard_fail` bit that
  `load_entries_loud` uses to tell "this crate genuinely has no effects" from "every report was
  corrupt", so a well-formed `"functions": []` report standing beside a sidecar exited **2** with
  *"refusing to report an empty (all-clear) answer over a corrupt report"*. The query answered nothing.
- **provenance was lost**: `report_build_version` reads the FIRST report by sorted path, and
  `r.app.hierarchy.json` sorts before `r.demo.scan.json`, so `gains --json` reported
  `baseline_version: ""` / `engine_version: ""` where the control gave the real build id — which also
  silences the §2.1 producing-build mismatch disclosure.
- `reports <prefix>` — the canonical "what counts as a report" oracle the wrapper's `--exists` check
  joins on — **listed the sidecars as reports**.

Fixed at the GLOB, not at the parse: `candor_report::SIDECAR_KINDS` names the reserved trailing
segments (`callgraph`, `hierarchy`, `calibrated`, `layerreach`, `locs`, `gate`) and `report_files`
excludes a `<type>` that is one of them, so a sidecar never enters the candidate set and there is
nothing left to diagnose. Suppressing the *message* would have left the file in the set, left all three
consequences above live, and been one refactor from returning.

This is a **denylist over `<type>`, deliberately**. The accept rule still takes any
`<base>.<a>.<b>.json`; only names that are provably not crate types are carved out (`<type>` is a
rustc `CrateType` or another engine's `Swift`/`jar`, never an artifact name). The allowlist inversion —
accept only known types — would make any report whose type segment we failed to anticipate silently
invisible to every query, a false all-clear. A denylist can only be *incomplete*, and incompleteness
here is LOUD: an unregistered sidecar suffix falls back into the candidate set and prints the
disclosure on every query. Noise, never a swallowed report. A crate legitimately NAMED `hierarchy` is
untouched — it sits in the `<crate>` position — and that is pinned as a test row.

The control that makes the fix meaningful: with the same sidecars present, a genuinely corrupt REPORT
must still be disclosed, must name the report and not a sidecar, and must still exit 2. Verified to
discriminate — with the exclusion disabled both tests go red; with the *wrong* fix (the `eprintln!`
deleted) the quiet test passes and only the control fails.

Engine-local. candor-ts (`query-core.mjs` `isReport`), candor-java (`Query.java`) and candor-swift
(`FixCLI.swift`) already exclude these suffixes by name; candor-rust was the one engine discriminating
by segment count.

### soundness ⟨0.24⟩ — the dispatch frontier silently dropped this engine's own dominant reason

`callers --include-unknown` built `possibleViaUnknownDispatch` by splitting each `dispatch:` detail into
an owner and a member on the LAST DOT, then asking condition (3): is some confirmed reacher an override
of `OWNER.M`? candor-scan emits `dispatch:untyped cross-package receiver` when it cannot type the
receiver of a call into a chained dependency — a DOT-FREE detail, and in a 1062-report census EVERY
dispatch reason on this engine was that form. With no dot, `simple_method`/`declaring_type` both fall
back to the whole string, so the lookup could never hit and the entry was **dropped with no diagnostic**.

Measured: a report where `mod.Caller.run` carries `dispatch:mod.Base.handle` and `mod.Dotfree.run`
carries `dispatch:untyped cross-package receiver` produced a frontier containing only `mod.Caller.run`,
in **both** the hierarchy arm and the no-hierarchy fallback arm. That omission is a false all-clear: a
consumer reads absence from `possibleViaUnknownDispatch` as "no function may reach the target through an
unresolved dispatch", which is exactly the claim the engine is not entitled to make.

A dot-free detail names no owner and no member, so condition (3) is **unanswerable — and an unanswerable
condition must not be scored as a failed one**. Such an entry is now disclosed with `viaDispatchOn` set
to the raw detail verbatim. This is the direction the no-hierarchy fallback already takes one rung up
(no sidecar → the subtype test is unanswerable → over-list rather than drop); the frontier over-lists by
construction and asserts nothing into `transitive`, so a spurious entry costs precision while a dropped
one costs the answer. Detected structurally (the detail contains no `.`), never by matching the
scanner's wording — an allowlist of known reason strings silently drops the ones it forgets.

The same whole-string fallback also produced the mirror defect, now closed: a dot-free detail that
happened to equal a confirmed reacher's dot-free Rust qual (`dispatch:app::Sub::handle`) MATCHED, with
the subtype check passing only by reflexivity over a string that is not a type name; and
`dispatch:handle` was disclosed with no hierarchy but dropped with one. Every dot-free detail now takes
the same branch, before the reacher index is consulted at all, so the answer no longer depends on
whether a sidecar happens to exist.

SPEC §2 chaining rule 3 turns a report's SILENCE into a purity claim, and registering its crate as
COVERED is exactly what silences the κ ledger's `invisible` hedge so the silence can be read that
way. A chained report carrying a non-empty ⟨0.21⟩ `unanalyzed` has just said it never read some of
its own source — and candor-scan granted it full coverage anyway, so chaining it was strictly WORSE
than not chaining it: the dependency's own gate refuses to certify itself over unanalyzed code
(`--gate-json` exits 2 for precisely this) and the consumer certified one on its behalf.

Live on crates.io code: `signal-hook-registry` 1.4.8's whole `src/lib.rs` fails to parse, so its
report carries **2 functions and an `unanalyzed` manifest naming the library itself**. Chained,
`signal-hook`'s `PendingSignals::add_signal` — which calls `signal_hook_registry::register_sigaction`,
i.e. installs a signal handler — read as a confident purity claim about that crate. It now carries
`invisible: ['signal_hook_registry']`, and the crate appears in the coverage ledger.

**The treatment differs from staleness, and the difference is the whole point.** A stale report's
entries are assertions from a build this engine will not repeat, so they are downgraded to `Unknown`.
An incomplete report's entries were derived from source it DID read and are true, so they are kept
exactly as they are — effects, literal surfaces, reason classes and all — and only COVERAGE is
withheld. An answered key still answers; only an unanswered one falls back to the hedge. Absent or
explicitly empty `unanalyzed` means COMPLETE (the writer omits the key when the manifest is empty);
anything else, malformed included, fails closed. Announced on stderr, and in `--help`.

Ported from candor-ts `21277eb` (java `d1d3045`, swift `74cd8f1`) — rust was the last engine gating
coverage on staleness alone.

### soundness — the IMPLICIT-STRINGIFICATION vein closed in BOTH backends (cardinal sin)

A formatting site runs the formatted value's `Display`/`Debug` impl. candor analysed those impls
correctly but never edged to them **when the value's type was not a concrete local ADT** — so
`fn describe<T: Display>(e: T) -> String { format!("{e}") }` read **silent-pure** even though it runs
`<T as Display>::fmt`, and in the deep engine even a fully concrete caller (`describe(Loud)`) was a
**false all-clear**. This is the four-way common-mode vein recorded in
`candor-spec/SOUNDNESS-VEIN-implicit-stringify.md` (found on HikariCP by the RQ1 runtime oracle).

Closed on both backends by extending the existing implicit-coercion machinery — no parallel mechanism:

- **candor-scan**: `charge_coercion` now falls through to bounded CHA over the BOUND, for the stringify
  family only (`Display`/`Debug`, plus `ToString` as Display's blanket alias) — a `T: Display` /
  `impl Display` / `&dyn Display` operand, or a LOCAL trait that inherits the formatter as a supertrait
  (`trait Entry: Display`, the narrow precise case). ≤12 local implementors → edges to each `Ty::fmt`;
  wider → honest `Unknown`; **no local implementor → nothing** (no edge, no flood). Every other
  coercion (operators, `Deref`, `Index`, the `write!` writer side) is untouched.
- **candor-scan**: NAMED and INLINE-CAPTURED format holes (`format!("{val}")`, `format!("{v}", v = x)`)
  were skipped outright — they now charge the same coercion. This alone recovered a genuine miss on
  third-party code: `cargo-llvm-cov`'s `ProcessBuilder::run`/`read`/`run_with_output` reported `[Exec]`
  while `format!("… {self}")` runs a `Display` impl that reads the environment → now `[Env, Exec]`.
- **deep engine**: `fmt_argument_local_edge` resolved only a local ADT; a `Param`/`dyn` formatted value
  now takes the same bounded CHA (`fmt_trait_local_cha`), and the generic `.to_string()` spelling is
  covered by `generic_to_string_edge`. Beyond the bound it discloses `Unknown` under a `dispatch:`
  reason class (spec 0.19).

**A/B, zero fabrication.** candor-scan over the whole local registry — **962 crates / 470,971 analyzed
functions**: **0 functions gained a concrete effect** other than the 4 genuine `cargo-llvm-cov`
recoveries, **0 functions lost anything**, 93 gained an honest `Unknown` (0.020%, all at real generic
format sites in formatting-heavy crates: tracing-subscriber, chrono, color-eyre, clap, tokio, winnow,
rustls…). Deep engine over 6 real crates: 0 concrete gains, 0 losses, 6 `Unknown` (all chrono).

## 0.23.1 (spec 0.23) — 2026-07-20

### performance — O(V²) propagation fixpoint replaced with a worklist (no output change)

The transitive effect-propagation fixpoint (`propagate`/`propagate_str`) used a naive
`while changed { for f in all }` sweep whose pass count equals the longest back-to-front call chain — up to
V for a single deep `f0→f1→…→fN` chain, so **O(V²)** on pathological long chains (real crates converge in
2–4 passes, so it never bit realistic code — `wide-4000` is unchanged). Replaced with a worklist over a
callee→callers reverse index: a function is reprocessed only when a callee actually gained an effect. Same
monotone set-union least fixed point → order-independent → **output byte-for-byte identical** (verified via
`--json` + full stdout/stderr across synthetic + real crates; `cargo test --workspace` 334 green, clippy
clean). ~3× on deep-chain corpora; realistic corpora unchanged.

## spec 0.19 — reason-scoped Unknown (2026-07-17) — current floor

Reason-scoped `Unknown` policies (SPEC §6.2): `deny E Unknown[reflect,dispatch,indirect,native,unresolved,setup]`
narrows the `Unknown` part of a deny to a fixed reason-class vocabulary projecting the §4 `unknownWhy` reasons,
with the built-in `dynamic`/`*` aliases and config-defined `.candor/config` `unknown-alias <name> = <class…>`
names. Bare `deny E Unknown` is unchanged (`Unknown[*]`, fires on any); an unrecognized reason maps to
`unresolved` (conservative); the reason class propagates **transitively** along the call graph like the effect.
An AS-EFF-006 `--gate-json` verdict whose `effects` include `Unknown` now carries a **`reasonClass`** array (all
classes on the fn). Report bytes unchanged. Conformance PART 4 (parse + `unknownClasses` + config alias) and
PART 12 (`reasonClass` invariant) pin it four-way.

## spec 0.18 — the trust-trio (2026-07-16)

candor-scan and candor-query now declare **spec `0.18`** (both at crate **0.18.0**). A pinned-tool-surface
rung — no report-schema or verdict change — closing three ways the tool could quietly mislead, each pinned
four-way in the conformance suite:

- **`--strict` advisory-verb CI gate** (§3.3.1): `fix-gate`, `gains`, and `unverified` are advisory (exit 0);
  `--strict` makes each a CI gate (exit 1 while a finding remains). A typo'd flag is rejected loud (exit 2),
  never swallowed into a disarmed gate; `gains` has no `--policy` (a passed one is an exit-2 error naming the
  scan-time `deny <E> gained` gate, `AS-EFF-005`).
- **mostly-Unknown disclosure**: the scan opener and `tour` never print "nothing hidden" over a ≥⅓-Unknown
  graph — they qualify + point at `blindspots`; `tour --json` carries an additive `unknown: {count, total}`.
- **hardening from a Fable-model code review**: gains rejects single-dash typos + tolerates cross-engine
  `--text`; a valueless `--policy` exits 2.

## spec 0.17 — the callgraph-aware baseline guard (2026-07-16)

candor-scan and candor-query now declare **spec `0.16`** (both at crate **0.16.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.16.0**). **0.16 is the current spec
floor** — the ratchet from 0.15. It sharpens the scan-time baseline ratchet so the hardest
supply-chain shape can no longer slip through as "new code".

### 🕸️ callgraph-aware baseline guard ⟨0.16⟩ — pure→effectful is caught

The scan-time ratchet (`candor-scan --gate --baseline`) now keys function EXISTENCE on the baseline
**callgraph sidecar** when present (the resolved report path with `.json` swapped for
`.callgraph.json`; SPEC §2.2 records every analyzed fn, including PURE leaves that reports omit). A fn
that is a baseline callgraph node — even a **baseline-pure leaf** with an empty effect set — that now
performs ANY effect is a **GAIN violation** (exit 1). This closes the report-only blind spot where a
**formerly-pure function turning effectful** was absent from the baseline report and so read as exempt
"new code". A fn genuinely absent from the callgraph stays exempt (real new code). This is the `gains`
`origin` existence rule (§3.1 ⟨0.12⟩) applied to the scan ratchet. When the sidecar is **absent** the
guard degrades to the pre-0.16 report-only existence (with a one-time stderr note that it is weaker);
a **corrupt** sidecar is `Invalid` (exit 2), never a silent narrowing.

### ⚠️ Unknown-only gain is advisory, not exit 1 ⟨0.16⟩

A baseline→current gain consisting **only of `Unknown`** (no real §6 boundary effect gained) is now an
**advisory**, not a hard violation: the ratchet fires only on gaining a REAL boundary effect. An
Unknown-only widening surfaces as a disclosed advisory (verdict-preserving), keeping the gate focused
on genuine supply-chain effect gains rather than resolution noise.

## spec 0.15 — the coverage envelope (2026-07-15)

candor-scan and candor-query now declare **spec `0.15`** (both at crate **0.15.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.15.0**). **0.15 is the current spec
floor** — the ratchet from 0.14, and unlike 0.14 it is a **feature rung for this engine**: the
coverage envelope, two host-resolution recall upgrades, and three soundness fixes from real-world
corpus testing.

### 📦 the coverage envelope ⟨0.15⟩ — the κ ledger travels WITH the report

The κ-coverage ledger is no longer stderr-only: a scan with an uncovered dependency now emits the
§2 **`coverage` envelope field** — `{"uncovered":[{"name","calls"}]}` — **omitted when empty**, so a
fully-covered scan's report stays **byte-identical** with a ⟨0.14⟩ one (the wire-compatibility rule).
One shared ledger computation feeds all three surfaces: the stderr line (bytes unchanged), the
envelope, and the **`--gate-json` coverage advisory** (`{"uncovered":N,"packages":[…]}`) — which is
**verdict-preserving**: `ok` / `violations` / the exit code are untouched (the ⟨0.9⟩ advisory
precedent). `gains --json` re-discloses the current ledger and adds **`coverageDelta`**
(`{nowUncovered, noLongerUncovered}`, names-only — the java reference shape); the TSV output is
untouched. `candor-query gate-verdict` gains an optional `--report` as the advisory source. Pinned by
conformance PART 4s.

### 🔎 host-resolution recall — statically-known hosts

Two upgrades let a **statically-known host** run the §1 Llm/Db/Net host refinement exactly like an
inline literal (previously bare `Net`):

- **const-string propagation** — a URL/host built from a literal-valued `const`/`static`
  (`const API_BASE = "…"; format!("{}/x", API_BASE)` — interpolation, bare ref, or const-left
  concat), matching candor-java's inlined static-final String. The const index is folded into the
  decl-cache digest, so incremental scans stay correct. PART 4q.
- **literal-head extraction** — a `format!` whose literal completes the authority before the first
  placeholder (`format!("https://api.openai.com/v1/{}", p)`).

Both keep the conservative boundary: a split authority, whole-host placeholder, runtime/config host,
or plain variable stays bare `Net` — no fabrication. PARTs 4q/4r.

### 🧯 soundness fixes — real-world corpus testing finds

Three silent-drop classes found by real-world corpus testing, each recovering real effects with
**zero fabrication** (the 1337-crate realworld-oracle stays green; clap/ripgrep byte-identical):

- **glob-reexport / `use crate::` rebind** — a cross-crate effectful call reached via
  `use x::prelude::*` or a `use crate::name` submodule rebind read pure and undisclosed, even under
  `--deps` chaining (real hit: sqlx-postgres's TCP dial to Postgres — `PgListener::connect` now
  discloses `Net`, `begin` discloses `Db`, and both chain).
- **`cfg_if!` macro arms** — effects inside a `cfg_if::cfg_if!` arm were dropped as a misleading
  "uncovered" ledger entry; the arm grammar is now parsed and every arm walked (all-arm
  over-approximation, like the existing cfg-branch handling). Recovers sqlx-core's `connect_tcp` →
  `Net`; a non-conforming shape falls back to the opaque path, never panics.
- **block-nested `use` bindings** — a `use path::X` inside a nested block (if/else arm, loop body)
  was not collected, so a call through it resolved to nothing → pure. The whole body tree is now
  walked, with a scope guard so an inner fn's imports don't leak. Recovers fd's gls-check → `Exec`.

## spec 0.14 — floor alignment (2026-07-14)

candor-scan and candor-query now declare **spec `0.14`** (both at crate **0.14.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.14.0**). **0.14 is the current spec
floor** — the ratchet from 0.13. This is a **declared-version alignment only**: reports and
`--gate-json` verdicts are **byte-identical** with 0.13 — there is no engine-local behaviour change.

The ⟨0.14⟩ rung is a **top-level-initializer fix in candor-ts / candor-swift**: those engines were
dropping a module's top-level effects as false-pure (the `<module>` / `<main>` initializer unit went
unattributed). **Rust has no top-level executable code** — a `const` / `static` must be
const-evaluable, so nothing runs at module load to attribute — so the rung is **N/A** for this engine.
candor-scan declares 0.14 purely to keep the family floor uniform; see the candor-spec 0.14 entry for
the contract change.

## spec 0.13 — the Llm effect (2026-07-14)

candor-scan and candor-query now declare **spec `0.13`** (both at crate **0.13.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.13.0**). **0.13 is the current spec
floor** — the ratchet from 0.12. The report schema is unchanged (`Llm` is a value in the existing
effect set), but a report that previously read `Net` on a model-provider call now reads `Llm` — the
new precision the bump pins into the contract.

### 🤖 the `Llm` effect — a model-provider call, refining Net

A call to a machine-learning model provider is now its own boundary effect, **`Llm`**, refining the
broad `Net` the same way `Db` does (both engines — the stable scanner and the deep dylint). It fires
on two signals: a **known model-host literal** (the `api.anthropic.com`-class hosts in the shared
`MODEL_HOSTS` table, verbatim from the java reference — a loopback Ollama `:11434` counts, an
arbitrary host on that port does not), OR a **model-SDK crate** (a curated list — `async-openai`,
`aws-sdk-bedrockruntime`, `ollama-rs`, …). A model call always carries `Net` alongside `Llm`, so it
never evades a `Net` gate; `Llm` sits in the boundary / ambient / injection / salience sets and takes
its own `deny`/`allow` gate, with a masked model host failing closed. The host classifier is tightened
to match the reference across engines (Bedrock keys off the first-label service, not a bare `bedrock`
substring that caught an S3 bucket).

### 🔗 the reqwest builder-chain Net gap, fixed alongside

The `Client::builder()…post(url).send()` builder chain was not being read as `Net`, so a real
model call made through it was seen as `Env`-only — a claimed-covered crate dropping its dominant
idiom. The chain is now classified `Net` with the host captured, so `Llm` fires on the model hosts
reached through it (verified on a real `api.anthropic.com` call).

## spec 0.12 — the gains origin field (2026-07-14)

candor-scan and candor-query now declare **spec `0.12`** (both at crate **0.12.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.12.0**). **0.12 is the current spec
floor** — the ratchet from 0.11. No report-schema or verdict change — a 0.11 report and a 0.11
`--gate-json` verdict are byte-identical under 0.12 — the bump pins the gains `origin` field and the
comparative-verb loud-fail completion into the contract.

### 🧬 gains `--json` carries `origin` — existing | new | unknown

Every gained effect in `candor gains --json` now says whether the function **existed at the
baseline**: a fn that shipped pure and now does Net (the supply-chain attack signal) is a different
alarm from a brand-new fn that does Net (a feature). Reports omit pure functions (§2), so existence
is keyed on the **baseline callgraph** sidecar (caller keys + callees), and the ladder never guesses:
baseline report/graph hit → `existing`; no sidecar or a **partial** graph (a disclosed-and-dropped
corrupt file) → `unknown` — never a downgrade to `new`. JSON-only: the human TSV is a pinned consumer
surface (the `candor-run.sh` seen-file dedup) and stays byte-stable; `byFunction` keys are emitted
alphabetically (`effect`, `fn`, `origin`). The JSON also carries **`baseline_version` /
`engine_version`** provenance (envelope `candor.version`, `meta.version` fallback, empty when
unknown) plus the §2.1 version-mismatch ⚠ stderr disclosure. Pinned four-way by conformance
**PART 5b**.

### 🔊 the comparative verbs complete the loud-fail rule

`gains`, `diff`, and `containment` loaded reports through a quiet path, so a FOUND-but-corrupt
report yielded an exit-0 empty answer — a supply-chain all-clear over corrupt input, the same §4
cardinal sin the 0.11 rung closed for the single-report verbs. The loud rule now covers **both
locators** of the comparatives (and containment's current AND baseline): found-but-corrupt →
**exit 2** with a disclosure; a well-formed empty report is still valid. The quiet loader is
deleted, so no future verb can reach for it. (The plural-`packages` `tour` header shipped inside
0.11.0 and is recorded under the 0.11 entry below.)

## spec 0.11 — the surprising-reach surface (2026-07-13)

candor-scan and candor-query now declare **spec `0.11`** (both at crate **0.11.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.11.0**). **0.11 is the current spec
floor** — the ratchet from 0.10. No report-schema or verdict change — a 0.10 report and a 0.10
`--gate-json` verdict are byte-identical under 0.11 — the bump pins the surprising-reach surface and the
corrupt-report loud-fail rule into the contract.

### ✨ the surprising-reach surface — scan-time opener + `tour` top-N + `candor path` suggestions

After the effect summary and coverage ledger, a scan emits ONE more stderr line: the single most
surprising transitive reach — a benign-named function (settings/config/util/load/…) inheriting a
boundary effect from a few hops away — with a ready-to-run **`candor path <fn> <effect>`** that
re-derives the chain, so a find is never wrong. **`candor tour [N]`** (default 10) lists the top-N
such reaches on demand from an existing report — no re-scan — with `--json` and the §3.3.1 grammar;
wired into `cargo candor tour`. The ranking is **deterministic** (pure call-graph + name analysis, no
LLM) and lives in one shared place, so the scan-time note and `tour` can't drift (conformance
**PART 4f**). Two calibration rules keep the opener from over-promising: a **salience floor** —
Clock/Log/Rand reaches never surface as "surprising" (**PART 4j**) — and test code is excluded by
whole module segment. A repo whose reaches are all mundane gets a plain "nothing hidden" line, never a
manufactured find.

### 🔊 a corrupt report fails loud — never an empty all-clear

A report that is FOUND but unparseable — or that parses as JSON of the wrong shape, a bare junk array
with every entry dropped — now **fails loud with exit 2** and a disclosure on every loud-consuming
verb. Previously it degraded to an empty entry list: `tour` printed "nothing hidden" and a policy
map/gate over the empty report would PASS — the §4 cardinal sin over corrupt input. A well-formed
`functions: []` report (a genuinely effect-free crate) is **still valid** and still exits 0, and one
corrupt file among several still yields the others (disclosed). Pinned four-way by conformance
**PART 4k**.

### 🏷 tour header — the plural `packages` envelope

The `tour` header honours the plural `packages` envelope (SPEC §2, the JVM shape): one package
verbatim, several by their longest common dotted prefix (whole segments only), basename fallback when
none is shared (conformance 4g addendum). 0.11.0 is also the first tagged build carrying the coverage
ledger's plain-English marker — **`classifier doesn't cover`** (was the internal `κ`; SPEC §7 item 14,
PART 4c) — recorded in detail under the 0.10 entry below, where it landed post-tag.

## spec 0.10 — the canonical query grammar (2026-07-12)

candor-scan and candor-query now declare **spec `0.10`** (both at crate **0.10.0**; the internal
**candor-report** and **candor-classify** libs move lockstep to **0.10.0**). **0.10 is the current spec
floor** — the ratchet from 0.9. This is a **tier-2 (pinned-tool-surface) rung** (candor-spec §"Conformance
tiers"): no report-schema or verdict change — a 0.9 report and a 0.9 `--gate-json` verdict are byte-identical
under 0.10 — the bump promotes the new §3.3.1 query grammar into the pinned contract.

### ✨ §3.3.1 canonical query grammar — report discovery + explicit `--report` / `--json` / `--policy` flags

The query surface gains a single canonical grammar (candor-spec §3.3.1): a query auto-**discovers** the
report (the `.candor/report*.json` in scope) so the report path no longer has to be spelled out, and the
inputs are named by explicit flags — **`--report <path>`**, **`--json`** (machine output), and
**`--policy <path>`** — rather than by argument position. The **old positional forms are deprecated but
still accepted** (a deprecation note on stderr; they parse and run exactly as before), so existing scripts
and the conformance goldens keep working unchanged. Pinned by the cross-engine conformance suite as
**PART 17**. No behavioural change to classification, the report schema, or the gate verdict.

### 🔤 coverage-ledger wording — drop the internal `κ` from user- and agent-facing output

The coverage-ledger line and the `--help`/README wording that mentioned the internal classifier shorthand
`κ` now read in plain English — nobody outside the maintainers can decode the Greek letter, and it was the
first thing a cold user met in scan output. The ledger line is now
`candor-scan: candor's classifier doesn't cover N dependencies this code calls into — their effects are
INVISIBLE to the scan (absent from the report, NOT a claim they're pure): …`, and the **stable greppable
marker** every engine shares is **`classifier doesn't cover`** (was `κ doesn't know`). `κ` stays only as
internal maintainer vocabulary (code identifiers, these history entries). No behavioural or schema change.

## spec 0.9 — the remedial-loop rung (2026-07-11)

candor-scan and candor-query now declare **spec `0.9`** (both at crate **0.9.0**). The internal library
crates **candor-report** and **candor-classify** are **aligned to `0.9.0`** too (from 0.5.8 / 0.5.9): every
candor crate now shares the toolchain version, so a crates.io visitor doesn't read the schema/classifier
libs as lagging the engines. They carry no external-consumer contract, so the one-time 0.5→0.9 jump costs
nothing; from here they move lockstep with the spec. 0.9 is a **tier-2 (pinned-tool-surface) rung** (candor-spec §"Conformance tiers"):
no report-schema or verdict change — a 0.8 report and a 0.8 `--gate-json` verdict are byte-identical under
0.9 — but the remedial tool loop (`fix`/`fix-gate`, `unverified`, and the gate's provable-purity
auto-disclosure, all detailed below) is promoted into the pinned §3.1/§3.3 contract. `SPEC_VERSION` is
`"0.9"` in candor-report; the envelope and `--gate-json` verdict declare it.

## [candor-scan 0.9.0] — 2026-07-11

### ✨ Gate scans auto-disclose the provable-purity gap (no need to know to run `unverified`)

A `candor-scan --policy` run now emits the `unverified` disclosure automatically as a stderr note: after the
gate verdict, any function in a `pure`/`deny <E>` scope that PASSES but is `Unknown` (an unresolvable call — the
classic fn/closure-injected "port") is named, with the `deny <E> Unknown <scope>` upgrade that would make the
layer PROVABLY clean. This closes the discovery gap — an author learns their "pure" layer isn't *provably* pure
without having to know the `unverified` subcommand exists. **Advisory only**: it's a note, never a violation, so
the exit code, the gate verdict, and `--gate-json` are all untouched. New `gate::unverified_holes` helper;
emitted from `scan.rs` after `record_gate_violations`. Mirrors the port to candor-java/ts/swift (four-engine
parity). Existing gate tests unchanged (115 pass). The gate note and `candor unverified` share ONE predicate
(`candor_classify::policy::unverified_hole_rule` + `rule_and_upgrade`, candor-classify 0.9.0) — a single
definition of "what a hole is", so the scan-path and query-path disclosures cannot drift (PART 12d pins it).

## [candor-query 0.9.0] — 2026-07-11

### ✨ `unverified` — the provable-purity disclosure (policy guidance from the fix-loop investigation)

New subcommand `unverified <prefix> [policy] [--strict]`. A `pure`/`deny <E>` layer PASSES a function that has
no such effect — but if that function is `Unknown` (candor couldn't resolve one of its calls), the pass is
UNVERIFIED: the Unknown could hide the very effect the rule forbids. The classic case is a fn/closure-injected
"port" — the domain reads as Unknown, so `deny Net domain`/`pure domain` clear it though it may reach Net at
runtime (eval/fixloop/DISPATCH-NOTE.md). `unverified` names every such function in a governed layer, its
`unknownWhy`, and the `deny <E> Unknown <scope>` upgrade that makes the layer PROVABLY clean. Advisory (exit 0);
`--strict` → exit 1 so CI can require provable purity. Text or `--json {ok, unverified[]}`. The gate verdict is
untouched — this only discloses the gap. Wired into `cargo candor unverified` + the `candor_unverified` MCP
tool. 2 regression tests. Rust-only for now; a java/ts/swift port is a natural follow-on.

## [candor-query 0.8.9] — 2026-07-11

### `fix`: the no-clean-hoist advice names the port purity hierarchy (soundness investigation)

Following the fix-loop eval's finding that models reach for a TRAIT port (which candor's gate rejects — it
resolves the dispatch back to the effect-performing impl), an empirical investigation (eval/fixloop/DISPATCH-
NOTE.md) confirmed candor's behaviour is CORRECT (accepting a trait port would silently under-report the effect
the layer reaches at runtime — the cardinal sin), and pinned the three fix shapes' distinct classifications:
trait dispatch → the effect (resolved); fn/closure value → Unknown; plain data → pure. The no-clean-hoist
advice now names the hierarchy: (a) hoist + thread DATA = provably pure (recommended); (b) fn/closure injection
clears `deny E` but leaves an Unknown hole a `deny E Unknown` policy would flag; (c) a trait port doesn't clear
the gate. Text-only; no gate change (the resolution is sound). A candor-scan test guards the classification.

## [candor-query 0.8.8] — 2026-07-11

### `fix`: no-clean-hoist advice rewritten (eval-driven — the remedy was steering agents wrong)

The fix-loop eval (candor-rust/eval/fixloop) measured that on the no-clean-hoist case candor's remedy did NOT
help and HURT weaker models (fable 60% vs control 100%): agents followed the literal "introduce a PORT (a
trait)" advice and wrote a trait port, which candor's OWN gate then rejected — it resolves the trait dispatch
back to the effect-performing impl, so the layer still violates. And "NO CLEAN HOIST" was computed on the
existing graph, so it wrongly declared impossible the simplest valid fix (add a thin composition root above
the layer). The advice now (a) LEADS with the composition-root hoist, and (b) recommends fn/closure injection
with candor's trait-dispatch caveat ("a trait port whose impl performs the effect still trips the gate").
Text-only (the cut/JSON is unchanged; conformance PART 12b still MATCHES). Re-running the eval: the fixed
remedy recovers the treatment arm to 100% across all four models (fable 60% → 100%). See eval/fixloop/RESULTS.md.

## [candor-query 0.8.7] — 2026-07-11

### `fix`: the sandwiched-layer case is now handled (last correctness gap closed)

When an ALLOWED layer is CALLED BY a forbidden one (`D1 → A → D2 → site`, deny on the D layer), hoisting the
effect to the nearest allowed frontier `A` would leave `D1` still inheriting it. `cleanHoist` is now `false`
in that case (a forbidden fn calls into the frontier), with a message that names the sandwich and offers the
port/relax options — instead of a misleading "hoist to A". Detected in the same upward climb that gathers
`hoistHigher`; identical across all four engines, pinned four-way by conformance PART 12b's sandwiched
sub-check. Read-only; additive.

## [candor-query 0.8.6] — 2026-07-11

### `fix`: cross-engine parity fixes (from a high-effort /code-review)

- **Start resolution** now prefers a name match that PERFORMS the effect (so `fix save Net` resolves to the
  effectful `Repo.save`, not a pure `Cache.save` that sorts first) — matching candor-ts/swift. Previously
  Rust/Java could give a false "nothing to hoist" all-clear while the other engines emitted the real fix.
- **`byName`-absent caller** in the up-walk is now skipped (a pure callgraph-only node never routes the
  effect), instead of being classified into the span/hoist — matching candor-swift; avoids naming a pure
  node as a hoist target.
- **`fix-gate` determinism**: functions/effects are iterated in sorted order, so the collapsed remedy's `fn`
  representative is deterministic across engines (the remedy set + order were already dedup-key-sorted).
- **`cargo candor fix`** now resolves the policy the way the `policy` command does (`CANDOR_POLICY` →
  `.candor/config` → `.candor/policy`), so the MCP `candor_fix` tool works zero-config in a repo that checks
  its policy into `.candor/config` — where it previously failed "policy required" though `candor_gate` worked.

## [candor-query 0.8.5] — 2026-07-11

### `fix`/`fix-gate`: the higher-hoist trade-off (FIX-SPEC's last refinement)

Each remedy now carries `hoistHigher` alongside `hoistTo`: the allowed-layer transitive callers of the
minimal frontier that also route the effect — every place you could originate it *further up*. The text
surfaces the trade-off (hoisting higher keeps the frontier pure too, at the cost of threading the value
through more signatures). `hoistTo` (the minimal fix) is unchanged. All four engines compute it identically,
pinned by candor-spec conformance PART 12b (the leaf-normalized remedy tuple now includes it). Read-only,
additive JSON field.

## [candor-query 0.8.4] — 2026-07-11

### `fix`/`fix-gate`: the pure span is now site-anchored (root-independent)

The remedy's `deniedSpan` (the forbidden-layer functions that must become pure) was computed as the
caller-closure from the querying function, so in `fix-gate` — where many inheritors of one crossing collapse
to a single plan — which inheritor won the dedup changed the span (a higher inheritor's closure omitted the
functions *below* it). The cut now anchors on the direct site and walks UP through the denied layer, so the
span is the complete set of forbidden-layer functions between the site and the hoist frontier, identical
whichever function triggered it. Same fix applied to the candor-java port (0.8.8) — the two engines share the
algorithm byte-for-byte. Regression tests unchanged (they already asserted the complete span).

## [candor-query 0.8.3] — 2026-07-11

### ✨ `candor-query fix-gate` + the edit-time loop hands the agent the FIX, not just the finding

New subcommand `fix-gate <prefix> [policy] [0|1]`: a remedy for EVERY deny/`pure` (AS-EFF-006) boundary
crossing in a report, not just one function. It reuses `fix`'s cut (now extracted to `compute_remedy`) and
**collapses the inheritors of one root cause to a single plan** — a `deny Net domain` that trips five domain
functions yields one remedy (one site, one hoist target), keyed by `(effect, layer, site, hoist)`. Text
prints the plan block(s); `--json` emits `{ok, remedies:[…]}`.

`integrations/claude-code/candor-review-source.sh` now folds that plan into the block message: when the
edit-time gate fails (an `AS-EFF` finding) and a policy is set, the loop calls `candor-query fix-gate` and
appends the remedy under the finding — so the agent self-corrects toward the *right* architecture instead of
guessing (adding `allow Net domain`, shuffling the I/O one call up, or threading a client handle the wrong
way). Graceful no-op when `candor-query` is absent or can't read the engine's report shape (`CANDOR_QUERY`
overrides the binary; today the candor-scan report — ts/swift/java remedies are FIX-SPEC P3). This is P2 of
the fix capability. Three regression tests pin the collapse, the clean case, and the fail-loud contract.

## [candor-query 0.8.2] — 2026-07-11

### ✨ `candor-query fix` — the boundary fix (the remedial inverse of `whatif`)

New subcommand: `fix <prefix> <fn> <Effect> [policy] [0|1]`. When an edit makes a function perform an effect
its architecture layer forbids, `whatif` reports the violation; `fix` computes the *architectural remedy* —
where the effect belongs and the smallest refactor that puts it there. Deterministic graph-plus-policy cut
over what the report already holds (integrations/FIX-SPEC.md): the direct **site** (BFS through the effect-
carrying subgraph to the direct source), the **pure span** (the forbidden-layer functions that must thread
the value), and the **hoist frontier** (the nearest allowed-layer caller to perform the effect). Emits a
plan (text or `--json`: `{fn, effect, layer, site, deniedSpan, hoistTo, policyAlternative, cleanHoist}`) and
always offers the policy-relax alternative (`allow <Effect> <scope>`). No clean hoist (every caller up to the
entry is also forbidden) → the two honest options (introduce a port / relax the boundary), never an invented
target. Advisory only: candor names the structure, not the syntax; the gate re-scan stays the ground truth
(a bad suggestion blocks again). `denied_layer` mirrors `whatif`'s violation predicate exactly; fail-loud on
an unreadable/absent policy (exit 2). Six regression tests pin the worked example + every branch. This is P1
of the fix capability; P2 folds a `remedy` field into `--gate-json` + the agent-loop block message.

## [candor-scan 0.8.8] — 2026-07-11

### ⚠ candor-scan: a trait DEFAULT method via an empty impl now charges (soundness R30 — report-affecting)

A trait's provided (default) method reached through an EMPTY `impl Trait for T {}` read silent-pure —
`impl Logger for FileLogger {}` + `l.flush()` inheriting `Logger::flush`'s effect was dropped. The
caller-fallback that edges `t.m()` → the inherited `Trait::m` default body already existed, but a type whose
ONLY impl is an (empty or non-overriding) trait impl has no fn unit of its OWN, so it never entered
`local_types` — which made its typed call un-resolvable and GATED OUT the fallback. Fix: register every type
with a local trait impl as local. An OVERRIDE still wins (only the override's effect flows; the default is
not also charged — no fabrication); a pure default stays pure. Found by an autonomous cross-engine soundness
sweep; gated by `trait_default_method_via_empty_impl_charges_the_default_body`.

## [candor-scan 0.8.7] — 2026-07-10

### ⚠ candor-scan: a bounded-generic struct field now dispatches (soundness R31 — report-affecting)

A stored field typed as the STRUCT's own bounded generic param — `struct Pipe<T: Saver> { item: T }`
reaching `self.item.save()` — read silent-pure: field types were resolved with an EMPTY generic-bounds
map, so `T` never resolved to its `Saver` bound and `item.save()` never dispatched to the trait's
implementors. Now the struct's own `<T: Bound>` (and `where T: Bound`) seeds the field's trait leaves, so
the existing dispatch-typed-field CHA fires. An unconstrained-generic field read (no method call) stays
pure (no fabrication). Found by an autonomous cross-engine soundness sweep (the swift R27 analog); gated by
`generic_struct_field_resolves_to_its_trait_bound_dispatch`. (Known open: R30 — a trait DEFAULT method used
via an empty `impl Trait for T {}` still reads pure; tracked in candor-spec SOUNDNESS.md.)

## [candor-scan 0.8.6] — 2026-07-10

### ⚠ candor-scan: the `baseline` config key now ACTIVATES the AS-EFF-005 guard (spec §7 item 5)

The stable scanner implements the family-MUST baseline regression guard, with candor-java's
`checkBaseline` as the exact model: `CANDOR_BASELINE=<saved report path or --out prefix>` (or the
`.candor/config` `baseline` key — **previously recognized-but-inert with a loud warning; a repo that
already checked one in gets a LIVE ratchet on its next scan**). Semantics: an *existing* function
that gained an effect vs its baseline transitive set → one `[AS-EFF-005]` violation per fn + exit 1,
joined into the `--gate-json` verdict by the same accumulator as the policy gate; new fns exempt
(regressions in existing code only); baseline file absent → one stderr note ("guard not active;
record one: candor-scan <dir> --out <prefix>") + exit unchanged; baseline unparseable (incl. any
dropped entry), missing its provenance header, or produced by a **different scanner build** (envelope
`candor.version` vs this build) → exit 2 WITHOUT evaluating (the §2.1 stale-baseline posture — never
a silent skip, never a stale compare); a configured-but-EMPTY value → exit 2 (the bare-`policy`
posture); a guard over a crate with an unparseable source file → exit 2. Dependency scans under
`--deps` stay guard-free. Same advisory-floor caveat as the scan policy gate.

### Docs — reference-engine attribution, Path A `Unknown` claims, status refresh

README/AGENTS corrections (with a new in-repo grep gate + a behavioral pin holding them): candor-java
is the family's REFERENCE engine (this repo is the deep Rust engine — the README claimed "the
reference implementation"); Path A/candor-scan does emit `Unknown` for invoked fn-values, FFI
`extern` calls and untrusted chained reports (AGENTS.md and the scan docs claimed it "never emits
`Unknown`" — other misses remain silent by design, now stated precisely); the status section now
reflects the 35-crate calibration/fuzzer/oracle reality and spec 0.8 (it still said "Prototype,
validated on ebman"); the conformance blurb counts four code engines + the agents domain engine;
stale version examples became placeholders.

## [candor-scan 0.8.5 · candor-query 0.8.1] — 2026-07-10

### ⚠ `pure` no longer counts `Unknown` as a violation (family ruling — verdict-affecting)

An Unknown-only function no longer trips a `pure` rule in candor-scan: `Unknown` is the §4 trust
marker, not an effect — §6.2's `pure` forbids every EFFECT, AS-EFF-003 owns the uncertainty
residual, and **`deny Unknown <scope>`** is the explicit knob (it keeps firing, effects
`["Unknown"]`). The reference engine (candor-java) and the rust deep engine already read the
predicate this way; candor-scan (with candor-ts and candor-swift, fixed in their repos) was
counting the marker — a cross-engine verdict split on the same policy file. Pinned four-way by
conformance PART 16.

### ⚠ Deep-engine `deny <Effect>` no longer fires on `Unknown` (SEMANTICS §6 alignment, family ruling)

The nightly gate folded `Unknown` into EVERY `deny` projection: a function whose only marker was
`Unknown` (an unresolvable call) tripped `deny Net` — a **false-positive divergence** from the
family. The reference engine (candor-java), candor-scan and candor-ts all implement the SEMANTICS §6
predicate exactly: AS-EFF-006 fires iff `I(f) ∩ Forbidden(r) ≠ ∅` — an effect PROVABLY in the
transitive set. The deep engine now agrees: `deny X` fires only when `X` is provably in `I(f)`, and
`Unknown` never appears in a deny-X verdict's effects. **The strictness knob is explicit**: where a
boundary must also exclude uncertainty, pair it with **`deny Unknown <scope>`** (the §6.2 grammar's
denyable token — it keeps firing, with effects `["Unknown"]`). A `pure` rule likewise forbids every
*real* effect but not the `Unknown` visibility marker (AS-EFF-003's concern), matching the reference
engine. `candor-query whatif` mirrors the same projection, so the pre-edit verdict cannot diverge
from the gate.

### Internal — candor-query main.rs split into per-command-family modules (byte-identity gated)

The 2.9k-line `crates/candor-query/src/main.rs` is now 9 modules (load / matching / audit / show /
callers / policy / diff / containment / state + `tests.rs`), flat namespace preserved via
`pub(crate)` re-exports. Gated by a 37-run command battery against fixed deep+scan reports
(stdout/stderr/exit codes byte-identical).

### Internal — deep-engine mega-fns decomposed (no behavior change, byte-identity gated)

`check_expr` (~615 lines) and `check_crate_post` (~795 lines) in src/lib.rs each carried a dozen
concerns inline. Twelve per-concern helper methods extracted (value-reference/static-force/
thread_local/deref edge probes, callback flow both sides, unresolved-dispatch disclosure, resolved-
call recording, explain, layering, report emission, gate verdict) — bodies moved verbatim. Gated by
a deep-engine byte-identity battery (reports, sidecars, ledgers, violations sentinels, gate
verdicts, AS-EFF diagnostics — identical modulo the git build-id stamp).

### Internal — candor-scan main.rs split into modules (no behavior change, byte-identity gated)

The 7.9k-line `crates/candor-scan/src/main.rs` is now 12 modules along its natural seams
(model / lang / lazy / deps / collector / decls / cache / config / gate / propagate / scan + a
`tests.rs` file module). One flat crate namespace is preserved via `pub(crate) use` re-exports, so
no call site changed. Gated by a byte-identity battery (conformance fixtures incl. the gate/policy
runs, sample crates, a workspace root, minreq/fs_extra/xshell from crates.io — reports, sidecars,
gate verdicts, stdout/stderr and exit codes all identical) plus the 120-edit incremental-cache
equivalence run on tokio.

### Added — the coverage wave's pins (tests only, no behavior change)

- candor-scan: the `--deps` registry-tree mode is covered end-to-end for the first time (hermetic
  fake registry — discovery, the documented `.candor/deps/<name>@<version>/` layout, effect + literal
  surface crossing the chain, policy-free dep scans, lockless exit 2, cache reuse); the nested
  `cfg(all/any/not)` evaluator, `push_quoted`, `is_non_nominal_type` and tuple-destructured binding
  are unit-pinned (with an anti-fabrication rebind twin).
- candor-query: in-repo pins for the previously conformance-only arms — `callers --include-unknown`
  (hierarchy-gated frontier), `blindspots` (ranking + blast radius), `rewire` (exit contract),
  `locate` — so an engine-local regression fails this repo's own CI, not just the spec repo's.

## [candor-scan 0.8.4 · candor-query 0.8.0 · candor-report 0.5.8] — 2026-07-09

### ⚠ Fail-open paths that used to pass GREEN now exit 2 — intentional bug-fix semantics

If your CI went red on one of these after updating the clone, the gate was previously **not
running** and telling you it passed. Exit 2 always means "the gate could NOT evaluate", never a
violation (that's exit 1):

- **`cargo candor policy` on a run that couldn't complete.** A crate that failed to build under
  dylint (or the engine's own §6.2 unreadable-policy exit) was swallowed by `|| true` and printed
  "policy OK" with exit 0. Now: no report snapshot → exit 2 before enforcing (a snapshot-less
  enforce also silently dropped cross-crate `allow` resolution); a nonzero dylint exit → exit 2
  ("policy NOT evaluated").
- **`cargo candor guard` with no baseline at all** (never snapshotted / typo'd prefix) exits 2 with
  the snapshot incantation — the engine used to warn "guard NOT active" and exit 0. A **per-crate
  baseline gap** (a new workspace member) is disclosed by the engine as a `GUARD-UNAVAILABLE`
  sentinel and also exits 2. (Completes the fail-closed arc started by the stale-baseline and
  absent-provenance-sidecar fixes of 2026-07-08.)
- **A configured-but-EMPTY `policy`** (a bare `policy` line in `.candor/config`) exits 2 — never a
  silently skipped gate.

### Changed — `CANDOR_CONFIG` → `CANDOR_RULES` (deep engine, clean rename, NO fallback)

The lint's classifier-extension rules file is now pointed at by **`CANDOR_RULES`**. `CANDOR_CONFIG`
now means one thing family-wide: the spec-§3.4 config-file override path (which candor-scan already
implemented). One variable meaning two incompatible things was worse than a break.

### Added

- **`.candor/config` on the deep path** (spec §3.4): the wrapper discovers the checked-in config
  (walk up from the project; `$CANDOR_CONFIG` overrides), wires `policy`/`baseline`/`deps`, warns on
  unknown keys, exits 2 on a configured-but-unusable file, and DISCLOSES recognized-but-unwired keys.
  Relative path values anchor to the config's HOME directory (the one containing `.candor/`) in both
  the wrapper and candor-scan — never the process CWD.
- **`--gate-json` on the deep path** (spec §3.3): `cargo candor policy|guard --gate-json <path|->`
  emit the structured verdict `{ spec, ok, violations }` — same shape, field names and byte layout as
  candor-scan's (one shared `candor_report::GateViolation` + serializer, candor-report **0.5.8**),
  assembled from the engine's `CANDOR_GATE_JSON` NDJSON records by the new
  `candor-query gate-verdict`. Violation → exit 1; incomplete/unwritable verdict → exit 2, file
  removed, never a stale or silent verdict.
- **candor-scan 0.8.4**: the κ ledger honors the §2 rule-3 coverage exemption for CHAINED reports,
  keyed on the envelope `package`/`packages` field — an EMPTY chained report is a purity claim, not
  a "κ doesn't know" line; config keys the engine recognizes but does not implement
  (baseline/strict/no-ambient/closed-world/taint) warn loudly instead of reading as an active gate.
- **candor-query 0.8.0**: joins the spec-tracks-version convention; full crates.io metadata + a crate
  README; candor-report floor 0.5.8 (resolution can never produce a pre-0.8 spec declaration).

## [candor-scan 0.8.3] — 2026-07-02

- **`.candor/config`** (spec §3.4): the checked-in configuration file — target-anchored discovery
  (walk up from the scan target), `policy`/`deps` wired, unknown keys warn, configured-but-unusable
  exits 2. CI becomes "point at the repo".
- Allocation-free violation sort (identical order).

## [candor-scan 0.8.2] — 2026-07-02

- `--gate-json` takes a real path only (a flag-shaped/valueless value exits 2 — it used to swallow
  `--policy` and scan the wrong target, gateless); `--gate-json -` streams the verdict to stdout,
  which stays pure JSON (AS-EFF lines → stderr).

## [candor-scan 0.8.1] — 2026-07-02

- Workspace `--gate-json` accumulates across members (spec §3.3 MUST: the verdict agrees with the
  exit code — a clean last member no longer overwrites an earlier violator's verdict).

## [candor-scan 0.8.0 + candor-report 0.5.7] — 2026-07-01/02 — spec 0.8

- **`--gate-json <file>`** (spec §3.3 ⟨0.8⟩): the structured gate verdict
  `{ spec, ok, violations: [{rule, fn, effects, detail}] }`, written from the SAME check that sets
  the exit code — the machine analog of the AS-EFF console lines (feeds the PR-native SARIF
  reporter). candor-report 0.5.7 declares `SPEC_VERSION = "0.8"`; candor-scan 0.8.0 pins it as a
  floor so dependency resolution can't produce a pre-0.8 declaration.
- In the same window (in-tree, not a crates.io release): the nightly lint gained the
  CANDOR_VIOLATIONS machine-readable violation sentinel + a deep-engine dynamic-oracle lane;
  `cargo candor guard` turned fail-closed on a stale baseline and an absent provenance sidecar.

### (bridge) 0.4 → 0.7, briefly

The arc between this entry and 0.3.7 below: spec 0.5, **spec 0.6** (2026-06-19: the `blindspots`
query + `unknownWhy` required on direct Unknown sources — candor-report 0.5.5, candor-scan 0.5.19),
a long candor-scan 0.5.x soundness run (FFI/drop-glue seams, lazy-static deferred init,
iterator-forcing, masked-literal fail-closed, implicit conversion, 20-crate coverage calibration),
then **spec 0.7** (2026-06-19: engine versions aligned to the spec — candor-scan/candor-query
0.7.0, candor-report 0.5.6) and the 0.7.x review fixes. Detail: `git log` around those dates.

## [0.23.0] — 2026-07-20 (crates: candor-report / candor-classify / candor-scan / candor-query, lockstep at the spec floor)

Spec floor → **0.23**. Soundness-increasing, report-shape-neutral:
- **cross-package interface dispatch** (interfaceUnion, the 0.23 rung): a chained consumer's trait-object
  dispatch resolves to the impl's effect (gated behind `CANDOR_WORKSPACE_CHAIN`; a default report is
  byte-identical). PART 18 conformance.
- **⚠ opaque callable → synchronous invoker** (`Iterator::for_each(cb)` direct-pass, Option/Result
  combinators) discloses `Unknown` — the four-way sync-callback rung (PART 1 `sync_callback_opaque`).
- trait-union emission guarded against same-leaf-trait name collision (never fabricate an unrelated impl's
  effect on a colliding trait tail).

## [0.22.0] — 2026-07-18 (crates: candor-report / candor-classify / candor-scan / candor-query, lockstep at the spec floor)

Spec floor → **0.22** (the `verify` oracle rung, shipped on the java/ts arms). candor-scan / candor-query declare
`0.22`; the report and verdict schema are unchanged from 0.21, so this engine's output is byte-identical across
the bump. No functional change to the Rust engine.

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
