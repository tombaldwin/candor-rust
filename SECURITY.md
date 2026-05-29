# Security

## candor is not a security boundary

Read this before relying on candor for anything security-critical.

candor is an **advisory static analysis**, not a sandbox or an enforcement mechanism. It tells you
what code *appears* to do; it does not *stop* code from doing anything. It also has documented blind
spots — calls it cannot resolve are marked `Unknown` (dynamic dispatch over non-local traits,
function pointers, closures through callbacks), its classifier only knows a curated set of crates,
and macro-generated functions are skipped. See [CRITIQUE.md](CRITIQUE.md) and [BACKLOG.md](BACKLOG.md).

**Do not use candor as a security control.** A `&Fs` token is a lint convention, not a gate — you
can still call `std::fs` without it. For real, compile-enforced capability isolation use
[cap-std](https://github.com/bytecodealliance/cap-std); for supply-chain API restrictions use
[Cackle](https://github.com/cackle-rs/cackle). candor complements these as a *visibility* layer; it
does not replace them.

Where candor *does* help your security posture: surfacing the effect surface (what reaches the
network / filesystem / spawns processes), and the regression guard (catching a change that makes a
previously-pure function start doing I/O). Treat both as defense-in-depth signals, not guarantees.

## Reporting an issue

The bug class that matters most here is a **false negative**: candor reporting a function as
effect-free (or missing an effect) when it actually performs that effect — a soundness hole. Those
are prioritized, because a checker that's silently wrong is worse than no checker (PRINCIPLES #1).

Please report via a GitHub issue with a minimal reproducer (the smallest function + crate that's
misclassified). If you'd rather report privately, email tom@polymorphism.co.uk.

## Supported versions

candor is a prototype pinned to a specific nightly toolchain. There is no formal support window;
fixes land on `main`.
