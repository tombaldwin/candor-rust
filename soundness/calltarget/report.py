#!/usr/bin/env python3
"""Summarise a call-target matrix run over the three spellings, and compare it with the committed baseline.

usage: report.py <work> <bin> [--pre <bin>] [--write-baseline <file>] [--check <file>] [--outdir <dir>]

<work>/{crate,bare,use}/results.jsonl come from run.py. <bin> is the build whose verdicts are summarised (and written
or checked as the baseline); `--pre <bin>` adds the per-cell transitions from that build (e.g. the published one).

What it prints, and why each part exists:
  * per spelling: effect-arm classes (CHARGED/DISCLOSED/LEDGER-ONLY/SILENT) and no-effect classes (incl. FABRICATED);
    every program built and its executed effect matched the generator (GT) — a mismatch is a generator defect and
    makes the run NOT a measurement (exit 3);
  * the SPELLING CONTROL: per cell x arm, the class under crate / bare / use. `use` imports every type value the
    caller names and writes it bare, so a class that differs between `crate` (or `bare`, which still spells a
    dependency type `ctlib::S`) and `use` is path-value TYPING, not the mechanism the cell is about;
  * the SILENT TABLE BY MECHANISM under `use` (the spelling that isolates each class), with the body-visibility
    calibration (is the effectful body charged in a report the arm saw?);
  * transitions pre -> bin, with every move toward silence and every new FABRICATED listed by name.
Baseline lines are `cid<TAB>spelling<TAB>arm<TAB>class`; `--check` exits 1 on any difference and names it.
"""
import json, os, sys, glob, collections

args = sys.argv[1:]
WORK, BIN = args[0], args[1]
def opt(k):
    return args[args.index(k) + 1] if k in args else None
PRE = opt("--pre"); WB = opt("--write-baseline"); CK = opt("--check"); OUTD = opt("--outdir") or WORK
SPELLS = ["crate", "bare", "use"]

def base(c):
    if c.startswith("PURE+"): return "PURE+UNKNOWN"
    return c.split(":")[0].split("+")[0]

R = {}
bad = []
for sp in SPELLS:
    p = os.path.join(WORK, sp, "results.jsonl")
    R[sp] = {}
    if not os.path.exists(p):
        continue
    for l in open(p):
        r = json.loads(l)
        R[sp][r["cid"]] = r
        if r["run"].startswith("BUILD") or not r.get("gt_ok"):
            bad.append((sp, r["cid"], r["run"], r.get("happened"), r["gt"]))

out = []
def P(*a): out.append(" ".join(str(x) for x in a))

P("== programs per spelling (built + executed; GT = observed effect matched the generator) ==")
for sp in SPELLS:
    rs = R[sp].values()
    P("  %-5s programs %d  arms %d  build-fail/GT-mismatch %d" % (sp, len(R[sp]), sum(len(r.get("arms", [])) for r in rs),
      sum(1 for b in bad if b[0] == sp)))
for b in bad[:40]:
    P("  NOT MEASURED", *b)

def tally(sp, bn):
    eff = collections.Counter(); ne = collections.Counter()
    for r in R[sp].values():
        for a in r.get("arms", []):
            c = base(a[bn]["class"])
            (eff if r["gt"] else ne)[c] += 1
    return eff, ne
for bn in ([PRE] if PRE else []) + [BIN]:
    P("\n== classes, build %s ==" % bn)
    for sp in SPELLS:
        if not R[sp]: continue
        e, n = tally(sp, bn)
        P("  %-5s effect: CHARGED %d DISCLOSED %d LEDGER-ONLY %d SILENT %d | no-effect: PURE-OK %d PURE+UNKNOWN %d OVER %d FABRICATED %d | ERR %d"
          % (sp, e["CHARGED"], e["DISCLOSED"], e["LEDGER-ONLY"], e["SILENT"], n["PURE-OK"], n["PURE+UNKNOWN"], n["OVER"],
             n["FABRICATED"], e["ERR"] + n["ERR"]))

# family x class under each spelling for BIN
P("\n== effect arms by family, build %s (crate / bare / use) ==" % BIN)
fam = collections.defaultdict(lambda: {sp: collections.Counter() for sp in SPELLS})
for sp in SPELLS:
    for r in R[sp].values():
        if not r["gt"]: continue
        for a in r.get("arms", []):
            fam[r["family"]][sp][base(a[BIN]["class"])] += 1
for f in sorted(fam):
    P("  %-15s " % f + " | ".join("%s C%d D%d L%d S%d" % (sp, fam[f][sp]["CHARGED"], fam[f][sp]["DISCLOSED"],
        fam[f][sp]["LEDGER-ONLY"], fam[f][sp]["SILENT"]) for sp in SPELLS))

# spelling control
P("\n== spelling control, build %s: arms whose class differs across spellings ==" % BIN)
sens = collections.Counter(); sensrows = []
for cid, r in R["use"].items():
    for i, a in enumerate(r.get("arms", [])):
        cl = {}
        for sp in SPELLS:
            rr = R[sp].get(cid)
            if rr and rr.get("arms"):
                cl[sp] = base(rr["arms"][i][BIN]["class"])
        if len(set(cl.values())) > 1:
            k = (r["family"], r["where"], "/".join("%s=%s" % (s, cl.get(s, "-")) for s in SPELLS))
            sens[k] += 1; sensrows.append((cid, a["arm"], cl))
for k, v in sorted(sens.items(), key=lambda x: -x[1])[:60]:
    P("  %4d  %-15s %-12s %s" % (v, k[0], k[1], k[2]))
P("  total spelling-sensitive arms:", len(sensrows))

# silent by mechanism under `use`
def mech(r):
    f, k, w = r["family"], r["kind"], r["where"]
    if f == "hook":
        if k.startswith("drop"): pre = "H6/drop"
        else: pre = "hook"
        m = {"local": "H1 direct in-crate", "mod": "H1 direct other-module", "dep": "H2 direct dep implementor",
             "local_gen": "H5 local generic helper", "local_dyn": "H5 local dyn helper",
             "dep_gen": "H3 consumer generic helper, dep implementor", "dep_dyn": "H3 consumer dyn helper, dep implementor",
             "rev_gen": "H4r dep generic helper, consumer implementor", "rev_dyn": "H4r dep dyn helper, consumer implementor",
             "depH_gen": "H4 dep's own generic helper on its own type"}.get(w, w)
        return "%s: %s" % (pre, m)
    return "%s: %s/%s" % (f, k if f in ("ident", "callable", "recv") else w, w if f in ("ident", "callable", "recv") else "")

def body_visible(sp, r, a):
    d = os.path.join(WORK, sp, "cells", r["cid"], "o-%s-%s" % (BIN, a["arm"]))
    reps = glob.glob(os.path.join(d, "a.*.scan.json")) + glob.glob(os.path.join(d, "dep", "*.scan.json"))
    for rp in reps:
        try:
            for f in json.load(open(rp)).get("functions", []):
                if r["effect"] in f.get("inferred", []) and f.get("fn") != "zz_caller":
                    return True
        except Exception:
            pass
    return False

P("\n== SILENT effect arms by mechanism, build %s, under each spelling (use = the isolating spelling) ==" % BIN)
sil = collections.defaultdict(lambda: {sp: 0 for sp in SPELLS}); vis = collections.Counter(); silrows = []
for sp in SPELLS:
    for r in R[sp].values():
        if not r["gt"]: continue
        for a in r.get("arms", []):
            if base(a[BIN]["class"]) == "SILENT":
                sil[mech(r)][sp] += 1
                if sp == "use":
                    v = body_visible(sp, r, a)
                    vis[(mech(r), a["arm"], "body-charged" if v else ("unch-body-in-dep" if a["arm"] == "unch" else "BODY-NOT-CHARGED"))] += 1
                    silrows.append((r["cid"], a["arm"], mech(r), "visible" if v else "not-visible"))
for m in sorted(sil, key=lambda x: (-sil[x]["use"], x)):
    P("  %-62s crate %3d  bare %3d  use %3d" % (m, sil[m]["crate"], sil[m]["bare"], sil[m]["use"]))
P("  TOTAL", " ".join("%s %d" % (sp, sum(v[sp] for v in sil.values())) for sp in SPELLS))
P("\n  body calibration under `use` (mechanism, arm, is the effectful body charged in a report the arm saw):")
for k, v in sorted(vis.items()):
    P("    %4d  %-55s %-5s %s" % (v, k[0], k[1], k[2]))

# transitions
if PRE:
    P("\n== transitions %s -> %s, per spelling ==" % (PRE, BIN))
    toward_silent = []; newfab = []
    for sp in SPELLS:
        tr = collections.Counter()
        for r in R[sp].values():
            for a in r.get("arms", []):
                x, y = base(a[PRE]["class"]), base(a[BIN]["class"])
                if x != y:
                    tr[(x, y)] += 1
                    rank = {"CHARGED": 3, "DISCLOSED": 2, "LEDGER-ONLY": 1, "SILENT": 0}
                    if r["gt"] and rank.get(y, 9) < rank.get(x, 9):
                        toward_silent.append((sp, r["cid"], a["arm"], x, y))
                    if not r["gt"] and y == "FABRICATED":
                        newfab.append((sp, r["cid"], a["arm"], a[BIN]["class"]))
        P("  %-5s %s" % (sp, ", ".join("%s->%s %d" % (k[0], k[1], v) for k, v in sorted(tr.items()))))
    P("  moves toward silence (effect cells):", len(toward_silent))
    for t in toward_silent: P("    ", *t)
    P("  new FABRICATED:", len(newfab))
    for t in newfab: P("    ", *t)

txt = "\n".join(out) + "\n"
open(os.path.join(OUTD, "summary-%s.txt" % BIN), "w").write(txt)
with open(os.path.join(OUTD, "silent-%s.tsv" % BIN), "w") as f:
    for x in silrows: f.write("\t".join(x) + "\n")
print(txt)

lines = []
for sp in SPELLS:
    for cid in sorted(R[sp]):
        r = R[sp][cid]
        for a in r.get("arms", []):
            lines.append("%s\t%s\t%s\t%s" % (cid, sp, a["arm"], a[BIN]["class"]))
if WB:
    open(WB, "w").write("\n".join(lines) + "\n")
if CK:
    want = set(open(CK).read().split("\n")) - {""}
    have = set(lines)
    diff = sorted(want ^ have)
    for d in diff[:200]:
        print(("MISSING " if d in want else "NEW     ") + d)
    print("baseline check: %d differing line(s)" % len(diff))
    sys.exit(1 if diff else (3 if bad else 0))
sys.exit(3 if bad else 0)
