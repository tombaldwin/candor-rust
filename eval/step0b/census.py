#!/usr/bin/env python3
"""Unit-level census: which effects does candor-scan miss, PER UNIT, against an independent engine, and which site loses each.

    census.py --oracle DIR --sidecar DIR --report SCAN_JSON [--root SRC] [--pub-report SCAN_JSON] [--json OUT] [--label L]

THE INDEPENDENT ENGINE is the dylint lint in this repo (rustc-typed, HIR + MIR drop edges), whose per-unit TRANSITIVE effect
set the oracle dump writes beside its site records (`*.census`, `CANDOR_ORACLE_DUMP`). It shares candor-scan's leaf
classifier (`candor-classify`), so a classification difference is held constant and what remains is resolution, typing,
dispatch and coverage. Its own under-reports are invisible here (both engines silent): the matrix calibration measures that.

A GATE FLIP is a (unit, effect E) where the lint's unit carries E and candor-scan's unit does not, so `deny E <unit>` reads 0
on candor-scan. `flipEU` additionally requires candor-scan's unit to carry no `Unknown`, so `deny E Unknown <unit>` reads 0 too.

ATTRIBUTION. For each flip, the lint's own per-expression record says which expressions in the unit's body contributed E:
a leaf (a classified call), or a local edge to a callee whose lint set carries E. Each is resolved against candor-scan:
  * an edge candor-scan's FINAL call graph also has (any route, including post-fixpoint ones such as `.into()` -> `From`):
    the loss is downstream, in the callee -> recurse;
  * otherwise the comparator's verdict and class at that site (compare.py), or LINT-MECHANISM when the lint formed the
    edge with no call site of its own (deref adjustment, static force, callback flow, implicit drop).
The leaf-level causes reached are the flip's ROOT causes.
"""
import argparse
import collections
import glob
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import compare as C  # noqa: E402  (the in-tree comparator; never copy it -- R288)

UNKNOWN = "Unknown"


def load_census(d):
    units, contrib, post = {}, collections.defaultdict(list), collections.defaultdict(list)
    files = sorted(glob.glob(os.path.join(d, "*.census")))
    for f in files:
        for line in open(f):
            r = json.loads(line)
            k = r["kind"]
            if k == "unit":
                units[r["def"]] = r
            elif k == "contrib":
                contrib[r["caller"]].append(r)
            elif k == "postedge":
                post[r["caller"]].append(r)
    return files, units, contrib, post


def key3(k):
    return (C.norm(k["file"]), k["line"], k["col"])


class Report:
    def __init__(self, path):
        d = json.load(open(path))
        self.doc = d
        self.fns = {}
        for f in d.get("functions", []):
            self.fns.setdefault(f["fn"], set()).update(f.get("inferred", []))
        cg = path.replace(".scan.json", ".scan.callgraph.json")
        self.cg = {}
        if os.path.exists(cg):
            self.cg = {k: set(v) for k, v in json.load(open(cg)).items()}
        self.excluded_classes = [x.get("class") for x in d.get("excluded", [])]

    def eff(self, q):
        return self.fns.get(q)


def path_qual(path):
    """rustc's printed item path to the scanner's qual: `<impl A for B>` segments become B's last segment, generic
    arguments are dropped. `None` when the path has a shape this does not read."""
    import re
    p = path
    # `<X as Tr>::m` (the UFCS form rustc prints for a trait impl) -> `X::m`
    m0 = re.match(r"^<(.+?) as .+>::(\w+)$", p)
    if m0:
        p = C.strip_generics(m0.group(1)).strip().lstrip("&").replace("mut ", "").strip() + "::" + m0.group(2)
    while True:
        m = re.search(r"<impl\b", p)
        if not m:
            break
        d, i = 0, m.start()
        while i < len(p):
            if p[i] == "<": d += 1
            elif p[i] == ">":
                d -= 1
                if d == 0: break
            i += 1
        inner = p[m.start() + 1:i]
        ty = inner.split(" for ", 1)[1] if " for " in inner else inner[len("impl "):]
        ty = C.strip_generics(ty).strip().lstrip("&").strip()
        ty = ty.replace("mut ", "").split("::")[-1]
        p = p[:m.start()] + ty + p[i + 1:]
    p = re.sub(r"(::)+", "::", C.strip_generics(p)).strip(":")
    return p if re.fullmatch(r"[A-Za-z_][\w]*(::[A-Za-z_][\w]*)*", p) else None


_SRC = {}


def i1_kind(u, root):
    """SOUNDNESS R1541 (I1) — WHICH macro made an unmapped unit, read from the source at the lint unit's `ident` (rustc's ident
    span of an expanded item is inside the INVOCATION): the nearest preceding `name!(`, `#[derive(..)]` or attribute.
    `: L <name>` a crate-local `macro_rules!`; `: D1 <name>` a bang macro the crate does not define (a dependency's);
    `: P1` a derive; `: P2` an attribute macro; `` when nothing is found. A heuristic over text, stated as one."""
    import re
    idn = u.get("ident") or {}
    f = C.norm(idn.get("file", ""))
    if not f or not os.path.exists(f):
        return ""
    if f not in _SRC:
        _SRC[f] = open(f, errors="replace").read().split("\n")
    lines = _SRC[f]
    ln = idn.get("line", 0)
    text = "\n".join(lines[max(0, ln - 40):ln])
    best, kind = -1, ""
    for m in re.finditer(r"([A-Za-z_][\w:]*)!\s*[({\[]", text):
        if m.start() > best and m.group(1).split("::")[-1] != "macro_rules":
            best, kind = m.start(), "bang:" + m.group(1).split("::")[-1]
    for m in re.finditer(r"#\[derive\(", text):
        if m.start() > best:
            best, kind = m.start(), "P1"
    for m in re.finditer(r"#\[(?!derive|cfg|doc|allow|warn|deny|inline|repr|must_use|non_exhaustive|deprecated|test)[\w:]+", text):
        if m.start() > best:
            best, kind = m.start(), "P2"
    if kind.startswith("bang:"):
        name = kind[5:]
        if "__local_macros" not in _SRC:
            names = set()
            for g in glob.glob(os.path.join(root, "**", "*.rs"), recursive=True):
                try:
                    names |= set(re.findall(r"macro_rules!\s*([A-Za-z_]\w*)", open(g, errors="replace").read()))
                except OSError:
                    pass
            _SRC["__local_macros"] = names
        return ": %s %s" % ("L" if name in _SRC["__local_macros"] else "D1", name)
    return (": " + kind) if kind else ""


def map_unit(u, scan):
    """The candor-scan qual of a lint unit, or (None, why)."""
    sp = u.get("span") or {}
    f = C.norm(sp.get("file", ""))
    if u.get("exp") and (not f or not f.startswith(scan.root + os.sep)):
        # A LOCAL unit a DEPENDENCY's macro generated (sqlx_core's item macros): its span is in the dependency's source,
        # but it is this crate's code and candor-scan has no unit for it. ABSENT, not NOT-COVERED (Fable census-2 §1.5):
        # the tree test used to run first and filed it as outside the scan.
        return None, "ABSENT(macro-generated unit, span in dependency source)"
    if not f or not f.startswith(scan.root + os.sep):
        return None, "NOT-COVERED(outside scanned tree)"
    if f in scan.excluded:
        return None, "NOT-COVERED(excluded, disclosed: %s)" % scan.excluded[f]
    rows = scan.fn_rows.get(f) or []
    line, col = sp.get("line"), sp.get("col")
    leaf = C.strip_generics(u.get("path") or "").split("::")[-1].strip()
    exact = [q for (l, c, q) in rows if l == line and c == col]
    if len(set(exact)) == 1:
        return exact[0], None
    # SOUNDNESS R1802 — N expansions of ONE macro can share a span (one per invocation, or — before candor-scan re-spanned
    # same-file expansions to the invocation — every one at the macro DEFINITION). Among rows sharing a position the leaf
    # decides; the first-row-wins reading filed N-1 of hickory-resolver's eleven `*_lookup` units as ABSENT.
    if exact:
        byleaf = [q for q in exact if q.split("::")[-1] == leaf]
        if len(set(byleaf)) == 1:
            return byleaf[0], None
    idl = (u.get("ident") or {}).get("line")
    same = [q for (l, c, q) in rows if l in (line, idl)]
    if len(set(same)) == 1:
        return same[0], None
    byleaf = [q for q in same if q.split("::")[-1] == leaf]
    if len(set(byleaf)) == 1:
        return byleaf[0], None
    # candor-scan's row starts at the item's first ATTRIBUTE or doc comment; rustc's def span at the item itself. The row
    # is the nearest one at or before rustc's span whose leaf is this unit's name, with no other unit's row in between.
    # R1802: every row AT that nearest position is a candidate, not only the last one sorted.
    before = [(l, c, q) for (l, c, q) in rows if (l, c) <= (line, col)]
    if before:
        l0, c0, _ = before[-1]
        at = [q for (l, c, q) in before if (l, c) == (l0, c0) and q.split("::")[-1] == leaf]
        if len(set(at)) == 1:
            return at[0], None
    if u.get("exp"):
        # SOUNDNESS R1541 / R1802 — an expanded unit's rustc span and ident can both sit in the macro DEFINITION
        # (`fn sample` written in `float_impls!`'s body) while candor-scan, re-spanning to the invocation, puts the row at
        # the invocation. Fall back to the QUALIFIED path rustc prints (`m::<impl Tr<X> for T>::f` -> `m::T::f`), matched
        # against the rows of the same file only, and only when exactly one row carries it.
        q = path_qual(u.get("path") or "")
        cands = {qq for (_, _, qq) in rows if qq == q} if q else set()
        if not cands and q:
            # `#[path]` modules are spelled differently by rustc and the scanner (`uniform::int` vs `uniform_int`): the
            # last two segments (`UniformInt::sample_single`), within the same FILE, unique as a set of quals.
            t2 = "::".join(q.split("::")[-2:])
            cands = {qq for (_, _, qq) in rows if "::".join(qq.split("::")[-2:]) == t2}
        if len(cands) == 1:
            return cands.pop(), None
        return None, "ABSENT(macro-generated unit%s)" % i1_kind(u, scan.root)
    if not rows:
        return None, "ABSENT(file has no scanned unit)"
    return None, "ABSENT(no scanner unit at this definition)"


def t_cause(row, osite, site_rows, osites_by_key):
    """Split a T site by WHY the receiver was untyped, from rustc's provenance of the receiver value."""
    steps = (osite or {}).get("recv_origin") or []
    if not steps:
        return "T:no-provenance"
    upstream = []
    for s in steps:
        if s.get("k") in ("call", "mcall") and s.get("key"):
            r = site_rows.get(key3(s["key"]))
            if r is not None:
                upstream.append((s["k"], r.get("verdict"), r.get("class")))
            else:
                o = osites_by_key.get(key3(s["key"])) or {}
                st = (o.get("resolved") or {}).get("status")
                upstream.append((s["k"], "AGREE(ctor/const)" if st == "not-fn" else "NO-ROW", None))
    if any(v and not v.startswith("AGREE") and c == "N/M" for (_, v, c) in upstream):
        return "T<-N (a name upstream did not resolve)"
    # The NEAREST producer decides (Fable census-2 review §1.2): the steps run from the receiver outward, and almost every
    # chain bottoms out in a parameter (`self`). `self.connection().await?.execute(..)` was lost at `connection()`'s
    # return type, not at `self`; reading the parameter first misfiled such sites as "annotated fn parameter".
    ui = iter(upstream)
    for s in steps:
        k = s.get("k")
        if k in ("call", "mcall") and s.get("key"):
            (_, v, c) = next(ui)
            if v == "AGREE(ctor/const)":
                return "T:typer(value built by a constructor)"
            if v and v.startswith("AGREE"):
                return "T:typer(return type of a resolved call not followed)"
            if v and c == "T":
                return "T<-T (an untyped value upstream)"
            if v and v != "NO-ROW":
                return "T<-%s upstream" % (c or v)
            continue
        if k == "param":
            if s.get("closure") and not s.get("annotated"):
                return "T:typer(unannotated closure parameter)"
            if s.get("annotated") and not s.get("closure"):
                return "T:typer?(annotated fn parameter%s)" % (", destructured" if s.get("destructured") else "")
            return "T:typer(parameter)"
    return "T:typer(%s)" % ",".join(s.get("k", "?") for s in steps[:3])


def run(oracle_dir, sidecar_dir, report, root=None, pub_report=None, label=""):
    crates, rows, excluded = C.compare(oracle_dir, sidecar_dir, root, None, report)
    _, osites = C.load_oracle(oracle_dir)
    cfiles, units, contrib, post = load_census(oracle_dir)
    if not cfiles:
        raise SystemExit("census: NOT A MEASUREMENT -- no *.census in %s (old oracle, or the crate did not build)" % oracle_dir)
    docs = C.load_sidecar(sidecar_dir)
    root = C.norm(root) if root else None
    docs = [d for d in docs if not root or C.norm(d["root"]).startswith(root)]
    if len(docs) != 1:
        raise SystemExit("census: expected one sidecar doc under the root, found %d" % len(docs))
    scan = C.Scan(docs[0])
    if not scan.has_excluded_record:
        raise SystemExit("census: the sidecar carries no `excluded` record (pre-R1768 binary): NOT A MEASUREMENT")
    rep = Report(report)
    prep = Report(pub_report) if pub_report else None
    site_rows = {}
    for r in rows:
        if r["side"] == "oracle" and r.get("file"):
            k = (C.norm(r["file"]), r["line"], r["col"])
            # `x?`'s desugared `branch` call is keyed at the operand's start, the same key as the operand's own call
            # (`raw_sql(..)?`): the written call's row wins.
            if k not in site_rows or (site_rows[k].get("desugar") and not r.get("desugar")):
                site_rows[k] = r
    osite_by_raw = {}
    for o in osites:
        if o.get("raw"):
            osite_by_raw.setdefault(key3(o["raw"]), o)
    osite_by_key = {}
    for o in osites:
        if o.get("key") and not o.get("exp"):
            osite_by_key.setdefault(key3(o["key"]), o)

    # CLEANED LINT SETS. The lint charges a calibrated crate's OWN items by the crate's name (getrandom, rand -> Rand) or
    # by member name (no-std-net's own `to_socket_addrs` -> Net). Excluding those leaves only at the root (LINT-ARTEFACT)
    # is not enough: the artefact effect PROPAGATES, and a caller's flip then gets an innocent-looking root (a D site
    # whose callee carries only an artefact Rand). So the lint's sets are recomputed from its own per-expression record
    # with artefact leaves dropped, and every flip is judged against the cleaned set. Self-check: recomputed WITHOUT the
    # drop, the sets must equal the dump's, or the record is incomplete and the run says so.
    def is_artefact_leaf(c):
        # Judge by the oracle's OWN record of this expression (matched on its raw span, which is unique even inside a
        # macro expansion where many sites share one key), falling back to the comparator row and the lint's own edge.
        o = osite_by_raw.get(key3(c["raw"])) if c.get("raw") else None
        if o is None:
            o = osite_by_key.get(key3(c["key"])) or {}
        tdef = ((o.get("resolved") or {}).get("def")) or (o.get("named") or {})
        if ((tdef.get("parent") or {}).get("kind")) == "ForeignMod":
            return False
        if tdef.get("local"):
            return True
        r = site_rows.get(key3(c["key"]))
        return (r is not None and r.get("target_kind") == "local") or bool(c.get("edges"))

    def recompute(drop_artefacts):
        direct, calls = collections.defaultdict(set), collections.defaultdict(set)
        for d0, cs in contrib.items():
            for c in cs:
                eff = set(c.get("eff", []))
                if drop_artefacts and eff - {UNKNOWN} and is_artefact_leaf(c):
                    eff &= {UNKNOWN}
                direct[d0] |= eff | set(c.get("cross", []))
                calls[d0] |= set(c.get("edges", []))
        for d0, ps in post.items():
            for pe in ps:
                calls[d0].add(pe["target"])
        acc = {d0: set(direct.get(d0, set())) for d0 in set(units) | set(direct)}
        changed = True
        while changed:
            changed = False
            for f0, gs in calls.items():
                add = set()
                for g in gs:
                    add |= acc.get(g, set())
                cur = acc.setdefault(f0, set())
                if not add <= cur:
                    cur |= add
                    changed = True
        return acc

    raw = recompute(False)
    mism = [d0 for d0, u in units.items() if u["dk"] in ("Fn", "AssocFn") and
            {x for x in u["eff"] if x != UNKNOWN} != {x for x in raw.get(d0, set()) if x != UNKNOWN}]
    clean = recompute(True)

    def lint_eff(d):
        return set(clean.get(d, set())) if d in units else set()

    qual_of, why_absent = {}, {}
    for d, u in units.items():
        if u["dk"] not in ("Fn", "AssocFn"):
            continue
        q, why = map_unit(u, scan)
        qual_of[d] = q
        if why:
            why_absent[d] = why

    memo = {}

    def causes(d, e, depth=0, seen=None):
        """Root causes of the scanner lacking e at lint unit d (which the lint charges)."""
        seen = seen or set()
        if (d, e) in memo:
            return memo[(d, e)]
        if (d, e) in seen or depth > 40:
            return [{"cls": "CYCLE"}]
        seen = seen | {(d, e)}
        out = []
        q = qual_of.get(d)
        for c in contrib.get(d, []):
            k = key3(c["key"])
            r = site_rows.get(k)
            loc = "%s:%s:%s" % (os.path.relpath(k[0], scan.root), k[1], k[2])
            if e in c.get("eff", []) or e in c.get("cross", []):
                o = osite_by_key.get(k) or {}
                tdef = ((o.get("resolved") or {}).get("def")) or (o.get("named") or {})
                foreign = ((tdef.get("parent") or {}).get("kind")) == "ForeignMod"
                if foreign:
                    # A locally DECLARED FFI import (`extern "C" { fn getentropy(..); }`) is a local DefId whose leaf charge
                    # is genuine (the classifier's FFI-thin tier charges by leaf name), so it is never an artefact (Fable
                    # census-2 §1.1). Judge it as an ordinary leaf.
                    pass
                if not foreign and e in c.get("eff", []) and is_artefact_leaf(c):
                    # A LEAF charge on a call whose rustc target is a LOCAL item (or that the lint itself edged to a local
                    # unit): the lint classified this crate's own function by the crate's calibrated NAME (getrandom ->
                    # Rand) or by the member's name (no-std-net's own `to_socket_addrs` -> Net). Not an effect the body
                    # reaches; an artefact of the independent engine, measured and excluded, never a flip's cause.
                    out.append({"cls": "LINT-ARTEFACT(leaf charged on a local item)", "site": loc, "unit": d, "truth": r.get("truth")})
                elif c.get("nosite") or r is None:
                    out.append({"cls": "LINT-MECHANISM(leaf, %s)" % c.get("expr"), "site": loc, "unit": d})
                elif r["verdict"].startswith("AGREE"):
                    out.append({"cls": "LEAF-REACHED-NOT-CHARGED", "site": loc, "unit": d, "truth": r.get("truth")})
                else:
                    out.append(site_cause(r, loc, d))
            for g in c.get("edges", []):
                if e not in lint_eff(g):
                    continue
                gq = qual_of.get(g)
                has_edge = q is not None and gq is not None and (gq == q or gq in rep.cg.get(q, set()))
                if has_edge:
                    ge = rep.eff(gq) or set()
                    if e in ge:
                        out.append({"cls": "PROPAGATION(scanner edge present, callee charged, caller not)", "site": loc, "unit": d})
                    else:
                        out.extend(causes(g, e, depth + 1, seen))
                    continue
                if gq is None and g in why_absent:
                    out.append({"cls": "CALLEE-" + why_absent[g], "site": loc, "unit": d, "callee": units[g]["path"]})
                    continue
                if c.get("nosite") or r is None:
                    out.append({"cls": "LINT-MECHANISM(edge, %s)" % c.get("expr"), "site": loc, "unit": d, "callee": units.get(g, {}).get("path")})
                elif r["verdict"].startswith("AGREE"):
                    cp = units.get(g, {}).get("path")
                    if r.get("truth") and cp and C.strip_generics(r["truth"]) != C.strip_generics(cp):
                        # The site resolved to a GENERIC callee; the lint's edge is to the impl the caller's type argument
                        # selects inside it (monomorphisation), which candor-scan did not form.
                        out.append({"cls": "D(generic callee: the caller's type argument dispatches inside it)", "site": loc,
                                    "unit": d, "callee": cp, "truth": r.get("truth")})
                    else:
                        # The site agrees (it named this target) but the final graph has no edge: post-site loss.
                        out.append({"cls": "SITE-AGREE-EDGE-MISSING", "site": loc, "unit": d, "callee": cp})
                else:
                    out.append(site_cause(r, loc, d, units.get(g, {}).get("path")))
                # The callee may ALSO lack e on the scanner: a second loss behind the first.
                if gq is not None and e not in (rep.eff(gq) or set()):
                    for x in causes(g, e, depth + 1, seen):
                        out.append(dict(x, behind=True))
        for pe in post.get(d, []):
            g = pe["target"]
            if e not in lint_eff(g):
                continue
            gq = qual_of.get(g)
            if q is not None and gq is not None and (gq == q or gq in rep.cg.get(q, set())):
                if e not in (rep.eff(gq) or set()):
                    out.extend(causes(g, e, depth + 1, seen))
                continue
            out.append({"cls": "LINT-MECHANISM(post-edge, %s)" % pe["ekind"], "unit": d, "callee": units.get(g, {}).get("path")})
        if not out:
            out.append({"cls": "UNATTRIBUTED", "unit": d})
        # One root per (class, site, callee): the lint records a call and its callee path as two expressions.
        seen_r, uniq = set(), []
        for x in out:
            k = (x.get("cls"), x.get("site"), x.get("callee"), x.get("unit"), x.get("behind"))
            if k not in seen_r:
                seen_r.add(k)
                uniq.append(x)
        out = uniq
        memo[(d, e)] = out
        return out

    def site_cause(r, loc, d, callee=None):
        cls = r.get("class") or r["verdict"]
        x = {"cls": cls, "verdict": r["verdict"], "site": loc, "unit": d, "target_kind": r.get("target_kind"),
             "truth": r.get("truth"), "member": r.get("member"), "form": r.get("form")}
        if callee:
            x["callee"] = callee
        if cls == "T":
            x["tcause"] = t_cause(r, osite_by_key.get((C.norm(r["file"]), r["line"], r["col"])), site_rows, osite_by_key)
        if r["verdict"] == "NOT-COVERED":
            x["cls"] = "NOT-COVERED(%s)" % (r.get("why") or "")
        return x

    flips = []
    pub_flips = []
    population = collections.Counter()
    def enclosing_qual(u):
        sp = u.get("span") or {}
        ow = scan.owner_of(sp.get("file", ""), sp.get("line", 0), sp.get("col", 0))
        return ow[2] if ow else None

    for d, u in sorted(units.items()):
        if u["dk"] not in ("Fn", "AssocFn"):
            continue
        le = {x for x in lint_eff(d) if x != UNKNOWN}
        if {x for x in u["eff"] if x != UNKNOWN} - le:
            population["lint units losing an effect to artefact cleaning"] += 1
        q = qual_of.get(d)
        if q is None and why_absent.get(d, "").startswith("ABSENT(no scanner unit") and le:
            # An item nested in a fn body (`fn with_features() { fn detect_family_inner() .. }`, an `impl Drop` inside a
            # fn) is not a unit of candor-scan: its body is folded into the enclosing unit. No gate can name it; the gate
            # that can is the enclosing unit's. Count it as folded when that unit carries every effect, else as a flip.
            eq = enclosing_qual(u)
            ee = rep.eff(eq) if eq else None
            if ee is not None and le <= ee:
                population["ABSENT nested items folded into an enclosing unit that carries their effects (not flips)"] += 1
                continue
        population["lint units"] += 1
        if le:
            population["lint units with a concrete effect"] += 1
        if q is None:
            why = why_absent.get(d, "ABSENT")
            population["unmapped: " + why] += 1
            if not le:
                continue
            if why.startswith("NOT-COVERED"):
                population["NOT-COVERED units carrying a concrete effect (not flips)"] += 1
                continue
            for e in sorted(le):
                flips.append({"unit": d, "path": u["path"], "qual": None, "effect": e, "unit_state": why,
                              "scan_has_unknown": False, "pub": None, "roots": [{"cls": why}]})
            continue
        se = rep.eff(q)
        if se is None:
            population["mapped, but no functions[] row (pure, or silent)"] += 1
            se = set()
        else:
            population["mapped units"] += 1
        miss = sorted(le - se)
        if prep is not None:
            pe = prep.eff(q) or set()
            for e in sorted(le - pe):
                pub_flips.append({"unit": d, "path": u["path"], "qual": q, "effect": e, "pub_has_unknown": UNKNOWN in pe,
                                  "head_missing": e in miss})
        for e in miss:
            pub = None
            if prep is not None:
                pe = prep.eff(q)
                pub = {"missing": e not in (pe or set()), "unknown": UNKNOWN in (pe or set())}
            flips.append({"unit": d, "path": u["path"], "qual": q, "effect": e, "unit_state": "mapped",
                          "scan_has_unknown": UNKNOWN in se, "pub": pub, "roots": causes(d, e)})
    # Reverse: units where candor-scan charges a concrete effect the lint does not (a fabrication candidate, or a lint
    # under-report -- the census cannot tell which; listed for the over-charge control, not counted as flips).
    extra = []
    for d, q in qual_of.items():
        if q is None:
            continue
        se = {x for x in (rep.eff(q) or set()) if x != UNKNOWN}
        le = {x for x in lint_eff(d) if x != UNKNOWN}
        if se - le:
            extra.append({"unit": d, "qual": q, "scan_only": sorted(se - le)})
    population["SELF-CHECK: units whose recomputed lint set differs from the dump (must be 0)"] = len(mism)
    return {"label": label, "self_check_mismatch": mism[:20], "population": population, "flips": flips, "pub_flips": pub_flips, "scan_only": extra, "excluded": excluded,
            "census_files": cfiles, "site_rows": len(rows)}


def root_sig(f):
    roots = [r for r in f["roots"] if not r.get("behind")]
    clss = sorted({(r["cls"] + ("[" + r["tcause"] + "]" if r.get("tcause") else "")) for r in roots})
    return " + ".join(clss)


def summarize(res):
    out = ["== census %s ==" % res["label"]]
    for k, n in sorted(res["population"].items()):
        out.append("  %-60s %d" % (k, n))
    fl = res["flips"]
    out.append("gate flips (unit, effect): %d   of which deny-E-Unknown also passes: %d" % (
        len(fl), sum(1 for f in fl if not f["scan_has_unknown"])))
    pf = res.get("pub_flips") or []
    out.append("published-arm gate flips (mapped units): %d   (also flipped at HEAD: %d; fixed at HEAD: %d)" % (
        len(pf), sum(1 for f in pf if f["head_missing"]), sum(1 for f in pf if not f["head_missing"])))
    out.append("HEAD flips on mapped units not flipped on the published arm (HEAD-only): %d" % sum(
        1 for f in fl if f.get("pub") and not f["pub"]["missing"]))
    by = collections.Counter(root_sig(f) for f in fl)
    for k, n in by.most_common():
        out.append("  %4d  %s" % (n, k))
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", required=True)
    ap.add_argument("--sidecar", required=True)
    ap.add_argument("--report", required=True)
    ap.add_argument("--pub-report")
    ap.add_argument("--root")
    ap.add_argument("--label", default="")
    ap.add_argument("--json")
    a = ap.parse_args()
    res = run(a.oracle, a.sidecar, a.report, a.root, a.pub_report, a.label)
    print(summarize(res))
    if a.json:
        with open(a.json, "w") as fh:
            json.dump(res, fh, indent=1, default=list)


if __name__ == "__main__":
    main()
