#!/usr/bin/env python3
"""Step 0b comparator: rustc's resolution oracle against candor-scan's per-call-site sidecar.

    compare.py --oracle DIR --sidecar DIR [--root SRC_ROOT] [--label L] [--json OUT] [--sites OUT.tsv]
               [--not-covered REASON]   # the crate did not build under this configuration

Inputs
  --oracle   the directory `CANDOR_ORACLE_DUMP` wrote (one `<crate>.<types>.<pid>.jsonl` per compiled crate)
  --sidecar  the directory `CANDOR_SIDECAR` wrote (one JSON per scanned crate)
  --root     restrict both sides to files under this directory (the scanned source tree)

Every oracle site and every scanner site lands in exactly one verdict:
  AGREE-EXACT, AGREE-SUPERSET, SCANNER-REFUSED, DISAGREE-TARGET, SCANNER-SILENT, SILENT-IN-EXPANSION, UNMATCHED, NOT-COVERED
and every non-AGREE verdict on an oracle site gets a class: T, N/M, D, or UNCLASSIFIED(<why>) -- never forced.

IDENTITY (never a leaf):
  * a LOCAL target (rustc `resolved.def.local`) is the function item whose definition contains rustc's item identifier
    position: the scanner function row in that file with the greatest (line, col) <= the identifier's. A scanner edge
    names a qual; a qual may own several rows (a merged unit) and every row it owns is a member of the scanner's set.
  * an EXTERNAL target is (crate, owner, member), owner = the impl's self type / the trait / the parent module's last
    segment. This is coarser than a DefPath (stated limitation: two same-named owners inside one external crate compare
    equal); local identity is exact.
TRUTH LEVELS: resolution is judged against `resolved` (Instance::try_resolve in the caller's typing env); a site whose
`resolved` is `unresolved`/`virtual` is a dispatch site at typeck level (`named` is the trait item).
"""
import argparse
import bisect
import collections
import glob
import json
import os
import sys

STD = {"std", "core", "alloc", "proc_macro", "test"}
VERDICTS = ["AGREE-EXACT", "AGREE-SUPERSET", "SCANNER-REFUSED", "DISAGREE-TARGET", "SCANNER-SILENT",
            "SILENT-IN-EXPANSION", "UNMATCHED", "NOT-COVERED"]
CALL_FORMS = {"method", "path", "path1", "qualified", "type_path", "callexpr", "op", "op:Index", "await", "try", "for"}


def norm(p):
    try:
        return os.path.realpath(p)
    except Exception:
        return p


def load_oracle(d):
    crates, sites = [], []
    for f in sorted(glob.glob(os.path.join(d, "*.jsonl"))):
        with open(f) as fh:
            hdr = None
            for line in fh:
                r = json.loads(line)
                if r["kind"] == "crate":
                    hdr = r
                    crates.append(r)
                else:
                    r["_crate"] = hdr["crate"] if hdr else "?"
                    sites.append(r)
    return crates, sites


def load_sidecar(d):
    docs = []
    for f in sorted(glob.glob(os.path.join(d, "*.json"))):
        with open(f) as fh:
            docs.append(json.load(fh))
    return docs


def head_of_path(p):
    """`a::b::Owner::m` -> ('a', 'Owner', 'm'); markers (`<...>`) are dropped."""
    segs = [s for s in p.split("::") if s and not s.startswith("<")]
    if not segs:
        return None
    member = segs[-1]
    owner = segs[-2] if len(segs) >= 2 else ""
    krate = segs[0] if len(segs) >= 3 else ""
    if krate in STD:
        krate = "std"
    return (krate, owner, member)


def strip_generics(s):
    out, depth = [], 0
    for ch in s:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth = max(0, depth - 1)
        elif depth == 0:
            out.append(ch)
    return "".join(out)


def ext_identities(d):
    """The external identities rustc's item answers to: {(crate, owner, member)}."""
    if not d:
        return set()
    krate = d.get("krate", "")
    krate = "std" if krate in STD else krate
    member = d.get("name", "")
    owners = set()
    par = d.get("parent") or {}
    if par.get("self"):
        owners.add(par["self"])
    if par.get("trait"):
        owners.add(strip_generics(par["trait"]).split("::")[-1])
    pp = strip_generics(par.get("path") or "")
    if pp:
        last = pp.split("::")[-1].strip()
        if " as " not in (par.get("path") or ""):
            owners.add(last)
    path = strip_generics(d.get("path", ""))
    segs = [s for s in path.split("::") if s]
    if len(segs) >= 2:
        owners.add(segs[-2].split(" ")[-1])
    return {(krate, o, member) for o in owners if o}


class Scan:
    """One crate's sidecar, indexed."""

    def __init__(self, doc):
        self.root = norm(doc["root"])
        self.crates = set()
        self.fn_rows = collections.defaultdict(list)  # file -> sorted [(line, col, qual)]
        self.qual_rows = collections.defaultdict(list)  # qual -> [(file, line, col)]
        self.fns = doc["fns"]
        for f in doc["fns"]:
            at = f.get("at") or ""
            try:
                line, col = (int(x) for x in at.split(":")[:2])
            except Exception:
                continue
            fl = norm(f["file"])
            self.fn_rows[fl].append((line, col, f["qual"]))
            self.qual_rows[f["qual"]].append((fl, line, col))
        for v in self.fn_rows.values():
            v.sort()
        # sites: (file, line, col, flag) -> {"calls": [...], "notes": [...]}
        self.sites = collections.defaultdict(lambda: {"calls": [], "notes": [], "callers": set()})
        self.groups = collections.defaultdict(list)  # file -> [(start(l,c), end(l,c), name)]
        self.nospan_calls = 0
        for c in doc["calls"]:
            if not c.get("sp"):
                self.nospan_calls += 1
                continue
            l, col, fl = c["sp"]
            k = (norm(c["file"]), l, col, fl)
            self.sites[k]["calls"].append(c)
            self.sites[k]["callers"].add(c["caller"])
        self.nospan_notes = 0
        for n in doc["notes"]:
            if not n.get("sp"):
                self.nospan_notes += 1
                continue
            l, col, fl = n["sp"]
            k = (norm(n["file"]), l, col, fl)
            note = n["note"]
            if note.startswith("macgroup|"):
                _, name, s, e = note.split("|")
                s = tuple(int(x) for x in s.split(":"))
                e = tuple(int(x) for x in e.split(":"))
                self.groups[norm(n["file"])].append((s, e, name))
                continue
            self.sites[k]["notes"].append(note)
            self.sites[k]["callers"].add(n["caller"])

    def owner_of(self, file, line, col):
        rows = self.fn_rows.get(norm(file))
        if not rows:
            return None
        i = bisect.bisect_right(rows, (line, col, "￿")) - 1
        return rows[i] if i >= 0 else None

    def group_of(self, file, line, col):
        for s, e, name in self.groups.get(norm(file), []):
            if s <= (line, col) <= e:
                return s, e, name
        return None


def site_forms(site):
    return {n.split("|")[1] for n in site["notes"] if n.startswith("site|")}


def answer(scan, site_list, member=None):
    """The scanner's answer at one site (several records may share it)."""
    calls = [c for s in site_list for c in s["calls"]]
    if member is not None:
        calls = [c for c in calls if c["call"]["leaf"] == member or c["call"]["path"].split("::")[-1] == member]
    notes = [n for s in site_list for n in s["notes"]]
    edges, ext, deferred, refusals, dispatch, effects, tys = set(), set(), [], set(), set(), set(), set()
    blind, dep_hits = set(), set()
    untyped = False
    for c in calls:
        edges.update(c["edges"])
        effects.update(c["effects"])
        for u in c["unknown"]:
            refusals.add(u)
        blind.update(c.get("blind", []))
        for key, effectful in c.get("dep_hits", []) or []:
            krate, _, tail = key.partition("#")
            segs = [x for x in tail.split("::") if x]
            if len(segs) >= 2:
                dep_hits.add((krate, segs[-2], segs[-1], effectful))
        for d in c["deferred"]:
            deferred.append(d)
        p = c["call"]["path"]
        if "<untyped>" in p or "<chain>" in p:
            untyped = True
        if c["call"]["typed"]:
            segs = [s for s in p.split("::") if s and not s.startswith("<")]
            if len(segs) >= 2:
                tys.add(segs[-2])
        if c["call"].get("exact_ty"):
            tys.add(c["call"]["exact_ty"].split("::")[-1])
        if not c["edges"] and not c["call"]["macro"] and "<" not in p and "::" in p and not p.startswith("crate::"):
            # An EXTERNAL resolution only when the path's head is a crate (std or one the build linked), or a typed
            # `Type::method` the typer produced; a bare local `T5::make` that linked nothing is an unresolved path.
            segs = [x for x in p.split("::") if x]
            head = segs[0]
            if head in STD or head in scan.crates:
                ext.add(("std" if head in STD else head, segs[-2] if len(segs) >= 2 else "", segs[-1]))
            elif c["call"]["typed"] and len(segs) == 2:
                ext.add(("", segs[0], segs[1]))
        if c["call"]["method"] and not c["call"]["typed"] and "::" not in p:
            untyped_method = True
        else:
            untyped_method = False
        if untyped_method:
            untyped = untyped or not tys
    for n in notes:
        k, _, d = n.partition("|")
        if k in ("refusal", "unresolved", "late_refusal"):
            refusals.add(d or k)
        elif k == "dispatch":
            dispatch.add(d)
        elif k == "foreign_dispatch":
            dispatch.add(d)
    for d in deferred:
        for q in d.get("cands", []) or []:
            edges.add(q)
    # A dependency-report entry a route looked up counts as the scanner's target when the site was charged from it, or
    # when the entry is pure (nothing to charge). An effectful entry looked up and not charged is not a reach.
    charged = bool(effects or refusals)
    for (kr, ow, mem, effectful) in dep_hits:
        if charged or not effectful:
            ext.add((kr, ow, mem))
    targets = set()
    for q in edges:
        for row in scan.qual_rows.get(q, []):
            targets.add(("L",) + row)
    return {
        "calls": calls, "edges": edges, "targets": targets, "ext": ext, "deferred": deferred, "refusals": refusals,
        "dispatch": dispatch, "effects": effects, "tys": tys, "untyped": untyped and not tys, "blind": blind,
        "visited": bool(site_forms_of(site_list)),
    }


def site_forms_of(site_list):
    out = set()
    for s in site_list:
        out |= site_forms(s)
    return out


def classify_site(o, scan, sites_written, sites_exp, used):
    """Verdict + class for one oracle site."""
    key = o["key"]
    file, line, col = norm(key["file"]), key["line"], key["col"]
    member = o.get("member") or None
    res = o.get("resolved") or {}
    status = res.get("status")
    d = res.get("def")
    dispatch_site = status in ("unresolved", "virtual")
    in_exp = o["exp"]
    desugar = o.get("desugar")
    # Which scanner records answer this site?
    matched = []
    via = None
    if not in_exp:
        k = (file, line, col, 0)
        if k in sites_written and not desugar:
            matched = [sites_written[k]]
            used.add(k)
            via = "written"
        elif desugar:
            # `?`: branch is keyed at the operand's start, from_residual at the `?`; `.await`: get_context at the
            # operand. The scanner keys its `try`/`await`/`for` site at the token; map by line + form.
            want = {"QuestionMark": "try", "Await": "await", "ForLoop": "for"}.get(desugar.split(" ")[0])
            for k2, s in sites_written.items():
                if k2[0] == file and k2[1] == line and want in site_forms(s):
                    matched.append(s)
                    used.add(k2)
            via = "desugar"
        if not matched:
            g = scan.group_of(file, line, col)
            if g:
                k = (file, g[0][0], g[0][1], 1)
                if k in sites_exp:
                    matched = [sites_exp[k]]
                    used.add(k)
                    via = "group"
    else:
        k = (file, line, col, 1)
        if k in sites_exp:
            matched = [sites_exp[k]]
            used.add(k)
            via = "exp"
    filt = member if via in ("group", "exp") else None
    a = answer(scan, matched, filt) if matched else None
    has_output = a and (a["edges"] or a["ext"] or a["refusals"] or a["dispatch"] or a["effects"] or a["deferred"])

    def silent():
        return "SILENT-IN-EXPANSION" if in_exp else "SCANNER-SILENT"

    # Truth.
    truth_local = None
    truth_ext = set()
    if d and not dispatch_site:
        if d.get("local") and d.get("ident"):
            ow = scan.owner_of(d["ident"]["file"], d["ident"]["line"], d["ident"]["col"])
            if ow and norm(d["ident"]["file"]) == norm(d["ident"]["file"]):
                truth_local = ("L", norm(d["ident"]["file"]), ow[0], ow[1])
            else:
                truth_local = ("L?", norm(d["ident"]["file"]), d["ident"]["line"], d["ident"]["col"])
        else:
            truth_ext = ext_identities(d)
    named = o.get("named") or {}
    if dispatch_site:
        truth_ext = ext_identities(named)

    # Verdict.
    verdict, extras = None, 0
    if not matched or not has_output:
        verdict = silent()
    else:
        reached = False
        size = len(a["targets"]) + len(a["ext"])
        if truth_local and truth_local[0] == "L":
            reached = truth_local in a["targets"]
        elif truth_ext:
            reached = bool(truth_ext & a["ext"]) or any(
                (k == "" and (o_, m_) in {(t[1], t[2]) for t in truth_ext}) for (k, o_, m_) in a["ext"])
            if not reached and not dispatch_site:
                # Autoderef: the scanner names the member on the receiver as written (`Vec::iter`) where rustc's item
                # sits on a deref target (`<[T]>::iter`). Same member, both owners on rustc's own autoderef chain.
                chain = set(((o.get("recv") or {}).get("chain")) or [])
                towners = {o_ for (_, o_, _) in truth_ext}
                reached = any(m_ == member and ow in chain for (_, ow, m_) in a["ext"]) and bool(towners & chain)
            if not reached and dispatch_site:
                # the scanner names the trait member (`Tr::m` dispatch note) -> the typeck-level item
                nm = named.get("name", "")
                reached = any(x.split("::")[-1] == nm and x.split("::")[-2:-1] and
                              x.split("::")[-2] in {o_ for (_, o_, _) in truth_ext} for x in a["dispatch"])
                if reached:
                    size = max(size, 1)
        if reached:
            verdict = "AGREE-EXACT" if size <= 1 else "AGREE-SUPERSET"
            extras = max(0, size - 1)
        elif a["refusals"] or a["dispatch"]:
            verdict = "SCANNER-REFUSED"
        elif a["edges"] or a["ext"] or a["deferred"]:
            verdict = "DISAGREE-TARGET"
        else:
            verdict = silent()

    # Class (non-AGREE only).
    cls = None
    if not verdict.startswith("AGREE"):
        recv = o.get("recv") or {}
        chain = set(recv.get("chain") or [])
        has_recv = bool(chain)
        mono = o.get("mono") or {}
        if desugar:
            cls = "UNCLASSIFIED(desugared:%s)" % desugar.split(" ")[0]
        elif dispatch_site:
            if a and (a["dispatch"] or a["refusals"] or len(a["targets"]) + len(a["ext"]) > 1):
                cls = "D"
            else:
                cls = "UNCLASSIFIED(dispatch-site,no-dispatch-answer)"
        elif in_exp or via in ("group",):
            cls = "UNCLASSIFIED(in-expansion)"
        elif not a or not a["calls"]:
            cls = "UNCLASSIFIED(no-scanner-record)"
        else:
            untyped_ref = any("untyped" in r for r in a["refusals"])
            if o["form"] == "method" and has_recv:
                # A method call: the scanner's receiver type set against rustc's whole autoderef chain.
                if a["untyped"] or untyped_ref or not a["tys"] or not (a["tys"] & chain):
                    cls = "T"
                else:
                    cls = "N/M"
            else:
                # A path / operator call: the PATH names the item; the scanner is T only where it says it could not
                # type the value the path dispatches on.
                cls = "T" if (untyped_ref or a["untyped"]) else "N/M"
    if truth_local:
        target_kind = "local"
    elif dispatch_site:
        target_kind = "dispatch"
    elif truth_ext:
        target_kind = "std" if any(k == "std" for (k, _, _) in truth_ext) else "dep"
    else:
        target_kind = "none"
    return {
        "verdict": verdict, "class": cls, "extras": extras, "via": via, "form": o["form"], "member": member,
        "file": file, "line": line, "col": col, "exp": in_exp, "desugar": desugar, "target_kind": target_kind,
        "truth": (d or {}).get("path") or named.get("path"), "status": status, "caller": o.get("caller"),
        "scanner": None if not a else {
            "edges": sorted(a["edges"]), "ext": sorted("::".join(x) for x in a["ext"]),
            "refusals": sorted(a["refusals"]), "dispatch": sorted(a["dispatch"]), "tys": sorted(a["tys"]),
            "untyped": a["untyped"], "effects": sorted(a["effects"]), "blind": sorted(a["blind"]),
        },
        "rustc_recv": (o.get("recv") or {}).get("chain"),
        "macro_origin": macro_origin(o),
    }


def macro_origin(o):
    if not o["exp"]:
        return None
    ex = o.get("expn") or []
    names = [e for e in ex if not e.startswith("desugar:")]
    outer = names[-1] if names else "?"
    raw = (o.get("raw") or {}).get("file", "")
    std = "/rustlib/src/" in raw or "/library/" in raw
    return ("std-macro:" if std else "macro:") + outer


def compare(oracle_dir, sidecar_dir, root=None, not_covered=None):
    crates, osites = load_oracle(oracle_dir) if oracle_dir and os.path.isdir(oracle_dir) else ([], [])
    docs = load_sidecar(sidecar_dir)
    root = norm(root) if root else None
    rows = []
    excluded = collections.Counter()
    scanner_unmatched = []
    for doc in docs:
        scan = Scan(doc)
        if root and not scan.root.startswith(root):
            continue
        sites_written = {k: v for k, v in scan.sites.items() if k[3] == 0}
        sites_exp = {k: v for k, v in scan.sites.items() if k[3] == 1}
        used = set()
        if not_covered or not crates:
            reason = not_covered or "no oracle output (crate did not build)"
            for k, s in scan.sites.items():
                if s["calls"]:
                    rows.append({"side": "scanner", "verdict": "NOT-COVERED", "class": None, "why": reason,
                                 "file": k[0], "line": k[1], "col": k[2], "forms": sorted(site_forms(s))})
            continue
        mine = [o for o in osites if norm(o["key"]["file"]).startswith(scan.root + os.sep)]
        for o in osites:
            for lvl in ("named", "resolved", "mono"):
                v = o.get(lvl) or {}
                v = v.get("def", v) if lvl != "named" else v
                if v and v.get("krate") and not v.get("local"):
                    scan.crates.add(v["krate"])
        outside = [o for o in osites if not norm(o["key"]["file"]).startswith(scan.root + os.sep)]
        for o in outside:
            excluded["oracle-site-outside-scanned-tree(OUT_DIR/include/other crate)"] += 1
            rows.append({"side": "oracle", "verdict": "NOT-COVERED", "class": None,
                         "why": "span outside the scanned source tree", "file": o["key"]["file"],
                         "line": o["key"]["line"], "col": o["key"]["col"], "form": o["form"], "member": o.get("member")})
        for o in mine:
            st = (o.get("resolved") or {}).get("status")
            if st in ("not-fn", None) and o["form"] != "callexpr":
                excluded["not-a-fn-call(ctor/const)"] += 1
                used.add((norm(o["key"]["file"]), o["key"]["line"], o["key"]["col"], 1 if o["exp"] else 0))
                continue
            if o["form"] == "callexpr" and not o.get("named"):
                excluded["callexpr-without-fndef(closure/fnptr)"] += 1
                continue
            r = classify_site(o, scan, sites_written, sites_exp, used)
            r["side"] = "oracle"
            rows.append(r)
        # Scanner-side sites no oracle site claimed.
        callers_with_match = set()
        for k in used:
            callers_with_match |= scan.sites[k]["callers"]
        oracle_files = {norm(o["key"]["file"]) for o in mine}
        for k, s in scan.sites.items():
            if k in used:
                continue
            forms = site_forms(s)
            outputs = any(c["edges"] or c["effects"] or c["unknown"] or c["deferred"] for c in s["calls"]) or any(
                n.split("|")[0] in ("refusal", "unresolved", "dispatch", "foreign_dispatch") for n in s["notes"])
            if not s["calls"] and not outputs:
                excluded["scanner-visited-no-record(builtin op / local / unmatched fnref)"] += 1
                continue
            if forms <= {"fnref"} and not outputs:
                excluded["scanner-fnref-without-output"] += 1
                continue
            if k[3] == 2:
                why = "aux(non-call expression)"
            elif k[0] not in oracle_files:
                rows.append({"side": "scanner", "verdict": "NOT-COVERED", "class": None,
                             "why": "file not compiled under this configuration", "file": k[0], "line": k[1],
                             "col": k[2], "forms": sorted(forms)})
                continue
            elif not (s["callers"] & callers_with_match):
                rows.append({"side": "scanner", "verdict": "NOT-COVERED", "class": None,
                             "why": "enclosing fn has no compiled site (cfg arm not compiled, or test-only)",
                             "file": k[0], "line": k[1], "col": k[2], "forms": sorted(forms)})
                continue
            elif forms <= {"macro"}:
                why = "macro-path record"
            elif "fnref" in forms and not forms - {"fnref"}:
                why = "fnref (callee path or value)"
            else:
                why = "call form: " + ",".join(sorted(forms) or ["none"])
            rows.append({"side": "scanner", "verdict": "UNMATCHED", "class": None, "why": why, "file": k[0],
                         "line": k[1], "col": k[2], "forms": sorted(forms),
                         "paths": sorted({c["call"]["path"] for c in s["calls"]})})
        excluded["scanner-call-without-site(no position: splice/derive/respan)"] += scan.nospan_calls
    # Oracle sites with no scanner doc at all.
    if not docs:
        for o in osites:
            rows.append({"side": "oracle", "verdict": "UNMATCHED", "class": None, "why": "no sidecar",
                         "file": o["key"]["file"], "line": o["key"]["line"], "col": o["key"]["col"], "form": o["form"]})
    return crates, rows, excluded


def summarize(rows, excluded, label=""):
    out = []
    by = collections.Counter()
    for r in rows:
        by[(r["side"], r["verdict"])] += 1
    out.append("== %s ==" % label)
    out.append("oracle sites by verdict:")
    for v in VERDICTS:
        out.append("  %-20s %d" % (v, by[("oracle", v)]))
    out.append("scanner-only sites by verdict:")
    for v in VERDICTS:
        if by[("scanner", v)]:
            out.append("  %-20s %d" % (v, by[("scanner", v)]))
    ext = sum(r.get("extras", 0) for r in rows if r.get("verdict") == "AGREE-SUPERSET")
    out.append("AGREE-SUPERSET extra members: %d" % ext)
    cls = collections.Counter((r["verdict"], r["class"]) for r in rows if r["side"] == "oracle" and r.get("class"))
    out.append("non-AGREE oracle sites by class:")
    for (v, c), n in sorted(cls.items()):
        out.append("  %-20s %-45s %d" % (v, c, n))
    tk = collections.Counter((r["target_kind"], r["verdict"]) for r in rows if r["side"] == "oracle")
    out.append("oracle sites by target kind x verdict:")
    for (k, v), n in sorted(tk.items()):
        out.append("  %-9s %-20s %d" % (k, v, n))
    out.append("excluded from the population (counted, not verdicts):")
    for k, n in sorted(excluded.items()):
        out.append("  %-70s %d" % (k, n))
    un = collections.Counter(r.get("why") for r in rows if r["verdict"] in ("UNMATCHED", "NOT-COVERED") and r["side"] == "scanner")
    if un:
        out.append("scanner-side UNMATCHED / NOT-COVERED composition:")
        for k, n in un.most_common():
            out.append("  %-70s %d" % (k, n))
    # per-route refusal rate beside resolvability
    ref = collections.Counter()
    ref_res = collections.Counter()
    for r in rows:
        if r["side"] != "oracle" or not r.get("scanner"):
            continue
        for x in r["scanner"]["refusals"]:
            route = x.split("`")[0][:60]
            ref[route] += 1
            if r["target_kind"] in ("local", "std", "dep"):
                ref_res[route] += 1
    if ref:
        out.append("refusal reasons at oracle sites (route ~ reason prefix): total / where rustc resolves a concrete item:")
        for k, n in ref.most_common():
            out.append("  %-62s %d / %d" % (k, n, ref_res[k]))
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle")
    ap.add_argument("--sidecar", required=True)
    ap.add_argument("--root")
    ap.add_argument("--label", default="")
    ap.add_argument("--json")
    ap.add_argument("--sites")
    ap.add_argument("--not-covered")
    a = ap.parse_args()
    crates, rows, excluded = compare(a.oracle, a.sidecar, a.root, a.not_covered)
    print(summarize(rows, excluded, a.label))
    if a.json:
        with open(a.json, "w") as fh:
            json.dump({"label": a.label, "crates": crates, "rows": rows, "excluded": excluded}, fh, indent=1, default=list)
    if a.sites:
        with open(a.sites, "w") as fh:
            for r in rows:
                fh.write("\t".join(str(x) for x in [r["side"], r["verdict"], r.get("class"), r.get("form") or ",".join(r.get("forms", [])),
                                                    r.get("member"), "%s:%s:%s" % (r["file"], r["line"], r["col"]), r.get("truth"),
                                                    json.dumps(r.get("scanner")), r.get("why", "")]) + "\n")


if __name__ == "__main__":
    main()
