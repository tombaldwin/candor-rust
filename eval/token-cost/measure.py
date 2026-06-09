#!/usr/bin/env python3
"""Measure the INFORMATION cost of candor's blast-radius answer vs reading the source for it.

The recurring claim is "candor saves AI tokens." This makes it falsifiable and reproducible for the one
question where candor's value is real — the transitive blast radius ("who is affected if I add an effect to
X?"). For each sampled function it compares:

  - candor's answer  = the tokens of `candor-query callers <fn>` (one query, the complete transitive set)
  - the manual cost  = the tokens of source an agent must read to trace the SAME answer by hand

The manual figure is the COMPLETE-answer ceiling: total crate source. That's the honest comparison, because
candor's value IS completeness — agents that don't pay it get ~6% of the blast radius (see eval/scaled). It
is NOT a claim about the cheapest possible grep; it's the cost of being *exhaustive*, which is what the
question demands and what candor makes cheap.

Usage:  python3 eval/token-cost/measure.py <crate-dir> [sample_size]
Token estimate: chars/4 (model-agnostic; ratios are stable under any fixed tokenizer)."""
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
    k = int(sys.argv[2]) if len(sys.argv) > 2 else 12

    subprocess.run([SCAN, crate], capture_output=True)
    reps = [f for f in glob.glob(f"{crate}/**/.candor/report.*.scan.json", recursive=True) if "callgraph" not in f]
    if not reps:
        print(f"no report produced for {crate}")
        sys.exit(1)
    rep = reps[0]
    prefix = rep.rsplit(".", 3)[0]  # <dir>/.candor/report  (strip .<crate>.scan.json)

    src = glob.glob(f"{crate}/**/*.rs", recursive=True)
    src = [f for f in src if "/.candor/" not in f and "/target/" not in f]
    src_tokens = sum(tok(open(f, encoding="utf-8", errors="ignore").read()) for f in src)

    fns = [e["fn"] for e in json.load(open(rep))["functions"]]
    seen, leaves = set(), []
    for f in fns:                       # dedupe by leaf, keep a stable sample
        leaf = f.split("::")[-1]
        if leaf not in seen:
            seen.add(leaf)
            leaves.append(leaf)
    sample = leaves[:k]

    answers = []
    for leaf in sample:
        out = subprocess.run([QUERY, "callers", prefix, leaf, "0"], capture_output=True, text=True).stdout
        answers.append(tok(out))
    avg = sum(answers) // max(1, len(answers))

    print(f"crate:           {crate}")
    print(f"source:          {len(src)} files, ~{src_tokens:,} tokens (the complete-trace ceiling)")
    print(f"candor answer:   ~{avg} tokens (avg `callers <fn>` over {len(sample)} functions)")
    print(f"compression:     ~{src_tokens // max(1, avg)}x for a COMPLETE blast-radius answer")
    print(f"\ncaveat: this is the blast-radius/graph question (candor's strength). For 'what does this one")
    print(f"function do', reading it is cheap and candor saves ~nothing — the value is question-specific.")


if __name__ == "__main__":
    main()
