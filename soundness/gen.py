#!/usr/bin/env python3
"""Construction-based soundness fuzzer for candor (Bet 1, phase 1).

Generates a compilable Rust crate that threads a KNOWN effect (Fs/Net/Exec/Env) from a `sink` function
up through a random chain of functions, where each call edge uses a randomly-chosen call FORM — the
exact forms that have produced silent under-reports (direct call, inline closure, generic `impl Fn`,
a named fn handed to a combinator, a boxed `dyn Fn` value, `dyn`-method dispatch, and the RECEIVING
side: a function that takes a `Box<dyn Fn>` / `impl Fn` parameter and invokes it).

Every emitted function transitively reaches the effect, so candor must report each one with the effect
in its `inferred` set OR with `Unknown` (a sound over-approximation). A function reported PURE — or
omitted from the report entirely (candor omits effect-free fns) — is a SILENT UNDER-REPORT: the bug
class this harness exists to catch. The generator writes `truth.json` listing the functions that must
be effect-or-Unknown; `run.sh` checks the report against it.

Usage:  gen.py <seed> <out-dir>     # writes <out-dir>/{Cargo.toml, src/main.rs, truth.json}
"""
import json
import os
import random
import sys

# effect -> (the std leaf call that performs it, a distinctive runtime MARKER greppable in a syscall
# trace). The marker lets the dynamic oracle (phase 2) attribute the observed syscall to THIS effect
# and filter out the runtime's own startup syscalls (every binary opens libc, etc.). A `None` marker
# means the effect isn't syscall-observable (`Env` reads process memory — no syscall) so the oracle
# skips it. The leaf calls are chosen to emit their syscall even on failure (missing file, refused
# connection): `openat`, `connect(127.0.0.1)`, `execve(echo candor_fuzz_marker)`.
def effects_for(seed):
    return {
        "Fs":   ('let _ = std::fs::read_to_string("/tmp/candor_fuzz_%d");' % seed, "/tmp/candor_fuzz_%d" % seed),
        "Net":  ('let _ = std::net::TcpStream::connect("127.0.0.1:9");', "127.0.0.1"),
        "Exec": ('let _ = std::process::Command::new("echo").arg("candor_fuzz_marker").status();', "candor_fuzz_marker"),
        "Env":  ('let _ = std::env::var("CANDOR_FUZZ");', None),
    }

# Shared helpers, emitted once when any edge needs them. Keyed so we only emit each at most once.
HELPERS = {
    "apply":      "fn apply<F: Fn()>(f: F) { f() }",
    "run_trait":  "trait Run { fn run(&self); }\nstruct W<F>(F);\nimpl<F: Fn()> Run for W<F> { fn run(&self) { (self.0)(); } }",
    "recv_boxed": "fn recv_boxed(cb: Box<dyn Fn()>) { cb(); }",          # receiving side: boxed dyn Fn param
    "recv_impl":  "fn recv_impl<G: Fn()>(cb: G) { cb(); }",              # receiving side: generic Fn param
}

# Each "edge form" returns (body_calling_callee, helpers_needed, extra_expected_fns).
# `extra_expected_fns` are helper fns that ALSO transitively reach the effect via this edge and so must
# themselves be effect-or-Unknown (the receiving-side checks — where the real bugs live).
def edge_forms(callee):
    return {
        "direct":     (f"{callee}();", [], []),
        "iife":       (f"(|| {callee}())();", [], []),
        "stored":     (f"{{ let c = || {callee}(); c(); }}", [], []),
        # named fn handed to a generic combinator (passing side) + apply itself invokes its param.
        "generic":    (f"apply({callee});", ["apply"], ["apply"]),
        # named fn boxed as a dyn Fn value, then called.
        "boxed_val":  (f"{{ let b: Box<dyn Fn()> = Box::new({callee}); b(); }}", [], []),
        # named fn in a generic struct dispatched via &dyn Trait (dyn dispatch + a field closure call).
        "dyn_method": (f"{{ let w = W({callee}); let r: &dyn Run = &w; r.run(); }}", ["run_trait"], []),
        # RECEIVING side: the effect reaches a fn ONLY through a Box<dyn Fn> param it invokes.
        "recv_boxed": (f"recv_boxed(Box::new(|| {callee}()));", ["recv_boxed"], ["recv_boxed"]),
        # RECEIVING side: the effect reaches a fn ONLY through a generic Fn param it invokes.
        "recv_impl":  (f"recv_impl(|| {callee}());", ["recv_impl"], ["recv_impl"]),
    }


def main():
    seed = int(sys.argv[1])
    out = sys.argv[2]
    rng = random.Random(seed)

    EFFECTS = effects_for(seed)
    # The oracle restricts to syscall-observable effects via CANDOR_FUZZ_EFFECTS (e.g. "Fs Net Exec").
    allowed = os.environ.get("CANDOR_FUZZ_EFFECTS", "").split() or list(EFFECTS)
    effect = rng.choice([e for e in EFFECTS if e in allowed])
    leaf, marker = EFFECTS[effect]
    n = rng.randint(3, 9)  # chain length

    fns = [f"f{i:02d}" for i in range(n)]
    bodies = {}
    needed_helpers = set()
    expected = set(fns) | {"sink", "main"}

    # sink performs the effect directly.
    bodies["sink"] = leaf

    forms_log = {}
    for i in range(n):
        callee = fns[i + 1] if i + 1 < n else "sink"
        form_name = rng.choice(list(edge_forms(callee)))
        body, helpers, extra = edge_forms(callee)[form_name]
        bodies[fns[i]] = body
        needed_helpers.update(helpers)
        expected.update(extra)
        forms_log[fns[i]] = form_name

    # Assemble the source.
    lines = ["// GENERATED by soundness/gen.py — do not edit. seed=%d effect=%s" % (seed, effect), ""]
    for h in HELPERS:
        if h in needed_helpers:
            lines.append(HELPERS[h])
    lines.append("")
    for name in ["sink"] + fns:
        lines.append("fn %s() { %s }" % (name, bodies[name]))
    lines.append("")
    lines.append("fn main() { %s(); }" % fns[0])
    src = "\n".join(lines) + "\n"

    os.makedirs(os.path.join(out, "src"), exist_ok=True)
    with open(os.path.join(out, "Cargo.toml"), "w") as f:
        f.write('[package]\nname = "candor_fuzz"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n')
    with open(os.path.join(out, "src", "main.rs"), "w") as f:
        f.write(src)
    with open(os.path.join(out, "truth.json"), "w") as f:
        json.dump(
            {"seed": seed, "effect": effect, "marker": marker, "expect": sorted(expected), "forms": forms_log},
            f,
            indent=2,
        )


if __name__ == "__main__":
    main()
