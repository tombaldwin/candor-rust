# `unknownWhy` validate-at-scale sweep (candor-rust)

**Question.** On candor-java, the `unknownWhy` origin tag revealed that 97% of a real Spring app's
`Unknown`s were *resolvable dispatch* — leading to two fixes that cut direct `Unknown`s 183→13. Does the
Rust impl hide the same class of resolvable-but-unresolved `Unknown`s? The red flag to hunt: an
`Unknown` tagged `dispatch:<trait>` where the trait is **local to the analysed crate** (CHA should have
resolved it to a visible impl).

**Method.** Ran the nightly lint (`CANDOR_JSON`) over 22 diverse real crates — 2 local apps
(`pgman`, `tb-tui-common`) + 20 registry libraries (itoa, ryu, bitflags, memchr, smallvec, arrayvec,
anyhow, log, serde_json, url, indexmap, hex, aho-corasick, regex-syntax, httparse, unicode-width,
textwrap, heck, strsim, base64-simd…). Tallied every directly-introduced `unknownWhy` tag and
classified each `dispatch:` by trait origin: `std` / external-dep / **own crate (suspect)**.

**Result.** 845 functions; **171** carry a direct `unknownWhy`:

| origin | count | verdict |
|---|---:|---|
| `callback:` (fn-pointer / closure) | 118 | irreducible — an indirect call candor can't see through |
| `dispatch:std::io::Write` / `Read` / `BufRead` / `Iterator` | 45 | honest — generic over an effectful std trait (the reader/writer behind it could be a file/socket) |
| `dispatch:` over `aho_corasick::automaton::Automaton` | 9 | honest — see below |
| **`dispatch:` over the analysed crate's OWN resolvable trait** | **0** | **no Java-equivalent precision bug** |

**The one case worth scrutiny — and why it's correct.** `aho-corasick`'s `AhoCorasick::try_*` methods
dispatch over `Arc<dyn AcAutomaton>`. `Automaton` is `pub unsafe trait Automaton: private::Sealed` —
*sealed*, so all impls are in fact local. candor flags `Unknown` because a `dyn` over a **public** trait
object generally admits downstream impls that could perform any effect; it soundly will not certify the
call pure. It happens to be sealed, but (a) the seal is a convention candor doesn't model, and (b) even
a complete CHA routes through the provided `try_find_iter` into runtime-selected required methods. This
is exactly what `Unknown` is for — a sound conservative refusal, made *legible* by the tag.

**Conclusion.** Unlike candor-java (where `unknownWhy` exposed mass resolvable dispatch + real bugs), the
Rust impl's `Unknown`s are **essentially all honest**: irreducible callbacks, generics over effectful std
traits, and one sealed-trait-object boundary. This matches the structural reason established earlier —
the Rust backend resolves dispatch via rustc's `Instance::try_resolve`, which already does the work the
JVM port had to hand-roll (and where it had two bugs). The sweep found **no `Unknown` candor should have
resolved**. Secondary win: `unknownWhy` let 171 unknowns across 22 crates be triaged mechanically in one
pass and the single interesting case pinpointed in seconds — the legibility feature pays for itself.

**Possible (low-priority, risky) future precision tweak.** Sealed-trait detection could let CHA treat a
`dyn SealedTrait` as a closed impl set and tighten those 9 `Unknown`s — but mis-detecting a seal would be
unsound, and the payoff is narrow. Not recommended unless sealed-trait-object dispatch shows up as a
material false-positive source.
