#!/usr/bin/env python3
"""eval/i1/xcheck.py CRATE_DIR SCANBIN OUTDIR [--all-features] [K=V ...] — the I1 rustc-expansion oracle (SOUNDNESS R1541).

What it is independent of: the scanner's `macro_rules!` expander. rustc expands the crate (`-Zunpretty=expanded`, the pinned
nightly, offline, the crate's own lock), and the SAME scanner binary reads the ORIGINAL tree and the EXPANDED tree. The
expanded tree has no macro left, so every expansion and every macro hedge model is out of play in that arm. What it shares
with the scanner: name resolution, typing and classification. So it can say "the expander minted the wrong set or the
wrong body", not "the resolver is right".

Two legs:
  PRESENCE   every unit in the expanded scan's call-graph sidecar (pure units included) must exist in the original scan.
             A missing unit is EFFECTFUL (it carries something in the expanded scan) or PURE, and is tagged with its
             likely cause: REFUSED (its leaf is a `fn` in a macro the original scan reported as I1REFUSE / I1PARTIAL),
             ARM-P (its owner type carries a non-std derive or an attribute macro), or UNEXPLAINED.
  EFFECTS    for every unit in BOTH scans, `inferred` must agree. UNDER = the expanded scan carries an effect or Unknown the
             original lacks (the silence direction); OVER = the reverse (expected noise: rustc lowers `?`, `format_args!`,
             `async` and std paths differently).

Guards: an expanded scan that judges nothing or fails to parse is NOT A MEASUREMENT (exit 3). `#[path]` modules are spelled
differently by rustc and the scanner, so presence falls back to the last two segments and prints how often. std-derive
impls (`#[automatically_derived]` with a core derive trait) are dropped from the expanded text. The oracle sees only the
cfg rustc compiled (default features unless --all-features): it checks "every compiled unit is present", never the reverse.

Seeds (both recorded as switches by the scanner): CANDOR_I1_SEED_DROP=<macro> must show up in PRESENCE;
CANDOR_I1_SEED_BODY=<macro> must show up in EFFECTS/UNDER.
"""
import glob, json, os, re, shutil, subprocess, sys
args = [a for a in sys.argv[1:] if not a.startswith("--")]
crate, scan, out = args[0], args[1], args[2]
extra = dict(a.split("=", 1) for a in args[3:])
allf = "--all-features" in sys.argv
os.makedirs(out, exist_ok=True)
X = os.path.join(out, "x"); shutil.rmtree(X, ignore_errors=True); os.makedirs(X + "/src")
env = {k: v for k, v in os.environ.items() if not k.startswith("CANDOR")}
cmd = ["cargo", "+nightly-2026-06-14", "rustc", "--offline", "-q", "--lib"] + (["--all-features"] if allf else []) + ["--", "-Zunpretty=expanded"]
r = subprocess.run(cmd, cwd=crate, env=dict(env, CARGO_TARGET_DIR=os.path.join(out, "tgt")), capture_output=True, text=True)
if r.returncode != 0:
    print("NOT A MEASUREMENT: rustc expansion failed:", r.stderr[-600:]); sys.exit(3)
DER = r"::core::(fmt::Debug|clone::Clone|marker::Copy|cmp::PartialEq|cmp::Eq|cmp::PartialOrd|cmp::Ord|hash::Hash|default::Default|marker::StructuralPartialEq)\b"
def strip_std_derives(s):
    o, i, n = [], 0, 0
    while True:
        j = s.find("#[automatically_derived]", i)
        if j < 0: o.append(s[i:]); break
        k = s.find("{", j)
        if not re.search(DER, s[j:k]): o.append(s[i:k]); i = k; continue
        d, m = 0, k
        while m < len(s):
            if s[m] == "{": d += 1
            elif s[m] == "}":
                d -= 1
                if d == 0: break
            m += 1
        o.append(s[i:j]); i = m + 1; n += 1
    return "".join(o), n
def strip_macro_defs(s):
    """rustc keeps every `macro_rules!` DEFINITION in its expanded output, and the scanner's hedge models (R828, R452) read
    their bodies — so an expanded tree would carry the very hedges the original's expansion retired. Drop them: by now
    every invocation is expanded, so a definition is dead text."""
    o, i, n = [], 0, 0
    while True:
        j = s.find("macro_rules!", i)
        if j < 0: o.append(s[i:]); break
        # start of the item: back up over attributes / visibility on the same item
        a = s.rfind("\n", 0, j) + 1
        while a > 0:  # and its doc comments / attributes on the lines above
            pa = s.rfind("\n", 0, a - 1) + 1
            line = s[pa:a].strip()
            if line.startswith("///") or line.startswith("#[") or line.startswith("//!"):
                a = pa
            else:
                break
        k = j + len("macro_rules!")
        while k < len(s) and s[k] not in "{([": k += 1
        if k >= len(s): o.append(s[i:]); break
        op = s[k]; cl = {"{": "}", "(": ")", "[": "]"}[op]; d, m = 0, k
        while m < len(s):
            if s[m] == op: d += 1
            elif s[m] == cl:
                d -= 1
                if d == 0: break
            m += 1
        m += 1
        if m < len(s) and s[m] == ";": m += 1
        o.append(s[i:a]); i = m; n += 1
    return "".join(o), n
src, ndrop = strip_std_derives(r.stdout)
src, nmac = strip_macro_defs(src)
open(X + "/src/lib.rs", "w").write(src)
shutil.copy(os.path.join(crate, "Cargo.toml"), X + "/Cargo.toml")
if os.path.exists(os.path.join(crate, "Cargo.lock")): shutil.copy(os.path.join(crate, "Cargo.lock"), X + "/Cargo.lock")
def run(path, tag, e):
    o = os.path.join(out, tag); shutil.rmtree(o, ignore_errors=True)
    p = subprocess.run([scan, path, "--out", o + "/r"], env=e, capture_output=True, text=True)
    rep = [f for f in glob.glob(o + "/r*.json") if not f.endswith((".callgraph.json", ".locs.json", ".hierarchy.json"))]
    cg = glob.glob(o + "/r*.callgraph.json")
    if not rep or not cg: print("NOT A MEASUREMENT: the %s scan wrote no report" % tag); sys.exit(3)
    d = json.load(open(rep[0])); g = json.load(open(cg[0]))
    if d.get("unanalyzed") or not d.get("analyzed", {}).get("count"):
        print("NOT A MEASUREMENT: the %s scan judged nothing or could not parse a file: %s" % (tag, d.get("unanalyzed"))); sys.exit(3)
    rows, keys = {}, {}
    for f in d["functions"]:
        if f.get("interfaceUnion"): continue
        rows.setdefault(f["fn"], set()).update(f.get("inferred", []))
        keys.setdefault(f["fn"], set()).update(f.get("dispatchesOn", []) or [])
    return rows, g, p.stderr, keys
orow, og, oerr, okeys = run(crate, "orig", dict(env, CANDOR_I1_INSTR="1", **extra))
xrow, xg, _, xkeys = run(X, "exp", env)
def t2(q): return "::".join(q.split("::")[-2:])
def count(keys):
    c = {}
    for q in keys: c[t2(q)] = c.get(t2(q), 0) + 1
    return c
ot2, xt2 = count(og), count(xg)
raw = sorted(set(xg) - set(og))
missing = [u for u in raw if ot2.get(t2(u), 0) < xt2.get(t2(u), 0)]
# tags
refused = set(re.findall(r"I1(?:REFUSE\t[a-z-]+|PARTIAL)\t(\S+)", oerr))
srcs = "".join(open(f, errors="replace").read() for f in glob.glob(crate + "/src/**/*.rs", recursive=True))
refused_leaves = set()
for mname in refused:
    for m in re.finditer(r"macro_rules!\s*" + re.escape(mname) + r"\s*\{(.*?)(?=\nmacro_rules!|\Z)", srcs, re.S):
        refused_leaves |= set(re.findall(r"\bfn\s+([A-Za-z_]\w*)", m.group(1)))
STD = {"Debug", "Clone", "Copy", "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Default"}
argp = set()
for m in re.finditer(r"#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union)\s+(\w+)", srcs):
    if any(x.strip().split("::")[-1] not in STD for x in m.group(1).split(",") if x.strip()): argp.add(m.group(2))
for m in re.finditer(r"#\[(?!derive|cfg|doc|allow|warn|deny|inline|repr|must_use|non_exhaustive|deprecated|test)[\w:]+[^\]]*\]\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|union)\s+(\w+)", srcs):
    argp.add(m.group(1))
pres = {"EFFECTFUL": [], "PURE": []}
for u in missing:
    tag = "REFUSED" if u.split("::")[-1] in refused_leaves else ("ARM-P" if len(u.split("::")) > 1 and u.split("::")[-2] in argp else "UNEXPLAINED")
    pres["EFFECTFUL" if xrow.get(u) else "PURE"].append((u, tag, sorted(xrow.get(u, set()))))
under, over = [], []
for u in sorted(set(xg) & set(og)):
    a, b = orow.get(u, set()), xrow.get(u, set())
    if b - a: under.append((u, sorted(a), sorted(b)))
    if a - b: over.append((u, sorted(a), sorted(b)))
# KEYS — a `dispatchesOn` key the hand-written (expanded) program's row carries and the original's lacks: the ⟨0.39⟩
# obligation-1 join channel the expansion lost relative to what the engine does for the same program written out.
keys_lost = [(u, sorted(xkeys.get(u, set()) - okeys.get(u, set()))) for u in sorted(set(xg) & set(og))
             if xkeys.get(u, set()) - okeys.get(u, set())]
res = {"keys_lost": keys_lost, "std_derives_dropped": ndrop, "macro_defs_dropped": nmac, "units_orig": len(og), "units_exp": len(xg), "tail2_fallbacks": len(raw) - len(missing),
       "missing": pres, "under": under, "over": over, "refused_macros": sorted(refused)}
json.dump(res, open(os.path.join(out, "xcheck.json"), "w"), indent=1)
tags = lambda L: {t: sum(1 for x in L if x[1] == t) for t in ("REFUSED", "ARM-P", "UNEXPLAINED")}
print("KEYS lost vs expanded program: %d units" % len(keys_lost))
print("units orig %d exp %d | missing %d (tail2 fallbacks %d) effectful %s pure %s | effects UNDER %d OVER %d"
      % (len(og), len(xg), len(missing), len(raw) - len(missing), tags(pres["EFFECTFUL"]), tags(pres["PURE"]), len(under), len(over)))
for x in pres["EFFECTFUL"][:15]: print("  MISSING-EFF", x)
for x in under[:15]: print("  UNDER", x)
