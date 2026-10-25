#!/usr/bin/env python3
"""Call-target matrix generator for candor-scan (calltarget-analysis, 2026-10-24).

Generates one tiny program per CELL under <out>/cells/<cid>/ :
  lib/  (crate ctlib, optional)   app/ (crate ctapp: src/lib.rs, src/other.rs, src/main.rs)
The caller is always `ctapp::zz_caller()` in app/src/lib.rs; main.rs calls it and nothing else.
Each cell's meta.json records its axes, the effect E, the ground-truth expectation and whether a
charge on a no-effect run is a sound over-approximation (over_ok) or a fabrication.

Axes (see REPORT.md section 2):
  family  : explicit-trait | explicit-plain | callable | hook | recv | ident
  kind    : the call form
  where   : placement of trait (TL) / implementor (IL) / dispatching helper (HL)
  n       : one | mix (effectful runs, a pure sibling exists) | mixP (pure sibling runs, effectful exists) | pure
  effect  : Fs | Net | Exec
  arms    : solo | ch (dependency report chained) | unch (dependency present, not chained)
"""
import json, os, sys, shutil, itertools, re

OUT = sys.argv[1]  # never this directory: the cells are ~1 GB of build inputs
MARK = os.path.join(OUT, "markers")
PORT = int(os.environ.get("CT_PORT", "47123"))
# SPELLING axis (calltarget round 1). `crate`: every caller-side path written from the crate root (`crate::S`,
# `crate::other::S`, `ctlib::S`) — the analysis run's first spelling. `bare`: the same programs with the root's own
# items bare (`S`, `other::S`, `ctlib::S`) — its control. `use`: every TYPE VALUE the caller names is imported and
# written bare (`use ctlib::S; .. S ..`, `use crate::other::S;`), helpers and traits spelled as in `bare` — the
# control that isolates path-value typing from the mechanism a cell is about, which the dependency placements lacked
# (FABLE-SYNTHESIS §1.2). `CT_BARE=1` is the old spelling of `CT_SPELL=bare`.
SPELL = os.environ.get("CT_SPELL") or ("bare" if os.environ.get("CT_BARE") == "1" else "crate")
assert SPELL in ("crate", "bare", "use"), SPELL
BARE = SPELL in ("bare", "use")
EFFECTS = ["Fs", "Net", "Exec"]

def eff(E, cid):
    m = os.path.join(MARK, cid)
    if E == "Fs":
        return 'let _ = std::fs::write("%s.fs", b"x");' % m
    if E == "Exec":
        return 'let _ = std::process::Command::new("/usr/bin/touch").arg("%s.exec").status();' % m
    if E == "Net":
        return ('if let Ok(mut s) = std::net::TcpStream::connect("127.0.0.1:%d") '
                '{ let _ = std::io::Write::write_all(&mut s, b"%s\\n"); }' % (PORT, cid))
    raise ValueError(E)

PURE = "let _z = 1u8.wrapping_add(1);"

CELLS = []
PRUNED = []

def add(cid, family, kind, where, n, E, files, arms, gt, over_ok, note="", src=None):
    if src is not None and use_collides(src):
        PRUNED.append((cid, SPELL, "the `use` spelling's import collides with a root item of the same name"))
        return
    CELLS.append(dict(cid=cid, family=family, kind=kind, where=where, n=n, effect=E,
                      files=files, arms=arms, gt=gt, over_ok=over_ok, note=note))

def rot(i):
    return EFFECTS[i % 3]

# ---------------------------------------------------------------------------------------------
# helpers for assembling the three source files
class Src:
    def __init__(self):
        self.use_types = []   # (loc, name) the caller imports in the `use` spelling
        self.root = []    # app/src/lib.rs items
        self.other = []   # app/src/other.rs items
        self.lib = []     # lib/src/lib.rs items
        self.caller = []  # statements in zz_caller
        self.uses_root = []
    def put(self, loc, code):
        {"root": self.root, "other": self.other, "dep": self.lib}[loc].append(code)
    def files(self):
        f = {}
        root = []
        if self.other:
            root.append("pub mod other;")
        root += self.uses_root
        for loc, name in self.use_types:
            line = "use %s%s;" % ({"other": "crate::other::", "dep": "ctlib::"}[loc], name)
            if line not in root:
                root.append(line)
        root += self.root
        root.append("pub fn zz_caller() {\n    " + "\n    ".join(self.caller) + "\n}")
        f["app/src/lib.rs"] = "#![allow(unused)]\n" + "\n".join(root) + "\n"
        if self.other:
            f["app/src/other.rs"] = "#![allow(unused)]\n" + "\n".join(self.other) + "\n"
        f["app/src/main.rs"] = "fn main() { ctapp::zz_caller(); }\n"
        if self.lib:
            f["lib/src/lib.rs"] = "#![allow(unused)]\n" + "\n".join(self.lib) + "\n"
        return f

def path_from_root(loc):
    if BARE:
        return {"root": "", "other": "other::", "dep": "ctlib::"}[loc]
    return {"root": "crate::", "other": "crate::other::", "dep": "ctlib::"}[loc]

def path_from(src_loc, loc):
    """path prefix to name an item at `loc` from code at `src_loc`."""
    if src_loc == "dep":
        if loc != "dep":
            return "UNNAMEABLE::"
        return "crate::"
    if src_loc == "other":
        return {"root": "crate::", "other": "crate::other::", "dep": "ctlib::"}[loc]
    return path_from_root(loc)

def tyv(s, loc, name):
    """A TYPE VALUE (or type) the caller names, placed at `loc`: spelled per SPELL. In the `use` spelling a non-root
    type is imported and written bare; a name the root module also declares makes the program not compile, which
    `use_collides` reports so the cell is pruned rather than generated wrong."""
    if SPELL == "use" and loc != "root":
        s.use_types.append((loc, name))
        return name
    return path_from_root(loc) + name

def use_collides(s):
    if SPELL != "use":
        return False
    names = {n for _, n in s.use_types}
    root_src = "\n".join(s.root + (s.uses_root or []))
    return any(re.search(r"\b(struct|enum|trait|fn|type|mod)\s+%s\b" % re.escape(n), root_src) for n in names)

def arms_for(*locs):
    return ["ch", "unch"] if "dep" in locs else ["solo"]

# ---------------------------------------------------------------------------------------------
# 1. EXPLICIT TRAIT DISPATCH
# placements: name -> (TL, IL, HL)
TPLACE = {
    "local": ("root", "root", "root"),
    "mod":   ("other", "other", "root"),
    "dep":   ("dep", "dep", "root"),
    "revT":  ("dep", "root", "root"),
    "revH":  ("dep", "root", "dep"),
    "depH":  ("dep", "dep", "dep"),
}
# kind -> (needs_helper, helper_template, call_template, dispatch_is_abstract)
# templates: {T}=trait path from helper loc, {S}=type path from caller, {H}=helper path from caller
TKINDS = {
    "conc":      (False, None, "{S}.act();", False),
    "conc_ref":  (False, None, "let r = &{S}; r.act();", False),
    "ufcs":      (False, None, "{Tc}::act(&{S});", False),
    "qself":     (False, None, "<{S} as {Tc}>::act(&{S});", False),
    "provided":  (False, None, "{S}.go();", False),
    "generic":   (True, "pub fn h<X: {T}>(t: &X) {{ t.act() }}", "{H}h(&{S});", True),
    "where":     (True, "pub fn h<X>(t: &X) where X: {T} {{ t.act() }}", "{H}h(&{S});", True),
    "implarg":   (True, "pub fn h(t: &impl {T}) {{ t.act() }}", "{H}h(&{S});", True),
    "gen_ufcs":  (True, "pub fn h<X: {T}>(t: &X) {{ X::act(t) }}", "{H}h(&{S});", True),
    "dyn":       (True, "pub fn h(t: &dyn {T}) {{ t.act() }}", "{H}h(&{S});", True),
    "boxdyn":    (True, "pub fn h(t: Box<dyn {T}>) {{ t.act() }}", "{H}h(Box::new({S}));", True),
    "vecdyn":    (True, "pub fn h(v: &[Box<dyn {T}>]) {{ for t in v {{ t.act() }} }}",
                  "let v: Vec<Box<dyn {Tc}>> = vec![Box::new({S})]; {H}h(&v);", True),
    "genfield":  (True, "pub struct Hd<X: {T}> {{ pub t: X }}\nimpl<X: {T}> Hd<X> {{ pub fn run(&self) {{ self.t.act() }} }}",
                  "{H}Hd {{ t: {S} }}.run();", True),
    "dynfield":  (True, "pub struct Hd {{ pub t: Box<dyn {T}> }}\nimpl Hd {{ pub fn run(&self) {{ self.t.act() }} }}",
                  "{H}Hd {{ t: Box::new({S}) }}.run();", True),
    "dynret":    (True, "pub fn mk(b: bool) -> Box<dyn {T}> {{ MKBODY }}", "{H}mk(true).act();", True),
    "assocfn":   (True, "pub fn h<X: {T}>() {{ X::make() }}", "{H}h::<{S}>();", True),
    "genprov":   (True, "pub fn h<X: {T}>(t: &X) {{ t.go() }}", "{H}h(&{S});", True),
    # R1429: a trait object inside an Option-of-tuple payload, destructured by `if let Some((_, t))`.
    "opt_tuple_dyn": (True, "pub fn h(c: Option<(u8, &dyn {T})>) {{ if let Some((_, t)) = c {{ t.act() }} }}", "{H}h(Some((1, &{S})));", True),
}
TRAIT_DEF = "pub trait Tr { fn act(&self); fn go(&self) { self.act() } fn make() where Self: Sized; }"

def gen_explicit_trait():
    i = 0
    for kind, (needs_h, htpl, ctpl, abstract) in TKINDS.items():
        for pname, (TL, IL, HL) in TPLACE.items():
            if not needs_h and pname in ("revH", "depH"):
                PRUNED.append((kind, pname, "no helper in this call form; placement collapses to revT/dep"))
                continue
            if kind == "dynret" and pname in ("revH",):
                PRUNED.append((kind, pname, "helper constructs the implementor; cannot name an app type from the dependency"))
                continue
            for n in ("one", "mix", "mixP", "pure"):
                for E in (EFFECTS if n in ("one",) and kind in ("conc", "generic", "dyn", "provided") else [rot(i)]):
                    i += 1
                    cid = "t_%s_%s_%s_%s" % (kind, pname, n, E)
                    s = Src()
                    s.put(TL, TRAIT_DEF)
                    tpath_il = path_from(IL, TL) + "Tr"
                    body_S = PURE if n == "pure" else eff(E, cid)
                    s.put(IL, "pub struct S;\nimpl %s for S { fn act(&self) { %s } fn make() { %s } }" % (tpath_il, body_S, body_S))
                    if n in ("mix", "mixP"):
                        s.put(IL, "pub struct P;\nimpl %s for P { fn act(&self) { %s } fn make() { %s } }" % (tpath_il, PURE, PURE))
                    run_ty = "P" if n == "mixP" else "S"
                    Spath = tyv(s, IL, run_ty)
                    Tc = path_from_root(TL) + "Tr"
                    if needs_h:
                        T_h = path_from(HL, TL) + "Tr"
                        h = htpl.format(T=T_h)
                        if kind == "dynret":
                            sp = path_from(HL, IL)
                            if n in ("mix", "mixP"):
                                other_ty = "P" if run_ty == "S" else "S"
                                h = h.replace("MKBODY", "if b { Box::new(%s%s) } else { Box::new(%s%s) }" % (sp, run_ty, sp, other_ty))
                            else:
                                h = h.replace("MKBODY", "Box::new(%sS)" % sp)
                        s.put(HL, h)
                    Hp = path_from_root(HL) if needs_h else ""
                    s.uses_root.append("use %s as _;" % Tc)
                    call = ctpl.format(S=Spath, Tc=Tc, H=Hp)
                    s.caller.append(call)
                    gt = n in ("one", "mix")
                    over_ok = (n == "mixP") and abstract
                    add(cid, "explicit-trait", kind, pname, n, E, s.files(), arms_for(TL, IL, HL), gt, over_ok, src=s)

# ---------------------------------------------------------------------------------------------
# 2. EXPLICIT PLAIN (static fn, inherent method)
def gen_explicit_plain():
    i = 0
    for kind in ("static", "inherent", "assoc_inherent", "static_use", "static_use_glob", "static_use_rename", "reexport"):
        for loc in ("root", "other", "dep"):
            if kind in ("static_use", "static_use_glob", "static_use_rename", "reexport") and loc == "root":
                PRUNED.append((kind, loc, "a `use` of an item in the same module is not a different resolution path"))
                continue
            for n in ("one", "mixP", "pure"):
                if n == "mixP" and kind not in ("inherent",):
                    continue
                for E in (EFFECTS if (n == "one" and kind in ("static", "inherent")) else [rot(i)]):
                    i += 1
                    cid = "p_%s_%s_%s_%s" % (kind, loc, n, E)
                    s = Src()
                    body = PURE if n == "pure" else eff(E, cid)
                    pp = path_from_root(loc)
                    if kind.startswith("static") or kind == "reexport":
                        s.put(loc, "pub fn callee() { %s }" % body)
                        if kind == "static":
                            s.caller.append("%scallee();" % pp)
                        elif kind == "static_use":
                            s.uses_root.append("use %scallee;" % pp)
                            s.caller.append("callee();")
                        elif kind == "static_use_glob":
                            s.uses_root.append("use %s*;" % pp)
                            s.caller.append("callee();")
                        elif kind == "static_use_rename":
                            s.uses_root.append("use %scallee as renamed_target;" % pp)
                            s.caller.append("renamed_target();")
                        elif kind == "reexport":
                            # re-exported under a new name from the implementor's own crate/module
                            if loc == "dep":
                                s.put("dep", "pub mod inner { pub fn callee2() { %s } }\npub use inner::callee2 as reexported;" % body)
                            else:
                                s.put(loc, "pub mod inner { pub fn callee2() { %s } }\npub use self::inner::callee2 as reexported;" % body)
                            s.caller.append("%sreexported();" % pp)
                    elif kind == "inherent":
                        s.put(loc, "pub struct S;\nimpl S { pub fn act(&self) { %s } }" % body)
                        if n == "mixP":
                            s.put(loc, "pub struct P;\nimpl P { pub fn act(&self) { %s } }" % PURE)
                        s.caller.append("%s.act();" % tyv(s, loc, "P" if n == "mixP" else "S"))
                    elif kind == "assoc_inherent":
                        s.put(loc, "pub struct S;\nimpl S { pub fn make() { %s } }" % body)
                        s.caller.append("%s::make();" % tyv(s, loc, "S"))
                    add(cid, "explicit-plain", kind, loc, n, E, s.files(), arms_for(loc), n == "one", False, src=s)

# ---------------------------------------------------------------------------------------------
# 3. CALLABLES (fn values, closures, deferred)
# callee location CL; higher-order helper location HL
CALLABLE = {
    # kind: (helper or None, caller template, callee signature, driven)
    "fnptr":      (None, "let f: fn() = {C}callee0; f();", "0", True),
    "fnptr_u8":   (None, "let f: fn(u8) -> u8 = {C}callee_u8; let _ = f(1);", "u8", True),
    "apply_item": ("pub fn apply<F: Fn()>(f: F) {{ f() }}", "{H}apply({C}callee0);", "0", True),
    "apply_clos": ("pub fn apply<F: Fn()>(f: F) {{ f() }}", "{H}apply(|| {C}callee0());", "0", True),
    "apply_dyn":  ("pub fn apply(f: &dyn Fn()) {{ f() }}", "{H}apply(&{C}callee0);", "0", True),
    "apply_ptr":  ("pub fn apply(f: fn()) {{ f() }}", "{H}apply({C}callee0);", "0", True),
    "boxfn":      (None, "let b: Box<dyn Fn()> = Box::new({C}callee0); b();", "0", True),
    "stored":     ("pub struct Cb {{ pub f: Box<dyn Fn()> }}\npub fn run(c: &Cb) {{ (c.f)() }}",
                   "{H}run(&{H}Cb {{ f: Box::new({C}callee0) }});", "0", True),
    "stored_later": ("pub struct Cb {{ pub f: fn() }}\nimpl Cb {{ pub fn fire(&self) {{ (self.f)() }} }}",
                   "let c = {H}Cb {{ f: {C}callee0 }}; c.fire();", "0", True),
    "ret_closure": ("pub fn mk() -> impl Fn() {{ || CALLEE() }}", "{H}mk()();", "0", True),
    "opt_map":    (None, "let _ = Some(1u8).map({C}callee_u8);", "u8", True),
    "opt_map_clos": (None, "let _ = Some(1u8).map(|v| {C}callee_u8(v));", "u8", True),
    "iter_map_collect": (None, "let _: Vec<u8> = vec![1u8].into_iter().map({C}callee_u8).collect();", "u8", True),
    "iter_for_each": (None, "vec![1u8].into_iter().for_each(|v| {{ {C}callee_u8(v); }});", "u8", True),
    "iter_map_undriven": (None, "let _it = vec![1u8].into_iter().map({C}callee_u8);", "u8", False),
    "unwrap_or_else": (None, "let _ = None::<u8>.unwrap_or_else({C}callee_ret);", "ret", True),
    "thread_spawn": (None, "let _ = std::thread::spawn({C}callee0).join();", "0", True),
    "oncelock":   (None, "static O: std::sync::OnceLock<u8> = std::sync::OnceLock::new(); let _ = O.get_or_init({C}callee_ret);", "ret", True),
    "lazylock":   (None, "LAZY", "ret", True),
    "sort_by_key": (None, "let mut v = vec![2u8, 1u8]; v.sort_by_key({C}callee_ref);", "ref", True),
}
SIGS = {
    "0": "pub fn callee0() {{ {B} }}",
    "u8": "pub fn callee_u8(x: u8) -> u8 {{ {B} x }}",
    "ret": "pub fn callee_ret() -> u8 {{ {B} 1 }}",
    "ref": "pub fn callee_ref(x: &u8) -> u8 {{ {B} *x }}",
}
def gen_callable():
    i = 0
    for kind, (htpl, ctpl, sig, driven) in CALLABLE.items():
        places = [("root", "root"), ("other", "root"), ("dep", "root")]
        if htpl:
            places += [("root", "dep"), ("dep", "dep")]
        for CL, HL in places:
            if kind == "ret_closure" and CL == "root" and HL == "dep":
                PRUNED.append((kind, CL + "/" + HL, "the dependency cannot name an app callee"))
                continue
            for n in ("one", "pure"):
                E = rot(i); i += 1
                cid = "c_%s_%s-%s_%s_%s" % (kind, CL, HL, n, E)
                s = Src()
                body = PURE if n == "pure" else eff(E, cid)
                s.put(CL, SIGS[sig].format(B=body))
                Cp = path_from_root(CL)
                if htpl:
                    h = htpl.format()
                    if kind == "ret_closure":
                        h = h.replace("CALLEE", path_from(HL, CL) + "callee0")
                    s.put(HL, h)
                Hp = path_from_root(HL) if htpl else ""
                if kind == "lazylock":
                    s.root.append("static LZ: std::sync::LazyLock<u8> = std::sync::LazyLock::new(%scallee_ret);" % Cp)
                    s.caller.append("let _ = *LZ;")
                else:
                    s.caller.append(ctpl.format(C=Cp, H=Hp))
                gt = (n == "one") and driven
                over_ok = (n == "one") and not driven
                add(cid, "callable", kind, "%s-%s" % (CL, HL), n, E, s.files(), arms_for(CL, HL), gt, over_ok)

# ---------------------------------------------------------------------------------------------
# 4. IMPLICIT std -> user HOOKS
# each hook: impl(S, B) code at IL (uses std paths only; {S} type name),
#            trig: statements over an owned binding `x` (and {TY} = type path from trigger loc, {IP} = IL path),
#            bound: generic bound for a helper (None = no generic helper),
#            dyn: (dyn type, statements over `x: &mut dyn ...`) or None,
#            aux: extra item placed at the trigger location
D_PE = "#[derive(PartialEq, Eq)]\n"
HOOKS = {
 "display_fmt": dict(impl="impl std::fmt::Display for S {{ fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ {B} write!(f, \"s\") }} }}",
                     trig="let _ = format!(\"{{}}\", x);", bound="std::fmt::Display",
                     dyn=("dyn std::fmt::Display", "let _ = format!(\"{{}}\", x);")),
 "display_tostring": dict(impl="@display_fmt", trig="let _ = x.to_string();", bound="std::fmt::Display",
                     dyn=("dyn std::fmt::Display", "let _ = x.to_string();")),
 "display_nested": dict(impl="@display_fmt",
                     aux="pub struct Wd<X>(pub X);\nimpl<X: std::fmt::Display> std::fmt::Display for Wd<X> {{ fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ write!(f, \"{{}}\", self.0) }} }}",
                     trig="let _ = Wd(x).to_string();", bound="std::fmt::Display", dyn=None),
 "debug_fmt": dict(impl="impl std::fmt::Debug for S {{ fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {{ {B} write!(f, \"s\") }} }}",
                     trig="let _ = format!(\"{{:?}}\", x);", bound="std::fmt::Debug",
                     dyn=("dyn std::fmt::Debug", "let _ = format!(\"{{:?}}\", x);")),
 "debug_vec": dict(impl="@debug_fmt", trig="let _ = format!(\"{{:?}}\", vec![x]);", bound="std::fmt::Debug", dyn=None),
 "debug_option": dict(impl="@debug_fmt", trig="let _ = format!(\"{{:?}}\", Some(x));", bound="std::fmt::Debug", dyn=None),
 "debug_tuple": dict(impl="@debug_fmt", trig="let _ = format!(\"{{:?}}\", (1u8, x));", bound="std::fmt::Debug", dyn=None),
 "debug_derive": dict(impl="@debug_fmt", aux="#[derive(Debug)]\npub struct Wd<X>(pub X);",
                     trig="let _ = format!(\"{{:?}}\", Wd(x));", bound="std::fmt::Debug", dyn=None),
 "eq": dict(impl="impl PartialEq for S {{ fn eq(&self, _o: &Self) -> bool {{ {B} true }} }}",
                     trig="let _ = x == x;", bound="PartialEq", dyn=None),
 "ne": dict(impl="@eq", trig="let _ = x != x;", bound="PartialEq", dyn=None),
 "eq_contains": dict(impl="@eq", trig="let v = vec![x]; let _ = v.contains(&v[0]);", bound="PartialEq", dyn=None),
 "eq_vec": dict(impl="@eq", trig="let v = vec![x]; let _ = v == v;", bound="PartialEq", dyn=None),
 "partialord_lt": dict(impl="impl PartialEq for S {{ fn eq(&self, _o: &Self) -> bool {{ true }} }}\nimpl PartialOrd for S {{ fn partial_cmp(&self, _o: &Self) -> Option<std::cmp::Ordering> {{ {B} Some(std::cmp::Ordering::Equal) }} }}",
                     trig="let _ = x < x;", bound="PartialOrd", dyn=None),
 "ord_max": dict(impl="impl PartialEq for S {{ fn eq(&self, _o: &Self) -> bool {{ true }} }}\nimpl Eq for S {{}}\nimpl PartialOrd for S {{ fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {{ Some(std::cmp::Ord::cmp(self, o)) }} }}\nimpl Ord for S {{ fn cmp(&self, _o: &Self) -> std::cmp::Ordering {{ {B} std::cmp::Ordering::Equal }} }}",
                     trig="let _ = std::cmp::max(&x, &x);", bound="Ord", dyn=None),
 "ord_sort": dict(impl="@ord_max", trig="let mut v = [&x, &x]; v.sort();", bound="Ord", dyn=None),
 "ord_btree": dict(impl="@ord_max", trig="let mut b = std::collections::BTreeSet::new(); b.insert(&x); b.insert(&x);", bound="Ord", dyn=None),
 "hash_set": dict(impl=D_PE + "impl std::hash::Hash for S {{ fn hash<H: std::hash::Hasher>(&self, _h: &mut H) {{ {B} }} }}",
                     trig="let mut h = std::collections::HashSet::new(); h.insert(x);", bound="std::hash::Hash + Eq", dyn=None),
 "hash_derive": dict(impl="@hash_set", aux="#[derive(Hash, PartialEq, Eq)]\npub struct Wh<X>(pub X);",
                     trig="let mut h = std::collections::HashSet::new(); h.insert(Wh(x));", bound="std::hash::Hash + Eq", dyn=None),
 "clone": dict(impl="impl Clone for S {{ fn clone(&self) -> Self {{ {B} S }} }}", trig="let _ = x.clone();", bound="Clone", dyn=None),
 "clone_ufcs": dict(impl="@clone", trig="let _ = Clone::clone(&x);", bound="Clone", dyn=None),
 "clone_vec": dict(impl="@clone", trig="let v = vec![x]; let _ = v.clone();", bound="Clone", dyn=None),
 "clone_option": dict(impl="@clone", trig="let _ = Some(x).clone();", bound="Clone", dyn=None),
 "clone_tovec": dict(impl="@clone", trig="let _ = [x].to_vec();", bound="Clone", dyn=None),
 "clone_cloned": dict(impl="@clone", trig="let a = [x]; let _: Vec<_> = a.iter().cloned().collect();", bound="Clone", dyn=None),
 "clone_box": dict(impl="@clone", trig="let _ = Box::new(x).clone();", bound="Clone", dyn=None),
 "clone_derive": dict(impl="@clone", aux="#[derive(Clone)]\npub struct Wc<X>(pub X);", trig="let _ = Wc(x).clone();", bound="Clone", dyn=None),
 "clone_toowned": dict(impl="@clone", trig="let _ = x.to_owned();", bound="Clone", dyn=None),
 "drop_scope": dict(impl="impl Drop for S {{ fn drop(&mut self) {{ {B} }} }}", trig="{{ let _y = x; }}", bound="NOBOUND", dyn=None),
 "drop_mem": dict(impl="@drop_scope", trig="std::mem::drop(x);", bound="NOBOUND", dyn=None),
 "drop_vec": dict(impl="@drop_scope", trig="let _v = vec![x];", bound="NOBOUND", dyn=None),
 "drop_option": dict(impl="@drop_scope", trig="let _o = Some(x);", bound="NOBOUND", dyn=None),
 "drop_box": dict(impl="@drop_scope", trig="let _b = Box::new(x);", bound="NOBOUND", dyn=None),
 "drop_field": dict(impl="@drop_scope", aux="pub struct Wf<X> { pub f: X }", trig="let _w = Wf { f: x };", bound="NOBOUND", dyn=None),
 "drop_param": dict(impl="@drop_scope", aux="pub fn eat<X>(v: Vec<X>) -> usize { v.len() }", trig="let _ = eat(vec![x]);", bound="NOBOUND", dyn=None),
 "deref_method": dict(impl="impl std::ops::Deref for S {{ type Target = str; fn deref(&self) -> &str {{ {B} \"x\" }} }}",
                     trig="let _ = x.len();", bound="std::ops::Deref<Target = str>", dyn=None),
 "deref_star": dict(impl="@deref_method", trig="let _y: &str = &*x;", bound="std::ops::Deref<Target = str>", dyn=None),
 "deref_coerce": dict(impl="@deref_method", aux="pub fn takes(_s: &str) {}", trig="takes(&x);", bound="std::ops::Deref<Target = str>", dyn=None),
 "iter_for": dict(impl="impl Iterator for S {{ type Item = u8; fn next(&mut self) -> Option<u8> {{ {B} None }} }}",
                     trig="for _ in x {{}}", bound="Iterator<Item = u8>",
                     dyn=("dyn Iterator<Item = u8>", "for _ in x {{}}")),
 "iter_next": dict(impl="@iter_for", trig="let mut x = x; let _ = x.next();", bound="Iterator<Item = u8>",
                     dyn=("dyn Iterator<Item = u8>", "let _ = x.next();")),
 "iter_collect": dict(impl="@iter_for", trig="let _: Vec<u8> = x.collect();", bound="Iterator<Item = u8>", dyn=None),
 "iter_count": dict(impl="@iter_for", trig="let _ = x.count();", bound="Iterator<Item = u8>",
                     dyn=("dyn Iterator<Item = u8>", "let _ = x.count();")),
 "iter_extend": dict(impl="@iter_for", trig="let mut v: Vec<u8> = Vec::new(); v.extend(x);", bound="Iterator<Item = u8>", dyn=None),
 "iter_fromiter": dict(impl="@iter_for", trig="let _ = Vec::<u8>::from_iter(x);", bound="Iterator<Item = u8>", dyn=None),
 "iter_sum": dict(impl="@iter_for", trig="let _: u32 = x.map(|b| b as u32).sum();", bound="Iterator<Item = u8>", dyn=None),
 "iter_issorted": dict(impl="@iter_for", trig="let _ = x.is_sorted();", bound="Iterator<Item = u8>", dyn=None),
 "iter_zip": dict(impl="@iter_for", trig="let _ = (0..1u8).zip(x).count();", bound="Iterator<Item = u8>", dyn=None),
 "iter_undriven": dict(impl="@iter_for", trig="let _m = x.map(|b| b);", bound="Iterator<Item = u8>", dyn=None, driven=False),
 "intoiter_for": dict(impl="impl IntoIterator for S {{ type Item = u8; type IntoIter = std::vec::IntoIter<u8>; fn into_iter(self) -> Self::IntoIter {{ {B} Vec::new().into_iter() }} }}",
                     trig="for _ in x {{}}", bound="IntoIterator<Item = u8>", dyn=None),
 "default_unwrap": dict(impl="impl Default for S {{ fn default() -> Self {{ {B} S }} }}", trig="let _ = None::<{TY}>.unwrap_or_default();", bound="Default", dyn=None, typeonly=True),
 "default_derive": dict(impl="@default_unwrap", aux="#[derive(Default)]\npub struct Wdf<X> { pub f: X }", trig="let _ = Wdf::<{TY}>::default();", bound="Default", dyn=None, typeonly=True),
 "default_take": dict(impl="@default_unwrap", trig="let mut y = x; let _ = std::mem::take(&mut y);", bound="Default", dyn=None),
 "into": dict(impl="pub struct Tgt;\nimpl From<S> for Tgt {{ fn from(_s: S) -> Tgt {{ {B} Tgt }} }}", trig="let _t: {IP}Tgt = x.into();", bound="Into<{IPH}Tgt>", dyn=None),
 "from_q": dict(impl="pub struct E2;\nimpl From<S> for E2 {{ fn from(_s: S) -> E2 {{ {B} E2 }} }}",
                     aux="pub fn q<X>(x: X) -> Result<(), {IPA}E2> where {IPA}E2: From<X> {{ Err(x)?; Ok(()) }}", trig="let _ = q(x);", bound="NOBOUND", dyn=None),
 "fromstr": dict(impl="impl std::str::FromStr for S {{ type Err = (); fn from_str(_s: &str) -> Result<S, ()> {{ {B} Ok(S) }} }}",
                     trig="let _ = \"x\".parse::<{TY}>();", bound="std::str::FromStr", dyn=None, typeonly=True),
 "op_add": dict(impl="impl std::ops::Add<u8> for S {{ type Output = u8; fn add(self, _o: u8) -> u8 {{ {B} 0 }} }}",
                     trig="let _ = x + 1u8;", bound="std::ops::Add<u8>", dyn=None),
 "op_addassign": dict(impl="impl std::ops::AddAssign<u8> for S {{ fn add_assign(&mut self, _o: u8) {{ {B} }} }}",
                     trig="let mut x = x; x += 1u8;", bound="std::ops::AddAssign<u8>", dyn=None),
 "op_neg": dict(impl="impl std::ops::Neg for S {{ type Output = u8; fn neg(self) -> u8 {{ {B} 0 }} }}", trig="let _ = -x;", bound="std::ops::Neg", dyn=None),
 "op_index": dict(impl="impl std::ops::Index<usize> for S {{ type Output = u8; fn index(&self, _i: usize) -> &u8 {{ {B} &0 }} }}",
                     trig="let _ = x[0usize];", bound="std::ops::Index<usize, Output = u8>", dyn=None),
 "fmt_write": dict(impl="impl std::fmt::Write for S {{ fn write_str(&mut self, _s: &str) -> std::fmt::Result {{ {B} Ok(()) }} }}",
                     trig="let mut x = x; let _ = std::fmt::Write::write_fmt(&mut x, format_args!(\"a\"));", bound="std::fmt::Write",
                     dyn=("dyn std::fmt::Write", "let _ = x.write_str(\"a\");")),
 "fmt_writemacro": dict(impl="@fmt_write", trig="use std::fmt::Write as _; let mut x = x; let _ = write!(x, \"a\");", bound="std::fmt::Write",
                     dyn=("dyn std::fmt::Write", "use std::fmt::Write as _; let _ = write!(x, \"a\");")),
 "io_write_all": dict(impl="impl std::io::Write for S {{ fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {{ {B} Ok(b.len()) }} fn flush(&mut self) -> std::io::Result<()> {{ Ok(()) }} }}",
                     trig="use std::io::Write as _; let mut x = x; let _ = x.write_all(b\"a\");", bound="std::io::Write",
                     dyn=("dyn std::io::Write", "use std::io::Write as _; let _ = x.write_all(b\"a\");")),
 "io_writeln": dict(impl="@io_write_all", trig="use std::io::Write as _; let mut x = x; let _ = writeln!(x, \"a\");", bound="std::io::Write",
                     dyn=("dyn std::io::Write", "use std::io::Write as _; let _ = writeln!(x, \"a\");")),
 "io_bufwriter": dict(impl="@io_write_all", trig="use std::io::Write as _; let mut w = std::io::BufWriter::new(x); let _ = w.write_all(b\"a\"); let _ = w.flush();",
                     bound="std::io::Write", dyn=None),
 "io_copy": dict(impl="@io_write_all", trig="let mut x = x; let _ = std::io::copy(&mut &b\"a\"[..], &mut x);", bound="std::io::Write", dyn=None),
 "io_read": dict(impl="impl std::io::Read for S {{ fn read(&mut self, _b: &mut [u8]) -> std::io::Result<usize> {{ {B} Ok(0) }} }}",
                     trig="use std::io::Read as _; let mut x = x; let mut v = Vec::new(); let _ = x.read_to_end(&mut v);", bound="std::io::Read",
                     dyn=("dyn std::io::Read", "use std::io::Read as _; let mut v = Vec::new(); let _ = x.read_to_end(&mut v);")),
 "io_lines": dict(impl="@io_read", trig="use std::io::BufRead as _; let _ = std::io::BufReader::new(x).lines().count();", bound="std::io::Read", dyn=None),
 "asref_path": dict(impl="impl AsRef<std::ffi::OsStr> for S {{ fn as_ref(&self) -> &std::ffi::OsStr {{ {B} std::ffi::OsStr::new(\"x\") }} }}",
                     trig="let _ = std::path::Path::new(&x);", bound="AsRef<std::ffi::OsStr>", dyn=None),
}

HPLACE = {
    # name: (IL, trigger location, helper kind)
    "local":     ("root", "root", "direct"),
    "mod":       ("other", "root", "direct"),
    "dep":       ("dep", "root", "direct"),
    "local_gen": ("root", "root", "gen"),
    "dep_gen":   ("dep", "root", "gen"),
    "rev_gen":   ("root", "dep", "gen"),
    "depH_gen":  ("dep", "dep", "gen"),
    "local_dyn": ("root", "root", "dyn"),
    "dep_dyn":   ("dep", "root", "dyn"),
    "rev_dyn":   ("root", "dep", "dyn"),
}
def resolve_impl(name):
    v = HOOKS[name]["impl"]
    while v.startswith("@"):
        v = HOOKS[v[1:]]["impl"]
    return v

def gen_hooks():
    i = 0
    for hname, h in HOOKS.items():
        impl = resolve_impl(hname)
        driven = h.get("driven", True)
        for pname, (IL, TRL, hk) in HPLACE.items():
            if hk == "gen" and h["bound"] is None:
                continue
            if hk == "gen" and h["bound"] == "NOBOUND":
                hb = None
            else:
                hb = h["bound"]
            if hk == "dyn" and not h["dyn"]:
                PRUNED.append(("hook:" + hname, pname, "std trait not object-safe or no dyn form for this trigger"))
                continue
            if hk == "direct" and TRL == "dep":
                continue
            ns = ["one", "pure"] + (["mixP"] if hk in ("gen", "dyn") else [])
            for n in ns:
                E = rot(i); i += 1
                cid = "h_%s_%s_%s_%s" % (hname, pname, n, E)
                s = Src()
                body = PURE if n == "pure" else eff(E, cid)
                def with_struct(code, ty):
                    lines = code.split("\n")
                    der = [x for x in lines if x.startswith("#[derive")]
                    rest = [x for x in lines if not x.startswith("#[derive")]
                    return "\n".join(der + ["pub struct %s;" % ty] + rest)
                if n == "mixP" and hname in ("into", "from_q"):
                    PRUNED.append(("hook:" + hname, pname + "/mixP", "the target type is shared; a second source type needs a second impl on one target (not generated)"))
                    continue
                s.put(IL, with_struct(impl.format(B=body), "S"))
                if n == "mixP":
                    s.put(IL, with_struct(impl.replace("for S ", "for P ").replace("Ok(S)", "Ok(P)").replace("Result<S,", "Result<P,").replace("{{ {B} S }}", "{{ {B} P }}").replace("From<S>", "From<P>").replace("_s: S)", "_s: P)").format(B=PURE).replace("-> Self { let _z = 1u8.wrapping_add(1); S }", "-> Self { let _z = 1u8.wrapping_add(1); P }"), "P"))
                    # P is the type that runs; S (effectful) exists and is reachable by bounded CHA from the helper
                run_ty = "P" if n == "mixP" else "S"
                ty_from_trig = tyv(s, IL, run_ty) if TRL == "root" else path_from(TRL, IL) + run_ty
                ip_trig = path_from(TRL, IL)
                trig = h["trig"]
                aux = h.get("aux")
                def fill(t, ip):
                    return (t.replace("{TY}", ty_from_trig).replace("{IP}", ip).replace("{IPA}", ip)
                             .replace("{{", "{").replace("}}", "}"))
                if hk == "direct":
                    if aux:
                        s.put(TRL, fill(aux, ip_trig))
                    if h.get("typeonly"):
                        s.caller.append(fill(trig, ip_trig))
                    else:
                        s.caller.append("let x = %s; %s" % (tyv(s, IL, run_ty), fill(trig, ip_trig)))
                elif hk == "gen":
                    if aux:
                        s.put(TRL, fill(aux, ip_trig))
                    if h.get("typeonly"):
                        trig_g = trig.replace("{TY}", "X").replace("{{", "{").replace("}}", "}")
                        hdr = "pub fn hg<X: %s>() { %s }" % (hb, trig_g)
                        call = "%shg::<%s>();" % (path_from_root(TRL), tyv(s, IL, run_ty))
                    else:
                        trig_g = trig.replace("{IP}", ip_trig).replace("{{", "{").replace("}}", "}")
                        if hb:
                            bnd = hb.replace("{IPH}", ip_trig)
                            if hname == "into" and TRL == "dep" and IL == "root":
                                PRUNED.append(("hook:into", pname, "Into<app::Tgt> bound cannot be named in the dependency"))
                                s = None
                                break
                            hdr = "pub fn hg<X: %s>(x: X) { %s }" % (bnd, trig_g)
                        else:
                            if hname == "from_q":
                                hdr = "pub fn hg<X>(x: X) where %sE2: From<X> { %s }" % (ip_trig, trig_g.replace("q(x)", "q(x)"))
                                if TRL == "dep" and IL == "root":
                                    PRUNED.append(("hook:from_q", pname, "dependency cannot name the app's E2"))
                                    s = None
                                    break
                            else:
                                hdr = "pub fn hg<X>(x: X) { %s }" % trig_g
                        call = "%shg(%s);" % (path_from_root(TRL), tyv(s, IL, run_ty))
                    s.put(TRL, hdr)
                    s.caller.append(call)
                elif hk == "dyn":
                    dty, dtrig = h["dyn"]
                    dtrig = dtrig.replace("{{", "{").replace("}}", "}")
                    s.put(TRL, "pub fn hd(x: &mut %s) { %s }" % (dty, dtrig))
                    s.caller.append("let mut x = %s; %shd(&mut x);" % (tyv(s, IL, run_ty), path_from_root(TRL)))
                if s is None:
                    break
                gt = (n == "one") and driven
                over_ok = (n == "mixP") or (n == "one" and not driven)
                add(cid, "hook", hname, pname, n, E, s.files(), arms_for(IL, TRL), gt, over_ok, src=s)

# ---------------------------------------------------------------------------------------------
# 5. RECEIVER PROVENANCE (how the receiver value reaches a method call on a concrete type)
RECV = {
    "param":      ("pub fn inner(s: {S}) {{ s.act() }}", "inner({S}{{}});"),
    "let_ctor":   (None, "let s = {S}{{}}; s.act();"),
    "let_typed_fn": ("pub fn mk() -> {S} {{ {S}{{}} }}", "let s: {S} = mk(); s.act();"),
    "let_untyped_fn": ("pub fn mk() -> {S} {{ {S}{{}} }}", "let s = mk(); s.act();"),
    "chain_ret":  ("pub fn mk() -> {S} {{ {S}{{}} }}", "mk().act();"),
    "method_ret": ("pub struct Hld {{ pub s: {S} }}\nimpl Hld {{ pub fn get_s(&self) -> &{S} {{ &self.s }} }}", "let h = Hld {{ s: {S}{{}} }}; h.get_s().act();"),
    "field":      ("pub struct Hld {{ pub s: {S} }}", "let h = Hld {{ s: {S}{{}} }}; h.s.act();"),
    "self_field": ("pub struct Hld {{ pub s: {S} }}\nimpl Hld {{ pub fn run(&self) {{ self.s.act() }} }}", "Hld {{ s: {S}{{}} }}.run();"),
    "opt_iflet":  (None, "if let Some(s) = Some({S}{{}}) {{ s.act(); }}"),
    "opt_ret_match": ("pub struct Hld {{ pub s: Option<{S}> }}\nimpl Hld {{ pub fn inner(&self) -> Option<&{S}> {{ self.s.as_ref() }} }}",
                   "let h = Hld {{ s: Some({S}{{}}) }}; let i = match h.inner() {{ Some(i) => i, None => return }}; i.act();"),
    "opt_unwrap": (None, "let s = Some({S}{{}}).unwrap(); s.act();"),
    "vec_for":    (None, "for s in vec![{S}{{}}] {{ s.act(); }}"),
    "vec_index":  (None, "let v = vec![{S}{{}}]; v[0].act();"),
    "tuple":      (None, "let t = (1u8, {S}{{}}); t.1.act();"),
    "closure_cap": (None, "let s = {S}{{}}; let f = || s.act(); f();"),
    "box":        (None, "let b = Box::new({S}{{}}); b.act();"),
    "rc":         (None, "let r = std::rc::Rc::new({S}{{}}); r.act();"),
    "arc_clone":  (None, "let r = std::sync::Arc::new({S}{{}}); let r2 = r.clone(); r2.act();"),
    "ref_param":  ("pub fn inner(s: &{S}) {{ s.act() }}", "inner(&{S}{{}});"),
    "macro_wrapped": ("macro_rules! call_it {{ ($e:expr) => {{ $e.act() }} }}", "call_it!({S}{{}});"),
    "shadowed":   (None, "let s = 1u8; let s = {S}{{}}; s.act();"),
}
def gen_recv():
    i = 0
    for rname, (aux, trig) in RECV.items():
        # kind: inherent method / trait method (trait from where-placement)
        for mk, place in (("inh", "local"), ("inh", "dep"), ("tr", "local"), ("tr", "dep"), ("tr", "revT")):
            for n in ("one", "pure"):
                E = rot(i); i += 1
                cid = "r_%s_%s_%s_%s_%s" % (rname, mk, place, n, E)
                s = Src()
                body = PURE if n == "pure" else eff(E, cid)
                if mk == "inh":
                    IL = "root" if place == "local" else "dep"
                    s.put(IL, "pub struct S {}\nimpl S { pub fn act(&self) { %s } }" % body)
                    tl = None
                else:
                    TL, IL, _ = TPLACE[place]
                    s.put(TL, "pub trait Tr { fn act(&self); }")
                    s.put(IL, "pub struct S {}\nimpl %sTr for S { fn act(&self) { %s } }" % (path_from(IL, TL), body))
                    s.uses_root.append("use %sTr as _;" % path_from_root(TL))
                    tl = TL
                Sp = tyv(s, IL, "S")
                if aux:
                    s.root.append(aux.format(S=Sp))
                if rname == "opt_ret_match":
                    s.caller.append(trig.format(S=Sp))
                else:
                    s.caller.append(trig.format(S=Sp))
                add(cid, "recv", rname, "%s-%s" % (mk, place), n, E, s.files(), arms_for(IL, tl or IL), n == "one", False, src=s)

# ---------------------------------------------------------------------------------------------
# 6. UNIT IDENTITY (how the implementor is declared / named)
def gen_ident():
    i = 0
    T = "pub trait Tr { fn act(&self); }"
    TD = "pub trait Tr { fn act(&self) { %s } }" % PURE   # pure default body
    cases = []
    # (name, loc-able, build(s, loc, body_eff, body_pure, run_eff) -> call, over_ok_when_pure_run)
    def same_leaf(s, loc, be, bp, run_eff, call_form):
        s.put(loc, T + "\npub mod m1 { pub struct Bytes; impl %sTr for Bytes { fn act(&self) { %s } } }\npub mod m2 { pub struct Bytes; impl %sTr for Bytes { fn act(&self) { %s } } }"
              % ("super::", be, "super::", bp))
        p = path_from_root(loc)
        ty = p + ("m1::Bytes" if run_eff else "m2::Bytes")
        s.uses_root.append("use %sTr as _;" % p)
        if call_form == "conc":
            return "%s.act();" % ty
        if call_form == "dyn":
            s.root.append("pub fn h(t: &dyn %sTr) { t.act() }" % p)
            return "h(&%s);" % ty
        if call_form == "gen":
            s.root.append("pub fn h<X: %sTr>(t: &X) { t.act() }" % p)
            return "h(&%s);" % ty
    for cf in ("conc", "dyn", "gen"):
        cases.append(("sameleaf_" + cf, lambda s, loc, be, bp, re, cf=cf: same_leaf(s, loc, be, bp, re, cf), cf != "conc"))
    def same_leaf_drop(s, loc, be, bp, re):
        s.put(loc, "pub mod m1 { pub struct Bytes; impl Drop for Bytes { fn drop(&mut self) { %s } } }\npub mod m2 { pub struct Bytes; impl Drop for Bytes { fn drop(&mut self) { %s } } }" % (be, bp))
        return "let _b = %s%s;" % (path_from_root(loc), "m1::Bytes" if re else "m2::Bytes")
    cases.append(("sameleaf_drop", same_leaf_drop, False))
    def two_traits(s, loc, be, bp, re, form):
        s.put(loc, "pub trait A { fn go(&self); }\npub trait B { fn go(&self); }\npub struct S;\nimpl A for S { fn go(&self) { %s } }\nimpl B for S { fn go(&self) { %s } }" % (be, bp))
        p = path_from_root(loc)
        tr = "A" if re else "B"
        if form == "ufcs":
            return "%s%s::go(&%sS);" % (p, tr, p)
        s.uses_root.append("use %s%s as _;" % (p, tr))
        return "%sS.go();" % p
    cases.append(("twotraits_ufcs", lambda s, l, be, bp, re: two_traits(s, l, be, bp, re, "ufcs"), False))
    cases.append(("twotraits_scope", lambda s, l, be, bp, re: two_traits(s, l, be, bp, re, "scope"), False))
    def samename_traits(s, loc, be, bp, re, form):
        s.put(loc, "pub mod ma { pub trait W { fn put(&self); } pub struct X; impl W for X { fn put(&self) { %s } } }\npub mod mb { pub trait W { fn put(&self); } pub struct Y; impl W for Y { fn put(&self) { %s } } }" % (be, bp))
        p = path_from_root(loc)
        mod, ty = ("ma", "X") if re else ("mb", "Y")
        if form == "dyn":
            s.root.append("pub fn h(t: &dyn %s%s::W) { t.put() }" % (p, mod))
        else:
            s.root.append("pub fn h<Z: %s%s::W>(t: &Z) { t.put() }" % (p, mod))
        return "h(&%s%s::%s);" % (p, mod, ty)
    cases.append(("samename_traits_dyn", lambda s, l, be, bp, re: samename_traits(s, l, be, bp, re, "dyn"), False))
    cases.append(("samename_traits_gen", lambda s, l, be, bp, re: samename_traits(s, l, be, bp, re, "gen"), False))
    def macro_method(s, loc, be, bp, re):
        s.put(loc, TD + "\nmacro_rules! mk_act { () => { fn act(&self) { %s } } }\npub struct S;\nimpl Tr for S { mk_act!(); }" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return "%sS.act();" % p
    cases.append(("macro_method", macro_method, None))
    def macro_impl(s, loc, be, bp, re):
        s.put(loc, T + "\nmacro_rules! mk_impl { ($t:ident) => { impl Tr for $t { fn act(&self) { %s } } } }\npub struct S;\nmk_impl!(S);" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        s.root.append("pub fn h(t: &dyn %sTr) { t.act() }" % p)
        return "%sS.act(); h(&%sS);" % (p, p)
    cases.append(("macro_impl", macro_impl, None))
    def block_impl(s, loc, be, bp, re):
        s.put(loc, TD + "\npub struct S;\nconst _: () = { impl Tr for S { fn act(&self) { %s } } };" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return "%sS.act();" % p
    cases.append(("block_impl", block_impl, None))
    def fnbody_impl(s, loc, be, bp, re):
        s.put(loc, TD + "\npub struct S;\npub fn setup() { impl Tr for S { fn act(&self) { %s } } }" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return "%sS.act();" % p
    cases.append(("fnbody_impl", fnbody_impl, None))
    def inh_vs_trait(s, loc, be, bp, re):
        # inherent method is PURE, trait method EFFECTFUL; `S.act()` picks the inherent one (truth: pure), `Tr::act(&S)` the trait
        s.put(loc, T + "\npub struct S;\nimpl S { pub fn act(&self) { %s } }\nimpl Tr for S { fn act(&self) { %s } }" % (bp, be))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return ("%sTr::act(&%sS);" % (p, p)) if re else ("%sS.act();" % p)
    cases.append(("inherent_vs_trait", inh_vs_trait, False))
    def free_vs_method(s, loc, be, bp, re):
        # R1265 shape: free fn `main2` and a same-name method in one module
        s.put(loc, "pub struct S;\nimpl S { pub fn main2(&self) { %s } }\npub fn main2(_x: u8) { %s }\npub fn via() { main2(1) }" % (bp if re else be, be if re else bp))
        return "%svia();" % path_from_root(loc)
    cases.append(("free_vs_method", free_vs_method, False))
    def cfg_twin(s, loc, be, bp, re):
        s.put(loc, "#[cfg(unix)]\npub fn callee() { %s }\n#[cfg(not(unix))]\npub fn callee() { %s }" % (be if re else bp, bp if re else be))
        return "%scallee();" % path_from_root(loc)
    cases.append(("cfg_twin", cfg_twin, False))
    def generic_impl_twin(s, loc, be, bp, re):
        s.put(loc, T + "\npub struct Wg<X>(pub X);\nimpl Tr for Wg<u8> { fn act(&self) { %s } }\nimpl Tr for Wg<u16> { fn act(&self) { %s } }" % (be, bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return "%sWg(%s).act();" % (p, "1u8" if re else "1u16")
    cases.append(("generic_impl_twin", generic_impl_twin, False))
    def glob_trait(s, loc, be, bp, re, form):
        s.put(loc, "pub mod j { pub trait JobA { fn run(&self); } pub struct K; impl JobA for K { fn run(&self) { %s } } }" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sj::*;" % p)
        if form == "gen":
            s.root.append("pub fn k<J: JobA>(j: &J) { j.run() }")
            return "k(&K);"
        return "K.run();"
    cases.append(("glob_trait_gen", lambda s, l, be, bp, re: glob_trait(s, l, be, bp, re, "gen"), None))
    cases.append(("glob_trait_conc", lambda s, l, be, bp, re: glob_trait(s, l, be, bp, re, "conc"), None))
    def supersuper(s, loc, be, bp, re):
        # R400: `use super::super::Runner` where the parent rebinds Runner
        if loc == "dep":
            return None
        s.root.append("pub mod a { pub fn runner() { %s } pub mod b { pub fn runner() { %s } pub mod c { use super::super::runner; pub fn go() { runner() } } } }" % (be if re else bp, bp if re else be))
        return "a::b::c::go();"
    cases.append(("supersuper_rebind", supersuper, False))
    def supersuper_norebind(s, loc, be, bp, re):
        if loc == "dep":
            return None
        s.root.append("pub mod a { pub fn runner() { %s } pub mod b { pub mod c { use super::super::runner; pub fn go() { runner() } } } }" % (be if re else bp))
        return "a::b::c::go();"
    cases.append(("supersuper_norebind", supersuper_norebind, None))
    def supersuper_use(s, loc, be, bp, re, depth):
        # R400 as filed: the GRANDPARENT binds a name by `use`, the PARENT rebinds it, the child writes `use
        # super::super::Name`. `re` = the grandparent's binding runs the effect (the parent's is pure); otherwise the
        # grandparent's is pure and the parent's effectful (a charge is the parent's binding read in its place).
        if loc == "dep":
            return None
        s.root.append("pub mod imps { pub struct Gp; impl Gp { pub fn run(&self) { %s } } pub struct Par; impl Par { pub fn run(&self) { %s } } }"
                      % (be if re else bp, bp if re else be))
        g, p = "Gp", "Par"
        if depth == 2:
            s.root.append("pub mod a { pub use crate::imps::%s as Runner; pub mod b { pub use crate::imps::%s as Runner; pub mod c { use super::super::Runner; pub fn go() { Runner.run() } } } }" % (g, p))
            return "a::b::c::go();"
        s.root.append("pub mod a { pub use crate::imps::%s as Runner; pub mod b { pub use crate::imps::%s as Runner; pub mod c { pub use crate::imps::%s as Runner; pub mod d { use super::super::super::Runner; pub fn go() { Runner.run() } } } } }" % (g, p, p))
        return "a::b::c::d::go();"
    cases.append(("supersuper_use", lambda s, l, be, bp, re: supersuper_use(s, l, be, bp, re, 2), False))
    cases.append(("supersuper_use3", lambda s, l, be, bp, re: supersuper_use(s, l, be, bp, re, 3), False))
    def shadow_local_dep(s, loc, be, bp, re):
        # a local fn with the same name as a dependency fn; caller names one of them
        if loc != "dep":
            return None
        s.lib.append("pub fn helper() { %s }" % (be if re else bp))
        s.root.append("pub fn helper() { %s }" % (bp if re else be))
        return "ctlib::helper();"
    cases.append(("shadow_local_dep", shadow_local_dep, False))
    def supertrait_member(s, loc, be, bp, re):
        s.put(loc, "pub trait Base { fn act(&self); }\npub trait Tr: Base {}\npub struct S;\nimpl Base for S { fn act(&self) { %s } }\nimpl Tr for S {}" % (be if re else bp))
        p = path_from_root(loc)
        s.root.append("pub fn h(t: &dyn %sTr) { t.act() }" % p)
        return "h(&%sS);" % p
    cases.append(("supertrait_dyn", supertrait_member, None))
    def blanket(s, loc, be, bp, re):
        s.put(loc, "pub trait Marker {}\npub trait Tr { fn act(&self); }\nimpl<X: Marker> Tr for X { fn act(&self) { %s } }\npub struct S;\nimpl Marker for S {}" % (be if re else bp))
        p = path_from_root(loc)
        s.uses_root.append("use %sTr as _;" % p)
        return "%sS.act();" % p
    cases.append(("blanket_impl", blanket, None))

    for cname, build, pure_run_over in cases:
        for loc in ("root", "dep"):
            variants = [("one", True, True)]
            if pure_run_over is None:
                variants.append(("pure", False, False))
            else:
                variants.append(("twinP", True, False))  # both bodies exist, the pure one runs
            for n, has_eff_body, run_eff in variants:
                E = rot(i); i += 1
                cid = "u_%s_%s_%s_%s" % (cname, loc, n, E)
                s = Src()
                be = eff(E, cid) if has_eff_body else PURE
                call = build(s, loc, be, PURE, run_eff)
                if cname == "twotraits_scope" and loc == "root":
                    call = None  # E0034: two traits' `go` in scope at the root is ambiguous (does not compile)
                if call is None:
                    PRUNED.append(("ident:" + cname, loc, "shape is defined for one placement only"))
                    continue
                s.caller.append(call)
                gt = run_eff and has_eff_body
                over_ok = bool(pure_run_over) and n == "twinP"
                add(cid, "ident", cname, loc, n, E, s.files(), arms_for(loc), gt, over_ok)

# ---------------------------------------------------------------------------------------------
# 7. PATH VALUES (calltarget round 1, R1426): an enum variant / unit value written as a PATH and used as a receiver or an
# argument. A UNIT variant is a value of the enum; a PAYLOAD variant's path is its constructor FUNCTION, so the
# `payload_ctor` cells are controls (the program never runs the impl; a charge is a fabrication).
def gen_pathval():
    i = 0
    for kind in ("variant_recv", "variant_arg", "variant_imported", "variant_self", "tuple_ctor_recv", "payload_ctor"):
        for loc in ("root", "other", "dep"):
            if kind == "variant_self" and loc != "root":
                continue
            for n in ("one", "pure"):
                E = rot(i); i += 1
                cid = "v_%s_%s_%s_%s" % (kind, loc, n, E)
                s = Src()
                body = PURE if n == "pure" else eff(E, cid)
                s.put(loc, "pub enum Kd { Aa, Bb(u8) }\nimpl Kd { pub fn act(&self) { %s } }\nimpl Clone for Kd { fn clone(&self) -> Self { %s Kd::Aa } }" % (body, body))
                pp = path_from_root(loc)
                if kind in ("variant_arg", "payload_ctor"):
                    s.root.append("pub fn hg<X: Clone>(x: X) { let _ = x.clone(); }")
                if kind == "variant_recv":
                    s.caller.append("%sKd::Aa.act();" % pp)
                elif kind == "variant_arg":
                    s.caller.append("hg(%sKd::Aa);" % pp)
                elif kind == "variant_imported":
                    s.uses_root.append("use %sKd::Aa;" % ({"root": "crate::", "other": "crate::other::", "dep": "ctlib::"}[loc]))
                    s.caller.append("Aa.act();")
                elif kind == "variant_self":
                    s.root.append("impl Kd { pub fn via_self(&self) { Self::Aa.act() } }")
                    s.caller.append("Kd::Bb(1).via_self();")
                elif kind == "tuple_ctor_recv":
                    s.caller.append("%sKd::Bb(1).act();" % pp)
                elif kind == "payload_ctor":
                    s.caller.append("hg(%sKd::Bb);" % pp)
                gt = (n == "one") and kind != "payload_ctor"
                add(cid, "pathval", kind, loc, n, E, s.files(), arms_for(loc), gt, False)

# ---------------------------------------------------------------------------------------------
# 8. CONTROLS (call-target round 2, FABLE-REVIEW §3) — every cell here performs NO effect (EXECUTED) unless it is
# marked as a positive twin, so any concrete charge at the caller is FABRICATED. Added BEFORE the round-2 change and
# measured on published 0.40.17 first.
#   wx_unrel_*   — wx_unrelated_override: the consumer implements a member of the same NAME on a type/trait UNRELATED to
#                  the dispatched receiver's static type (an unrelated trait, a same-LEAF trait of another crate, a
#                  sibling sub-trait of a shared base).
#   wx_exact_*   — wx_exact_producer: the producer dispatches on a value of EXACT type while the consumer implements the
#                  trait for its own type (a concrete parameter, UFCS on a concrete local, an instantiated holder — the
#                  rust analogue of the re-key leg; rust has no subclassing, so an "open subclass" leg is not expressible).
#   nl_*         — the NON-INVOKING LOOK-ALIKE axis: syntax that resembles a hook call but runs none, beside a positive
#                  twin that does run it (`nl_*_pos`, gt = effect).
#   arm `chns`   — chained with the dependency report's call-graph SIDECAR deleted (FABLE-REVIEW §3.5).
def gen_controls():
    i = 0
    def cell(cid, where, files_fn, gt, arms):
        nonlocal i
        E = rot(i); i += 1
        cid = "%s_%s" % (cid, E)
        s = Src()
        body = eff(E, cid)
        files_fn(s, body)
        add(cid, "control", cid.rsplit("_", 1)[0], where, "one" if gt else "pure", E, s.files(), arms, gt, False, src=s)
    DEP3 = ["ch", "unch", "chns"]
    # --- wx_unrelated_override
    def unrel_trait(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub fn run(t: &dyn Tr) { t.act() }" % PURE)
        s.put("root", "pub trait Tr2 { fn act(&self); }\npub struct S;\nimpl Tr2 for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::run(&ctlib::P); let _s = S;")
    cell("wx_unrel_trait", "dep", unrel_trait, False, DEP3)
    def unrel_leaf(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub fn run(t: &dyn Tr) { t.act() }" % PURE)
        s.put("root", "pub mod m { pub trait Tr { fn act(&self); } }\npub struct S;\nimpl m::Tr for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::run(&ctlib::P); let _s = S;")
    cell("wx_unrel_leaf", "dep", unrel_leaf, False, DEP3)
    def unrel_gen(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub fn run<X: Tr>(t: &X) { t.act() }" % PURE)
        s.put("root", "pub trait Tr2 { fn act(&self); }\npub struct S;\nimpl Tr2 for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::run(&ctlib::P); let _s = S;")
    cell("wx_unrel_gen", "dep", unrel_gen, False, DEP3)
    def unrel_sib(s, b):
        s.put("dep", "pub trait Base { fn act(&self); }\npub trait SubA: Base {}\npub trait SubB: Base {}\npub struct P;\nimpl Base for P { fn act(&self) { %s } }\nimpl SubA for P {}\npub fn run(t: &dyn SubA) { t.act() }" % PURE)
        s.put("root", "pub struct S;\nimpl ctlib::Base for S { fn act(&self) { %s } }\nimpl ctlib::SubB for S {}" % b)
        s.caller.append("ctlib::run(&ctlib::P); let _s = S;")
    cell("wx_unrel_sib", "dep", unrel_sib, False, DEP3)
    def unrel_local(s, b):
        s.put("root", "pub trait Tr { fn act(&self); }\npub trait Tr2 { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub struct S;\nimpl Tr2 for S { fn act(&self) { %s } }\npub fn run(t: &dyn Tr) { t.act() }" % (PURE, b))
        s.caller.append("run(&P); let _s = S;")
    cell("wx_unrel_local", "root", unrel_local, False, ["solo"])
    def unrel_local_sib(s, b):
        s.put("root", "pub trait Base { fn act(&self); }\npub trait SubA: Base {}\npub trait SubB: Base {}\npub struct P;\nimpl Base for P { fn act(&self) { %s } }\nimpl SubA for P {}\npub struct S;\nimpl Base for S { fn act(&self) { %s } }\nimpl SubB for S {}\npub fn run(t: &dyn SubA) { t.act() }" % (PURE, b))
        s.caller.append("run(&P); let _s = S;")
    cell("wx_unrel_local_sib", "root", unrel_local_sib, False, ["solo"])
    # --- wx_exact_producer
    def exact_param(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub fn run(p: &P) { p.act() }" % PURE)
        s.put("root", "pub struct S;\nimpl ctlib::Tr for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::run(&ctlib::P); let _s = S;")
    cell("wx_exact_param", "dep", exact_param, False, DEP3)
    def exact_ufcs(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub fn run() { let p = P; Tr::act(&p) }" % PURE)
        s.put("root", "pub struct S;\nimpl ctlib::Tr for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::run(); let _s = S;")
    cell("wx_exact_ufcs", "dep", exact_ufcs, False, DEP3)
    def exact_holder(s, b):
        s.put("dep", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub struct H<T: Tr>(pub T);\nimpl H<P> { pub fn run(&self) { self.0.act() } }" % PURE)
        s.put("root", "pub struct S;\nimpl ctlib::Tr for S { fn act(&self) { %s } }" % b)
        s.caller.append("ctlib::H(ctlib::P).run(); let _s = S;")
    cell("wx_exact_holder", "dep", exact_holder, False, DEP3)
    def exact_local(s, b):
        s.put("root", "pub trait Tr { fn act(&self); }\npub struct P;\nimpl Tr for P { fn act(&self) { %s } }\npub struct S;\nimpl Tr for S { fn act(&self) { %s } }\npub fn run(p: &P) { p.act() }" % (PURE, b))
        s.caller.append("run(&P); let _s = S;")
    cell("wx_exact_local", "root", exact_local, False, ["solo"])
    # --- the NON-INVOKING LOOK-ALIKE axis (in-crate and dependency-placed hooks)
    for loc in ("root", "dep"):
        tp = path_from_root(loc)
        arms = ["solo"] if loc == "root" else ["ch", "unch"]
        def lk(name, impl, trig, gt):
            def f(s, b, impl=impl, trig=trig):
                s.put(loc, impl.replace("{B}", b))
                s.caller.append(trig.replace("{P}", tp))
            cell("nl_%s_%s" % (name, loc), loc, f, gt, arms)
        CL = "pub struct S;\nimpl Clone for S { fn clone(&self) -> Self { {B} S } }"
        DR = "pub struct S;\nimpl Drop for S { fn drop(&mut self) { {B} } }"
        DI = "pub struct S;\nimpl std::fmt::Display for S { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { {B} write!(f, \"s\") } }"
        IN = "pub struct S;\nimpl S { pub fn act(&self) { {B} } }\npub fn callee0() { {B} }"
        FR = "pub struct S;\npub struct E2;\nimpl From<S> for E2 { fn from(_s: S) -> E2 { {B} E2 } }"
        lk("let_underscore", CL, "let x = {P}S; let _ = x;", False)
        lk("clone_pos", CL, "let x = {P}S; let _ = x.clone();", True)
        lk("forget", DR, "let x = {P}S; std::mem::forget(x);", False)
        lk("drop_pos", DR, "let x = {P}S; drop(x);", True)
        lk("struct_update", CL + "\npub struct W { pub a: u8, pub s: S }", "let x = {P}W { a: 1, s: {P}S }; let _y = {P}W { a: 2, ..x };", False)
        lk("fn_item_stored", IN, "let f: fn() = {P}callee0; let _ = f;", False)
        lk("method_item_stored", IN, "let g = {P}S::act; let _ = g;", False)
        lk("closure_uncalled", IN, "let x = {P}S; let f = || x.act(); let _ = &f;", False)
        lk("format_args_unused", DI, "let x = {P}S; let _ = format_args!(\"{}\", x);", False)
        lk("format_pos", DI, "let x = {P}S; let _ = format!(\"{}\", x);", True)
        lk("q_neg", FR, "let r: Result<(), {P}S> = Err({P}S); let _ = r.is_err();", False)
    # `?` is the positive twin that DOES call `From::from` (in-crate: the conversion target is local).
    def q_pos(s, b):
        s.put("root", "pub struct S;\npub struct E2;\nimpl From<S> for E2 { fn from(_s: S) -> E2 { %s E2 } }\npub fn q() -> Result<(), E2> { Err(S)?; Ok(()) }" % b)
        s.caller.append("let _ = q();")
    cell("nl_q_pos_root", "root", q_pos, True, ["solo"])

def main():
    gen_explicit_trait(); gen_explicit_plain(); gen_callable(); gen_hooks(); gen_recv(); gen_ident(); gen_pathval(); gen_controls()
    cdir = os.path.join(OUT, "cells")
    if os.path.exists(cdir):
        shutil.rmtree(cdir)
    os.makedirs(cdir); os.makedirs(MARK, exist_ok=True)
    seen = set()
    for c in CELLS:
        assert c["cid"] not in seen, c["cid"]
        seen.add(c["cid"])
        d = os.path.join(cdir, c["cid"])
        for rel, txt in c["files"].items():
            p = os.path.join(d, rel)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            open(p, "w").write(txt)
        has_lib = "lib/src/lib.rs" in c["files"]
        if has_lib:
            open(os.path.join(d, "lib/Cargo.toml"), "w").write('[package]\nname = "ctlib"\nversion = "0.1.0"\nedition = "2021"\n')
        open(os.path.join(d, "app/Cargo.toml"), "w").write(
            '[package]\nname = "ctapp"\nversion = "0.1.0"\nedition = "2021"\n' + ('[dependencies]\nctlib = { path = "../lib" }\n' if has_lib else ""))
        meta = {k: v for k, v in c.items() if k != "files"}
        meta["has_lib"] = has_lib
        json.dump(meta, open(os.path.join(d, "meta.json"), "w"))
    json.dump(PRUNED, open(os.path.join(OUT, "pruned.json"), "w"), indent=0)
    fams = {}
    for c in CELLS:
        fams[c["family"]] = fams.get(c["family"], 0) + 1
    print("cells", len(CELLS), fams, "arms", sum(len(c["arms"]) for c in CELLS), "pruned", len(PRUNED))

if __name__ == "__main__":
    main()
