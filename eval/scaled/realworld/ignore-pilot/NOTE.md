# ignore-pilot — proved the harness, target too easy (superseded by the delta target)

This was the first real-world freeze (ripgrep `crates/ignore` / `pathutil::strip_prefix` / 32-function
tree). The N=4/arm **sonnet pilot** validated the harness end-to-end (deep-engine report →
`candor-query callers` → mechanical leaf-name grading) and showed a large efficiency + reliability win:

| arm | recall | perfect | tokens | tool calls | wall-clock |
|---|---|---|---|---|---|
| treatment (candor) | 100% | 4/4 | ~21.8k | 1 | ~8.6s |
| control (source) | 97.7% | 1/4 | ~81.8k | ~52 | ~268s |

But sonnet **control reached 97.7%** by brute force — a 6.7k-LOC crate is readable with effort, so the
*completeness* premise (the reason the real-world cell exists) didn't bite. The target was therefore
**hardened** to a 30k-LOC crate (`delta`) where the call graph exceeds comfortable reading; see the
parent [MANIFEST.md](../MANIFEST.md) / [GROUND_TRUTH.md](../GROUND_TRUTH.md). The files here are kept as
the harness-validation record and the easy-end data point.
