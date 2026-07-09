# candor-query

The **read-only query surface** over [candor](https://github.com/tombaldwin/candor-rust) effect
reports — one typed binary answering the questions an agent (or a human) asks of a finished scan,
*instantly*, without recompiling anything. It is the engine behind `cargo candor show/where/callers/…`
and works identically over reports from **any** candor backend (the stable scanner, the nightly lint,
or a sibling engine emitting the same [candor-spec](https://github.com/tombaldwin/candor-spec) shape).

```sh
cargo install candor-query
candor-query --help          # the full command list
candor-query --agents        # the embedded agent contract for this installed build
```

## The questions it answers

```sh
candor-query show     <prefix> <fn-substring> <0|1>    # a function's effects (direct vs inherited)
candor-query where    <prefix> <Effect>       <0|1>    # who performs an effect (sources vs inheritors)
candor-query callers  <prefix> <fn-substring> <0|1>    # who reaches a function — its blast radius
candor-query map      <prefix>                <0|1>    # module → effects overview
candor-query impact   <prefix> <fn> [--json]           # transitive callers + downstream entry points
candor-query path     <prefix> <fn> <Effect> [--json]  # the call chain to the effect's source
candor-query whatif   <prefix> <fn> <Effect> [policy]  # pre-edit verdict: propagation × policy
candor-query diff     <cur> <base> <0|1> <bver> <ever> # per-function effect delta vs a baseline
candor-query containment <prefix> [baseline] [--json]  # effect-leakage ratchet (AS-EFF-010: exit 1)
candor-query reachable / blindspots / gains / rewire / receipt / parsepolicy / gate-verdict / …
```

`<prefix>` is the report path prefix (the part before `.<crate>.<type>.json`) — a workspace's
per-member reports under one prefix are merged automatically. The trailing `0|1` on the older
commands is the want-JSON flag. A prefix matching **no** report files fails loud (exit 2) — a silent
`{}` is never "wrong path".

## Fail-closed by design

The gate-adjacent commands share the candor family's posture: an unreadable policy, a missing
report, a corrupt gate record, or an unwritable verdict is **exit 2** ("not evaluated"), never a
green pass. `containment <prefix> <baseline>` is a CI ratchet: a boundary effect leaking into a new
layer is exit 1.

## Version

`candor-query -V` prints the build and the candor-spec contract it speaks (the same
`candor_report::SPEC_VERSION` that stamps report envelopes, so the two can never drift). Fully
offline — candor never phones home.

Dual-licensed under MIT or Apache-2.0.
