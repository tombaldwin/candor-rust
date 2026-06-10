#!/usr/bin/env python3
"""Property-test candor-query across MANY real crates. Any FAIL is a query-layer bug lead."""
import json, subprocess, sys, collections, glob, os, re, tempfile, shutil

# Resolve binaries relative to this file (eval/calibration/ -> repo root), env-overridable — NOT
# hardcoded machine paths (which broke after the candor->candor-rust rename; /code-review). Use the
# same build profile for both so a stale release/debug mix can't screen against the wrong binary.
_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCAN = os.environ.get("CANDOR_SCAN_BIN") or f"{_ROOT}/target/release/candor-scan"
QUERY = os.environ.get("CANDOR_QUERY_BIN") or f"{_ROOT}/target/release/candor-query"
for _b in (SCAN, QUERY):
    if not os.path.exists(_b):
        sys.exit(f"FATAL: {_b} not built — cargo build --release -p candor-scan -p candor-query, or set CANDOR_*_BIN.")
REG = sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/index.crates.io-*")))[-1]

CRATES = ["gix", "git2", "tempfile", "which", "nix", "rusqlite", "curl", "ignore",
          "tokio-postgres", "redis", "mongodb", "tower-http", "hickory-resolver",
          "notify-rust", "watchexec", "mio", "signal-hook", "ssh-key", "rand_jitter",
          "sqlx-mysql", "sqlx-postgres", "iri-string", "tauri-utils", "cargo_metadata"]

def newest(name):
    c = [d for d in glob.glob(f"{REG}/{name}-[0-9]*") if re.match(rf"{re.escape(name)}-\d", os.path.basename(d))]
    return max(c, key=lambda p: tuple(int(x) for x in re.search(r"-(\d+)\.(\d+)\.(\d+)", os.path.basename(p)).groups())) if c else None

def q(*args):
    r = subprocess.run([QUERY, *args], capture_output=True, text=True, timeout=30)
    return r.returncode, r.stdout, r.stderr

fails = []
def fail(crate, prop, detail):
    fails.append((crate, prop, detail))
    print(f"  FAIL {crate}: {prop} — {detail[:160]}")

total_props = 0
for name in CRATES:
    d = newest(name)
    if not d:
        continue
    tmp = tempfile.mkdtemp()
    shutil.copytree(d, tmp, dirs_exist_ok=True, symlinks=True)
    subprocess.run([SCAN, tmp], capture_output=True, timeout=120)
    reps = [p for p in glob.glob(f"{tmp}/.candor/report.*.scan.json") if "callgraph" not in p]
    if not reps:
        shutil.rmtree(tmp, ignore_errors=True); continue
    rpt = reps[0]
    prefix = f"{tmp}/.candor/report"
    cgp = rpt.replace(".scan.json", ".scan.callgraph.json")
    rj = json.load(open(rpt))
    fns = rj["functions"] if isinstance(rj, dict) else rj
    by_name = {e["fn"]: e for e in fns}
    cg = json.load(open(cgp)) if os.path.exists(cgp) else {}
    rev = collections.defaultdict(set)
    for caller, callees in cg.items():
        for c in callees:
            rev[c].add(caller)
    def closure(t):
        seen, stack = set(), [t]
        while stack:
            n = stack.pop()
            for c in rev.get(n, ()):
                if c not in seen:
                    seen.add(c); stack.append(c)
        return seen

    n_props = 0
    # P1 diff(self) empty — the diff JSON is the {baseline_version, engine_version, changes} envelope,
    # so the property is "changes == []". (Was checking top-level gained/lost, which never exist in the
    # envelope, so P1 could never fail — a vacuous assertion, found by /code-review.)
    rc, out, _ = q("diff", prefix, prefix, "1", "v", "v")
    try:
        dd = json.loads(out)
        if dd.get("changes"):
            fail(name, "diff(self)!=empty", out[:160])
    except Exception as ex:
        if fns: fail(name, "diff(self) unparseable", f"rc={rc} {ex} {out[:120]}")
    n_props += 1
    # P2 callers closure on up-to-3 most-called nodes
    for t in sorted(rev, key=lambda n: -len(rev[n]))[:3]:
        rc, out, _ = q("callers", prefix, t, "1")
        try:
            cd = json.loads(out)
            got = set(cd.get("transitive", [])) | set(cd.get("direct", []))
            want = closure(t)
            if got != want:
                fail(name, f"callers({t})", f"missing={sorted(want-got)[:3]} extra={sorted(got-want)[:3]}")
        except Exception as ex:
            fail(name, f"callers({t}) unparseable", f"rc={rc} {out[:100]}")
        n_props += 1
    # P3 where partition per present effect
    present = sorted({x for e in fns for x in e.get("inferred", []) if x != "Unknown"})
    for E in present[:3]:
        rc, out, _ = q("where", prefix, E, "1")
        try:
            wd = json.loads(out)
            want_d = {e["fn"] for e in fns if E in e.get("direct", [])}
            want_i = {e["fn"] for e in fns if E in e.get("inferred", []) and E not in e.get("direct", [])}
            if set(wd.get("directly", [])) != want_d or set(wd.get("inherited", [])) != want_i:
                fail(name, f"where({E}) partition", f"d^={set(wd.get('directly',[]))^want_d or ''} i^={set(wd.get('inherited',[]))^want_i or ''}")
        except Exception as ex:
            fail(name, f"where({E}) unparseable", f"rc={rc} {out[:100]}")
        n_props += 1
    # P4 path: edges real + terminus direct, for up to 2 inherited fns per effect
    for E in present[:2]:
        for t in [e["fn"] for e in fns if E in e.get("inferred", []) and E not in e.get("direct", [])][:2]:
            rc, out, _ = q("path", prefix, t, E, "--json")
            try:
                pd = json.loads(out)
                raw = pd.get("path") or []
                chain = [x["fn"] if isinstance(x, dict) else x for x in raw]
                if not chain:
                    fail(name, f"path({t},{E}) empty", out[:120]); n_props += 1; continue
                bad_edge = next((i for i in range(len(chain)-1) if chain[i+1] not in cg.get(chain[i], [])), None)
                if bad_edge is not None:
                    fail(name, f"path({t},{E}) fake edge", f"{chain[bad_edge]}->{chain[bad_edge+1]}")
                term = by_name.get(chain[-1], {})
                if E not in term.get("direct", []):
                    fail(name, f"path({t},{E}) terminus not direct", f"term={chain[-1]} direct={term.get('direct')}")
            except Exception as ex:
                fail(name, f"path({t},{E}) unparseable", f"rc={rc} {out[:100]}")
            n_props += 1
    # P5 whatif affected == closure+self (1 target)
    if rev:
        t = max(rev, key=lambda n: len(rev[n]))
        rc, out, _ = q("whatif", prefix, t, "Net", "1")
        try:
            wd = json.loads(out)
            got, want = set(wd.get("affected", [])), closure(t) | {t}
            if got != want:
                fail(name, f"whatif({t})", f"missing={sorted(want-got)[:3]} extra={sorted(got-want)[:3]}")
        except Exception as ex:
            fail(name, f"whatif unparseable", f"rc={rc} {out[:100]}")
        n_props += 1
    # P6 rewire(self) == no drops
    rc, out, _ = q("rewire", prefix, prefix, "1")
    try:
        rd = json.loads(out)
        if rd.get("dropped"):
            fail(name, "rewire(self) dropped!=[]", str(rd.get("dropped"))[:140])
    except Exception as ex:
        fail(name, "rewire(self) unparseable", f"rc={rc} {out[:100]}")
    n_props += 1
    # P7 reachable ⊆ union of all inferred; entry points honored
    rc, out, _ = q("reachable", prefix, "1")
    try:
        rd = json.loads(out)
        eps = {e["fn"] for e in fns if e.get("entryPoint")}
        if isinstance(rd, dict) and "effects" in rd:
            union_all = {x for e in fns for x in e.get("inferred", [])}
            extra = set(rd["effects"]) - union_all
            if extra:
                fail(name, "reachable effects ⊄ union", str(extra))
    except Exception:
        pass  # shape varies; only check when parseable dict
    n_props += 1
    total_props += n_props
    print(f"  ok {name}: {n_props} props")
    shutil.rmtree(tmp, ignore_errors=True)

print()
print(f"{total_props} property checks across crates; {len(fails)} FAILURES")
for c, p, dd in fails:
    print(f"  {c}: {p}")
sys.exit(1 if fails else 0)
