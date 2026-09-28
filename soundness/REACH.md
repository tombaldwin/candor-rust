# The drop-glue REACH probe — what it counts, and what it cannot see

SOUNDNESS **R766**. Run it with `bash soundness/reach.sh`. Source: `soundness/reach/`.

## Why it exists

The previous reach probe counted *the crate declares a local `impl Drop`* **AND** *the shape appears*.
That is not the conjunction a FIX needs. The engine marks a construction only when its leaf is in
`drop_relevant` (`crates/candor-scan/src/collector.rs:694`), so a shape landing on a destructor-less
type cannot move a row however the drop/escape model is changed. R189 was ordered at *146 sites across
60 of 473 crates* on the old predicate and measured, with the fix in hand, **REACH 204 / ADDED 0 /
REMOVED 0 / CHANGED 0**. The decision fired 204 times and not one row followed.

It lives in `soundness/` rather than in a scratchpad because R288 is the same failure one level up:
fifteen copies of one A/B script in one session, every one keyed wrong because it was copied from the
last one. A measurement procedure with no owner gets re-derived per lane, and each copy carries its
own defect.

## What it counts

Two passes, per crate.

**Pass A** rebuilds the crate's `drop_relevant` set to the engine's own definition
(`scan.rs:1596`): `drop_types` (every local type leaf with a local `impl Drop`) ∪ `owned_drops.keys()`
(the monotone fixpoint of "T owns a drop-type through a NON-BORROWED field, directly, through a
container element, or transitively" — R718's rule included).

**Pass B** runs one matcher per open drop-glue row, each written from the row's RE-VERIFIED cells
rather than its title, and emits every site twice over:

- **LOOSE** — the shape is present.
- **TIGHT** — the shape is present **and** the value at the shape's own value position is
  drop-relevant in this crate.

Only TIGHT can order work. LOOSE is kept beside it because the ratio says how much of the old
ordering was shape and how much was payoff.

## The calibration, and why it is a gate

`reach.sh` runs five planted fixtures and **refuses to print a census number if any is off**.

| fixture | what it plants | required |
|---|---|---|
| `pos` | every row's own re-verified cell, on a type with a local `impl Drop` | TIGHT ≥ 1 each |
| `neg-nodrop` | the same file with `impl Drop for H` deleted, nothing else moved | TIGHT 0 each |
| `neg-otherdrop` | the same file plus an UNRELATED `impl Drop for G` | TIGHT 0 each |
| `neg-shape` | the rows' own CHARGED controls, all on a type WITH a destructor | TIGHT 0 each |
| `pos-owned` | the `owned_drops` half, with R718's borrowed controls beside it | see its expect file |

`neg-otherdrop` is R766's own defect as a fixture: the old predicate scores 9 hits on it, the
predicate a fix needs scores 0. `neg-shape` exists because **drop-relevance alone turned out not to be
enough** — six of its functions are real census hits that a hand audit threw out:

| census hit | why it was not the row | control |
|---|---|---|
| `allocator_api2::repeat`, `anyhow::render` | inline, operand-local constructions at two exits, which R189's own remedy keeps unioned | `r189_inline`, `r189_one_name` |
| `async_nats::send_request` | the leaf was the method's RETURN type — the await's OUTPUT, handed to the caller | `r201_output_leaf` |
| `async_compression::poll_partial_flush_buf` | `Poll::Ready(Ok(Buffer{..}))` — a wrapper argument to an enum-variant CONSTRUCTOR, which cannot drop it | `r200c_variant_ctor` |
| `async_executor::run`, `async_nats::send_request` | the leaf was the method's RECEIVER, a binding the future BORROWS | `r201_receiver_place` / `pos::r201_receiver_rvalue` |
| `async_std::File::open` | the construction is in a CLOSURE BODY handed to the awaited callee; it happens wherever the callee runs it | `r201_closure_arg` |
| `h2::queue_frame` (×5) | `tracing::trace_span!("…", ?stream.id)` — `?` as a field-capture SIGIL, not the try operator | `r209a_prefix_sigil` |

Every one of those was found by reading the source at a hit, not by reasoning about the matcher.
**The audit trail is the deliverable**: `--hits N` prints sample TIGHT hits and `--tsv F` dumps all of
them, and a number nobody has spot-checked at the source is the thing this row is about.

## THE NOISE FLOOR — read this before comparing any two rows

**R189 is carried in the table although it is CLOSED, as the probe's real-world calibration.** R766
measured its true payoff on this census as **zero rows**. This probe scores it at **15 tight sites
across 10 entries / 4 distinct crate names**.

So **a tight count around 15 sites / 4 names is indistinguishable from zero payoff.** The residual is
the fix-level conditions no source-only probe can evaluate: whether the site is already charged from
somewhere else, whether the drop actually executes on a reachable path, and the fix's own further
discriminators. Read the table against that floor, not against zero.

## What this probe CANNOT see

1. **It has no type checker.** Types are inferred syntactically — struct literals, `T::assoc()`,
   declared fn/param/field types, and `let` initialisers. A value whose type is only knowable by
   inference (a generic, a closure return, a trait method through a bound) is invisible.
   **Direction: UNDER-count.**
2. **It is leaf-keyed, exactly like the engine.** A crate that declares its own `Vec`, `File` or
   `Error` collides with the std name and the probe cannot separate them — `allocator-api2` and
   `async-std` both do. **Direction: OVER-count, and the engine shares it.**
3. **Method return types are used only where the method name resolves uniquely crate-wide.** A
   colliding name contributes nothing rather than a guess. **Direction: UNDER-count.**
4. **Cross-crate `Drop` is invisible.** `drop_relevant` is local types only — same as the engine
   without `CANDOR_DEPS`. Real user code routinely holds a *dependency's* guard, and that reach is
   neither counted here nor measurable without running the engine over a chained scan.
   **Direction: UNDER-count, and it is the biggest one.**
5. **215 of 47,006 census files did not parse** (macro-heavy or non-2021 syntax). Their shapes are
   silence, and the run prints the count so the silence is visible.
6. **`tests/`, `benches/`, `examples/`, `fuzz/` and `#[cfg(test)]` items are excluded**, matching the
   engine's `include_tests = false` default. Counting them would inflate reach with code no user's
   gate runs over.
7. **REACH IS NOT PAYOFF.** A tight hit says the decision could fire and the leaf has a destructor. It
   does **not** say a drop executes there, that the unit is not already charged from elsewhere, or
   that the resulting row change is desirable. See the noise floor above.
8. **R200's callee behaviour is not modelled.** R200's own register entry read ~105 candidate bodies
   across 21 crates and found **every one STORES, FORWARDS or LEAKS rather than drops**. So the
   `R200c`/`R200d` tight counts are the population a widening would TOUCH — which is R230's
   fabrication cost, not R200's payoff. Do not read them as benefit.
9. **`R297` is split from `R297opt` for the same reason.** Where the assigned-to place is an `Option`,
   the previous value may be `None` and the assignment may drop nothing. Those sites still MOVE a row
   (R297's remedy accepts the over-charge on purpose) but they are blast radius, not payoff.
10. **It counts SITES, not gate flips.** The user-visible cost of a change is how many gates flip on
    real code, and no source-only probe can answer that. A number here orders investigation; it never
    settles a ship/decline decision on its own.

## The counts are a function of HEAD — print them, do not quote this file

`bash soundness/reach.sh`. The snapshot below is **2026-09-28**, over the pinned rust census
(`candor/bin/corpus-census-rust.tsv`, 1,625 entries, 47,006 files parsed, 510 entries declaring a local
`impl Drop`), recorded so a later run has something to diff against — not as a figure to cite.

**Read the `names` column, not `entries`.** The census holds 1,625 entries but only **937 distinct
crate names** — `async-nats` at two versions, `h2` at two, `crossbeam-utils` at three. Every "N crates"
figure in the old ordering counted entries, so a three-version crate voted three times.

| row | LOOSE | entries | TIGHT | entries | names | most concentrated in |
|---|---|---|---|---|---|---|
| R189 *(closed — the noise floor)* | 310 | 166 | **15** | 10 | 4 | hyper 8, jiff 3, h2 2 |
| R198 | 569 | 134 | **4** | 3 | 2 | bindgen 2, diesel 2 |
| R200c *(call site)* | 8,073 | 747 | **349** | 77 | 36 | lapin 151, hyper 32, bindgen 16 |
| R200d *(declaration)* | 7,561 | 454 | **440** | 83 | 40 | lapin 126, mongodb 78, moka 35 |
| R201 | 1,547 | 99 | **234** | 23 | 12 | mongodb 164, mysql_async 21, sqlx-core 18 |
| R209a | 790 | 176 | **18** | 9 | 7 | hyper 7, openssl 5, tokio 2 |
| R297 *(non-`Option` place)* | 84,134 | 1,250 | **494** | 97 | 42 | regex-syntax 80, hyper 76, syn 75 |
| R297opt *(`Option` place)* | 7,858 | 603 | **186** | 56 | 29 | moka 26, mongodb 20, mysql_async 17 |
| R300 | 1,267 | 348 | **23** | 16 | 10 | jiff 9, mongodb 4, compact_str 2 |
| R323 | 56 | 41 | **9** | 9 | 3 | mongodb 4, crossbeam-utils 3, mysql_async 2 |

Three things this table says that the old ordering did not:

- **R201's 234 is 164 sites in ONE crate.** 70% of it is `mongodb`, and 12 distinct names carry the
  whole row. The old figure of *346 sites / 21 crates* read as a broad vein; it is one async house
  style. R200c/R200d are the same shape with `lapin`.
- **R198, R323, R209a and R300 are at or below the R189 noise floor** (15 sites / 4 names, against a
  payoff measured at zero). Nothing separates them from nothing.
- **R297's 494 is the only count that is both large and spread** (42 names, no crate over 17%) — and
  it is the row whose sites are in the CHARGING direction, so a large part of that is blast radius
  rather than payoff.
