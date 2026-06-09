#!/usr/bin/env python3
"""Measure candor's token cost for the blast-radius question vs a REALISTIC manual baseline.

The recurring claim is "candor saves AI tokens." This makes it falsifiable and reproducible for the one
question where candor's value is real — the transitive blast radius ("who is affected if I add an effect to
X?"). For each sampled function it compares:

  - candor       = tokens of `candor-query callers <fn>` (one query -> the complete transitive caller set)
  - grep-trace   = tokens an agent spends tracing the SAME complete set by hand: grep each function in the
                   transitive closure (`grep -rn <fn> src`). This is the realistic baseline — agents grep,
                   they do NOT read the whole crate.

Earlier versions of this script compared against reading the ENTIRE crate (~700-2000x); that's a strawman
denominator — no competent agent does it. The grep-trace below is the honest baseline. (`--ceiling` also
prints the full-source figure, but as an INFORMATION-COMPRESSION number, not a token-savings claim.)

Usage:  python3 eval/token-cost/measure.py <crate-dir> [sample_size] [--ceiling]
Token estimate: chars/4 (model-agnostic; ratios stable under any fixed tokenizer)."""
import glob, json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
SCAN = os.environ.get("CANDOR_SCAN_BIN", os.path.join(ROOT, "target", "debug", "candor-scan"))
QUERY = os.environ.get("CANDOR_QUERY_BIN", os.path.join(ROOT, "target", "debug", "candor-query"))
tok = lambda s: max(1, len(s) // 4)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    crate = sys.argv[1]
    k = next((int(a) for a in sys.argv[2:] if a.isdigit()), 12)
    ceiling = "--ceiling" in sys.argv

    subprocess.run([SCAN, crate], capture_output=True)
    reps = [f for f in glob.glob(f"{crate}/**/.candor/report.*.scan.json", recursive=True) if "callgraph" not in f]
    if not reps:
        print(f"no report produced for {crate}")
        sys.exit(1)
    rep = reps[0]
    prefix = rep.rsplit(".", 3)[0]

    fns = [e["fn"] for e in json.load(open(rep))["functions"]]
    seen, leaves = set(), []
    for f in fns:
        leaf = f.split("::")[-1]
        if leaf not in seen:
            seen.add(leaf)
            leaves.append(leaf)
    sample = leaves[:k]

    print(f"crate: {crate}")
    print(f"{'fn':18s} {'closure':>7s} {'candor':>7s} {'grep-trace':>10s} {'ratio':>6s}")
    cand_tot = grep_tot = 0
    ratios = []
    for leaf in sample:
        cand = tok(subprocess.run([QUERY, "callers", prefix, leaf, "0"], capture_output=True, text=True).stdout)
        j = subprocess.run([QUERY, "callers", prefix, leaf, "1"], capture_output=True, text=True).stdout
        try:
            o = json.loads(j)
            closure = o.get("transitive", []) or o.get("direct", [])
        except Exception:
            closure = []
        members = {leaf} | {c.split("::")[-1] for c in closure}
        grep = sum(tok(subprocess.run(["grep", "-rn", m, f"{crate}/src"], capture_output=True, text=True).stdout)
                   for m in members)
        r = grep / max(1, cand)
        ratios.append(r)
        cand_tot += cand
        grep_tot += grep
        print(f"{leaf:18s} {len(members):7d} {cand:7d} {grep:10d} {r:5.1f}x")

    ratios.sort()
    med = ratios[len(ratios) // 2] if ratios else 0
    print(f"\ncandor total {cand_tot} tok  vs  grep-trace total {grep_tot} tok"
          f"   (median per-fn ratio ~{med:.0f}x, range {min(ratios):.1f}-{max(ratios):.0f}x)")
    print("candor's edge is largest where the closure has COMMON-named functions — exactly where grep is")
    print("also noisiest and least reliable (it can't tell a real call from a coincidental name match).")
    if ceiling:
        src = [f for f in glob.glob(f"{crate}/**/*.rs", recursive=True) if "/.candor/" not in f and "/target/" not in f]
        st = sum(tok(open(f, encoding="utf-8", errors="ignore").read()) for f in src)
        print(f"\n[ceiling] full crate source ~{st:,} tok (INFORMATION compression vs candor, NOT a savings claim)")


if __name__ == "__main__":
    main()
