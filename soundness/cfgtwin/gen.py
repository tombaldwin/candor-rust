#!/usr/bin/env python3
"""gen.py — render the cfg-twin attack matrix (MATRIX.md sections A-G) into checked-in fixture crates.

    python3 soundness/cfgtwin/gen.py            # (re)render fx/ and predict.tsv in place
    python3 soundness/cfgtwin/gen.py --check    # render to a temp dir and diff against the checked-in tree

One BIN crate per cell: fx/<cell>/{Cargo.toml,src/main.rs,...,cell.json}. `main()` dispatches argv[1] to ONE probe, so a
marker file written (or not) under $MARK is that probe's executed ground truth. Hold the conventions of MATRIX.md section 0:

  * the HIDDEN (macro-text) arm is #[cfg(unix)], the visible pure arm is #[cfg(not(unix))] (host is unix);
  * EFF is spelled in full inside every macro body; pure twins are uniquely named per cell and never appear in hidden text;
  * identifiers carry the cell id; probes are <id>_e (effectful) and <id>_q (pure control).

cell.json carries what the runner needs (probes, dependency layout, chained shape); predict.tsv carries the matrix's predicted
pub / main readings and the acceptance for the candidate (`cand`) so a fix lane has the whole acceptance test in one file.
Readings: `-` = ABSENT or a row with no effect and no Unknown (the matrix's `0` over an executed write, `P` over a pure probe),
`E` = concrete effect, `U` = Unknown, `E+U` = both.  A `|` separates alternatives.
"""
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
EFF = '::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap();'

HDR = r'''#![allow(unused)]
macro_rules! cfg_x { ($($i:item)*) => { $($i)* } }   // "produced": an arm written inside cfg_x!{} is spliced; outside it is hand-written
#[macro_export] macro_rules! wt   { ($(fn $n:ident(&self);)*) => { $( fn $n(&self) { EFF } )* } }          // trait-item text -> effectful default(s)
#[macro_export] macro_rules! wfn  { ($v:vis fn $n:ident();) => { $v fn $n() { EFF } } }                     // item text -> effectful free fn
#[macro_export] macro_rules! wim  { ($o:path, $t:path, $n:ident) => { impl $t for $o { fn $n(&self) { EFF } } } }  // trait impl, effectful member
#[macro_export] macro_rules! winh { ($o:path, $n:ident) => { impl $o { pub fn $n(&self) { EFF } } } }        // inherent effectful method
#[macro_export] macro_rules! wuse { ($n:ident, $src:path) => { pub use $src as $n; } }                      // re-export under a name
#[macro_export] macro_rules! wty  { ($n:ident, $src:path) => { pub type $n = $src; } }                      // type alias
#[macro_export] macro_rules! wmdef{ ($n:ident) => { macro_rules! $n { () => { EFF } } } }                   // a macro_rules! definition
#[macro_export] macro_rules! wmod { ($m:ident, $n:ident) => { pub mod $m { pub fn $n() { EFF } } } }        // nested module with an effectful fn
pub struct Q;
'''

CELLS = {}      # id -> dict(files, probes, deps, ...)
PREDICT = []    # (cell, probe, pub, main, cand)


def sub(s, **kw):
    for k, v in kw.items():
        s = s.replace('{' + k + '}', v)
    return s


def cell(cid, files, probes, deps=None, lib=False, extra=None, chain=None, note='', rustflags=None, hdr=True, edition='2021', nocompile=False,
         detail=False, dep_deps=None):
    """files: relpath-under-src -> text (main.rs/lib.rs text is the BODY; HDR and the dispatcher are added).
    probes: list of (name, kind, body) where body is the fn body; the fn is `pub fn <name>() { body }`
    (a probe whose body is None is declared in `files` itself). `hdr=False` omits HDR (its `macro_rules!` definitions), for a
    cell whose subject is a crate that defines NO macro (section N).
    `edition` is the cell crate's edition (a dependency crate stays 2021). `nocompile=True` records `NOCOMPILE(Exxxx)` as the exec answer for a
    cell rustc rejects (the reading of the engine over a program that does not compile is still recorded). `detail=True` records the row's
    `invisible` / `dispatchesOn` disclosure in each reading (run.py `reading(detail=)`), for a cell whose subject is a KEY. `dep_deps` maps a
    dependency crate dir to ITS dependencies, `{dname: {name: spec}}` (a two-hop chain)."""
    assert cid not in CELLS, cid
    CELLS[cid] = dict(id=cid, files=files, probes=probes, deps=deps or {}, lib=lib, extra=extra or {}, chain=chain, note=note, hdr=hdr,
                      edition=edition, nocompile=nocompile, detail=detail, dep_deps=dep_deps or {})


def pred(cid, probe, pub, main, cand):
    PREDICT.append((cid, probe, pub, main, cand))


# ----------------------------------------------------------------------------------------------- A. D x V
# V: root items, hidden-arm twin items (lt), visible-arm twin items (pt), control items (ctl), probe bodies.
V = {
    1: dict(root='', lt=['crate::wfn! { pub fn go_{id}(); }'], pt=['pub fn go_{id}() {}'], ctl=['pub fn pure_{id}() {}'],
            e='{M}::go_{id}()', q='{M}::pure_{id}()'),
    2: dict(root='impl {M}::T_{id} for Q {}', lt=['pub trait T_{id} { crate::wt! { fn go_{id}(&self); } }'],
            pt=['pub trait T_{id} { fn go_{id}(&self) {} }'], ctl=['pub fn pure_{id}() {}'],
            e='use {M}::T_{id}; Q.go_{id}()', q='{M}::pure_{id}()'),
    3: dict(root='', lt=['crate::winh! { S_{id}, go_{id} }'], pt=['impl S_{id} { pub fn go_{id}(&self) {} }'],
            ctl=['pub struct S_{id};', 'pub fn pure_{id}() {}'], e='{M}::S_{id}.go_{id}()', q='{M}::pure_{id}()'),
    4: dict(root='pub trait Tr_{id} { fn go_{id}(&self); }', lt=['crate::wim! { crate::Q, crate::Tr_{id}, go_{id} }'],
            pt=['impl crate::Tr_{id} for crate::Q { fn go_{id}(&self) {} }'], ctl=['pub fn pure_{id}() {}'],
            e='use crate::Tr_{id}; Q.go_{id}()', q='{M}::pure_{id}()'),
    5: dict(root='pub fn eff_{id}() { EFF } pub fn pur_{id}() {}', lt=['crate::wuse! { go_{id}, crate::eff_{id} }'],
            pt=['pub use crate::pur_{id} as go_{id};'], ctl=['pub use crate::pur_{id} as keep_{id};'],
            e='{M}::go_{id}()', q='{M}::keep_{id}()'),
    6: dict(root='pub struct E_{id}; impl E_{id} { pub fn go(&self) { EFF } } pub struct P_{id}; impl P_{id} { pub fn go(&self) {} } '
                 'pub fn via_{id}(x: &{M}::Tt_{id}) { x.go() } pub fn viaq_{id}(x: &{M}::Tq_{id}) { x.go() }',
            lt=['crate::wty! { Tt_{id}, crate::E_{id} }'], pt=['pub type Tt_{id} = crate::P_{id};'],
            ctl=['pub type Tq_{id} = crate::P_{id};'], e='via_{id}(&E_{id})', q='viaq_{id}(&P_{id})'),
    7: dict(root='', lt=['crate::wmdef! { k_{id} }'], pt=['macro_rules! k_{id} { () => {} }'], ctl=[],
            e='k_{id}!();', q=None),
    8: dict(root='', lt=['crate::wmod! { in_{id}, go_{id} }'], pt=['pub mod in_{id} { pub fn go_{id}() {} }'],
            ctl=['pub mod inq_{id} { pub fn pure_{id}() {} }'], e='{M}::in_{id}::go_{id}()', q='{M}::inq_{id}::pure_{id}()'),
}

# D-template predictions, per `_e`: (pub, main, cand);  per `_q`: (pub, main, cand)
#   '-' = reads nothing over the probe; `|` alternatives; `U*` = U or a NAMED residual (only where pub reads `-`).
DPRED = {
    'd1':  (('U', 'U', 'U'),   ('-', '-', '-')),
    'd2':  (('-', '-', 'U'),   ('-', '-', '-')),
    'd3':  (('-', '-', 'U'),   ('U', 'U', 'U')),
    'd3h': (('-', '-', 'U'),   ('-', '-', '-')),
    'd4':  (('-', '-', 'U'),   ('-', '-', '-')),
    'd5':  (('U', 'U', 'U'),   ('U', 'U', 'U')),
    'd5p': (('U', 'U', 'U'),   ('U', 'U', 'U')),
    'd5h': (('-', '-', 'U'),   ('-', '-', '-')),
    'd6':  (('-', '-', 'U'),   ('-', '-', '-')),
    'd6h': (('-', '-', 'U'),   ('-', '-', '-')),
    'd7':  (('-', '-', 'U'),   ('-', '-', '-')),
    'd8':  (('U', 'U', 'U'),   ('-', '-', '-')),   # predict U (lift) — record
    'd9':  (('-', '-', 'U'),   ('-', '-', '-')),   # predict 0 — record
    'd10': (('-', '-', 'U'),   ('-', '-', '-')),
    'd11': (('-', '-', 'U'),   ('-', '-', '-')),   # predict 0 — record; _q P (hard) = S1 control
    'd12': (('U', 'U', 'U'),   ('-', '-', '-')),   # cand =pub; S3 control
    'd12p': (('U', 'U', 'U'),  ('-', '-', '-')),   # D12 PRODUCED (added by the matrix lane: the matrix lists D12 as hand-written but cites R1857's produced p11deep)
}
DNAMES = ['d1', 'd2', 'd3', 'd3h', 'd4', 'd5', 'd5p', 'd5h', 'd6', 'd6h', 'd7', 'd8', 'd9', 'd10', 'd11', 'd12', 'd12p']


def arm_mods(M, mu, live, pure, cid):
    return M, mu, live, pure


def render_d(d, v):
    """Return (main_body, extra_files, deps) for D template `d` x V template `v`."""
    cid = '%sv%d' % (d, v)
    spec = V[v]
    M = 'm_' + cid
    mu = '#[macro_use] ' if v == 7 else ''
    Mq = M  # path prefix for the arm's items, from the crate root
    ctl = spec['ctl']
    live_items = spec['lt'] + ctl
    pure_items = spec['pt'] + ctl
    LIVE = ' '.join(live_items)
    PURE = ' '.join(pure_items)
    deps = {}
    extra = {}
    if d in ('d1', 'd2'):
        body = '#[cfg(unix)] %spub mod %s { %s }\n#[cfg(not(unix))] %spub mod %s { %s }\n' % (mu, M, LIVE, mu, M, PURE)
        if d == 'd1':
            body = 'cfg_x! {\n' + body + '}\n'
    elif d in ('d3', 'd3h'):
        body = '#[cfg(unix)] %spub mod %s { %s }\n#[cfg(not(unix))] %spub mod %s;\n' % (mu, M, LIVE, mu, M)
        if d == 'd3':
            body = 'cfg_x! {\n' + body + '}\n'
        extra['%s.rs' % M] = PURE + '\n'
    elif d == 'd4':
        body = '#[cfg(unix)] %spub mod %s { %s }\n#[cfg(not(unix))] %spub mod %s;\n' % (mu, M, LIVE, mu, M)
        extra['%s/mod.rs' % M] = PURE + '\n'
    elif d in ('d5', 'd5h'):
        body = '#[cfg(unix)] %spub mod %s;\n#[cfg(not(unix))] %spub mod %s { %s }\n' % (mu, M, mu, M, PURE)
        if d == 'd5':
            body = 'cfg_x! {\n' + body + '}\n'
        extra['%s.rs' % M] = LIVE + '\n'
    elif d == 'd5p':
        body = ('#[cfg(unix)] %spub mod %s;\ncfg_x! { #[cfg(not(unix))] %spub mod %s { %s } }\n' % (mu, M, mu, M, PURE))
        extra['%s.rs' % M] = LIVE + '\n'
    elif d in ('d6', 'd6h'):
        body = ('#[cfg(unix)] %s#[path = "live_%s.rs"] pub mod %s;\n#[cfg(not(unix))] %s#[path = "pure_%s.rs"] pub mod %s;\n'
                % (mu, cid, M, mu, cid, M))
        if d == 'd6':
            body = 'cfg_x! {\n' + body + '}\n'
        extra['live_%s.rs' % cid] = LIVE + '\n'
        extra['pure_%s.rs' % cid] = PURE + '\n'
    elif d == 'd7':
        body = ('%s#[cfg_attr(unix, path = "live_%s.rs")] #[cfg_attr(not(unix), path = "pure_%s.rs")] pub mod %s;\n' % (mu, cid, cid, M))
        extra['live_%s.rs' % cid] = LIVE + '\n'
        extra['pure_%s.rs' % cid] = PURE + '\n'
    elif d == 'd8':
        deps['cfg-if'] = '"1.0"'
        body = ('cfg_if::cfg_if! {\n    if #[cfg(unix)] { %spub mod %s { %s } } else { %spub mod %s { %s } }\n}\n' % (mu, M, LIVE, mu, M, PURE))
    elif d == 'd9':
        deps['cfg-if'] = '"1.0"'
        body = ('cfg_if::cfg_if! {\n    if #[cfg(unix)] { %s#[path = "live_%s.rs"] pub mod %s; } else { %s#[path = "pure_%s.rs"] pub mod %s; }\n}\n'
                % (mu, cid, M, mu, cid, M))
        extra['live_%s.rs' % cid] = LIVE + '\n'
        extra['pure_%s.rs' % cid] = PURE + '\n'
    elif d == 'd10':
        body = ('#[cfg(unix)] %spub mod %s { %s }\n#[cfg(all(not(unix), windows))] %spub mod %s { %s }\n'
                '#[cfg(all(not(unix), not(windows)))] %spub mod %s { %s }\n' % (mu, M, LIVE, mu, M, PURE, mu, M, PURE))
    elif d == 'd11':
        body = '#[cfg(unix)] %spub mod %s;\n#[cfg(not(unix))] %spub mod %s;\n' % (mu, M, mu, M)
        lines = ['#[cfg(unix)] ' + it for it in spec['lt']] + ['#[cfg(not(unix))] ' + it for it in spec['pt']] + ctl
        extra['%s.rs' % M] = '\n'.join(lines) + '\n'
    elif d in ('d12', 'd12p'):
        a, b = 'a_' + cid, 'b_' + cid
        Mq = '%s::%s::%s' % (M, a, b)
        body = ('#[cfg(unix)] %spub mod %s { %spub mod %s { %spub mod %s { %s } } }\n'
                '#[cfg(not(unix))] %spub mod %s { %spub mod %s { %s %spub mod %s { pub use super::*; } } }\n'
                % (mu, M, mu, a, mu, b, LIVE, mu, M, mu, a, PURE, mu, b))
        if d == 'd12p':
            body = 'cfg_x! {\n' + body + '}\n'
    else:
        raise ValueError(d)
    root = spec['root']
    probes = [('%s_e' % cid, 'e', spec['e'])]
    if spec['q']:
        probes.append(('%s_q' % cid, 'q', spec['q']))
    fix = lambda s: sub(s, id=cid, M=Mq)
    body_all = fix(root) + '\n' + fix(body) if root else fix(body)
    # root items must precede nothing in particular; probes are appended by cell()
    return cid, fix(body_all), {k: fix(x) for k, x in extra.items()}, deps, [(p, k, fix(b)) for p, k, b in probes]


for _d in DNAMES:
    for _v in range(1, 9):
        _cid, _body, _extra, _deps, _probes = render_d(_d, _v)
        cell(_cid, {'main.rs': _body}, _probes, deps=_deps, extra=_extra)
        pe, pq = DPRED[_d]
        # V-specific adjustments of the matrix's prediction
        cand_e = pe[2]
        if _d in ('d3', 'd6', 'd6h', 'd7', 'd9') and _v in (5, 6, 7):
            cand_e = 'U*'
        if _d == 'd3h' and _v in (5, 6, 7):
            cand_e = 'U*'
        if _d == 'd4' and _v in (5, 6, 7):
            cand_e = 'U*'
        if _d in ('d5h', 'd11') and _v in (5, 6, 7):
            cand_e = 'U'
        if _d == 'd5' or _d == 'd5p':
            cand_e = '=pub'
        if _d in ('d12', 'd12p'):
            cand_e = '=pub'
        pe_pub, pe_main = pe[0], pe[1]
        if _d == 'd6' and _v == 1:
            pe_pub, pe_main = '-|U', '-|U'   # R986 ambiguous:path-redirected module may read U — record
        if _d in ('d5', 'd5p') and _v in (5, 6, 7):
            pass
        pred(_cid, '%s_e' % _cid, pe_pub, pe_main, cand_e)
        if _v != 7:
            qp = pq
            cand_q = qp[2]
            if _d == 'd3':
                cand_q = '=pub(U)'
            if _d in ('d5', 'd5p'):
                cand_q = '=pub(U)'
            if _d in ('d3h', 'd4', 'd5h', 'd11'):
                cand_q = '-'
            pred(_cid, '%s_q' % _cid, qp[0], qp[1], cand_q)
        else:
            pass  # V7 has no _q probe (R208 hedges every invocation of a twin name)

# D13 / D13b / D14: impl member gated only through the enclosing module / the parent file's mod decl.
_c = 'd13'
cell(_c, {'main.rs': sub('''pub trait TB_{id} { fn go_{id}(&self) {} }
#[cfg(unix)] pub mod m_{id} { crate::wim! { crate::Q, crate::TB_{id}, go_{id} } }
#[cfg(not(unix))] pub mod m_{id} { impl crate::TB_{id} for crate::Q {} }
''', id=_c)}, [('d13_e', 'e', 'use crate::TB_d13; Q.go_d13()')])
pred(_c, 'd13_e', '-', '-', 'U')
_c = 'd13b'
cell(_c, {'main.rs': sub('''pub trait TB_{id} { fn go_{id}(&self) {} }
#[cfg(unix)] pub mod m_{id} { crate::wim! { crate::Q, crate::TB_{id}, go_{id} } }
#[cfg(not(unix))] pub mod m_{id} { impl crate::TB_{id} for crate::Q { fn go_{id}(&self) {} } }
''', id=_c)}, [('d13b_e', 'e', 'use crate::TB_d13b; Q.go_d13b()')],
    note='D13b: member-leg control: visible impl overrides, so pub reads U and cand must stay =pub')
pred(_c, 'd13b_e', 'U', 'U', '=pub')
_c = 'd14'
cell(_c, {'main.rs': sub('''pub trait TB_{id} { fn go_{id}(&self) {} }
#[cfg(unix)] mod u_{id};
#[cfg(not(unix))] mod w_{id};
''', id=_c)}, [('d14_e', 'e', 'use crate::TB_d14; Q.go_d14()')],
    extra={'u_d14.rs': 'crate::wim! { crate::Q, crate::TB_d14, go_d14 }\n', 'w_d14.rs': 'impl crate::TB_d14 for crate::Q {}\n'},
    note='MEASURE: no acceptance until queue item Q3 is decided')
pred(_c, 'd14_e', '-', '-', 'record')


# ----------------------------------------------------------------------------------------------- B. hidden text (H) at D2 x V2
def d2v2(cid, root, live, pure, probes, deps=None, extra=None, note=''):
    M = 'm_' + cid
    body = '%s\n#[cfg(unix)] pub mod %s { %s }\n#[cfg(not(unix))] pub mod %s { %s }\n' % (root, M, live, M, pure)
    fix = lambda s: sub(s, id=cid, M=M)
    cell(cid, {'main.rs': fix(body)}, [(p, k, fix(b)) for p, k, b in probes], deps=deps, extra=extra, note=note)


T2E = [('{id}_e', 'e', 'use {M}::T_{id}; Q.go_{id}()'), ('{id}_q', 'q', '{M}::pure_{id}()')]
T2PURE = 'pub trait T_{id} { fn go_{id}(&self) {} } pub fn pure_{id}() {}'
T2ROOT = 'impl {M}::T_{id} for Q {}'


def dep_crate(cid, src):
    return {'name': 'fxdep_' + cid, 'files': {'src/lib.rs': src}}


def addH(h, root, live, pure, probes, pr, deps=None, extra=None, note='', dep=None):
    cid = h
    ex = dict(extra or {})
    d = dict(deps or {})
    if dep:
        d['fxdep_' + cid] = '{ path = "fxdep_%s" }' % cid
        for rel, txt in dep['files'].items():
            ex['@dep/fxdep_%s/%s' % (cid, rel)] = sub(txt.replace('EFF', EFF), id=cid)
    d2v2(cid, root, live, pure, probes, deps=d, extra=ex, note=note)
    for p, pub_, main_, cand_ in pr:
        pred(cid, p, pub_, main_, cand_)


_t2 = lambda cid: [('%s_e' % cid, 'e', 'use m_%s::T_%s; Q.go_%s()' % (cid, cid, cid)), ('%s_q' % cid, 'q', 'm_%s::pure_%s()' % (cid, cid))]
_std = [('{id}_e', '-', '-', 'U'), ('{id}_q', '-', '-', '-')]
WTEFF = '$( fn $n:ident(&self); )*'


def stdpr(cid, cand_e='U', cand_q='-', pub='-', main='-'):
    return [('%s_e' % cid, pub, main, cand_e), ('%s_q' % cid, '-', '-', cand_q)]


addH('h1', 'impl m_h1::T_h1 for Q {}', 'pub trait T_h1 { crate::wt! { fn go_h1(&self); } } pub fn pure_h1() {}', T2PURE.replace('{id}', 'h1'),
     _t2('h1'), stdpr('h1'))
addH('h2', '#[macro_export] macro_rules! def_h2 { () => { pub trait T_h2 { $crate::wt! { fn go_h2(&self); } } } }\nimpl m_h2::T_h2 for Q {}',
     'crate::def_h2! {} pub fn pure_h2() {}', T2PURE.replace('{id}', 'h2'), _t2('h2'), stdpr('h2'))
addH('h3', '#[macro_export] macro_rules! wt_s_h3 { ($(fn $n:ident(&self);)*) => { $( fn $n(&self) { EFF } )* } }\nimpl m_h3::T_h3 for Q {}',
     'pub trait T_h3 { super::wt_s_h3! { fn go_h3(&self); } } pub fn pure_h3() {}', T2PURE.replace('{id}', 'h3'), _t2('h3'), stdpr('h3'))
addH('h4', 'use crate::wt as wtr_h4;\nimpl m_h4::T_h4 for Q {}',
     'use super::wtr_h4; pub trait T_h4 { wtr_h4! { fn go_h4(&self); } } pub fn pure_h4() {}', T2PURE.replace('{id}', 'h4'), _t2('h4'), stdpr('h4'),
     note='LIVE begins with `use super::wtr_h4;`: a root-level `use` is not visible bare inside the child module, so the import is re-exposed')
_dwt = '#[macro_export] macro_rules! dwt { ($(fn $n:ident(&self);)*) => { $( fn $n(&self) { EFF } )* } }\n'
addH('h5', 'impl m_h5::T_h5 for Q {}', 'pub trait T_h5 { fxdep_h5::dwt! { fn go_h5(&self); } } pub fn pure_h5() {}', T2PURE.replace('{id}', 'h5'),
     _t2('h5'), stdpr('h5'), dep=dep_crate('h5', _dwt))
_dfix = '#[macro_export] macro_rules! dfix_{id} { () => { fn go_{id}(&self) { EFF } } }\n'
addH('h5b', 'impl m_h5b::T_h5b for Q {}', 'pub trait T_h5b { fxdep_h5b::dfix_h5b! {} } pub fn pure_h5b() {}', T2PURE.replace('{id}', 'h5b'),
     _t2('h5b'), [('h5b_e', '-', '-', 'U'), ('h5b_q', '-', '-', '-|U')], dep=dep_crate('h5b', _dfix))
addH('h6', '', 'thread_local! { static TL_h6: u8 = 0; } pub fn pure_h6() {}', 'pub fn pure_h6() {}',
     [('h6_q', 'q', 'm_h6::pure_h6()')], [('h6_q', '-', '-', '-')], note='no _e probe exists: nothing callable can hide here')
addH('h6b', '', 'thread_local! { pub static TL_h6b: u8 = { EFF 0 }; } pub fn pure_h6b() {}', 'thread_local! { pub static TL_h6b: u8 = 0; } pub fn pure_h6b() {}',
     [('h6b_e', 'e', 'm_h6b::TL_h6b.with(|_| ())'), ('h6b_q', 'q', 'm_h6b::pure_h6b()')],
     [('h6b_e', 'E', 'E', 'E'), ('h6b_q', '-', '-', '-')], note='initialiser effect: a 0 on any arm is a row outside lane A')
addH('h7', '', '', '', [('h7_q', 'q', 'm_h7::pure_h7()')], [('h7_q', '-', '-', '-')],
     note='roles reversed: the std text (compile_error!) is in the NON-compiled arm')
# h7 needs its own body (arms differ in cfg polarity); override below
CELLS['h7']['files']['main.rs'] = (
    '#[cfg(not(unix))] pub mod m_h7 { compile_error!("unsupported"); }\n#[cfg(unix)] pub mod m_h7 { pub fn pure_h7() {} }\n')
addH('h8', '#[allow(unused_macros)] macro_rules! thread_local { ($($t:tt)*) => { pub fn go_h8() { EFF } } }',
     'thread_local! { static X_h8: u8 = 0; }', 'pub fn go_h8() {}', [('h8_e', 'e', 'm_h8::go_h8()')],
     [('h8_e', '-', '-', 'E+U|U')], note='R1811 local shadow of a std name; record pub/main. cand: >=U')
_tl = '#[macro_export] macro_rules! thread_local { ($($t:tt)*) => { pub fn go_h9() { EFF } } }\n'
addH('h9', 'use fxdep_h9::thread_local;', 'use super::thread_local; thread_local! { static X_h9: u8 = 0; }', 'pub fn go_h9() {}',
     [('h9_e', 'e', 'm_h9::go_h9()')], [('h9_e', '-', '-', 'E+U|U')], dep=dep_crate('h9', _tl),
     note='imported shadow; LIVE re-imports via `use super::thread_local` so the bare name resolves inside the child module')
addH('h10', '', 'paste::paste! { pub fn [<go _h10>]() { EFF } }', 'pub fn go_h10() {}', [('h10_e', 'e', 'm_h10::go_h10()')],
     [('h10_e', '-', '-', 'U')], deps={'paste': '"1.0"'})
addH('h11', '#[allow(unused_macros)] macro_rules! call_h11 { ($m:ident) => { $m! { fn go_h11(&self); } } }\nimpl m_h11::T_h11 for Q {}',
     'pub trait T_h11 { crate::call_h11! { wt } } pub fn pure_h11() {}', T2PURE.replace('{id}', 'h11'), _t2('h11'),
     [('h11_e', '-', '-', 'U'), ('h11_q', '-', '-', '-|U')],
     note='root call_h11 is a plain macro_rules; invoked bare-textually through the crate path is not possible, so the crate:: path is a root-level macro_export below')
# call_h11 must be path-invocable: make it #[macro_export]
CELLS['h11']['files']['main.rs'] = CELLS['h11']['files']['main.rs'].replace('#[allow(unused_macros)] macro_rules! call_h11', '#[macro_export] macro_rules! call_h11')

# ----------------------------------------------------------------------------------------------- C. position (P) at D3 and D6
# P1, P2 = V1, V2 at D3 / D6 (cells d3v1, d3v2, d6v1, d6v2); P5 = D5 (d5v*) at D3's position and D6 only position: aliased, not duplicated.
for dd in ('d3', 'd6'):
    for pn, lv, pv, root, probe_e in (
        ('p3', 'pub struct S_{id}; impl S_{id} { crate::wim_m_{id}! { go_{id} } } pub fn pure_{id}() {}',
         'pub struct S_{id}; impl S_{id} { pub fn go_{id}(&self) {} } pub fn pure_{id}() {}',
         '#[macro_export] macro_rules! wim_m_{id} { ($n:ident) => { pub fn $n(&self) { EFF } } }', '{M}::S_{id}.go_{id}()'),
        ('p4', 'pub mod deep_{id} { crate::wfn! { pub fn go_{id}(); } } pub fn pure_{id}() {}',
         'pub mod deep_{id} { pub fn go_{id}() {} } pub fn pure_{id}() {}', '', '{M}::deep_{id}::go_{id}()'),
        ('p6', 'pub fn host_{id}() { crate::wimb_{id}! {} } pub fn pure_{id}() {}',
         'impl crate::Trb_{id} for crate::Q { fn go_{id}(&self) {} } pub fn pure_{id}() {}',
         'pub trait Trb_{id} { fn go_{id}(&self); }\n#[macro_export] macro_rules! wimb_{id} { () => { impl crate::Trb_{id} for crate::Q { fn go_{id}(&self) { EFF } } } }',
         'use crate::Trb_{id}; Q.go_{id}()'),
    ):
        cid = '%s%s' % (pn, dd)
        M = 'm_' + cid
        extra = {}
        if dd == 'd3':
            body = '#[cfg(unix)] pub mod %s { %s }\n#[cfg(not(unix))] pub mod %s;\n' % (M, lv, M)
            body = 'cfg_x! {\n' + body + '}\n'
            extra['%s.rs' % M] = pv + '\n'
        else:
            body = 'cfg_x! {\n#[cfg(unix)] #[path = "live_%s.rs"] pub mod %s;\n#[cfg(not(unix))] #[path = "pure_%s.rs"] pub mod %s;\n}\n' % (cid, M, cid, M)
            extra['live_%s.rs' % cid] = lv + '\n'
            extra['pure_%s.rs' % cid] = pv + '\n'
        fix = lambda s: sub(s, id=cid, M=M)
        cell(cid, {'main.rs': fix(root + '\n' + body)}, [(cid + '_e', 'e', fix(probe_e)), (cid + '_q', 'q', fix('%s::pure_{id}()' % M))],
             extra={k: fix(x) for k, x in extra.items()})
        pred(cid, cid + '_e', '-', '-', 'U')
        pred(cid, cid + '_q', 'U' if dd == 'd3' else '-', 'U' if dd == 'd3' else '-', '=pub' if dd == 'd3' else '-')

# ----------------------------------------------------------------------------------------------- D. use site (U) at D6 x V2 and D13
def d6v2_use(un, probe_body, extra_root='', extra_in_files='', ret_probe=None):
    cid = 'u%dd6' % un
    M = 'm_' + cid
    live = 'pub trait T_{id} { crate::wt! { fn go_{id}(&self); } } pub fn pure_{id}() {} ' + extra_in_files
    pure = 'pub trait T_{id} { fn go_{id}(&self) {} } pub fn pure_{id}() {} ' + extra_in_files
    body = ('impl {M}::T_{id} for Q {}\n' + extra_root + '\ncfg_x! {\n#[cfg(unix)] #[path = "live_{id}.rs"] pub mod {M};\n'
            '#[cfg(not(unix))] #[path = "pure_{id}.rs"] pub mod {M};\n}\n')
    fix = lambda s: sub(s, id=cid, M=M)
    cell(cid, {'main.rs': fix(body)}, [(cid + '_e', 'e', fix(probe_body))],
         extra={'live_%s.rs' % cid: fix(live) + '\n', 'pure_%s.rs' % cid: fix(pure) + '\n'})
    pred(cid, cid + '_e', '-', '-', 'U')


d6v2_use(1, '{M}::call_{id}()', extra_in_files='pub fn call_{id}() { use self::T_{id}; crate::Q.go_{id}() }')
d6v2_use(2, '{M}::T_{id}::go_{id}(&Q)')
d6v2_use(3, 'Q.go_{id}()', extra_root='use {M}::T_{id};')
d6v2_use(4, 'use {M}::T_{id}; Q.go_{id}()')
d6v2_use(5, '<Q as {M}::T_{id}>::go_{id}(&Q)')
d6v2_use(6, 'via_{id}(Q)', extra_root='pub fn via_{id}<T: {M}::T_{id}>(t: T) { t.go_{id}() }')
d6v2_use(7, 'viad_{id}(&Q)', extra_root='pub fn viad_{id}(t: &dyn {M}::T_{id}) { t.go_{id}() }')


def d13_use(un, probe_body, extra_root=''):
    cid = 'u%dd13' % un
    body = ('pub trait TB_{id} { fn go_{id}(&self) {} }\n#[cfg(unix)] pub mod m_{id} { crate::wim! { crate::Q, crate::TB_{id}, go_{id} } }\n'
            '#[cfg(not(unix))] pub mod m_{id} { impl crate::TB_{id} for crate::Q {} }\n' + extra_root)
    fix = lambda s: sub(s, id=cid)
    cell(cid, {'main.rs': fix(body)}, [(cid + '_e', 'e', fix(probe_body))])
    pred(cid, cid + '_e', '-', '-', 'U')


d13_use(2, 'TB_{id}::go_{id}(&Q)')
d13_use(3, 'via3_{id}::call_{id}()', extra_root='pub mod via3_{id} { use crate::TB_{id}; pub fn call_{id}() { crate::Q.go_{id}() } }')
d13_use(4, 'Q.go_{id}()', extra_root='')   # method call needs the trait in scope: it is at the crate root, so it is
d13_use(5, '<Q as TB_{id}>::go_{id}(&Q)')
d13_use(6, 'via_{id}(Q)', extra_root='pub fn via_{id}<T: TB_{id}>(t: T) { t.go_{id}() }')
d13_use(7, 'viad_{id}(&Q)', extra_root='pub fn viad_{id}(t: &dyn TB_{id}) { t.go_{id}() }')

# U8: chained consumer. The producer must be a lib (a consumer cannot depend on a bin), so lib=True: src/lib.rs holds the
# body and src/main.rs is only the dispatcher. `cons/` is a nested crate depending on the producer by path.
for kind in ('d6', 'd13'):
    cid = 'u8%s' % kind
    if kind == 'd6':
        M = 'm_' + cid
        body = ('impl {M}::T_{id} for Q {}\ncfg_x! {\n#[cfg(unix)] #[path = "live_{id}.rs"] pub mod {M};\n'
                '#[cfg(not(unix))] #[path = "pure_{id}.rs"] pub mod {M};\n}\n')
        extra = {'live_%s.rs' % cid: 'pub trait T_{id} { crate::wt! { fn go_{id}(&self); } } pub fn pure_{id}() {}\n',
                 'pure_%s.rs' % cid: 'pub trait T_{id} { fn go_{id}(&self) {} } pub fn pure_{id}() {}\n'}
        cons = 'pub fn c_{id}_e() { use fx_{id}::{M}::T_{id}; fx_{id}::Q.go_{id}() }'
        prod_probe = ('%s_e' % cid, 'e', 'use {M}::T_{id}; Q.go_{id}()')
    else:
        body = ('pub trait TB_{id} { fn go_{id}(&self) {} }\n#[cfg(unix)] pub mod m_{id} { crate::wim! { crate::Q, crate::TB_{id}, go_{id} } }\n'
                '#[cfg(not(unix))] pub mod m_{id} { impl crate::TB_{id} for crate::Q {} }\n')
        extra = {}
        cons = 'pub fn c_{id}_e() { use fx_{id}::TB_{id}; fx_{id}::Q.go_{id}() }'
        prod_probe = ('%s_e' % cid, 'e', 'use crate::TB_{id}; Q.go_{id}()')
        M = 'm_' + cid
    fix = lambda s, cid=cid, M=M: sub(s, id=cid, M=M)
    cell(cid, {'lib.rs': fix(body)}, [(prod_probe[0], prod_probe[1], fix(prod_probe[2]))], lib=True,
         extra={k: fix(x) for k, x in extra.items()},
         chain=dict(kind='consumer', consumer_probe='c_%s_e' % cid,
                    consumer_files={'src/lib.rs': fix(cons) + '\n'}, dep_name='fx_%s' % cid, dep_package='cf_%s' % cid),
         note='producer is lib+bin; probe c_%s_e lives in cons/ and is scanned CHAINED against the producer report' % cid)
    pred(cid, cid + '_e', '-', '-', 'U')
    pred(cid, 'c_%s_e@chained' % cid, '>=U', '>=U', '>=U')


# ----------------------------------------------------------------------------------------------- E. lane B: stood member
def e_cell(cid, trait, default, ov, vis, use, swap=False, extra_decl='', uses=None, note='', deps=None, dep=None, std=False, pr=None,
           lib=False, chain=None):
    """Root: `pub struct QG_{id};` The visible impl is PRODUCED with its own cfg: cfg_x! { #[cfg(unix)] impl TR for QG_{id} VIS }.
    The hidden override arrives through a macro_export macro: #[cfg(not(unix))] impl TR for QG_{id} { OV } (swapped for L)."""
    vcfg, hcfg = ('not(unix)', 'unix') if swap else ('unix', 'not(unix)')
    if std:
        impl_head = 'impl Iterator for QG_{id}'
    else:
        impl_head = 'impl %s for QG_{id}' % trait
    body = '''pub struct QG_{id};
%s
#[macro_export] macro_rules! wov_{id} { () => { #[cfg(%s)] %s { %s } } }
crate::wov_{id}! {}
cfg_x! { #[cfg(%s)] %s %s }
''' % (extra_decl, hcfg, impl_head, ov, vcfg, impl_head, vis)
    fix = lambda s: sub(s, id=cid)
    probes = [(cid + '_e', 'e', fix(use))]
    if uses:
        probes = [(cid + '_' + k, 'e' if k.endswith('e') else 'q', fix(b)) for k, b in uses]
    ex = {}
    d = dict(deps or {})
    if dep:
        d['fxdep_' + cid] = '{ path = "fxdep_%s" }' % cid
        ex['@dep/fxdep_%s/src/lib.rs' % cid] = sub(dep.replace('EFF', EFF), id=cid)
    cell(cid, {('lib.rs' if lib else 'main.rs'): fix(body)}, probes, deps=d, extra=ex, note=note, lib=lib, chain=chain)
    for p, pub_, main_, cand_ in (pr or []):
        pred(cid, p, pub_, main_, cand_)
    return cid


def trait_decl(cid, default_eff, extra=''):
    return 'pub trait ExtG_%s { fn go_%s(&self) %s %s }' % (cid, cid, '{ EFF }' if default_eff else '{}', extra)


def E(cid, default_eff, ov_eff, vis, swap=False, use='QG_{id}.go_{id}()', trait_use='use ExtG_{id};', pre=None, note='', uses=None, extra_trait='',
      pr=None, extra_root=''):
    ov = 'fn go_%s(&self) %s' % (cid, '{ EFF }' if ov_eff else '{}')
    td = trait_decl(cid, default_eff, extra_trait) + '\n' + extra_root
    body_use = trait_use + ' ' + use if trait_use else use
    return e_cell(cid, 'ExtG_%s' % cid, default_eff, ov, vis, body_use, swap=swap, extra_decl=td, note=note,
                  uses=[(k, (trait_use.replace('{id}', cid) + ' ' + b) if trait_use else b) for k, b in uses] if uses else None, pr=pr)


E('e1', True, False, '{}', pr=[('e1_e', 'U', 'U', 'E+U')], note='calibration cell for lane B; pub reads U-only (deny Fs 0)')
E('e1l', True, False, '{}', swap=True, pr=[('e1l_e', 'U', 'U', 'U|E+U')])
E('e2', False, True, '{}', pr=[('e2_e', 'U', 'U', 'U')])
E('e2l', False, True, '{}', swap=True, pr=[('e2l_e', 'U', 'U', 'U|E+U')])
E('e3', True, True, '{}', pr=[('e3_e', 'U', 'U', 'E+U')])
E('e4', True, False, '{ fn other_e4(&self) {} }', extra_trait='fn other_e4(&self) {}',
  uses=[('e', 'QG_e4.go_e4()'), ('q', 'QG_e4.other_e4()')], pr=[('e4_e', 'U', 'U', 'E+U'), ('e4_q', '-', '-', '-')],
  note='other_e4 stays P')
E('e5', True, False, '{ fn go_e5(&self) {} }', pr=[('e5_e', 'U', 'U', 'U')], note='visible member overrides, pure; exec none')
E('e5b', True, False, '{ fn go_e5b(&self) { EFF } }', pr=[('e5b_e', 'E+U', 'E+U', 'E+U')], note='visible member effectful')
# E6-E9: E1 shape at UFCS / bound / dyn / chained
E('e6', True, False, '{}', use='<QG_{id} as ExtG_{id}>::go_{id}(&QG_{id})', trait_use='', pr=[('e6_e', 'U', 'U', 'E+U')])
E('e7', True, False, '{}', use='via_{id}(QG_{id})', trait_use='', extra_root='pub fn via_{id}<T: ExtG_{id}>(t: T) { t.go_{id}() }',
  pr=[('e7_e', 'U', 'U', 'E+U')], note='rk_e / S4(b)')
E('e8', True, False, '{}', use='viad_{id}(&QG_{id})', trait_use='', extra_root='pub fn viad_{id}(t: &dyn ExtG_{id}) { t.go_{id}() }',
  pr=[('e8_e', 'U', 'U', 'E+U')])
E('e10', False, True, '{}', use='via_{id}(QG_{id})', trait_use='', extra_root='pub fn via_{id}<T: ExtG_{id}>(t: T) { t.go_{id}() }',
  pr=[('e10_e', 'U', 'U', 'U')])
E('e11', False, True, '{}', use='viad_{id}(&QG_{id})', trait_use='', extra_root='pub fn viad_{id}(t: &dyn ExtG_{id}) { t.go_{id}() }',
  pr=[('e11_e', 'U', 'U', 'U')])

# E9: chained consumer (E1 shape). Producer is lib+bin; cons/ is scanned chained.
_cid = 'e9'
_ov = 'fn go_e9(&self) {}'
_td = trait_decl(_cid, True)
_body = sub('''pub struct QG_{id};
%s
#[macro_export] macro_rules! wov_{id} { () => { #[cfg(not(unix))] impl ExtG_{id} for QG_{id} { %s } } }
crate::wov_{id}! {}
cfg_x! { #[cfg(unix)] impl ExtG_{id} for QG_{id} {} }
''' % (_td, _ov), id=_cid)
cell(_cid, {'lib.rs': _body}, [('e9_e', 'e', 'use ExtG_e9; QG_e9.go_e9()')], lib=True,
     chain=dict(kind='consumer', consumer_probe='c_e9_e',
                consumer_files={'src/lib.rs': 'pub fn c_e9_e() { use fx_e9::ExtG_e9; fx_e9::QG_e9.go_e9() }\n'}, dep_name='fx_e9', dep_package='cf_e9'))
pred('e9', 'e9_e', 'U', 'U', 'E+U')
pred('e9', 'c_e9_e@chained', '>=U', '>=U', 'E+U')

# E12: std trait (Iterator)
_it_vis = '{ type Item = u8; fn next(&mut self) -> Option<u8> { None } }'
_it_ov = 'type Item = u8; fn next(&mut self) -> Option<u8> { None } fn count(self) -> usize { EFF 0 }'
for _cid, _swap in (('e12', False), ('e12l', True)):
    e_cell(_cid, '', False, _it_ov, _it_vis, '', swap=_swap, std=True,
           uses=[('e', 'QG_%s.count();' % _cid), ('b', 'via_%s(QG_%s);' % (_cid, _cid))],
           extra_decl='pub fn via_%s<I: Iterator>(i: I) { i.count(); }' % _cid,
           pr=[(_cid + '_e', 'U', 'U', '>=U'), (_cid + '_b', 'U', 'U', '>=U')])
    CELLS[_cid]['probes'] = [(_cid + '_e', 'e', 'QG_%s.count();' % _cid), (_cid + '_b', 'e', 'via_%s(QG_%s);' % (_cid, _cid))]

# E13 / E14: dependency trait, direct and chained over the dependency's report
_dtr13 = 'pub trait TD_{id} { fn go_{id}(&self) { EFF } }\n'
_dtr14 = 'pub trait TD_{id} { fn go_{id}(&self) {} }\n'
for _cid, _dtr, _ovbody, _pr in (('e13', _dtr13, 'fn go_e13(&self) {}', [('e13_e', 'U|-', 'U|-', '>=U'), ('e13_e@chained', 'record', 'record', 'E+U')]),
                                 ('e14', _dtr14, 'fn go_e14(&self) { EFF }', [('e14_e', 'U|-', 'U|-', 'U'), ('e14_e@chained', 'record', 'record', 'U')])):
    _body = sub('''pub struct QG_{id};
#[macro_export] macro_rules! wov_{id} { () => { #[cfg(not(unix))] impl fxdep_{id}::TD_{id} for QG_{id} { %s } } }
crate::wov_{id}! {}
cfg_x! { #[cfg(unix)] impl fxdep_{id}::TD_{id} for QG_{id} {} }
''' % _ovbody, id=_cid)
    cell(_cid, {'main.rs': _body}, [(_cid + '_e', 'e', sub('use fxdep_{id}::TD_{id}; QG_{id}.go_{id}()', id=_cid))],
         deps={'fxdep_' + _cid: '{ path = "fxdep_%s" }' % _cid},
         extra={'@dep/fxdep_%s/src/lib.rs' % _cid: sub(_dtr.replace('EFF', EFF), id=_cid)},
         chain=dict(kind='dep', dep_dir='fxdep_%s' % _cid))
    for p, a, b, c in _pr:
        pred(_cid, p, a, b, c)

# E13c: CHAIN-LIVENESS CONTROL for E13/E14 (added by the matrix lane). The same dependency trait with NO hidden twin at all: unchained the call
# reads `-`, chained it MUST read E (the dependency default's Fs). If it does not, the E13/E14 chained cells measured a dead join.
_cid = 'e13c'
cell(_cid, {'main.rs': sub('''pub struct QG_{id};
impl fxdep_{id}::TD_{id} for QG_{id} {}
''', id=_cid)}, [('e13c_e', 'e', sub('use fxdep_{id}::TD_{id}; QG_{id}.go_{id}()', id=_cid))],
     deps={'fxdep_' + _cid: '{ path = "fxdep_%s" }' % _cid},
     extra={'@dep/fxdep_%s/src/lib.rs' % _cid: sub(_dtr13.replace('EFF', EFF), id=_cid)},
     chain=dict(kind='dep', dep_dir='fxdep_%s' % _cid), note='chain-liveness control: unchained `-`, chained must be E')
pred('e13c', 'e13c_e', '-', '-', '-')
pred('e13c', 'e13c_e@chained', 'E', 'E', 'E')

# E15: module-gated visible impl with an effectful local default and a hidden pure override
_cid = 'e15'
_body = sub('''pub struct QG_{id};
pub trait ExtG_{id} { fn go_{id}(&self) { EFF } }
#[macro_export] macro_rules! wov_{id} { () => { #[cfg(not(unix))] impl ExtG_{id} for QG_{id} { fn go_{id}(&self) {} } } }
crate::wov_{id}! {}
#[cfg(unix)] pub mod vis_{id} { impl crate::ExtG_{id} for crate::QG_{id} {} }
#[cfg(not(unix))] pub mod vis_{id} {}
''', id=_cid)
cell(_cid, {'main.rs': _body}, [('e15_e', 'e', 'use ExtG_e15; QG_e15.go_e15()')])
pred('e15', 'e15_e', '-', '-', 'E+U')

# ----------------------------------------------------------------------------------------------- F. lane B: R1855 and item-position binding
cell('f1', {'main.rs': sub('''#[macro_use] mod defs_{id};
pub fn f1_e() { kx_{id}!(); }
macro_rules! kx_{id} { () => {} }
pub fn f1_q() { kx_{id}!(); }
''', id='f1')},
     [], extra={'defs_f1.rs': sub('macro_rules! def_{id} { () => { macro_rules! kx_{id} { () => { { EFF } } } } }\ndef_{id}!();\n', id='f1')})
CELLS['f1']['probes'] = [('f1_e', 'e', None), ('f1_q', 'q', None)]
pred('f1', 'f1_e', '-', '-', 'U'); pred('f1', 'f1_q', '-', '-', '-|U')
cell('f2', {'main.rs': sub('''#[macro_use] mod defs_{id};
pub fn f2_e() { ks_{id}!(); }
macro_rules! ks_{id} { () => {} }
pub fn f2_q() { ks_{id}!(); }
''', id='f2')}, [], extra={'defs_f2.rs': sub('macro_rules! ks_{id} { () => { { EFF } } }\n', id='f2')})
CELLS['f2']['probes'] = [('f2_e', 'e', None), ('f2_q', 'q', None)]
pred('f2', 'f2_e', 'U', 'U', '=pub'); pred('f2', 'f2_q', '-|U', '-|U', '-|U')
cell('f3', {'main.rs': sub('''#[macro_use] mod defs_{id};
mkf_{id}!(f3_e);
macro_rules! mkf_{id} { ($n:ident) => { pub fn $n() {} } }
mkf_{id}!(f3_q);
''', id='f3')}, [], extra={'defs_f3.rs': sub('macro_rules! mkf_{id} { ($n:ident) => { pub fn $n() { EFF } } }\n', id='f3')})
CELLS['f3']['probes'] = [('f3_e', 'e', None), ('f3_q', 'q', None)]
pred('f3', 'f3_e', '-', '-', '>=U'); pred('f3', 'f3_q', '-', '-', '-|U')
cell('f4', {'main.rs': sub('''macro_rules! defk_{id} { () => { macro_rules! kk_{id} { ($n:ident) => { pub fn $n() { EFF } } } } }
pub mod in_{id} { defk_{id}!(); kk_{id}!(f4_em); }
macro_rules! kk_{id} { ($n:ident) => { pub fn $n() {} } }
kk_{id}!(f4_qm);
''', id='f4')}, [('f4_e', 'e', 'in_f4::f4_em()'), ('f4_q', 'q', 'f4_qm()')],
     note='child module emits kk before the parent source defines it; invoked inside the child')
pred('f4', 'f4_e', 'record', 'record', '>=U'); pred('f4', 'f4_q', 'record', 'record', '-|U')
cell('f5', {'main.rs': sub('''mod defs_{id};
pub fn f5_e() { crate::kp_{id}!(); }
''', id='f5')}, [], extra={'defs_f5.rs': sub('#[macro_export] macro_rules! kp_{id} { () => { { EFF } } }\n', id='f5')},
     note='path-invocation class: record only (D5.2 known silence), not lane B acceptance')
CELLS['f5']['probes'] = [('f5_e', 'e', None)]
pred('f5', 'f5_e', 'record', 'record', 'record')

# ----------------------------------------------------------------------------------------------- G. controls
# G1 visible-visible (fabrication control): D2 with no macro at all
cell('g1', {'main.rs': sub('''#[cfg(unix)] pub mod m_{id} { pub fn go_{id}() { EFF } pub fn pure_{id}() {} }
#[cfg(not(unix))] pub mod m_{id} { pub fn go_{id}() {} pub fn pure_{id}() {} }
''', id='g1')}, [('g1_e', 'e', 'm_g1::go_g1()'), ('g1_q', 'q', 'm_g1::pure_g1()')], note='fabrication control: _e E with no U; _q P on every arm')
pred('g1', 'g1_e', 'E', 'E', 'E'); pred('g1', 'g1_q', '-', '-', '-')
# G2 hidden text naming nothing visible
cell('g2', {'main.rs': sub('''#[cfg(unix)] pub mod m_{id} { crate::wfn! { pub fn other_{id}(); } pub fn pure_{id}() {} }
#[cfg(not(unix))] pub mod m_{id} { pub fn other_{id}() {} pub fn pure_{id}() {} }
''', id='g2')}, [('g2_e', 'e', 'm_g2::other_g2()'), ('g2_q', 'q', 'm_g2::pure_g2()')], note='_q must read P on every arm; other_g2 (_e) U')
pred('g2', 'g2_e', '-', '-', 'U'); pred('g2', 'g2_q', '-', '-', '-')
# G5 cache coherence (S7): D6 x V2 with the live file holding NO hidden text; the runner rescans after swapping in edit/live_g5.rs
cell('g5', {'main.rs': sub('''impl m_{id}::T_{id} for Q {}
cfg_x! {
#[cfg(unix)] #[path = "live_{id}.rs"] pub mod m_{id};
#[cfg(not(unix))] #[path = "pure_{id}.rs"] pub mod m_{id};
}
''', id='g5')}, [('g5_e', 'e', 'use m_g5::T_g5; Q.go_g5()')],
     extra={'live_g5.rs': 'pub trait T_g5 { fn go_g5(&self) {} } pub fn pure_g5() {}\n',
            'pure_g5.rs': 'pub trait T_g5 { fn go_g5(&self) {} } pub fn pure_g5() {}\n',
            '@edit/live_g5.rs': 'pub trait T_g5 { crate::wt! { fn go_g5(&self); } } pub fn pure_g5() {}\n'},
     chain=dict(kind='cache-edit', edit_file='live_g5.rs'),
     note='state A: both arms pure (exec none). state B (after the edit): live arm has hidden text, exec Fs. The runner scans A, swaps '
          'in the edit, rescans with the SAME cache dir; the reading that matters is g5_e@rescan')
pred('g5', 'g5_e', '-', '-', '-')
pred('g5', 'g5_e@rescan', '-', '-', 'U')
pred('g5', 'g5_e@rescan-incr', '-', '-', 'U')


# ----------------------------------------------------------------------------------------------- N. no local macro (R1906; R1908 recorded)
# Every cell above carries HDR's `macro_rules!`, which is what made the arm cover run at all: in a crate that defines NO macro the
# cover never ran (SOUNDNESS R1906, Fable's cfg-twin lane A merge review, fixtures `nolocal*`). The hidden text here is a
# DEPENDENCY's macro. `_ctl` is the one-variable calibration twin: the same crate plus one unrelated local `macro_rules!`, which
# made the cover run on main.
_WFN = '#[macro_export] macro_rules! wfn { ($v:vis fn $n:ident();) => { $v fn $n() { EFF } } }\n'
for _cid, _ctl in (('n1', False), ('n1c', True)):
    cell(_cid, {'main.rs': sub('#![allow(unused)]\n' + ('macro_rules! unrelated { () => {} }\n' if _ctl else '') +
                               '#[cfg(unix)] pub mod m_{id} { fxdep_{id}::wfn! { pub fn go_{id}(); } pub fn pure_{id}() {} }\n'
                               '#[cfg(not(unix))] pub mod m_{id} { pub fn go_{id}() {} pub fn pure_{id}() {} }\n', id=_cid)},
         [(_cid + '_e', 'e', sub('m_{id}::go_{id}()', id=_cid)), (_cid + '_q', 'q', sub('m_{id}::pure_{id}()', id=_cid))],
         deps={'fxdep_' + _cid: '{ path = "fxdep_%s" }' % _cid}, extra={'@dep/fxdep_%s/src/lib.rs' % _cid: _WFN}, hdr=False,
         note='R1906: inline hand-written twins, hidden text a dependency macro' + (' (+ an unrelated local macro_rules!)' if _ctl else ''))
    pred(_cid, _cid + '_e', '-', 'U' if _ctl else '-', 'U'); pred(_cid, _cid + '_q', '-', 'U' if _ctl else '-', 'record')
for _cid, _ctl in (('n2', False), ('n2c', True)):
    cell(_cid, {'main.rs': sub('#![allow(unused)]\n' + ('macro_rules! unrelated { () => {} }\n' if _ctl else '') +
                               '#[cfg(unix)] #[path = "live_{id}.rs"] pub mod m_{id};\n'
                               '#[cfg(not(unix))] #[path = "pure_{id}.rs"] pub mod m_{id};\n', id=_cid)},
         [(_cid + '_e', 'e', sub('m_{id}::go_{id}()', id=_cid))],
         deps={'fxdep_' + _cid: '{ path = "fxdep_%s" }' % _cid},
         extra={'@dep/fxdep_%s/src/lib.rs' % _cid: _WFN, 'live_%s.rs' % _cid: sub('fxdep_{id}::wfn! { pub fn go_{id}(); }\n', id=_cid),
                'pure_%s.rs' % _cid: sub('pub fn go_{id}() {}\n', id=_cid)}, hdr=False,
         note='R1906: two #[path] arms, hidden text a dependency macro' + (' (+ an unrelated local macro_rules!)' if _ctl else ''))
    pred(_cid, _cid + '_e', 'U', 'U', 'U')
# R1908: a bare `thread_local!` beside a glob, with a hand-written gated pair whose BOTH arms are visible. A glob of a module this file
# declares cannot carry another crate's macro (`n3`: `gated_n3` must not be hedged); a dependency glob still may (`n3d`, recorded). Each
# defines one unrelated `macro_rules!`, so the cover runs on main too (R1906) and main's over-hold is visible.
_N3 = ('#![allow(unused)]\nmacro_rules! unrelated_{id} { () => {} }\npub mod helpers_{id} { pub fn h_{id}() {} }\nuse {G}::*;\nuse std::cell::Cell;\n'
       'thread_local! { static X_{id}: Cell<u8> = { EFF Cell::new(0) }; }\n'
       '#[cfg(unix)] pub fn gated_{id}() { EFF }\n#[cfg(not(unix))] pub fn gated_{id}() {}\n')
cell('n3', {'main.rs': sub(_N3.replace('{G}', 'helpers_{id}'), id='n3')},
     [('n3_e', 'e', 'gated_n3()'), ('n3_tl', 'e', 'X_n3.with(|c| c.get())')], hdr=False,
     note='R1908: local-module glob; main over-holds gated_n3 (E+U). Recorded for the toolchain-shadowing design, no acceptance')
pred('n3', 'n3_e', 'E', 'E+U', 'record'); pred('n3', 'n3_tl', 'E', 'E', 'E')
cell('n3d', {'main.rs': sub(_N3.replace('{G}', 'fxdep_{id}'), id='n3d')},
     [('n3d_e', 'e', 'gated_n3d()'), ('n3d_tl', 'e', 'X_n3d.with(|c| c.get())')],
     deps={'fxdep_n3d': '{ path = "fxdep_n3d" }'}, extra={'@dep/fxdep_n3d/src/lib.rs': 'pub fn hd() {}\n'}, hdr=False,
     note='R1908: dependency glob; may rebind thread_local, so the hold stays (recorded)')
pred('n3d', 'n3d_e', 'E', 'E+U', 'record'); pred('n3d', 'n3d_tl', 'E', 'E', 'E')


# ----------------------------------------------------------------------------------------------- R. R1919: the VISIBLE arms
# SOUNDNESS R1919 (lane rust-r1919). The row read the produced-arm spellings (a bare `wuse!` / `wov!{}` / `wt!` arm) as a cover gap.
# Measured: the SAME silences read on HAND-WRITTEN arms with no macro at all, so they are not the hidden-text cover's: once an arm is
# expanded (or written by hand) both arms are visible and the defect is in the resolver that reads two visible arms. Four resolvers:
#   r1  an alias arm naming a crate-ROOT fn (`pub use crate::eff as g` in each arm): R105's arm loop and the re-export index never placed it;
#   r2  a gated override beside an impl that INHERITS the default: a method call bound the override unit alone;
#   r3  `macro_rules!` twins gated through their enclosing MODULE: R208 read only the definition's own `#[cfg]`;
#   r4  two default bodies of one trait leaf (nested / `#[path]` twins of the trait): the default route skipped silently.
# Each cell: hand-written arms (`h`), the row's produced spelling (`p`, a BARE local macro, which the engine expands), pure controls.
_R1 = '''pub fn eff_{id}() { EFF } pub fn pur_{id}() {}
#[cfg(unix)] pub mod tw_{id} { pub use crate::eff_{id} as g; }
#[cfg(not(unix))] pub mod tw_{id} { pub use crate::pur_{id} as g; }
#[cfg(not(unix))] pub mod tr_{id} { pub use crate::pur_{id} as g; }
#[cfg(unix)] pub mod tr_{id} { pub use crate::eff_{id} as g; }
pub mod in_{id} { #[cfg(unix)] pub use crate::eff_{id} as g; #[cfg(not(unix))] pub use crate::pur_{id} as g; }
#[cfg(unix)] pub mod t3_{id} { pub use crate::eff_{id} as g; }
#[cfg(all(not(unix), windows))] pub mod t3_{id} { pub use crate::pur_{id} as g; }
#[cfg(all(not(unix), not(windows)))] pub mod t3_{id} { pub use crate::pur_{id} as g; }
#[cfg(unix)] pub mod rx_{id} { pub use crate::eff_{id} as gx_{id}; }
#[cfg(not(unix))] pub mod rx_{id} { pub use crate::pur_{id} as gx_{id}; }
pub use rx_{id}::gx_{id};
pub mod user_{id} { use crate::tw_{id}::g; pub fn run() { g() } }
pub mod sole_{id} { pub use crate::eff_{id} as g; }
cfg_x! {
#[cfg(unix)] pub mod pw_{id} { wuse! { g, crate::eff_{id} } pub use crate::pur_{id} as keep; }
#[cfg(not(unix))] pub mod pw_{id} { pub use crate::pur_{id} as g; pub use crate::pur_{id} as keep; }
}
'''
cell('r1', {'main.rs': sub(_R1, id='r1')},
     [('r1_tw_e', 'e', 'tw_r1::g()'), ('r1_rev_e', 'e', 'tr_r1::g()'), ('r1_in_e', 'e', 'in_r1::g()'), ('r1_three_e', 'e', 't3_r1::g()'),
      ('r1_crate_e', 'e', 'crate::tw_r1::g()'), ('r1_bodyuse_e', 'e', 'use crate::tw_r1::g; g()'), ('r1_bodyrel_e', 'e', 'use tw_r1::g; g()'),
      ('r1_moduse_e', 'e', 'user_r1::run()'), ('r1_fnptr_e', 'e', 'let f = tw_r1::g; f()'), ('r1_reexp_e', 'e', 'gx_r1()'),
      ('r1_reexp2_e', 'e', 'crate::gx_r1()'), ('r1_sole_e', 'e', 'use sole_r1::g; g()'), ('r1_p_e', 'e', 'pw_r1::g()'),
      ('r1_p_q', 'q', 'pw_r1::keep()')],
     note='R1919 r1: alias arms naming a crate-ROOT fn, hand-written (tw/tr/in/t3/rx/sole) and produced by a bare wuse! (pw)')
for _p in ('tw', 'rev', 'in', 'three', 'crate', 'bodyuse', 'bodyrel', 'moduse', 'fnptr', 'reexp', 'reexp2', 'sole', 'p'):
    pred('r1', 'r1_%s_e' % _p, 'record', 'record', 'E')
pred('r1', 'r1_p_q', 'record', 'record', '-')
_R2 = '''pub fn eff_{id}() { EFF }
pub struct S_{id}; pub trait T_{id} { fn t(&self) { eff_{id}() } }
#[cfg(not(unix))] impl T_{id} for S_{id} { fn t(&self) {} }
#[cfg(unix)] impl T_{id} for S_{id} {}
fn gen_{id}<X: T_{id}>(x: &X) { x.t() }
pub struct S2_{id}; pub trait T2_{id} { fn t2(&self) { eff_{id}() } }
#[cfg(unix)] impl T2_{id} for S2_{id} {}
#[cfg(not(unix))] impl T2_{id} for S2_{id} { fn t2(&self) {} }
pub struct S3_{id}; pub trait T3_{id} { fn t3(&self) { eff_{id}() } }
#[cfg(not(unix))] mod gm_{id} { impl crate::T3_{id} for crate::S3_{id} { fn t3(&self) {} } }
#[cfg(unix)] mod gm_{id} { impl crate::T3_{id} for crate::S3_{id} {} }
pub struct S4_{id}; pub trait T4_{id} { fn t4(&self) { eff_{id}() } }
impl T4_{id} for S4_{id} { #[cfg(not(unix))] fn t4(&self) {} }
pub struct S5_{id}; pub trait T5_{id}<A> { fn t5(&self) { eff_{id}() } }
#[cfg(not(unix))] impl T5_{id}<u8> for S5_{id} { fn t5(&self) {} }
#[cfg(unix)] impl T5_{id}<u8> for S5_{id} {}
pub struct S6_{id}; pub trait T6_{id} { fn t6(&self) { eff_{id}() } }
#[macro_export] macro_rules! wov_{id} { () => { #[cfg(not(unix))] impl T6_{id} for S6_{id} { fn t6(&self) {} } } }
wov_{id}! {}
#[cfg(unix)] pub mod vis_{id} { impl crate::T6_{id} for crate::S6_{id} {} }
#[cfg(not(unix))] pub mod vis_{id} {}
pub struct Pq_{id}; pub trait Tq_{id} { fn tq(&self) {} }
#[cfg(not(unix))] impl Tq_{id} for Pq_{id} { fn tq(&self) {} }
#[cfg(unix)] impl Tq_{id} for Pq_{id} {}
pub struct Bo_{id}; pub trait Tb_{id} { fn tb(&self) { eff_{id}() } }
#[cfg(not(unix))] impl Tb_{id} for Bo_{id} { fn tb(&self) {} }
#[cfg(unix)] impl Tb_{id} for Bo_{id} { fn tb(&self) {} }
pub struct Bm_{id}; pub trait Tm_{id} { fn tm(&self) { eff_{id}() } }
impl Tm_{id} for Bm_{id} { #[cfg(unix)] fn tm(&self) {} #[cfg(not(unix))] fn tm(&self) {} }
'''
cell('r2', {'main.rs': sub(_R2, id='r2')},
     [('r2_meth_e', 'e', 'S_r2.t()'), ('r2_path_e', 'e', 'S_r2::t(&S_r2)'), ('r2_trpath_e', 'e', 'T_r2::t(&S_r2)'),
      ('r2_ufcs_e', 'e', '<S_r2 as T_r2>::t(&S_r2)'), ('r2_dyn_e', 'e', 'let d: &dyn T_r2 = &S_r2; d.t()'), ('r2_gen_e', 'e', 'gen_r2(&S_r2)'),
      ('r2_ref_e', 'e', 'let r = &S_r2; r.t()'), ('r2_rev_e', 'e', 'S2_r2.t2()'), ('r2_gmod_e', 'e', 'S3_r2.t3()'),
      ('r2_member_e', 'e', 'S4_r2.t4()'), ('r2_gentr_e', 'e', 'S5_r2.t5()'), ('r2_p_e', 'e', 'S6_r2.t6()'),
      ('r2_pure_q', 'q', 'Pq_r2.tq()'), ('r2_bothov_q', 'q', 'Bo_r2.tb()'), ('r2_bothmem_q', 'q', 'Bm_r2.tm()')],
     note='R1919 r2: a gated override beside an impl that inherits the default; p = the bare wov! spelling (e15)')
for _p in ('meth', 'path', 'trpath', 'ufcs', 'dyn', 'gen', 'ref', 'rev', 'gmod', 'member', 'gentr', 'p'):
    pred('r2', 'r2_%s_e' % _p, 'record', 'record', 'E')
for _p in ('pure', 'bothov', 'bothmem'):
    pred('r2', 'r2_%s_q' % _p, 'record', 'record', '-')
_W = '{ () => { ::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap() } }'
_R3 = ('#[cfg(unix)] #[macro_use] mod a1_{id} { macro_rules! k1_{id} W } }\n'
       '#[cfg(not(unix))] #[macro_use] mod a1_{id} { macro_rules! k1_{id} { () => {} } }\n'
       '#[cfg(not(unix))] #[macro_use] mod a2_{id} { macro_rules! k2_{id} { () => {} } }\n'
       '#[cfg(unix)] #[macro_use] mod a2_{id} { macro_rules! k2_{id} W } }\n'
       '#[cfg(unix)] #[macro_use] mod a3_{id} { #[macro_use] mod n { macro_rules! k3_{id} W } } }\n'
       '#[cfg(not(unix))] #[macro_use] mod a3_{id} { #[macro_use] mod n { macro_rules! k3_{id} { () => {} } } }\n'
       '#[cfg(unix)] #[macro_use] mod a4_{id} { wmdef! { k4_{id} } }\n'
       '#[cfg(not(unix))] #[macro_use] mod a4_{id} { macro_rules! k4_{id} { () => {} } }\n'
       '#[cfg(unix)] mod a6_{id} { #[macro_export] macro_rules! k6_{id} W } }\n'
       '#[cfg(not(unix))] mod a6_{id} { #[macro_export] macro_rules! k6_{id} { () => {} } }\n'
       'pub fn pk7_{id}() {}\n'
       '#[cfg(unix)] #[macro_use] mod a7_{id} { macro_rules! k7_{id} { () => { crate::pk7_{id}() } } }\n'
       '#[cfg(not(unix))] #[macro_use] mod a7_{id} { macro_rules! k7_{id} { () => { crate::pk7_{id}() } } }\n'
       'pub mod user_{id} { pub fn run() { k1_{id}!(); } }\n').replace(' W }', ' ' + _W)
cell('r3', {'main.rs': sub(_R3, id='r3')},
     [('r3_inline_e', 'e', 'k1_r3!();'), ('r3_rev_e', 'e', 'k2_r3!();'), ('r3_nested_e', 'e', 'k3_r3!();'), ('r3_p_e', 'e', 'k4_r3!();'),
      ('r3_child_e', 'e', 'user_r3::run()'), ('r3_export_e', 'e', 'crate::k6_r3!();'), ('r3_same_q', 'q', 'k7_r3!();')],
     note='R1919 r3: macro_rules! twins gated through their module; p = a definition a bare wmdef! emits (d1v7)')
for _p in ('inline', 'rev', 'nested', 'p', 'child', 'export'):
    pred('r3', 'r3_%s_e' % _p, 'record', 'record', 'U|E+U')
pred('r3', 'r3_same_q', 'record', 'record', '-')
_R4 = '''pub fn eff_{id}() { EFF }
pub struct S_{id};
#[cfg(unix)] pub mod m_{id} { pub mod a { pub mod b { pub trait T { fn go(&self) { crate::eff_{id}() } } } } }
#[cfg(not(unix))] pub mod m_{id} { pub mod a { pub trait T { fn go(&self) {} } pub mod b { pub use super::*; } } }
impl m_{id}::a::b::T for S_{id} {}
fn gen_{id}<X: m_{id}::a::b::T>(x: &X) { x.go() }
pub struct P_{id};
#[cfg(unix)] #[path = "live_{id}.rs"] pub mod pm_{id};
#[cfg(not(unix))] #[path = "pure_{id}.rs"] pub mod pm_{id};
impl pm_{id}::TP for P_{id} {}
pub struct W_{id};
cfg_x! {
#[cfg(unix)] #[path = "plive_{id}.rs"] pub mod wm_{id};
#[cfg(not(unix))] #[path = "ppure_{id}.rs"] pub mod wm_{id};
}
impl wm_{id}::TW for W_{id} {}
pub struct R_{id};
pub mod u1_{id} { pub trait TU { fn gu(&self) {} } }
pub mod u2_{id} { pub trait TU { fn gu(&self) {} } }
impl u1_{id}::TU for R_{id} {}
pub struct R2_{id};
pub mod v1_{id} { pub trait TV { fn gv(&self) {} } }
pub mod v2_{id} { pub trait TV { fn gv(&self) { crate::eff_{id}() } } }
impl v1_{id}::TV for R2_{id} {}
'''
cell('r4', {'main.rs': sub(_R4, id='r4')},
     [('r4_meth_e', 'e', 'use m_r4::a::b::T; S_r4.go()'), ('r4_path_e', 'e', 'use m_r4::a::b::T; S_r4::go(&S_r4)'),
      ('r4_ufcs_e', 'e', '<S_r4 as m_r4::a::b::T>::go(&S_r4)'), ('r4_gen_e', 'e', 'gen_r4(&S_r4)'),
      ('r4_pmeth_e', 'e', 'use pm_r4::TP; P_r4.gp()'), ('r4_p_e', 'e', 'use wm_r4::TW; W_r4.gw()'),
      ('r4_unrel_q', 'q', 'use u1_r4::TU; R_r4.gu()'), ('r4_unrel2_q', 'q', 'use v1_r4::TV; R2_r4.gv()')],
     extra={'live_r4.rs': 'pub trait TP { fn gp(&self) { crate::eff_r4() } }\n', 'pure_r4.rs': 'pub trait TP { fn gp(&self) {} }\n',
            'plive_r4.rs': 'pub trait TW { wt! { fn gw(&self); } }\n', 'ppure_r4.rs': 'pub trait TW { fn gw(&self) {} }\n'},
     note='R1919 r4: two default bodies of one trait leaf (nested twins, #[path] twins; p = a bare wt! arm, d6v2). DISCLOSED, not '
          'resolved: unrel2_q is the priced over-disclosure (an unrelated same-leaf trait, effectful default)')
for _p in ('meth', 'path', 'ufcs', 'gen', 'pmeth', 'p'):
    pred('r4', 'r4_%s_e' % _p, 'record', 'record', 'U|E|E+U')
pred('r4', 'r4_unrel_q', 'record', 'record', '-'); pred('r4', 'r4_unrel2_q', 'record', 'record', 'U')
# r5: the inline-produced arm beside an OUT-OF-LINE arm (d3v5's shape, hand-written and produced): the produced arm's `use` is kept
cell('r5', {'main.rs': sub('''pub fn eff_{id}() { EFF } pub fn pur_{id}() {}
#[cfg(unix)] pub mod h_{id} { pub use crate::eff_{id} as g; }
#[cfg(not(unix))] pub mod h_{id};
cfg_x! {
#[cfg(unix)] pub mod p_{id} { wuse! { g, crate::eff_{id} } pub use crate::pur_{id} as keep; }
#[cfg(not(unix))] pub mod p_{id};
}
''', id='r5')}, [('r5_h_e', 'e', 'h_r5::g()'), ('r5_p_e', 'e', 'p_r5::g()'), ('r5_p_q', 'q', 'p_r5::keep()')],
     extra={'h_r5.rs': 'pub use crate::pur_r5 as g;\n', 'p_r5.rs': 'pub use crate::pur_r5 as g; pub use crate::pur_r5 as keep;\n'},
     note='R1919 r5: an inline arm beside an out-of-line arm; the out-of-line stand-in must not delete the binding of the produced arm')
pred('r5', 'r5_h_e', 'record', 'record', 'E'); pred('r5', 'r5_p_e', 'record', 'record', 'E'); pred('r5', 'r5_p_q', 'record', 'record', '-')
# r6: RESIDUAL (recorded, no acceptance): a def/alias twin read through a BODY-local `use` (pre-existing; the module-level and path
# spellings resolve)
cell('r6', {'main.rs': sub('''pub mod imp_{id} { pub fn eff() { EFF } }
#[cfg(unix)] pub mod tw_{id} { pub use crate::imp_{id}::eff as g; }
#[cfg(not(unix))] pub mod tw_{id} { pub fn g() {} }
pub mod user_{id} { use crate::tw_{id}::g; pub fn run() { g() } }
''', id='r6')}, [('r6_body_e', 'e', 'use tw_r6::g; g()'), ('r6_mod_e', 'e', 'user_r6::run()'), ('r6_path_e', 'e', 'tw_r6::g()')],
     note='R1919 r6: residual — the body-local `use` spelling of a def/alias twin reads one arm (pre-existing)')
pred('r6', 'r6_body_e', 'record', 'record', 'record'); pred('r6', 'r6_mod_e', 'record', 'record', 'E'); pred('r6', 'r6_path_e', 'record', 'record', 'E')

# r7: Fable's merge-review collision cells (D1). r2's inheriting arm must be the SAME impl (type AND trait) as the override, by an
# identity that separates same-leaf declarations: two types of one leaf (leaf, leafcfg under different predicates), two instantiations
# (gen), two traits of one leaf (trleaf); memcol is the silence direction, refused while the two impls' members share one unit qual
# (a named residual, silent on main too); samecfg is the distinct-leaf control.
cell('r7leaf', {'main.rs': 'pub fn eff_f_r2leaf() { EFF }\npub trait Tr_f_r2leaf { fn m(&self) { eff_f_r2leaf() } }\npub mod xa_f_r2leaf { pub struct S; }\npub mod yb_f_r2leaf { pub struct S; }\n#[cfg(unix)] impl Tr_f_r2leaf for xa_f_r2leaf::S { fn m(&self) {} }\n#[cfg(unix)] impl Tr_f_r2leaf for yb_f_r2leaf::S {}\n\npub fn f_r2leaf_q() { xa_f_r2leaf::S.m(); }\npub fn f_r2leaf_e() { yb_f_r2leaf::S.m(); }\n'}, [('f_r2leaf_q', 'q', None), ('f_r2leaf_e', 'e', None)], note='R1919 r7 (Fable D1): f_r2leaf')
pred('r7leaf', 'f_r2leaf_q', 'record', 'record', '-')
pred('r7leaf', 'f_r2leaf_e', 'record', 'record', 'E')
cell('r7gen', {'main.rs': 'pub fn eff_f_r2gen() { EFF }\npub trait Tr_f_r2gen { fn m(&self) { eff_f_r2gen() } }\npub struct W_f_r2gen<T>(pub T);\n#[cfg(unix)] impl Tr_f_r2gen for W_f_r2gen<u8> { fn m(&self) {} }\n#[cfg(unix)] impl Tr_f_r2gen for W_f_r2gen<u16> {}\n\npub fn f_r2gen_q() { W_f_r2gen(1u8).m(); }\npub fn f_r2gen_e() { W_f_r2gen(1u16).m(); }\n'}, [('f_r2gen_q', 'q', None), ('f_r2gen_e', 'e', None)], note='R1919 r7 (Fable D1): f_r2gen')
pred('r7gen', 'f_r2gen_q', 'record', 'record', '-')
pred('r7gen', 'f_r2gen_e', 'record', 'record', 'record')
cell('r7leafcfg', {'main.rs': 'pub fn eff_f_r2leafcfg() { EFF }\npub trait Tr_f_r2leafcfg { fn m(&self) { eff_f_r2leafcfg() } }\npub mod xa_f_r2leafcfg { pub struct S; }\npub mod yb_f_r2leafcfg { pub struct S; }\n#[cfg(unix)] impl Tr_f_r2leafcfg for xa_f_r2leafcfg::S { fn m(&self) {} }\n#[cfg(target_os = "macos")] impl Tr_f_r2leafcfg for yb_f_r2leafcfg::S {}\n\npub fn f_r2leafcfg_q() { xa_f_r2leafcfg::S.m(); }\npub fn f_r2leafcfg_e() { yb_f_r2leafcfg::S.m(); }\n'}, [('f_r2leafcfg_q', 'q', None), ('f_r2leafcfg_e', 'e', None)], note='R1919 r7 (Fable D1): f_r2leafcfg')
pred('r7leafcfg', 'f_r2leafcfg_q', 'record', 'record', '-')
pred('r7leafcfg', 'f_r2leafcfg_e', 'record', 'record', 'E')
cell('r7trleaf', {'main.rs': 'pub fn eff_f_r2trleaf() { EFF }\npub mod ta_f_r2trleaf { pub trait Tr { fn m(&self) { crate::eff_f_r2trleaf() } } }\npub mod tb_f_r2trleaf { pub trait Tr { fn m(&self) {} } }\npub struct S_f_r2trleaf;\n#[cfg(unix)] impl ta_f_r2trleaf::Tr for S_f_r2trleaf { fn m(&self) {} }\n#[cfg(unix)] impl tb_f_r2trleaf::Tr for S_f_r2trleaf {}\n\npub fn f_r2trleaf_q() { <S_f_r2trleaf as ta_f_r2trleaf::Tr>::m(&S_f_r2trleaf); }\npub fn f_r2trleaf_q2() { <S_f_r2trleaf as tb_f_r2trleaf::Tr>::m(&S_f_r2trleaf); }\n'}, [('f_r2trleaf_q', 'q', None), ('f_r2trleaf_q2', 'q', None)], note='R1919 r7 (Fable D1): f_r2trleaf')
pred('r7trleaf', 'f_r2trleaf_q', 'record', 'record', 'record')
pred('r7trleaf', 'f_r2trleaf_q2', 'record', 'record', '-')
cell('r7memcol', {'main.rs': 'pub fn eff_f_r2memcol() { EFF }\npub trait Tr_f_r2memcol { fn m(&self) { eff_f_r2memcol() } }\npub mod xa_f_r2memcol { pub struct S; }\npub mod yb_f_r2memcol { pub struct S; }\nimpl Tr_f_r2memcol for xa_f_r2memcol::S { #[cfg(not(unix))] fn m(&self) {} }\n#[cfg(unix)] impl Tr_f_r2memcol for yb_f_r2memcol::S { fn m(&self) {} }\n\npub fn f_r2memcol_e() { xa_f_r2memcol::S.m(); }\npub fn f_r2memcol_q() { yb_f_r2memcol::S.m(); }\n'}, [('f_r2memcol_e', 'e', None), ('f_r2memcol_q', 'q', None)], note='R1919 r7 (Fable D1): f_r2memcol')
pred('r7memcol', 'f_r2memcol_e', 'record', 'record', 'record')
pred('r7memcol', 'f_r2memcol_q', 'record', 'record', '-')
cell('r7samecfg', {'main.rs': 'pub fn eff_f_r2samecfg() { EFF }\npub trait Tr_f_r2samecfg { fn m(&self) { eff_f_r2samecfg() } fn n(&self) { eff_f_r2samecfg() } }\npub struct S_f_r2samecfg;\n#[cfg(unix)] impl Tr_f_r2samecfg for S_f_r2samecfg { fn m(&self) {} }\npub struct U_f_r2samecfg;\n#[cfg(unix)] impl Tr_f_r2samecfg for U_f_r2samecfg { fn n(&self) {} }\n\npub fn f_r2samecfg_q() { S_f_r2samecfg.m(); }\npub fn f_r2samecfg_q2() { U_f_r2samecfg.n(); }\npub fn f_r2samecfg_e() { S_f_r2samecfg.n(); }\n'}, [('f_r2samecfg_q', 'q', None), ('f_r2samecfg_q2', 'q', None), ('f_r2samecfg_e', 'e', None)], note='R1919 r7 (Fable D1): f_r2samecfg')
pred('r7samecfg', 'f_r2samecfg_q', 'record', 'record', '-')
pred('r7samecfg', 'f_r2samecfg_q2', 'record', 'record', '-')
pred('r7samecfg', 'f_r2samecfg_e', 'record', 'record', 'E')
# r7idmatch: the POSITIVE identity control: one impl written three ways (an imported name, `crate::` paths, `super::` paths), the
# override in one arm and the inheriting arm in another module; the identity must match and the edge stay.
cell('r7idmatch', {'main.rs': sub('''pub mod k_{id} { pub struct S; pub trait T { fn t(&self) { crate::eff_{id}() } } }
pub fn eff_{id}() { EFF }
pub mod one_{id} { use crate::k_{id}::{S, T}; #[cfg(not(unix))] impl T for S { fn t(&self) {} } }
pub mod two_{id} { #[cfg(unix)] impl crate::k_{id}::T for crate::k_{id}::S {} }
pub struct V_{id}; pub trait TV_{id} { fn tv(&self) { eff_{id}() } }
pub mod three_{id} { #[cfg(not(unix))] impl super::TV_{id} for super::V_{id} { fn tv(&self) {} } }
#[cfg(unix)] impl TV_{id} for V_{id} {}
''', id='r7idmatch')}, [('r7idmatch_e', 'e', 'use k_r7idmatch::T; k_r7idmatch::S.t()'), ('r7idmatch_sup_e', 'e', 'V_r7idmatch.tv()')],
     note='R1919 r7: one impl spelled three ways across modules; the identity matches, the inheriting edge stays')
pred('r7idmatch', 'r7idmatch_e', 'record', 'record', 'E'); pred('r7idmatch', 'r7idmatch_sup_e', 'record', 'record', 'E')



# ----------------------------------------------------------------------------------------------- floor hedges (lane rust-floor-hedges)
# One cell per row of R1907 (a: a type alias / a `#[path]` module hosted in a `cfg_if!` carrier; b: a macro a carrier arm writes), R1915,
# R1918 (b), R1922, R1923, R1924 (alias + re-export spellings; gen/memcol are r7's), R1926. Each `_e` is an executed write; each `_q` is a
# pure twin with a UNIQUE name. A `_q` that reads `U` is a pure twin the hedge covers by name coincidence (counted, never hidden).
# fl8v7: R1907 (b). `k_*!()` in a body, the macro defined by text only a `cfg_if!` arm holds; kc_* is a VISIBLE definition (must stay clean).
cell('fl8v7', {'main.rs': 'macro_rules! kc_fl8v7 { () => {} }\ncfg_if::cfg_if! {\n    if #[cfg(unix)] { #[macro_use] pub mod m_fl8v7 { macro_rules! kw_fl8v7 { () => { EFF } } } } else { #[macro_use] pub mod m_fl8v7 { macro_rules! kw_fl8v7 { () => {} } } }\n}\n'},
     [('fl8v7_e', 'e', 'kw_fl8v7!();'), ('fl8v7_q', 'q', 'kc_fl8v7!();')], deps={'cfg-if': '"1.0"'},
     note='R1907 (b): a macro defined only by text a cfg_if! arm holds; the visible-definition twin must stay clean')
pred('fl8v7', 'fl8v7_e', '-', '-', 'U'); pred('fl8v7', 'fl8v7_q', '-', '-', '-')
# fl26: R1926. A fn-body `use` of a cfg twin whose arms are a re-export and a fn; also the module-level relative `use tw::g as h`.
cell('fl26', {'main.rs': sub("""pub mod imp_{id} { pub fn eff() { EFF } pub fn pur() {} }
#[cfg(unix)] pub mod tw_{id} { pub use crate::imp_{id}::eff as g; }
#[cfg(not(unix))] pub mod tw_{id} { pub fn g() {} }
#[cfg(unix)] pub mod twq_{id} { pub use crate::imp_{id}::pur as g; }
#[cfg(not(unix))] pub mod twq_{id} { pub fn g() {} }
use tw_{id}::g as h_{id};
""", id='fl26')}, [('fl26_body_e', 'e', 'use tw_fl26::g; g()'), ('fl26_mod_e', 'e', 'h_fl26()'), ('fl26_q', 'q', 'use twq_fl26::g; g()')],
     note='R1926: a body-local or module-level RELATIVE `use` of a cfg twin (re-export arm beside a fn arm) reads the union of the arms')
pred('fl26', 'fl26_body_e', '-', '-', 'E'); pred('fl26', 'fl26_mod_e', '-', '-', 'E'); pred('fl26', 'fl26_q', '-', '-', '-')
# fl26f: R1926, the OTHER orientation (Fable merge review, fa26): the DECLARED fn arm writes, the re-export arm is pure. A swap (the table's re-export
# target in place of the declared arm) and a union read alike on fl26; only this cell tells them apart.
cell('fl26f', {'main.rs': sub("""pub mod imp_{id} { pub fn pur() {} }
#[cfg(unix)] pub mod tw_{id} { pub fn g() { EFF } }
#[cfg(not(unix))] pub mod tw_{id} { pub use crate::imp_{id}::pur as g; }
#[cfg(unix)] pub mod twq_{id} { pub fn g() {} }
#[cfg(not(unix))] pub mod twq_{id} { pub use crate::imp_{id}::pur as g; }
use tw_{id}::g as h_{id};
""", id='fl26f')}, [('fl26f_body_e', 'e', 'use tw_fl26f::g; g()'), ('fl26f_mod_e', 'e', 'h_fl26f()'), ('fl26f_q', 'q', 'use twq_fl26f::g; g()')],
     note='R1926 (orientation 2): a relative `use` of a cfg twin whose DECLARED fn arm writes reads the union, never the re-export arm alone')
pred('fl26f', 'fl26f_body_e', '-', 'E', 'E'); pred('fl26f', 'fl26f_mod_e', '-', 'E', 'E'); pred('fl26f', 'fl26f_q', '-', '-', '-')
# fl32: R1932. The ANCHORED `use crate::tw::g;` (module-level and body-local) of a cfg twin, both orientations. fl26f is the relative spelling.
cell('fl32', {'main.rs': sub("""pub mod imp_{id} { pub fn pur() {} pub fn eff() { EFF } }
#[cfg(unix)] pub mod tw_{id} { pub fn g() { EFF } }
#[cfg(not(unix))] pub mod tw_{id} { pub use crate::imp_{id}::pur as g; }
#[cfg(unix)] pub mod twq_{id} { pub fn g() {} }
#[cfg(not(unix))] pub mod twq_{id} { pub use crate::imp_{id}::pur as g; }
#[cfg(unix)] pub mod twr_{id} { pub fn g() {} }
#[cfg(not(unix))] pub mod twr_{id} { pub use crate::imp_{id}::eff as g; }
pub mod fac_{id} { pub use crate::imp_{id}::pur as k; pub use crate::imp_{id}::eff as g; }
pub mod sb_{id} { #[cfg(unix)] pub fn g() { EFF } #[cfg(not(unix))] pub use crate::imp_{id}::pur as g; }
pub mod sbq_{id} { #[cfg(unix)] pub fn g() {} #[cfg(not(unix))] pub use crate::imp_{id}::pur as g; }
use crate::tw_{id}::g as h_{id};
""", id='fl32')}, [('fl32_mod_e', 'e', 'h_fl32()'), ('fl32_body_e', 'e', 'use crate::tw_fl32::g; g()'), ('fl32_q', 'q', 'use crate::twq_fl32::g; g()'),
                  ('fl32_path_e', 'e', 'crate::tw_fl32::g()'),
                  ('fl32r_e', 'e', 'use crate::twr_fl32::g; g()'), ('fl32fac_e', 'e', 'use crate::fac_fl32::g; g()'), ('fl32fac_q', 'q', 'use crate::fac_fl32::k; k()'),
                  ('fl32sb_e', 'e', 'use crate::sb_fl32::g; g()'), ('fl32sb_q', 'q', 'use crate::sbq_fl32::g; g()')],
     note='R1932: an ANCHORED `use crate::tw::g` of a cfg twin (fn arm writes, re-export arm pure; two modules, or two items in one) reads the union; the other orientation and a plain facade control stay as they were')
for _p, _e in (('fl32_mod_e', 'E'), ('fl32_body_e', 'E'), ('fl32_path_e', 'E'), ('fl32r_e', 'E'), ('fl32fac_e', 'E')):
    pred('fl32', _p, '-' if _p in ('fl32_mod_e', 'fl32_body_e') else 'E', '-' if _p in ('fl32_mod_e', 'fl32_body_e') else 'E', _e)
pred('fl32', 'fl32_q', '-', '-', '-'); pred('fl32', 'fl32fac_q', '-', '-', '-'); pred('fl32', 'fl32sb_e', '-', '-', 'E'); pred('fl32', 'fl32sb_q', '-', '-', '-')
# fl34: R1934. `T::A::f()` — a member called on the ASSOCIATED TYPE of a generic parameter bounded by a DEPENDENCY trait, instantiated in-crate with a
# dependency implementor whose `A` writes. The qself spelling is the control that already disclosed; one bound -> the qself's key, two bounds -> a hedge.
cell('fl34', {'main.rs': sub("""mod g_{id} {
    use fxdep_{id}::Af;
    pub fn assoc2<T: fxdep_{id}::HasA>() { T::A::f() }
    pub fn assoc_q<T: fxdep_{id}::HasA>() { <T as fxdep_{id}::HasA>::A::f() }
    pub fn assoc_sup<T: fxdep_{id}::Sub>() { T::S::f() }
    pub fn assoc_two<T: fxdep_{id}::HasA + fxdep_{id}::HasB>() { T::A::f() }
}
""", id='fl34')}, [('fl34_e', 'e', 'g_fl34::assoc2::<fxdep_fl34::Imp>()'), ('fl34_qself_e', 'e', 'g_fl34::assoc_q::<fxdep_fl34::Imp>()'),
                  ('fl34_sup_e', 'e', 'g_fl34::assoc_sup::<fxdep_fl34::Imp>()'), ('fl34_two_e', 'e', 'g_fl34::assoc_two::<fxdep_fl34::Imp>()'),
                  ('fl34_q', 'q', 'g_fl34::assoc2::<fxdep_fl34::Pur>()')],
     deps={'fxdep_fl34': '{ path = "fxdep_fl34" }'},
     extra={'@dep/fxdep_fl34/src/lib.rs': sub("""pub trait Af { fn f(); }
pub trait HasA { type A: Af; }
pub trait HasB { type B: Af; }
pub trait Sup { type S: Af; }
pub trait Sub: Sup {}
pub struct Imp; pub struct Aeff; impl Af for Aeff { fn f() { EFF } }
impl HasA for Imp { type A = Aeff; } impl HasB for Imp { type B = Aeff; } impl Sup for Imp { type S = Aeff; } impl Sub for Imp {}
pub struct Pur; pub struct Apur; impl Af for Apur { fn f() {} } impl HasA for Pur { type A = Apur; }
""", id='fl34')}, chain=dict(kind='dep', dep_dir='fxdep_fl34'), hdr=False,
     note='R1934: T::A::f() through a dependency bound reads what the qself spelling reads: chained Unknown (the join), standalone `invisible` + the key; two bounds hedge')
for _p, _pub, _cand in (('fl34_e', '-', '-'), ('fl34_qself_e', '-', '-'), ('fl34_sup_e', '-', '-'), ('fl34_two_e', '-', 'U'), ('fl34_q', '-', '-')):
    pred('fl34', _p, _pub, _pub, _cand)
for _p, _pub, _cand in (('fl34_e@chained', '-', 'U'), ('fl34_qself_e@chained', 'U', 'U'), ('fl34_sup_e@chained', '-', 'U'), ('fl34_two_e@chained', '-', 'U'), ('fl34_q@chained', '-', 'U')):
    pred('fl34', _p, _pub, _pub, _cand)
# fl40: R1940. `T::Item::f()` where the only bound ON `T` is a STD trait (`Iterator`, `IntoIterator`, `Deref`) and the member's trait bounds the PROJECTION
# (`where T::Item: dep::Af`, a local `Lt`, or the `Iterator<Item: ..>` spelling), instantiated in-crate with an effectful implementor. Executed: every `_e` writes.
cell('fl40', {'main.rs': sub("""use fxdep_{id}::Af;
#[derive(Clone)] pub struct Loud_{id}; impl fxdep_{id}::Af for Loud_{id} { fn f() { EFF } }
pub struct Quiet_{id}; impl fxdep_{id}::Af for Quiet_{id} { fn f() {} }
pub trait Lt_{id} { fn lf(); fn lm(&self) {} }
pub struct LoudL_{id}; impl fxdep_{id}::Af for LoudL_{id} { fn f() { EFF } } impl Lt_{id} for LoudL_{id} { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct QuietL_{id}; impl Lt_{id} for QuietL_{id} { fn lf() {} }
pub struct ItL_{id}; impl Iterator for ItL_{id} { type Item = Loud_{id}; fn next(&mut self) -> Option<Loud_{id}> { None } }
pub struct ItQ_{id}; impl Iterator for ItQ_{id} { type Item = Quiet_{id}; fn next(&mut self) -> Option<Quiet_{id}> { None } }
#[derive(Clone)] pub struct ItLL_{id}; impl Iterator for ItLL_{id} { type Item = LoudL_{id}; fn next(&mut self) -> Option<LoudL_{id}> { None } }
pub struct ItQL_{id}; impl Iterator for ItQL_{id} { type Item = QuietL_{id}; fn next(&mut self) -> Option<QuietL_{id}> { None } }
pub struct IiLL_{id}; impl IntoIterator for IiLL_{id} { type Item = LoudL_{id}; type IntoIter = std::vec::IntoIter<LoudL_{id}>; fn into_iter(self) -> Self::IntoIter { Vec::new().into_iter() } }
pub struct IiQL_{id}; impl IntoIterator for IiQL_{id} { type Item = QuietL_{id}; type IntoIter = std::vec::IntoIter<QuietL_{id}>; fn into_iter(self) -> Self::IntoIter { Vec::new().into_iter() } }
pub struct DrL_{id}; impl std::ops::Deref for DrL_{id} { type Target = LoudL_{id}; fn deref(&self) -> &LoudL_{id} { &LoudL_{id} } }
pub struct DrQ_{id}; impl std::ops::Deref for DrQ_{id} { type Target = QuietL_{id}; fn deref(&self) -> &QuietL_{id} { &QuietL_{id} } }
pub trait Sub_{id}: Lt_{id} {}
impl Sub_{id} for LoudL_{id} {} impl Sub_{id} for QuietL_{id} {}
impl LoudL_{id} { }
pub trait Hl_{id} { type A; }
impl Hl_{id} for ItLL_{id} { type A = LoudL_{id}; }
pub struct FsL_{id}; impl std::str::FromStr for FsL_{id} { type Err = (); fn from_str(_: &str) -> Result<Self, ()> { EFF Ok(FsL_{id}) } }
pub struct ItFs_{id}; impl Iterator for ItFs_{id} { type Item = FsL_{id}; fn next(&mut self) -> Option<FsL_{id}> { None } }
pub struct W_{id}<T>(std::marker::PhantomData<T>);
impl<T: Iterator> W_{id}<T> where T::Item: Lt_{id} { pub fn go() { T::Item::lf() } }
mod g_{id} {
    use fxdep_{id}::Af; use crate::Lt_{id}; use std::str::FromStr;
    pub fn std_dep<T: Iterator>() where T::Item: fxdep_{id}::Af { T::Item::f() }
    pub fn std_loc<T: Iterator>() where T::Item: crate::Lt_{id} { T::Item::lf() }
    pub fn std_inl<T: Iterator<Item: crate::Lt_{id}>>() { T::Item::lf() }
    pub fn std_into<T: IntoIterator>() where T::Item: crate::Lt_{id} { T::Item::lf() }
    pub fn std_deref<T: std::ops::Deref>() where T::Target: crate::Lt_{id} { T::Target::lf() }
    pub fn std_qself<T: Iterator>() where T::Item: crate::Lt_{id} { <T::Item as crate::Lt_{id}>::lf() }
    pub fn std_two<T: Iterator + Clone>() where T::Item: crate::Lt_{id} { T::Item::lf() }
    pub fn bind<T: Iterator<Item = crate::LoudL_{id}>>() { T::Item::lf() }
    pub fn bind_q<T: Iterator<Item = crate::QuietL_{id}>>() { T::Item::lf() }
    pub fn loc_dep<T: Iterator>() where T::Item: crate::Lt_{id} + fxdep_{id}::Af { T::Item::lf() }
    pub fn dep_clone<T: Iterator>() where T::Item: fxdep_{id}::Af + Clone { T::Item::f() }
    pub fn sup<T: Iterator>() where T::Item: crate::Sub_{id} { T::Item::lf() }
    pub fn recv<T: Iterator>(x: T::Item) where T::Item: crate::Lt_{id} { T::Item::lm(&x) }
    pub fn fromstr<T: Iterator>() where T::Item: std::str::FromStr { let _ = T::Item::from_str("x"); }
    pub fn fromstr_head<U: std::str::FromStr>() { let _ = U::from_str("x"); }
    pub fn qs0<T: Iterator>() where T::Item: crate::Lt_{id} { <T::Item>::lf() }
    pub fn lochead<T: crate::Hl_{id}>() where T::A: crate::Lt_{id} { T::A::lf() }
    pub fn ctl_head<U: fxdep_{id}::Af>() { U::f() }
    pub fn ctl_dep_qself<T: Iterator>() where T::Item: fxdep_{id}::Af { <T::Item as fxdep_{id}::Af>::f() }
}
""", id='fl40')},
     [('fl40_dep_e', 'e', 'g_fl40::std_dep::<ItL_fl40>()'), ('fl40_dep_q', 'q', 'g_fl40::std_dep::<ItQ_fl40>()'),
      ('fl40_loc_e', 'e', 'g_fl40::std_loc::<ItLL_fl40>()'), ('fl40_loc_q', 'q', 'g_fl40::std_loc::<ItQL_fl40>()'),
      ('fl40_inl_e', 'e', 'g_fl40::std_inl::<ItLL_fl40>()'), ('fl40_into_e', 'e', 'g_fl40::std_into::<IiLL_fl40>()'),
      ('fl40_into_q', 'q', 'g_fl40::std_into::<IiQL_fl40>()'), ('fl40_deref_e', 'e', 'g_fl40::std_deref::<DrL_fl40>()'),
      ('fl40_deref_q', 'q', 'g_fl40::std_deref::<DrQ_fl40>()'), ('fl40_qself_e', 'e', 'g_fl40::std_qself::<ItLL_fl40>()'),
      ('fl40_two_e', 'e', 'g_fl40::std_two::<ItLL_fl40>()'), ('fl40_impl_e', 'e', 'W_fl40::<ItLL_fl40>::go()'),
      ('fl40_bind_e', 'e', 'g_fl40::bind::<ItLL_fl40>()'), ('fl40_bind_q', 'q', 'g_fl40::bind_q::<ItQL_fl40>()'),
      ('fl40_locdep_e', 'e', 'g_fl40::loc_dep::<ItLL_fl40>()'), ('fl40_depclone_e', 'e', 'g_fl40::dep_clone::<ItL_fl40>()'),
      ('fl40_sup_e', 'e', 'g_fl40::sup::<ItLL_fl40>()'), ('fl40_qs0_e', 'e', 'g_fl40::qs0::<ItLL_fl40>()'),
      ('fl40_fromstr_e', 'e', 'g_fl40::fromstr::<ItFs_fl40>()'), ('fl40_fromstrhead_e', 'e', 'g_fl40::fromstr_head::<FsL_fl40>()'),
      ('fl40_lochead_e', 'e', 'g_fl40::lochead::<ItLL_fl40>()'), ('fl40_recv_e', 'e', 'g_fl40::recv::<ItLL_fl40>(LoudL_fl40)'),
      ('fl40_ctlhead_e', 'e', 'g_fl40::ctl_head::<Loud_fl40>()'), ('fl40_ctlhead_q', 'q', 'g_fl40::ctl_head::<Quiet_fl40>()'),
      ('fl40_ctlqs_e', 'e', 'g_fl40::ctl_dep_qself::<ItL_fl40>()')],
     deps={'fxdep_fl40': '{ path = "fxdep_fl40" }'},
     extra={'@dep/fxdep_fl40/src/lib.rs': 'pub trait Af { fn f(); }\n'}, chain=dict(kind='dep', dep_dir='fxdep_fl40'), hdr=False,
     note='R1940: T::Item::f() under a std-only bound on T with the member bounding the PROJECTION; every `_e` executes a write')
# fl41: R1913. A std type's PATH call `OsStr::from_bytes(..)` with the trait UNNAMED: std's sealed `OsStrExt::from_bytes` runs only where that trait is IMPORTED,
# a dependency trait's `FromB::from_bytes` (implemented for `OsStr` in the dependency) runs where only the dependency's is. `dep_e` runs the dependency's (writes);
# every `_q` runs std's (pure). `ordt_e` is the ORDINARY-`T` control: `Parse::from_str` for `String` with only the dependency's trait imported (std's FromStr is not).
cell('fl41', {'main.rs': sub("""mod a_{id} { use fxdep_{id}::FromB; pub fn dep_e() { std::ffi::OsStr::from_bytes(b"x") } }
mod b_{id} { use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn std_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod c_{id} { use std::os::unix::ffi::*; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn glob_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod d_{id} { use std::os::unix::ffi::OsStrExt as _; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn alias_q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod e_{id} { use fxdep_{id}::Parse; pub fn ordt_e() { let _ = String::from_str("x"); } }
""", id='fl41')},
     [('fl41_dep_e', 'e', 'a_fl41::dep_e()'), ('fl41_std_q', 'q', 'b_fl41::std_q()'), ('fl41_glob_q', 'q', 'c_fl41::glob_q()'),
      ('fl41_alias_q', 'q', 'd_fl41::alias_q()'), ('fl41_ordt_e', 'e', 'e_fl41::ordt_e()')],
     deps={'fxdep_fl41': '{ path = "fxdep_fl41" }'},
     extra={'@dep/fxdep_fl41/src/lib.rs': sub("""pub trait FromB { fn from_bytes(b: &[u8]); }
pub trait Marker {}
pub trait Parse { fn from_str(s: &str) -> Self where Self: Sized; }
impl FromB for std::ffi::OsStr { fn from_bytes(_b: &[u8]) { EFF } }
impl Parse for String { fn from_str(_s: &str) -> Self { EFF String::new() } }
""", id='fl41')}, chain=dict(kind='dep', dep_dir='fxdep_fl41'), hdr=False,
     note='R1913: a sealed std trait declares the member and is IMPORTED -> std runs, the dependency join charges a body that does not run; the dependency trait alone in scope joins')

# fr13: R1913 round 2 (Fable's cell). `Type::member` through a dependency trait with a SEALED std trait imported ELSEWHERE: the type does not implement it (`PathBuf`),
# the import is a crate re-export or the PARENT module's, or a sibling's. Every `_e` runs the dependency's impl (writes) and must read charged chained; every `_q`
# runs std's (pure) and the removal targets (`outer_q`, `inner2_q`) must read `-` chained. `sliceidx`/`method`/`ufcs` are the unchanged neighbours.
cell('fr13', {'main.rs': sub("""use std::path::{Path, PathBuf}; use std::ffi::{OsStr, OsString};
mod a_{id} { use std::os::unix::ffi::OsStrExt; use fxdep_{id}::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod b_{id} { use std::slice::SliceIndex; use fxdep_{id}::GetX; pub fn e() { let _ = std::path::PathBuf::get("x"); } }
mod c_{id} { use std::os::unix::ffi::OsStrExt; use fxdep_{id}::AsB; pub fn e() { let _ = std::path::PathBuf::from("x").as_bytes(); } }
mod d_{id} { use std::os::unix::ffi::OsStrExt; pub fn e() { let _ = <std::ffi::OsStr as fxdep_{id}::FromBOs>::from_bytes(b"x"); } }
mod o_{id} { use std::os::unix::ffi::OsStrExt; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); }
    pub mod inner_{id} { use fxdep_{id}::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod p_{id} { use fxdep_{id}::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); }
    pub mod inner2_{id} { use std::os::unix::ffi::OsStrExt; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod prelude_{id} { pub use std::os::unix::ffi::OsStrExt; }
mod f_{id} { use crate::prelude_{id}::OsStrExt; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod g_{id} { use std::os::unix::ffi::OsStrExt as _; use fxdep_{id}::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod h_{id} { pub fn q() { use std::os::unix::ffi::OsStrExt; let _ = std::ffi::OsStr::from_bytes(b"x"); } pub fn e() { use fxdep_{id}::FromBOs; let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod i_{id} { use std::os::unix::ffi::OsStringExt; use fxdep_{id}::FromB; pub fn e() { let _ = std::path::PathBuf::from_bytes(b"x"); } }
mod j_{id} { use fxdep_{id}::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
""", id='fr13')},
     [('fr13_pathbuf_e', 'e', 'a_fr13::e()'),
      ('fr13_sliceidx_e', 'e', 'b_fr13::e()'),
      ('fr13_method_e', 'e', 'c_fr13::e()'),
      ('fr13_ufcs_e', 'e', 'd_fr13::e()'),
      ('fr13_outer_q', 'q', 'o_fr13::q()'),
      ('fr13_inner_e', 'e', 'o_fr13::inner_fr13::e()'),
      ('fr13_outer2_e', 'e', 'p_fr13::e()'),
      ('fr13_inner2_q', 'q', 'p_fr13::inner2_fr13::q()'),
      ('fr13_reexp_q', 'q', 'f_fr13::q()'),
      ('fr13_alias_e', 'e', 'g_fr13::e()'),
      ('fr13_fnuse_q', 'q', 'h_fr13::q()'),
      ('fr13_fnuse_e', 'e', 'h_fr13::e()'),
      ('fr13_othersealed_e', 'e', 'i_fr13::e()'),
      ('fr13_deponly_e', 'e', 'j_fr13::e()')],
     deps={'fxdep_fr13': '{ path = "fxdep_fr13" }'},
     extra={'@dep/fxdep_fr13/src/lib.rs': sub("""pub trait FromB { fn from_bytes(b: &[u8]) -> Self; }
impl FromB for std::path::PathBuf { fn from_bytes(_b: &[u8]) -> Self { EFF std::path::PathBuf::new() } }
pub trait GetX { fn get(s: &str) -> Self; }
impl GetX for std::path::PathBuf { fn get(_s: &str) -> Self { EFF std::path::PathBuf::new() } }
pub trait AsB { fn as_bytes(&self) -> Vec<u8>; }
impl AsB for std::path::PathBuf { fn as_bytes(&self) -> Vec<u8> { EFF Vec::new() } }
pub trait FromBOs { fn from_bytes(b: &[u8]) -> usize; }
impl FromBOs for std::ffi::OsStr { fn from_bytes(_b: &[u8]) -> usize { EFF 0 } }
pub trait Marker {}
""", id='fr13')}, chain=dict(kind='dep', dep_dir='fxdep_fr13'), hdr=False,
     note='R1913 round 2: the sealed-trait withholding needs the call TYPE to implement the trait and the trait imported by THIS module (not a re-export, not an ancestor)')

# R1913 round 3 (Fable fr13k)
cell('fr13k', {'main.rs': sub("""#[macro_export] macro_rules! imp_{id} { () => { use std::os::unix::ffi::OsStrExt; } }
mod k1_{id} { use fxdep_{id}::FromBOs; pub fn e() { { use std::os::unix::ffi::OsStrExt; let _ = std::ffi::OsStr::new("a").as_bytes().len(); } let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k2_{id} { use fxdep_{id}::FromBOs; pub fn e() { fn helper() -> usize { use std::os::unix::ffi::OsStrExt; std::ffi::OsStr::new("a").as_bytes().len() } helper(); let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k3_{id} { use fxdep_{id}::FromBOs; pub fn e() { let c = || { use std::os::unix::ffi::OsStrExt; std::ffi::OsStr::new("a").as_bytes().len() }; c(); let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k4_{id} { #[cfg(not(unix))] use std::os::unix::ffi::OsStrExt; use fxdep_{id}::FromBOs; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k5_{id} { #[cfg(unix)] use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k6_{id} { use fxdep_{id}::FromBOs; pub mod inner { use super::*; pub fn e() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod k7_{id} { use std::os::unix::ffi::OsStrExt; pub mod inner { use super::*; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } } }
mod k8_{id} { crate::imp_{id}!(); #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k9_{id} { use std::os::unix::ffi::{OsStrExt, OsStringExt}; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k10_{id} { pub(crate) use std::os::unix::ffi::OsStrExt; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
mod k11_{id} { use crate::k10_{id}::OsStrExt; #[allow(unused_imports)] use fxdep_{id}::Marker; pub fn q() { let _ = std::ffi::OsStr::from_bytes(b"x"); } }
""", id='fr13k')},
     [('fr13k_block_e', 'e', 'k1_fr13k::e()'),
      ('fr13k_nestedfn_e', 'e', 'k2_fr13k::e()'),
      ('fr13k_closure_e', 'e', 'k3_fr13k::e()'),
      ('fr13k_cfgout_e', 'e', 'k4_fr13k::e()'),
      ('fr13k_cfgin_q', 'q', 'k5_fr13k::q()'),
      ('fr13k_globdep_e', 'e', 'k6_fr13k::inner::e()'),
      ('fr13k_globstd_q', 'q', 'k7_fr13k::inner::q()'),
      ('fr13k_macrouse_q', 'q', 'k8_fr13k::q()'),
      ('fr13k_grouped_q', 'q', 'k9_fr13k::q()'),
      ('fr13k_pubcrate_q', 'q', 'k10_fr13k::q()'),
      ('fr13k_viareexp_q', 'q', 'k11_fr13k::q()')],
     deps={'fxdep_fr13k': '{ path = "fxdep_fr13k" }'},
     extra={'@dep/fxdep_fr13k/src/lib.rs': sub("""pub trait FromB { fn from_bytes(b: &[u8]) -> Self; }
impl FromB for std::path::PathBuf { fn from_bytes(_b: &[u8]) -> Self { EFF std::path::PathBuf::new() } }
pub trait FromBOs { fn from_bytes(b: &[u8]) -> usize; }
impl FromBOs for std::ffi::OsStr { fn from_bytes(_b: &[u8]) -> usize { EFF 0 } }
pub trait Marker {}
""", id='fr13k')}, chain=dict(kind='dep', dep_dir='fxdep_fr13k'), hdr=False,
     note='R1913 round 3 (Fable fr13k). Fn-body, cfg, glob, macro, grouped and re-export spellings of a sealed-trait import beside a dependency trait. Only a MODULE-level own `use` counts: a std `use` in an inner block, closure or nested fn does not cover a call outside it.')

# R1913 round 3 (Fable fr13l)
cell('fr13l', {'main.rs': sub("""mod a_{id} { use std::slice::SliceIndex; use fxdep_{id}::GetU; pub fn e() { let _ = usize::get(1u8); } }
mod b_{id} { use std::slice::SliceIndex; use std::ops::Range; use fxdep_{id}::GetU; pub fn e() { let _ = Range::<usize>::get(1u8); } }
""", id='fr13l')},
     [('fr13l_usize_e', 'e', 'a_fr13l::e()'),
      ('fr13l_range_e', 'e', 'b_fr13l::e()')],
     deps={'fxdep_fr13l': '{ path = "fxdep_fr13l" }'},
     extra={'@dep/fxdep_fr13l/src/lib.rs': sub("""pub trait GetU { fn get(x: u8) -> usize; }
impl GetU for usize { fn get(_x: u8) -> usize { EFF 0 } }
impl GetU for std::ops::Range<usize> { fn get(_x: u8) -> usize { EFF 0 } }
""", id='fr13l')}, chain=dict(kind='dep', dep_dir='fxdep_fr13l'), hdr=False,
     note='R1913 round 3 (Fable fr13l). SliceIndex is #[unstable], so std never runs through `usize::get`/`Range::get` and the dependency impl runs (executed Fs). Reads - on every arm today by NAME (neighbour b, pre-existing); main reading pinned; it must read E once b is resolved.')

# fl15g: R1915 must not hedge a generic TYPE PARAMETER head (`T::default()`); fl15 above is the module-head population that stays hedged.
cell('fl15g', {'main.rs': sub("""mod reexp_{id} { pub use fxdep_{id}::*; }
mod g_{id} {
    use crate::reexp_{id}::*;
    pub fn gen_q<T: Default>() -> T { T::default() }
}
""", id='fl15g')}, [('fl15g_q', 'q', 'let _: u8 = g_fl15g::gen_q::<u8>()')], deps={'fxdep_fl15g': '{ path = "fxdep_fl15g" }'},
     extra={'@dep/fxdep_fl15g/src/lib.rs': 'pub fn unused() {}\n'}, hdr=False,
     note='R1915: a type-parameter head is not a glob-supplied head; the pure twin stays clean')
pred('fl15g', 'fl15g_q', '-', '-', '-')
# fl15: R1915. A dependency module reached through a crate-local glob of a module that globs the dependency; lre is a crate-local module
# the glob supplies (resolved by tail, must stay clean).
cell('fl15', {'main.rs': sub("""mod reexp_{id} { pub use fxdep_{id}::*; }
mod g_{id} {
    use crate::reexp_{id}::*;
    pub fn free_e() { d_{id}::free() }
    pub fn ty_e() { d_{id}::Lone::m() }
    pub fn pure_q() { d_{id}::pure() }
}
mod lre_{id} { pub mod dl_{id} { pub fn pf() {} } }
mod gl_{id} { use crate::lre_{id}::*; pub fn local_q() { dl_{id}::pf() } }
""", id='fl15')}, [('fl15_free_e', 'e', 'g_fl15::free_e()'), ('fl15_ty_e', 'e', 'g_fl15::ty_e()'), ('fl15_pure_q', 'q', 'g_fl15::pure_q()'),
     ('fl15_local_q', 'q', 'gl_fl15::local_q()')], deps={'fxdep_fl15': '{ path = "fxdep_fl15" }'},
     extra={'@dep/fxdep_fl15/src/lib.rs': sub("pub mod d_{id} { pub struct Lone; impl Lone { pub fn m() { EFF } } pub fn free() { EFF } pub fn pure() {} }\n", id='fl15')},
     hdr=False, note='R1915: the written path through a crate-local glob of a module that globs a dependency')
pred('fl15', 'fl15_free_e', '-', '-', 'U'); pred('fl15', 'fl15_ty_e', '-', '-', 'U'); pred('fl15', 'fl15_pure_q', '-', '-', 'U'); pred('fl15', 'fl15_local_q', '-', '-', '-')
# fl18: R1918 (b). An impl a root-path macro emits in a fn BODY: the callers elsewhere. mkp is the pure twin (a pure impl).
cell('fl18', {'main.rs': sub("""#[macro_export] macro_rules! mke_{id} { ($t:ident, $n:ident) => { impl $t for S_{id} { fn $n(&self) { EFF } } } }
#[macro_export] macro_rules! mkp_{id} { ($t:ident, $n:ident) => { impl $t for S_{id} { fn $n(&self) {} } } }
pub trait Te_{id} { fn goe(&self); }
pub trait Tp_{id} { fn gop(&self); }
pub struct S_{id};
pub fn emit_{id}() { crate::mke_{id}!(Te_{id}, goe); crate::mkp_{id}!(Tp_{id}, gop); }
""", id='fl18')}, [('fl18_e', 'e', 'S_fl18.goe()'), ('fl18_q', 'q', 'S_fl18.gop()')],
     note='R1918 (b): an impl a root-path macro emits in a fn body is hoisted, so a caller elsewhere reads its member')
pred('fl18', 'fl18_e', '-', '-', 'E'); pred('fl18', 'fl18_q', '-', '-', '-')
# fl22: R1922. A gated pure override of a DEPENDENCY trait's effectful default; a second trait with a PURE default is the pure twin.
cell('fl22', {'main.rs': sub("""pub struct Qe_{id}; pub struct Qq_{id};
#[cfg(not(unix))] impl fxdep_{id}::TD for Qe_{id} { fn go(&self) {} }
#[cfg(unix)] impl fxdep_{id}::TD for Qe_{id} {}
#[cfg(not(unix))] impl fxdep_{id}::TQ for Qq_{id} { fn goq(&self) {} }
#[cfg(unix)] impl fxdep_{id}::TQ for Qq_{id} {}
""", id='fl22')}, [('fl22_e', 'e', 'use fxdep_fl22::TD; Qe_fl22.go()'), ('fl22_q', 'q', 'use fxdep_fl22::TQ; Qq_fl22.goq()')],
     deps={'fxdep_fl22': '{ path = "fxdep_fl22" }'},
     extra={'@dep/fxdep_fl22/src/lib.rs': 'pub trait TD { fn go(&self) { EFF } }\npub trait TQ { fn goq(&self) {} }\n'}, hdr=False,
     note='R1922: a gated override of a dependency trait default whose twin arm inherits it')
pred('fl22', 'fl22_e', '-', '-', 'U'); pred('fl22', 'fl22_q', '-', '-', 'U')
# fl23: R1923. host holds a path-invoked macro (unexpanded, writes cfg); its text declares `mod s`; the out-of-line stand-in is host_fl23/s.rs.
# `t` is a visible file module the text does NOT declare (the pure twin: must stay clean).
cell('fl23', {'main.rs': sub("""#[macro_export] macro_rules! wcfg_{id} {{ () => {{
    #[cfg(unix)] pub mod s {{ pub fn go() {{ EFF }} }}
    #[cfg(not(unix))] pub mod s;
}} }}
pub mod host_{id} {{ crate::wcfg_{id}! {{}} pub mod t; }}
""".replace('{{', '{').replace('}}', '}'), id='fl23')}, [('fl23_e', 'e', 'host_fl23::s::go()'), ('fl23_q', 'q', 'host_fl23::t::pure()')],
     extra={'host_fl23/s.rs': 'pub fn go() {}\n', 'host_fl23/t.rs': 'pub fn pure() {}\n'},
     note='R1923: a nested path into a module only macro text declares, beside an out-of-line stand-in')
pred('fl23', 'fl23_e', '-', '-', 'U'); pred('fl23', 'fl23_q', '-', '-', '-')
# fl35: R1935. The METHOD spelling of fl23: a value TYPED to a type declared in the module only macro text declares (`mod s { struct T; impl T { fn go } }`),
# beside an out-of-line stand-in `s.rs` whose `T::go` is pure. `U` in the visible module `t` is the pure twin (must stay clean).
cell('fl35', {'main.rs': sub("""#[macro_export] macro_rules! wcfg_{id} {{ () => {{
    #[cfg(unix)] pub mod s {{ pub struct T; impl T {{ pub fn go(&self) {{ EFF }} }} }}
    #[cfg(not(unix))] pub mod s;
}} }}
pub mod host_{id} {{ crate::wcfg_{id}! {{}} pub mod t; }}
""".replace('{{', '{').replace('}}', '}'), id='fl35')},
     [('fl35_let_e', 'e', 'let t = host_fl35::s::T; t.go()'), ('fl35_ann_e', 'e', 'let t: host_fl35::s::T = host_fl35::s::T; t.go()'),
      ('fl35_use_e', 'e', 'use host_fl35::s::T; T.go()'), ('fl35_ufcs_e', 'e', 'host_fl35::s::T::go(&host_fl35::s::T)'),
      ('fl35_q', 'q', 'let u = host_fl35::t::U; u.go()')],
     extra={'host_fl35/s.rs': 'pub struct T; impl T { pub fn go(&self) {} }\n', 'host_fl35/t.rs': 'pub struct U; impl U { pub fn go(&self) {} }\n'},
     note='R1935: a method call on a value typed to a type only macro text declares, beside an out-of-line stand-in')
for _p in ('fl35_let_e', 'fl35_ann_e', 'fl35_use_e', 'fl35_ufcs_e'):
    pred('fl35', _p, '-', '-', 'U')
pred('fl35', 'fl35_q', '-', '-', '-')
# fl35o: R1935 round 2. The module the macro text names is OUT-OF-LINE (`mod s;`): the engine READS s.rs, so no "hidden by an unexpanded macro" reason is true of it.
cell('fl35o', {'main.rs': sub("""#[macro_export] macro_rules! wcfg_{id} {{ () => {{
    #[cfg(unix)] pub mod s;
}} }}
pub mod host_{id} {{ crate::wcfg_{id}! {{}} }}
""".replace('{{', '{').replace('}}', '}'), id='fl35o')},
     [('fl35o_let_e', 'e', 'let t = host_fl35o::s::T; t.go()'), ('fl35o_q', 'q', 'let t = host_fl35o::s::T; t.quiet()')],
     extra={'host_fl35o/s.rs': 'pub struct T; impl T { pub fn go(&self) { EFF } pub fn quiet(&self) {} }\n'},
     note='R1935: a module only named (out-of-line) in macro text is read, so a typed method call on its type is NOT hedged')
pred('fl35o', 'fl35o_let_e', '-', '-', 'E'); pred('fl35o', 'fl35o_q', '-', '-', '-')
# fl34i: R1934 round 2. A std bound that declares the projection beside the dependency's bound: the dependency trait does not declare `Item`, so no key.
cell('fl34i', {'main.rs': sub("""use fxdep_{id}::{{Af, HasA}};
pub struct Loud; impl Af for Loud {{ fn f() {{ EFF }} }}
pub struct Quiet; impl Af for Quiet {{ fn f() {{}} }}
pub struct ItE; impl Iterator for ItE {{ type Item = Loud; fn next(&mut self) -> Option<Loud> {{ None }} }} impl HasA for ItE {{ type A = Loud; }}
pub struct ItQ; impl Iterator for ItQ {{ type Item = Quiet; fn next(&mut self) -> Option<Quiet> {{ None }} }} impl HasA for ItQ {{ type A = Quiet; }}
mod g_{id} {{
    use super::Af;
    pub fn iter<T: fxdep_{id}::HasA + Iterator>() where T::Item: Af {{ T::Item::f() }}
}}
""".replace('{{', '{').replace('}}', '}'), id='fl34i')},
     [('fl34i_e', 'e', 'g_fl34i::iter::<ItE>()'), ('fl34i_q', 'q', 'g_fl34i::iter::<ItQ>()')],
     deps={'fxdep_fl34i': '{ path = "fxdep_fl34i" }'},
     extra={'@dep/fxdep_fl34i/src/lib.rs': 'pub trait Af { fn f(); }\npub trait HasA { type A: Af; }\n'}, chain=dict(kind='dep', dep_dir='fxdep_fl34i'), hdr=False,
     note='R1934: T: dep::HasA + Iterator, T::Item::f(): the dependency bound is not the only bound that could declare `Item`, so the site is hedged, never keyed')
pred('fl34i', 'fl34i_e', '-', '-', 'U'); pred('fl34i', 'fl34i_q', '-', '-', 'U'); pred('fl34i', 'fl34i_e@chained', '-', '-', 'U'); pred('fl34i', 'fl34i_q@chained', '-', '-', 'U')
# fl24a / fl24b: R1924. A `type`-alias self type, and a trait named through a re-export; r7gen / r7memcol are the generic and member-collision shapes.
cell('fl24a', {'main.rs': sub("""pub trait Tr_{id} { fn m(&self) { eff_{id}() } }
pub fn eff_{id}() { EFF }
pub mod xa_{id} { pub struct S; }
pub type A_{id} = xa_{id}::S;
#[cfg(not(unix))] impl Tr_{id} for A_{id} { fn m(&self) {} }
#[cfg(unix)] impl Tr_{id} for xa_{id}::S {}
""", id='fl24a')}, [('fl24a_e', 'e', 'xa_fl24a::S.m()')], note='R1924: a type-alias self type in one arm, the aliased path in the other')
pred('fl24a', 'fl24a_e', '-', '-', 'U')
cell('fl24b', {'main.rs': sub("""pub fn eff_{id}() { EFF }
pub mod inner_{id} { pub trait Tr { fn m(&self) { crate::eff_{id}() } } }
pub mod api_{id} { pub use crate::inner_{id}::Tr; }
pub struct S_{id};
#[cfg(not(unix))] impl api_{id}::Tr for S_{id} { fn m(&self) {} }
#[cfg(unix)] impl api_{id}::Tr for S_{id} {}
""", id='fl24b')}, [('fl24b_e', 'e', 'use api_fl24b::Tr; S_fl24b.m()')], note='R1924: the trait named through a re-export')
pred('fl24b', 'fl24b_e', '-', '-', 'U')

# ----------------------------------------------------------------------------------------------- S. bound / projection dispatch (design rust-bound-dispatch, lane L0)
# The acceptance matrix for ~/candor-handoff/rust-bound-dispatch-design (DESIGN.md section 3, FABLE-REVIEW.md edits 1 and 2). It pins NO engine behaviour beyond what
# pub v0.43.0 and main `026a95b7` READ today: every cell is an executed fixture (`_e` writes the marker, `_q` does not) and `expect.tsv` records both arms; the `cand`
# column is left for the lane that changes the engine. Every cell sets detail=True, so a reading carries the join KEY (`E{key=cf_bm1#Lt::lf}`,
# `[]{inv=fxdep_bm1;key=fxdep_bm1#Dt::f}`): in this vein the key IS the answer a chained consumer joins on, and a bare `[]`/`E` hides a wrong one (R1954).
# The package prefix is `cf_` (this runner's), not the design scratch's `fa_`.
#   bm1-bm4  the design's executed matrix (generic head, projections, concrete std type, qself/method spellings), 65 probes, ported verbatim
#   bd1, bd2 Fable's cells: N1 through `where`, an impl-level bound and a generic METHOD, `Self::lsf()` in a subtrait default (W-g4's answer); W-c1's edition-2018 column
#   wg1      W-g1: every local-bound spelling with THIRTEEN implementors (the bounded-CHA cap is 12)
#   wg2      W-g2: a blanket impl of the bound B, of the declaring trait D, or of both, with a pure twin trait that has no blanket
#   wg3      W-g3: a dependency trait RENAMED at the import, and dependency traits named like std's (`Error`, `Read`, `Pointee`), on the path and projection routes
#   wc1      W-c1 edition 2021: the same call as bd2 compiles to an AMBIGUITY (TryFrom is in the 2021 prelude): a NOCOMPILE cell
#   wc3      W-c3: std `FromStr as _` beside a dependency `Parse`; rustc rejects `String::from_str`, so the cell documents resolver rule 1b
#   wg5, wg5g W-g5: the TWO-HOP chain (consumer -> producer -> dependency): the only column where a wrong key is a gate flip; wg5g = the same, the producer importing by glob
_ALLOW = '#![allow(unused, non_camel_case_types, non_snake_case, dead_code)]\n'


def bm_cell(cid, main, dep, note, deps_extra=None, edition='2021', nocompile=False):
    """A cell whose probes are the `pub fn <cid>_*()` declared in `main` (kind = the name's last letter), with one dependency crate `fxdep_<cid>`.
    `DEP` in `main` names it; `EFF` is the executed write."""
    main = main.replace('DEP', 'fxdep_' + cid)
    probes = [(l.split('pub fn ')[1].split('(')[0], l.split('pub fn ')[1].split('(')[0][-1], None)
              for l in main.splitlines() if ('pub fn %s_' % cid) in l]
    assert probes and all(k in 'eq' for _, k, _ in probes), cid
    cell(cid, {'main.rs': _ALLOW + main}, probes, deps={'fxdep_' + cid: '{ path = "fxdep_%s" }' % cid},
         extra={'@dep/fxdep_%s/src/lib.rs' % cid: dep}, chain=dict(kind='dep', dep_dir='fxdep_' + cid), hdr=False, detail=True, note=note,
         edition=edition, nocompile=nocompile)
    for p, _, _ in probes:
        pred(cid, p, 'record', 'record', 'record')


_BM_DEP = """pub trait Dt { fn f(); fn m(&self); }
pub trait SupD { fn sf(); fn sm(&self); }
pub trait SubD: SupD {}
pub trait HasA { type A; }
pub trait Parse { fn from_str(s: &str) -> Self; }
pub trait Df { fn default() -> Self; }
pub trait Marker {}
pub trait FromV { fn from_vec(v: Vec<u8>) -> Self; }
pub struct DImpE; impl Dt for DImpE { fn f() { EFF } fn m(&self) { EFF } }
pub struct DImpQ; impl Dt for DImpQ { fn f() {} fn m(&self) {} }
impl Dt for String { fn f() { EFF } fn m(&self) { EFF } }
impl Dt for u16 { fn f() {} fn m(&self) {} }
pub struct DSupE; impl SupD for DSupE { fn sf() { EFF } fn sm(&self) { EFF } } impl SubD for DSupE {}
pub struct DHas; impl HasA for DHas { type A = DImpE; }
impl Parse for String { fn from_str(_s: &str) -> Self { EFF String::new() } }
impl Df for std::time::Instant { fn default() -> Self { EFF std::time::Instant::now() } }
impl FromV for std::ffi::OsString { fn from_vec(_v: Vec<u8>) -> Self { EFF std::ffi::OsString::new() } }
"""
bm_cell('bm1', """use DEP::{Dt, SupD, SubD, HasA};
pub trait Lt { fn lf(); fn lm(&self); }
pub trait LSup { fn lsf(); } pub trait LSub: LSup {}
pub trait LSubD: DEP::SupD {}
pub struct LoudL; impl Lt for LoudL { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct QuietL; impl Lt for QuietL { fn lf() {} fn lm(&self) {} }
impl Lt for String { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct LoudD; impl DEP::Dt for LoudD { fn f() { EFF } fn m(&self) { EFF } }
pub struct QuietD; impl DEP::Dt for QuietD { fn f() {} fn m(&self) {} }
pub struct LS; impl LSup for LS { fn lsf() { EFF } } impl LSub for LS {}
pub struct OnlySup; impl LSup for OnlySup { fn lsf() { EFF } }
pub struct LSQ; impl LSup for LSQ { fn lsf() {} } impl LSub for LSQ {}
pub struct LDS; impl DEP::SupD for LDS { fn sf() { EFF } fn sm(&self) { EFF } } impl LSubD for LDS {} impl DEP::SubD for LDS {}
pub struct LoudFs; impl std::str::FromStr for LoudFs { type Err = (); fn from_str(_s: &str) -> Result<Self, ()> { EFF Ok(LoudFs) } }
pub struct LoudDef; impl Default for LoudDef { fn default() -> Self { EFF LoudDef } }
pub struct LoudIt; impl Iterator for LoudIt { type Item = u8; fn next(&mut self) -> Option<u8> { EFF None } }
// B1 local bound
fn g_b1_path<T: Lt>() { T::lf() }      pub fn bm1_b1_path_e() { g_b1_path::<LoudL>() }
fn g_b1_pathq<T: Lt>() { T::lf() }     pub fn bm1_b1_path_q() { g_b1_pathq::<QuietL>() }
fn g_b1_meth<T: Lt>(t: &T) { t.lm() }  pub fn bm1_b1_meth_e() { g_b1_meth(&LoudL) }
fn g_b1_qs<T: Lt>() { <T as Lt>::lf() } pub fn bm1_b1_qself_e() { g_b1_qs::<LoudL>() }
fn g_b1_ufcs<T: Lt>(t: &T) { Lt::lm(t) } pub fn bm1_b1_ufcs_e() { g_b1_ufcs(&LoudL) }
fn g_b1_std<T: Lt>() { T::lf() }        pub fn bm1_b1_stdimpl_e() { g_b1_std::<String>() }
// B2 dependency bound
fn g_b2_path_in<T: Dt>() { T::f() }     pub fn bm1_b2_path_incrate_e() { g_b2_path_in::<LoudD>() }
fn g_b2_path_inq<T: Dt>() { T::f() }    pub fn bm1_b2_path_incrate_q() { g_b2_path_inq::<QuietD>() }
fn g_b2_path_dep<T: Dt>() { T::f() }    pub fn bm1_b2_path_depty_e() { g_b2_path_dep::<DEP::DImpE>() }
fn g_b2_path_std<T: Dt>() { T::f() }    pub fn bm1_b2_path_stdty_e() { g_b2_path_std::<String>() }
fn g_b2_path_stdq<T: Dt>() { T::f() }   pub fn bm1_b2_path_stdty_q() { g_b2_path_stdq::<u16>() }
fn g_b2_meth_in<T: Dt>(t: &T) { t.m() } pub fn bm1_b2_meth_incrate_e() { g_b2_meth_in(&LoudD) }
fn g_b2_qs_in<T: Dt>() { <T as Dt>::f() } pub fn bm1_b2_qself_incrate_e() { g_b2_qs_in::<LoudD>() }
fn g_b2_ufcs_in<T: Dt>(t: &T) { Dt::m(t) } pub fn bm1_b2_ufcs_incrate_e() { g_b2_ufcs_in(&LoudD) }
// B3 std bound, in-crate implementor of the std trait (R1955 class; README documented miss)
fn g_b3_fromstr<T: std::str::FromStr>() { let _ = T::from_str("x"); } pub fn bm1_b3_path_fromstr_e() { g_b3_fromstr::<LoudFs>() }
fn g_b3_fromstrq<T: std::str::FromStr>() { let _ = T::from_str("1"); } pub fn bm1_b3_path_fromstr_q() { g_b3_fromstrq::<u8>() }
fn g_b3_default<T: Default>() { let _ = T::default(); } pub fn bm1_b3_path_default_e() { g_b3_default::<LoudDef>() }
fn g_b3_next<T: Iterator>(mut t: T) { t.next(); } pub fn bm1_b3_meth_next_e() { g_b3_next(LoudIt) }
fn g_b3_nextu<T: Iterator>(mut t: T) { Iterator::next(&mut t); } pub fn bm1_b3_ufcs_next_e() { g_b3_nextu(LoudIt) }
fn g_b3_nextq<T: Iterator>(mut t: T) { <T as Iterator>::next(&mut t); } pub fn bm1_b3_qself_next_e() { g_b3_nextq(LoudIt) }
// B4 mixed: dependency + local bound
fn g_b4_loc<T: Dt + Lt>() { T::lf() }   pub fn bm1_b4_mixed_local_e() { g_b4_loc::<String>() }
fn g_b4_dep<T: Dt + Lt>() { T::f() }    pub fn bm1_b4_mixed_dep_e() { g_b4_dep::<String>() }
fn g_b4_def<T: Lt + Default>() { T::lf() } pub fn bm1_b4_mixed_std_e() { g_b4_def::<String>() }
// B5 supertrait: local, dependency, and a local subtrait over a dependency supertrait
fn g_b5_loc<T: LSub>() { T::lsf() }     pub fn bm1_b5_localsup_e() { g_b5_loc::<LS>() }
fn g_b5_locq<T: LSub>() { T::lsf() }    pub fn bm1_b5_localsup_q() { g_b5_locq::<LSQ>() }
fn g_b5_dep<T: SubD>() { T::sf() }      pub fn bm1_b5_depsup_e() { g_b5_dep::<DEP::DSupE>() }
fn g_b5_depin<T: SubD>() { T::sf() }    pub fn bm1_b5_depsup_incrate_e() { g_b5_depin::<LDS>() }
fn g_b5_depm<T: SubD>(t: &T) { t.sm() } pub fn bm1_b5_depsup_meth_e() { g_b5_depm(&DEP::DSupE) }
fn g_b5_mix<T: LSubD>() { T::sf() }     pub fn bm1_b5_localsub_depsup_e() { g_b5_mix::<LDS>() }
""", _BM_DEP,
        'design bm1: a generic head T, bound shapes (local / dependency / std / mixed / supertrait) x implementor location x spelling (path, method, qself, UFCS)')
bm_cell('bm2', """use DEP::{Dt, SupD, SubD, HasA};
pub trait Lt { fn lf(); fn lm(&self); }
pub trait HasL { type A: Lt; }
pub trait LSup { fn lsf(); } pub trait LSub: LSup {}
pub struct LoudL; impl Lt for LoudL { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct QuietL; impl Lt for QuietL { fn lf() {} fn lm(&self) {} }
impl Lt for DEP::DImpE { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct HL; impl HasL for HL { type A = LoudL; }
pub struct HLq; impl HasL for HLq { type A = QuietL; }
pub struct LoudD; impl DEP::Dt for LoudD { fn f() { EFF } fn m(&self) { EFF } }
pub struct ItL(Option<LoudL>); impl Iterator for ItL { type Item = LoudL; fn next(&mut self) -> Option<LoudL> { None } }
pub struct ItD; impl Iterator for ItD { type Item = LoudD; fn next(&mut self) -> Option<LoudD> { None } }
pub struct ItDS; impl Iterator for ItDS { type Item = DEP::DSupE; fn next(&mut self) -> Option<DEP::DSupE> { None } }
pub struct ItS; impl Iterator for ItS { type Item = String; fn next(&mut self) -> Option<String> { None } }
pub struct LS; impl LSup for LS { fn lsf() { EFF } } impl LSub for LS {}
pub struct ItLS; impl Iterator for ItLS { type Item = LS; fn next(&mut self) -> Option<LS> { None } }
// P-local declaration (R1037)
fn g_p_ldecl<T: HasL>() { T::A::lf() }         pub fn bm2_localdecl_path_e() { g_p_ldecl::<HL>() }
fn g_p_ldeclq<T: HasL>() { T::A::lf() }        pub fn bm2_localdecl_path_q() { g_p_ldeclq::<HLq>() }
fn g_p_ldecl_qs<T: HasL>() { <T::A as Lt>::lf() } pub fn bm2_localdecl_qpa_e() { g_p_ldecl_qs::<HL>() }
fn g_p_ldecl_q0<T: HasL>() { <T::A>::lf() }    pub fn bm2_localdecl_q0_e() { g_p_ldecl_q0::<HL>() }
fn g_p_ldecl_m<T: HasL>(a: T::A) { a.lm() }    pub fn bm2_localdecl_meth_e() { g_p_ldecl_m::<HL>(LoudL) }
fn g_p_ldecl_qa<T: HasL>() { <T as HasL>::A::lf() } pub fn bm2_localdecl_qa_e() { g_p_ldecl_qa::<HL>() }
// std head, local bound on the projection (R1940, closed)
fn g_p_stdloc<T: Iterator>() where T::Item: Lt { T::Item::lf() } pub fn bm2_stdhead_localproj_e() { g_p_stdloc::<ItL>() }
fn g_p_stdloc_m<T: Iterator>(mut t: T) where T::Item: Lt { if let Some(x) = t.next() { x.lm() } } pub fn bm2_stdhead_localproj_meth_e() { g_p_stdloc_m(std::iter::once(LoudL)) }
// std head, DEPENDENCY bound on the projection
fn g_p_stddep<T: Iterator>() where T::Item: Dt { T::Item::f() } pub fn bm2_stdhead_depproj_incrate_e() { g_p_stddep::<ItD>() }
fn g_p_stddep_sup<T: Iterator>() where T::Item: SubD { T::Item::sf() } pub fn bm2_stdhead_depsupproj_e() { g_p_stddep_sup::<ItDS>() }
fn g_p_stddep_c<T: Iterator<Item: Dt>>() { T::Item::f() } pub fn bm2_stdhead_depconstraint_e() { g_p_stddep_c::<ItD>() }
// std head, local SUBTRAIT bound on the projection, member in the local supertrait (R1953 shape, implementor implements both)
fn g_p_locsup<T: Iterator>() where T::Item: LSub { T::Item::lsf() } pub fn bm2_stdhead_localsupproj_e() { g_p_locsup::<ItLS>() }
// dependency head, local bound on projection (R1939, fc34_loc_e) and dependency head, the dependency's own projection
fn g_p_deploc<T: HasA>() where T::A: Lt { T::A::lf() } pub fn bm2_dephead_localproj_e() { g_p_deploc::<DEP::DHas>() }
fn g_p_depown<T: HasA>() where T::A: Dt { T::A::f() } pub fn bm2_dephead_depproj_e() { g_p_depown::<DEP::DHas>() }
fn g_p_depbare<T: HasA>() { let _ = 0; } pub fn bm2_dephead_noproj_q() { g_p_depbare::<DEP::DHas>() }
// written binding
fn g_p_bindstd<T: Iterator<Item = String>>() { let _ = <T::Item as DEP::Parse>::from_str("x"); } pub fn bm2_bind_stdty_named_e() { g_p_bindstd::<ItS>() }
fn g_p_bindstd2<T: Iterator<Item = String>>() { use DEP::Parse; let _ = T::Item::from_str("x"); } pub fn bm2_bind_stdty_scope_e() { g_p_bindstd2::<ItS>() }
fn g_p_binddep<T: Iterator<Item = DEP::DImpE>>() { T::Item::lf() } pub fn bm2_bind_depty_localtrait_e() { g_p_binddep::<std::iter::Empty<DEP::DImpE>>() }
""", _BM_DEP, 'design bm2: projections: local declaration (R1037), std head + local or dependency bound on the projection (R1940/R1952/R1953), dependency head (R1939), written binding (N5)')
bm_cell('bm3', """
mod a { use DEP::Df; pub fn e() { let _ = std::time::Instant::default(); } }
mod b { use DEP::Parse; pub fn e() { let _ = String::from_str("x"); } }
mod c { use std::str::FromStr; use DEP::Dt; pub fn q() { let _ = String::from_str("x"); } }
mod d { use std::os::unix::prelude::*; use DEP::Marker; pub fn q() { let _ = std::ffi::OsString::from_vec(vec![]); } }
mod e { use DEP::FromV; pub fn e() { let _ = std::ffi::OsString::from_vec(vec![]); } }
mod f { use DEP::*; pub fn e() { let _ = std::time::Instant::default(); } }
mod g { pub fn e() { let _ = <std::time::Instant as DEP::Df>::default(); } }
mod h { use DEP::Dt; pub fn e() { <String as Dt>::f(); } pub fn e2() { String::f(); } pub fn e3() { "x".to_string().m(); } }
mod i { use DEP::Dt; pub fn q() { u16::f(); } }
pub fn bm3_prelude_name_not_impl_e() { a::e() }
pub fn bm3_nonprelude_name_e() { b::e() }
pub fn bm3_std_trait_in_scope_q() { c::q() }
pub fn bm3_sealed_via_prelude_glob_q() { d::q() }
pub fn bm3_sealed_name_dep_only_e() { e::e() }
pub fn bm3_dep_glob_prelude_name_e() { f::e() }
pub fn bm3_qself_named_e() { g::e() }
pub fn bm3_qself_dt_e() { h::e() }
pub fn bm3_path_dt_e() { h::e2() }
pub fn bm3_meth_dt_e() { h::e3() }
pub fn bm3_path_dt_q() { i::q() }
""", _BM_DEP, 'design bm3: a concrete std type and the scope of a trait name (R1757/R1913/R1951/R1956, N6/N7)')
bm_cell('bm4', """use DEP::{Dt, SupD, SubD};
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub trait LSubD: DEP::SupD {}
pub struct LS; impl LSup for LS { fn lsf() { EFF } fn lsm(&self) { EFF } } impl LSub for LS {}
pub struct LDS; impl DEP::SupD for LDS { fn sf() { EFF } fn sm(&self) { EFF } } impl LSubD for LDS {}
fn g4_lsm<T: LSub>(t: &T) { t.lsm() }        pub fn bm4_localsup_meth_e() { g4_lsm(&LS) }
fn g4_lsq<T: LSub>() { <T as LSup>::lsf() }  pub fn bm4_localsup_qself_e() { g4_lsq::<LS>() }
fn g4_mixm<T: LSubD>(t: &T) { t.sm() }       pub fn bm4_localsub_depsup_meth_e() { g4_mixm(&LDS) }
fn g4_mixq<T: LSubD>() { <T as SupD>::sf() } pub fn bm4_localsub_depsup_qself_e() { g4_mixq::<LDS>() }
fn g4_depq<T: SubD>() { <T as SupD>::sf() }  pub fn bm4_depsup_qself_e() { g4_depq::<DEP::DSupE>() }
mod h { use DEP::Dt; pub fn e1() { let s = String::new(); s.m(); } pub fn e2() { String::new().m(); } }
pub fn bm4_meth_dt_let_e() { h::e1() }
pub fn bm4_meth_dt_ctor_e() { h::e2() }
""", _BM_DEP, 'design bm4: the qself / method spellings that key right, and a method on a std type through a dependency trait')

# bd1 (Fable): N1 through a `where` clause, an impl-level bound and a generic METHOD; `Self::lsf()` in a subtrait default body called through `T::go()` (W-g4's executed answer:
# main E with keys on both traits). The qself spelling that names the SUBtrait (`<T as LSub>::lsf()`) does not compile (E0576: `lsf` is not in `LSub`), so the
# declaring-trait claim of design A1 holds by rule 1b and NO cell is written for it (FABLE-REVIEW edit 1).
cell('bd1', {'main.rs': _ALLOW + sub("""pub trait LSup_bd1 { fn lsf(); }
pub trait LSub_bd1: LSup_bd1 { fn go() { Self::lsf() } }
pub struct LS_bd1; impl LSup_bd1 for LS_bd1 { fn lsf() { EFF } } impl LSub_bd1 for LS_bd1 {}
pub struct LSQ_bd1; impl LSup_bd1 for LSQ_bd1 { fn lsf() {} } impl LSub_bd1 for LSQ_bd1 {}
pub struct LD_bd1; impl fxdep_bd1::SupD for LD_bd1 { fn sf() { EFF } } impl fxdep_bd1::SubD for LD_bd1 {}
pub struct W_bd1<T>(std::marker::PhantomData<T>);
impl<T: LSub_bd1> W_bd1<T> { pub fn go() { T::lsf() } }
pub struct H_bd1; impl H_bd1 { pub fn gm<T: LSub_bd1>(&self) { T::lsf() } }
fn g_where<T>() where T: LSub_bd1 { T::lsf() }
fn g_whereq<T>() where T: LSub_bd1 { T::lsf() }
fn g_selfdef<T: LSub_bd1>() { T::go() }
pub fn bd1_where_e() { g_where::<LS_bd1>() }
pub fn bd1_where_q() { g_whereq::<LSQ_bd1>() }
pub fn bd1_implgen_e() { W_bd1::<LS_bd1>::go() }
pub fn bd1_method_gen_e() { H_bd1.gm::<LS_bd1>() }
pub fn bd1_selfdef_e() { g_selfdef::<LS_bd1>() }
""", id='bd1')},
     [('bd1_where_e', 'e', None), ('bd1_where_q', 'q', None), ('bd1_implgen_e', 'e', None), ('bd1_method_gen_e', 'e', None), ('bd1_selfdef_e', 'e', None)],
     deps={'fxdep_bd1': '{ path = "fxdep_bd1" }'}, extra={'@dep/fxdep_bd1/src/lib.rs': 'pub trait SupD { fn sf(); }\npub trait SubD: SupD {}\n'},
     chain=dict(kind='dep', dep_dir='fxdep_bd1'), hdr=False, detail=True,
     note='N1 (an associated fn declared in a local SUPERtrait) through `where T: LSub`, `impl<T: LSub> W<T>` and a generic method; `Self::lsf()` in a subtrait default (W-g4)')
for _p in ('bd1_where_e', 'bd1_where_q', 'bd1_implgen_e', 'bd1_method_gen_e', 'bd1_selfdef_e'):
    pred('bd1', _p, 'record', 'record', 'record')
# bd2 (Fable, W-c1 edition-2018 column): `String::try_from(1u8)` through a dependency `TryFromX`; TryFrom is NOT in the 2018 prelude, so the dependency's impl runs.
cell('bd2', {'main.rs': _ALLOW + 'mod a_bd2 { use fxdep_bd2::TryFromX; pub fn e() { let _ = String::try_from(1u8); } }\npub fn bd2_tryfrom_2018_e() { a_bd2::e() }\n'},
     [('bd2_tryfrom_2018_e', 'e', None)], deps={'fxdep_bd2': '{ path = "fxdep_bd2" }'}, edition='2018',
     extra={'@dep/fxdep_bd2/src/lib.rs': 'pub trait TryFromX { fn try_from(x: u8) -> Self; }\nimpl TryFromX for String { fn try_from(_x: u8) -> Self { EFF String::new() } }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_bd2'), hdr=False, detail=True,
     note='W-c1, edition 2018: TryFrom is not in the prelude, so `String::try_from(1u8)` with only the dependency trait in scope runs the dependency (R1951 population)')
pred('bd2', 'bd2_tryfrom_2018_e', 'record', 'record', 'record')

# wg1 (W-g1): the CHA cap. A local trait with THIRTEEN implementors (one effectful, I0) is past the bounded-CHA band (12), so a bound-set answer must be a true
# `dispatch:<D>.<m>` hedge (or the real effect), never silence. Every spelling of the local-bound population (path, method, qself, UFCS, a member reached through a
# supertrait (N1), a projection bound on a std head (R1940), a local declaration (R1037), QA (N3)); every `_q` is its OWN helper over a pure implementor.
_W1 = ['I%d_wg1' % i for i in range(13)]
_w1_impls = ''.join('pub struct %s; impl Lt for %s { fn lf() %s fn lm(&self) %s }\n' % (n, n, '{ EFF }' if i == 0 else '{}', '{ EFF }' if i == 0 else '{}')
                    for i, n in enumerate(_W1))
_w1_sup = ''.join('pub struct S%d_wg1; impl LSup for S%d_wg1 { fn lsf() %s fn lsm(&self) %s } impl LSub for S%d_wg1 {}\n' % (i, i, '{ EFF }' if i == 0 else '{}', '{ EFF }' if i == 0 else '{}', i)
                  for i in range(13))
_w1_shapes = [
    # (name, generic helper body template with {g} for the helper name, e instantiation, q instantiation)
    ('path',   'fn {g}<T: Lt>() { T::lf() }',                                  '{g}::<I0_wg1>()',               '{g}::<I1_wg1>()'),
    ('meth',   'fn {g}<T: Lt>(t: &T) { t.lm() }',                              '{g}(&I0_wg1)',                  '{g}(&I1_wg1)'),
    ('qself',  'fn {g}<T: Lt>() { <T as Lt>::lf() }',                          '{g}::<I0_wg1>()',               '{g}::<I1_wg1>()'),
    ('ufcs',   'fn {g}<T: Lt>(t: &T) { Lt::lm(t) }',                           '{g}(&I0_wg1)',                  '{g}(&I1_wg1)'),
    ('suppath', 'fn {g}<T: LSub>() { T::lsf() }',                              '{g}::<S0_wg1>()',               '{g}::<S1_wg1>()'),
    ('supmeth', 'fn {g}<T: LSub>(t: &T) { t.lsm() }',                          '{g}(&S0_wg1)',                  '{g}(&S1_wg1)'),
    ('proj',   'fn {g}<T: Iterator>() where T::Item: Lt { T::Item::lf() }',    '{g}::<It0_wg1>()',              '{g}::<It1_wg1>()'),
    ('projm',  'fn {g}<T: Iterator>(mut t: T) where T::Item: Lt { if let Some(x) = t.next() { x.lm() } }', '{g}(std::iter::once(I0_wg1))', '{g}(std::iter::once(I1_wg1))'),
    ('decl',   'fn {g}<T: HasL>() { T::A::lf() }',                             '{g}::<H0_wg1>()',               '{g}::<H1_wg1>()'),
    ('qa',     'fn {g}<T: HasL>() { <T as HasL>::A::lf() }',                   '{g}::<H0_wg1>()',               '{g}::<H1_wg1>()'),
]
_w1_main = ('pub trait Lt { fn lf(); fn lm(&self); }\npub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}\npub trait HasL { type A: Lt; }\n'
            + _w1_impls + _w1_sup +
            'pub struct It0_wg1; impl Iterator for It0_wg1 { type Item = I0_wg1; fn next(&mut self) -> Option<I0_wg1> { None } }\n'
            'pub struct It1_wg1; impl Iterator for It1_wg1 { type Item = I1_wg1; fn next(&mut self) -> Option<I1_wg1> { None } }\n'
            'pub struct H0_wg1; impl HasL for H0_wg1 { type A = I0_wg1; }\npub struct H1_wg1; impl HasL for H1_wg1 { type A = I1_wg1; }\n')
_w1_probes = []
for _n, _g, _e, _q in _w1_shapes:
    for _k, _inst in (('e', _e), ('q', _q)):
        _h = 'g_%s_%s' % (_n, _k)
        _w1_main += _g.replace('{g}', _h) + '\npub fn wg1_%s_%s() { %s }\n' % (_n, _k, _inst.replace('{g}', _h) + ';')
        _w1_probes.append(('wg1_%s_%s' % (_n, _k), _k, None))
cell('wg1', {'main.rs': _ALLOW + _w1_main}, _w1_probes, hdr=False, detail=True,
     note='W-g1: thirteen implementors of the local bound / declaring trait (the bounded-CHA cap is 12). Executed: `_e` runs the one effectful implementor, `_q` a pure one')
for _p, _, _ in _w1_probes:
    pred('wg1', _p, 'record', 'record', 'record')

# wg2 (W-g2): a BLANKET impl of the bound B, of the declaring trait D, or of both. The blanket body is effectful, so every `_e` writes. `Ln` is the pure twin trait:
# no blanket, two pure implementors, the same spellings; it must read `-` (an over-charge on it is a fabrication). NB the PATH spelling of a member reached through a
# supertrait (`T: LSub`, `T::lsf()`) is N1 and reads `-` on every arm today whatever the impls, so the blanket-of-B/D/BOTH cells also carry a METHOD spelling, which does not.
_w2_main = '''pub trait Mk {}
pub trait Lt { fn lf(); fn lm(&self); }
pub struct M_wg2; impl Mk for M_wg2 {}
impl<X: Mk> Lt for X { fn lf() { EFF } fn lm(&self) { EFF } }
pub struct P_wg2; impl Lt for P_wg2 { fn lf() {} fn lm(&self) {} }
pub trait Ln { fn nf(); fn nm(&self); }
pub struct N0_wg2; impl Ln for N0_wg2 { fn nf() {} fn nm(&self) {} }
pub struct N1_wg2; impl Ln for N1_wg2 { fn nf() {} fn nm(&self) {} }
// blanket of D (declaring) and of B (bound) in a supertrait pair
pub trait LSup { fn lsf(); fn lsm(&self); } pub trait LSub: LSup {}
pub struct LM_wg2; impl Mk for LM_wg2 {} impl LSup for LM_wg2 { fn lsf() { EFF } fn lsm(&self) { EFF } }
impl<X: Mk + LSup> LSub for X {}                              // blanket of the BOUND B; D (LSup) is explicit
pub trait LSup2 { fn lsf2(); fn lsm2(&self); }  pub trait LSub2: LSup2 {}
impl<X: Mk> LSup2 for X { fn lsf2() { EFF } fn lsm2(&self) { EFF } }                  // blanket of the DECLARING trait D
impl LSub2 for M_wg2 {}
pub trait LSup3 { fn lsf3(); fn lsm3(&self); }  pub trait LSub3: LSup3 {}
impl<X: Mk> LSup3 for X { fn lsf3() { EFF } fn lsm3(&self) { EFF } }                  // blanket of BOTH
impl<X: Mk> LSub3 for X {}
pub trait LSupn { fn nsf(); } pub trait LSubn: LSupn {}
pub struct SN0_wg2; impl LSupn for SN0_wg2 { fn nsf() {} } impl LSubn for SN0_wg2 {}
pub struct It_wg2; impl Iterator for It_wg2 { type Item = M_wg2; fn next(&mut self) -> Option<M_wg2> { None } }
pub struct Itn_wg2; impl Iterator for Itn_wg2 { type Item = N0_wg2; fn next(&mut self) -> Option<N0_wg2> { None } }
fn g_path<T: Lt>() { T::lf() }                   pub fn wg2_path_e() { g_path::<M_wg2>() }
fn g_pathn<T: Ln>() { T::nf() }                  pub fn wg2_path_q() { g_pathn::<N0_wg2>() }
fn g_meth<T: Lt>(t: &T) { t.lm() }               pub fn wg2_meth_e() { g_meth(&M_wg2) }
fn g_methn<T: Ln>(t: &T) { t.nm() }              pub fn wg2_meth_q() { g_methn(&N1_wg2) }
fn g_qself<T: Lt>() { <T as Lt>::lf() }          pub fn wg2_qself_e() { g_qself::<M_wg2>() }
fn g_qselfn<T: Ln>() { <T as Ln>::nf() }         pub fn wg2_qself_q() { g_qselfn::<N0_wg2>() }
fn g_ufcs<T: Lt>(t: &T) { Lt::lm(t) }            pub fn wg2_ufcs_e() { g_ufcs(&M_wg2) }
fn g_bb<T: LSub>() { T::lsf() }                  pub fn wg2_blanketB_e() { g_bb::<LM_wg2>() }
fn g_bbm<T: LSub>(t: &T) { t.lsm() }             pub fn wg2_blanketB_meth_e() { g_bbm(&LM_wg2) }
fn g_bd<T: LSub2>() { T::lsf2() }                pub fn wg2_blanketD_e() { g_bd::<M_wg2>() }
fn g_bboth<T: LSub3>() { T::lsf3() }             pub fn wg2_blanketBoth_e() { g_bboth::<M_wg2>() }
fn g_bdm<T: LSub2>(t: &T) { t.lsm2() }           pub fn wg2_blanketD_meth_e() { g_bdm(&M_wg2) }
fn g_bbothm<T: LSub3>(t: &T) { t.lsm3() }        pub fn wg2_blanketBoth_meth_e() { g_bbothm(&M_wg2) }
fn g_supn<T: LSubn>() { T::nsf() }               pub fn wg2_blanketB_q() { g_supn::<SN0_wg2>() }
fn g_proj<I: Iterator>() where I::Item: Lt { I::Item::lf() }   pub fn wg2_proj_e() { g_proj::<It_wg2>() }
fn g_projn<I: Iterator>() where I::Item: Ln { I::Item::nf() }  pub fn wg2_proj_q() { g_projn::<Itn_wg2>() }
'''
cell('wg2', {'main.rs': _ALLOW + _w2_main},
     [(l.split('pub fn ')[1].split('(')[0], l.split('pub fn ')[1].split('(')[0][-1], None) for l in _w2_main.splitlines() if 'pub fn wg2_' in l],
     hdr=False, detail=True,
     note='W-g2: a blanket impl of the bound B, of the declaring trait D, or of both (the bound set "may cover"), beside a pure twin trait with no blanket')
for _l in _w2_main.splitlines():
    if 'pub fn wg2_' in _l:
        _p = _l.split('pub fn ')[1].split('(')[0]
        pred('wg2', _p, 'record', 'record', 'record')

# wg3 (W-g3): the ORIGIN of a bound trait is decided by its resolved root, never its leaf. A dependency trait renamed at the import (`use dep::Sub as DS`), and dependency
# traits named like std's (`Error`, `Read`, `Pointee`: R1952's population), on the head (`T::m`) route and the projection (`where I::Item: ..`) route. The `_q` twin
# uses the SAME-LEAF trait from the dependency's `q` module, implemented only by the pure `DQ` (a twin sharing the trait with the effectful `DE` would read charged
# on every arm, because a key names a trait, not a type).
_w3_dep = '''pub trait Sub { fn sf(); fn sm(&self); }
pub trait Error { fn ef(); fn em(&self); }
pub trait Read { fn rf(); fn rm(&self); }
pub trait Pointee { fn pf(); fn pm(&self); }
pub struct DE; pub struct DQ;
impl Sub for DE { fn sf() { EFF } fn sm(&self) { EFF } }
impl Error for DE { fn ef() { EFF } fn em(&self) { EFF } }
impl Read for DE { fn rf() { EFF } fn rm(&self) { EFF } }
impl Pointee for DE { fn pf() { EFF } fn pm(&self) { EFF } }
pub mod q {
    pub trait Sub { fn sf(); fn sm(&self); }
    pub trait Error { fn ef(); fn em(&self); }
    pub trait Read { fn rf(); fn rm(&self); }
    pub trait Pointee { fn pf(); fn pm(&self); }
    impl Sub for super::DQ { fn sf() {} fn sm(&self) {} }
    impl Error for super::DQ { fn ef() {} fn em(&self) {} }
    impl Read for super::DQ { fn rf() {} fn rm(&self) {} }
    impl Pointee for super::DQ { fn pf() {} fn pm(&self) {} }
}
'''
_w3_main = '''use fxdep_wg3::{DE, DQ};
use fxdep_wg3::Sub as DS;
use fxdep_wg3::q::Sub as DSQ;
pub struct ItE_wg3; impl Iterator for ItE_wg3 { type Item = DE; fn next(&mut self) -> Option<DE> { None } }
pub struct ItQ_wg3; impl Iterator for ItQ_wg3 { type Item = DQ; fn next(&mut self) -> Option<DQ> { None } }
'''
_w3_probes = []
for _n, _f, _m in (('ren', 'sf', 'sm'), ('err', 'ef', 'em'), ('read', 'rf', 'rm'), ('ptr', 'pf', 'pm')):
    for _k, _ty in (('e', 'DE'), ('q', 'DQ')):
        _tr = {('ren', 'e'): 'DS', ('ren', 'q'): 'DSQ'}.get((_n, _k)) or 'fxdep_wg3::%s%s' % ('q::' if _k == 'q' else '', {'err': 'Error', 'read': 'Read', 'ptr': 'Pointee'}[_n])
        # a projection's `I::Item::f()` needs the trait IN SCOPE (the head `T::f()` does not): import it under its leaf inside the helper's module
        _w3_main += 'mod m_%s_%s { use super::*; %s\n' % (_n, _k, '' if _n == 'ren' else 'use %s as Tr_%s_%s; ' % (_tr, _n, _k))
        _w3_main += '    pub fn path<T: %s>() { T::%s() }\n    pub fn meth<T: %s>(t: &T) { t.%s() }\n' % (_tr, _f, _tr, _m)
        _w3_main += '    pub fn proj<I: Iterator>() where I::Item: %s { I::Item::%s() }\n}\n' % (_tr, _f)
        _w3_main += ('pub fn wg3_%s_path_%s() { m_%s_%s::path::<%s>() }\npub fn wg3_%s_meth_%s() { m_%s_%s::meth(&%s) }\npub fn wg3_%s_proj_%s() { m_%s_%s::proj::<It%s_wg3>() }\n'
                     % (_n, _k, _n, _k, _ty, _n, _k, _n, _k, _ty, _n, _k, _n, _k, _k.upper()))
        _w3_probes += [('wg3_%s_%s_%s' % (_n, _s, _k), _k, None) for _s in ('path', 'meth', 'proj')]
cell('wg3', {'main.rs': _ALLOW + _w3_main}, _w3_probes, deps={'fxdep_wg3': '{ path = "fxdep_wg3" }'}, extra={'@dep/fxdep_wg3/src/lib.rs': _w3_dep},
     chain=dict(kind='dep', dep_dir='fxdep_wg3'), hdr=False, detail=True,
     note='W-g3: a dependency trait renamed at the import, and dependency traits named Error/Read/Pointee, on the head and projection routes; the pure twin uses the same-leaf trait of the `q` module')
for _p, _, _ in _w3_probes:
    pred('wg3', _p, 'record', 'record', 'record')

# wc1 (W-c1, edition 2021): the bd2 call under the 2021 prelude. TryFrom is in scope beside the dependency's TryFromX, so rustc reports the ambiguity (E0034) and the
# program does not compile: the cell documents the compiles-as-evidence rule (resolver 1b), the 2018 column (bd2) is the executed answer. The engine's readings are
# still recorded: a scan of a program that does not compile is what a user's editor sees mid-edit.
cell('wc1', {'main.rs': _ALLOW + 'mod a_wc1 { use fxdep_wc1::TryFromX; pub fn e() { let _ = String::try_from(1u8); } }\npub fn wc1_tryfrom_2021_e() { a_wc1::e() }\n'},
     [('wc1_tryfrom_2021_e', 'e', None)], deps={'fxdep_wc1': '{ path = "fxdep_wc1" }'}, nocompile=True,
     extra={'@dep/fxdep_wc1/src/lib.rs': 'pub trait TryFromX { fn try_from(x: u8) -> Self; }\nimpl TryFromX for String { fn try_from(_x: u8) -> Self { EFF String::new() } }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_wc1'), hdr=False, detail=True,
     note='W-c1, edition 2021: TryFrom is in the prelude beside the dependency trait: rustc E0034 (compile-fails cell; bd2 is the 2018 executed column)')
pred('wc1', 'wc1_tryfrom_2021_e', 'record', 'record', 'record')
# wc3 (W-c3): the std trait imported `as _` beside a dependency trait with the same member name: the call is ambiguous, rustc rejects it (E0034).
cell('wc3', {'main.rs': _ALLOW + 'mod a_wc3 { use std::str::FromStr as _; use fxdep_wc3::Parse; pub fn e() { let _ = String::from_str("x"); } }\npub fn wc3_fromstr_as_e() { a_wc3::e() }\n'},
     [('wc3_fromstr_as_e', 'e', None)], deps={'fxdep_wc3': '{ path = "fxdep_wc3" }'}, nocompile=True,
     extra={'@dep/fxdep_wc3/src/lib.rs': 'pub trait Parse { fn from_str(s: &str) -> Self; }\nimpl Parse for String { fn from_str(_s: &str) -> Self { EFF String::new() } }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_wc3'), hdr=False, detail=True,
     note='W-c3: std FromStr `as _` beside a dependency trait declaring from_str: rustc E0034 (compile-fails cell; documents resolver rule 1b)')
pred('wc3', 'wc3_fromstr_as_e', 'record', 'record', 'record')

# wg5 (W-g5): the TWO-HOP chain, consumer C (this crate) -> producer P (`fxdep_wg5p`) -> dependency D (`fxdep_wg5d`), after R1794's `sup/` fixture. D declares a flat trait
# `Flat`, a supertrait `BaseS` and a subtrait `SubS: BaseS`; P is generic over D's traits (every key cell of the design: head bound on the dependency SUBtrait with
# the member in its supertrait, path and method; a LOCAL subtrait over the dependency supertrait (N2); the qself spelling that keys right; a std head with a
# dependency-SUBtrait bound on the projection; a local supertrait (N1)); C supplies an effectful implementor (`Mine`) or a pure one (`Quiet`) and calls P. The
# producer's key must name the trait that DECLARES the member, or C's implementor (published under the declaring trait) never joins and C reads only an
# `Unknown`: `deny Fs` 0 over code that writes. run.py reads three arms for each probe: C alone, C chained against P, and the TWO-HOP arm
# (`<probe>@chained`: D's report, P's report scanned over D's, C scanned over both).
_w5_shapes = [
    # (name, producer fn template over {g} and the trait-family suffix {S}, consumer call template)
    ('flat',    'pub fn {g}<X: Flat{S}>(x: &X) { x.f_go() }',                                   '{p}::{g}(&{T})'),
    ('flatfn',  'pub fn {g}<X: Flat{S}>() { X::f_fn() }',                                       '{p}::{g}::<{T}>()'),
    ('supmeth', 'pub fn {g}<X: SubS{S}>(x: &X) { x.s_go() }',                                   '{p}::{g}(&{T})'),
    ('suppath', 'pub fn {g}<X: SubS{S}>() { X::s_fn() }',                                       '{p}::{g}::<{T}>()'),
    ('supqself', 'pub fn {g}<X: SubS{S}>() { <X as BaseS{S}>::s_fn() }',                        '{p}::{g}::<{T}>()'),
    ('lsdmeth', 'pub fn {g}<X: LSubD{S}>(x: &X) { x.s_go() }',                                  '{p}::{g}(&{T})'),
    ('lsdpath', 'pub fn {g}<X: LSubD{S}>() { X::s_fn() }',                                      '{p}::{g}::<{T}>()'),
    ('proj',    'pub fn {g}<I: Iterator>() where I::Item: SubS{S} { I::Item::s_fn() }',         '{p}::{g}::<It{K}_wg5>()'),
    ('lsup',    'pub fn {g}<X: LSub{S}>() { X::lsf() }',                                        '{p}::{g}::<{T}>()'),
    ('lsupm',   'pub fn {g}<X: LSub{S}>(x: &X) { x.lsm() }',                                    '{p}::{g}(&{T})'),
]
# Two trait FAMILIES: the `_e` probes use the plain traits, implemented only by `Mine` (effectful); the `_q` twins use the `Q`-suffixed copies, implemented only by `Quiet`
# (pure). A twin that shared a trait with `Mine` would read charged on every arm (the join charges every implementor of the keyed trait): a vacuous control.
# `wg5g` is the ONE-VARIABLE variant: the producer P names the dependency's traits through `use dep::*` instead of an explicit import. Same bytes otherwise; measured, the
# producer then publishes no key for its bound and the consumer's `deny Fs` reads 0 over executed writes on v0.43.0 and main (the flat control included).
def w5_cell(cid, glob):
    dn, pn = 'fxdep_%sd' % cid, 'fxdep_%sp' % cid
    d = ''.join('''pub trait Flat%s { fn f_go(&self); fn f_fn(); }
pub trait BaseS%s { fn s_go(&self); fn s_fn(); }
pub trait SubS%s: BaseS%s {}
''' % (S_, S_, S_, S_) for S_ in ('', 'Q'))
    imp = 'use %s::{Flat, FlatQ, BaseS, BaseSQ, SubS, SubSQ};\n' % dn
    p_ = ('use %s::*;\n' % dn if glob else imp) + ''.join('''pub trait LSubD%s: %s::BaseS%s {}
pub trait LSup%s { fn lsf(); fn lsm(&self); } pub trait LSub%s: LSup%s {}
''' % (S_, dn, S_, S_, S_, S_) for S_ in ('', 'Q'))
    m = imp + '''pub struct Mine; pub struct Quiet;
impl Flat for Mine { fn f_go(&self) { EFF } fn f_fn() { EFF } }   impl FlatQ for Quiet { fn f_go(&self) {} fn f_fn() {} }
impl BaseS for Mine { fn s_go(&self) { EFF } fn s_fn() { EFF } }  impl BaseSQ for Quiet { fn s_go(&self) {} fn s_fn() {} }
impl SubS for Mine {}  impl SubSQ for Quiet {}
impl @P@::LSubD for Mine {}  impl @P@::LSubDQ for Quiet {}
impl @P@::LSup for Mine { fn lsf() { EFF } fn lsm(&self) { EFF } }  impl @P@::LSupQ for Quiet { fn lsf() {} fn lsm(&self) {} }
impl @P@::LSub for Mine {}  impl @P@::LSubQ for Quiet {}
pub struct ItE_@C@; impl Iterator for ItE_@C@ { type Item = Mine; fn next(&mut self) -> Option<Mine> { None } }
pub struct ItQ_@C@; impl Iterator for ItQ_@C@ { type Item = Quiet; fn next(&mut self) -> Option<Quiet> { None } }
'''.replace('@P@', pn).replace('@C@', cid)
    probes = []
    for n_, g_, c_ in _w5_shapes:
        for k_, T_, S_ in (('e', 'Mine', ''), ('q', 'Quiet', 'Q')):
            h_ = 'p_%s_%s' % (n_, k_)
            p_ += g_.replace('{g}', h_).replace('{S}', S_) + '\n'
            m += 'pub fn %s_%s_%s() { %s; }\n' % (cid, n_, k_, c_.replace('{p}', pn).replace('{g}', h_).replace('{T}', T_).replace('{K}', k_.upper()).replace('_wg5>', '_%s>' % cid))
            probes.append(('%s_%s_%s' % (cid, n_, k_), k_, None))
    cell(cid, {'main.rs': _ALLOW + m}, probes, deps={dn: '{ path = "%s" }' % dn, pn: '{ path = "%s" }' % pn},
         extra={'@dep/%s/src/lib.rs' % dn: d, '@dep/%s/src/lib.rs' % pn: p_}, dep_deps={pn: {dn: '{ path = "../%s" }' % dn}},
         chain=dict(kind='twohop', dep_dir=dn, mid_dir=pn), hdr=False, detail=True,
         note=('W-g5: consumer -> producer -> dependency; the consumer implements the dependency traits effectfully (Mine) or purely (Quiet), the producer is generic over them'
               + ('; VARIANT: the producer imports the dependency traits by glob' if glob else '')))
    for p2, _, _ in probes:
        pred(cid, p2, 'record', 'record', 'record')


w5_cell('wg5', False)
w5_cell('wg5g', True)


# la1 (lane L1a, rust-bound-l1a): the spellings beside each L1a row that the design matrix does not hold, each a one-variable fixture with executed ground truth.
#   R1961 (N5)  a written `Item = String` binding with a dependency trait in scope and a member no std trait names (`T::Item::f()`), beside its concrete control
#               `String::f()` (bm3_path_dt_e); bm2_bind_stdty_scope_e's `from_str` is withheld by NAME on the concrete spelling too (R1951, lane L3)
#   R1959 (N3)  `<T as Tr>::A::f()` through a STD trait, through a trait that is not a DIRECT bound, through a DEPENDENCY trait (W-p1, kept), over a written binding
#               and over a `where` bound on the qualified projection
#   R1958 (N2)  the projection form (`type A: LSubD;`), and the GUARD: a second local bound that declares the member, so no dependency-supertrait key may be minted
#   R1957 (N1)  two supertrait hops, and the projection form (`type A: LSub;`)
#   R1960 (N4)  the qualified spelling of a driving std I/O member (`<R as Read>::read`) beside its UFCS control
#   R1952       a bare IMPORTED dependency `Error`, and a dependency trait sharing a LOCAL trait's leaf (`dep::Lt` beside `trait Lt`)
_la1_dep = """pub trait Dt { fn f(); }
pub trait SupD { fn sf(); }
pub trait NoSf {}
pub trait HasA { type A; }
pub trait Error { fn ef(); }
pub trait Lt { fn lf(); }
pub struct DImpE; impl Dt for DImpE { fn f() { EFF } } impl Error for DImpE { fn ef() { EFF } } impl Lt for DImpE { fn lf() { EFF } }
pub struct DHas; impl HasA for DHas { type A = DImpE; }
impl Dt for String { fn f() { EFF } }
impl Dt for u16 { fn f() {} }
"""
cell('la1', {'main.rs': _ALLOW + """use fxdep_la1::{SupD, HasA};
pub trait Lt { fn lf(); }
pub trait HasL { type A: Lt; }  pub trait HasLS: HasL {}
pub trait HasX { type A; }
pub trait LSup { fn lsf(); } pub trait LMid: LSup {} pub trait LSub2: LMid {} pub trait LSub: LSup {}
pub trait HasS { type A: LSub; }
pub trait LSubD: fxdep_la1::SupD {}  pub trait HasD { type A: LSubD; }
pub trait LSubN: fxdep_la1::NoSf {}  pub trait LsfHave { fn sf(); }
pub struct LoudL; impl Lt for LoudL { fn lf() { EFF } }
pub struct QuietL_la1; impl Lt for QuietL_la1 { fn lf() {} }
pub struct HL; impl HasL for HL { type A = LoudL; } impl HasLS for HL {}
pub struct HX; impl HasX for HX { type A = LoudL; }
pub struct ItL; impl Iterator for ItL { type Item = LoudL; fn next(&mut self) -> Option<LoudL> { None } }
pub struct LS2; impl LSup for LS2 { fn lsf() { EFF } } impl LMid for LS2 {} impl LSub2 for LS2 {} impl LSub for LS2 {}
pub struct HS; impl HasS for HS { type A = LS2; }
pub struct LDS; impl fxdep_la1::SupD for LDS { fn sf() { EFF } } impl LSubD for LDS {}
pub struct HD; impl HasD for HD { type A = LDS; }
pub struct LN; impl fxdep_la1::NoSf for LN {} impl LSubN for LN {} impl LsfHave for LN { fn sf() { EFF } }
pub struct ItS; impl Iterator for ItS { type Item = String; fn next(&mut self) -> Option<String> { None } }
pub struct ItU; impl Iterator for ItU { type Item = u16; fn next(&mut self) -> Option<u16> { None } }
pub struct ItDE; impl Iterator for ItDE { type Item = fxdep_la1::DImpE; fn next(&mut self) -> Option<fxdep_la1::DImpE> { None } }
pub struct Rd; impl std::io::Read for Rd { fn read(&mut self, _b: &mut [u8]) -> std::io::Result<usize> { EFF Ok(0) } }
fn g_bindstd<T: Iterator<Item = String>>() { use fxdep_la1::Dt; T::Item::f() }
fn g_bindstdq<T: Iterator<Item = u16>>() { use fxdep_la1::Dt; T::Item::f() }
fn g_concrete() { use fxdep_la1::Dt; String::f() }
fn g_qa_std<T: Iterator>() where T::Item: Lt { <T as Iterator>::Item::lf() }
fn g_qa_stdq<T: Iterator>() where T::Item: Lt { <T as Iterator>::Item::lf() }
fn g_qa_sup<T: HasLS>() { <T as HasL>::A::lf() }
fn g_qa_dep<T: HasA>() where T::A: fxdep_la1::Dt { use fxdep_la1::Dt; <T as HasA>::A::f() }
fn g_qa_bind<T: HasX<A = LoudL>>() { <T as HasX>::A::lf() }
fn g_qa_where<T: HasX>() where T::A: Lt { <T as HasX>::A::lf() }
fn g_n2proj<T: HasD>() { T::A::sf() }
fn g_n2guard<T: LSubN + LsfHave>() { T::sf() }
fn g_n1two<T: LSub2>() { T::lsf() }
fn g_n1proj<T: HasS>() { T::A::lsf() }
fn g_qself_read<R: std::io::Read>(mut r: R) { let mut b = [0u8; 1]; let _ = <R as std::io::Read>::read(&mut r, &mut b); }
fn g_ufcs_read<R: std::io::Read>(mut r: R) { let mut b = [0u8; 1]; let _ = std::io::Read::read(&mut r, &mut b); }
fn g_r1952_bare<I: Iterator>() where I::Item: fxdep_la1::Error { use fxdep_la1::Error; I::Item::ef() }
fn g_r1952_bare2<I: Iterator>() where I::Item: Error { I::Item::ef() }
fn g_collide<I: Iterator>() where I::Item: fxdep_la1::Lt { use fxdep_la1::Lt as DLt; I::Item::lf() }
use fxdep_la1::Error;
pub fn la1_bindstd_e() { g_bindstd::<ItS>() }
pub fn la1_bindstd_q() { g_bindstdq::<ItU>() }
pub fn la1_concrete_e() { g_concrete() }
pub fn la1_qa_std_e() { g_qa_std::<ItL>() }
pub fn la1_qa_std_q() { g_qa_stdq::<std::iter::Empty<QuietL_la1>>() }
pub fn la1_qa_sup_e() { g_qa_sup::<HL>() }
pub fn la1_qa_dep_e() { g_qa_dep::<fxdep_la1::DHas>() }
pub fn la1_qa_bind_e() { g_qa_bind::<HX>() }
pub fn la1_qa_where_e() { g_qa_where::<HX>() }
pub fn la1_n2proj_e() { g_n2proj::<HD>() }
pub fn la1_n2guard_e() { g_n2guard::<LN>() }
pub fn la1_n1two_e() { g_n1two::<LS2>() }
pub fn la1_n1proj_e() { g_n1proj::<HS>() }
pub fn la1_qself_read_e() { g_qself_read(Rd) }
pub fn la1_ufcs_read_e() { g_ufcs_read(Rd) }
pub fn la1_r1952_bare_e() { g_r1952_bare::<ItDE>() }
pub fn la1_r1952_bare2_e() { g_r1952_bare2::<ItDE>() }
pub fn la1_collide_e() { g_collide::<ItDE>() }
"""},
     [(n_, n_[-1], None) for n_ in ('la1_bindstd_e', 'la1_bindstd_q', 'la1_concrete_e', 'la1_qa_std_e', 'la1_qa_std_q', 'la1_qa_sup_e', 'la1_qa_dep_e',
                                    'la1_qa_bind_e', 'la1_qa_where_e', 'la1_n2proj_e', 'la1_n2guard_e', 'la1_n1two_e', 'la1_n1proj_e', 'la1_qself_read_e',
                                    'la1_ufcs_read_e', 'la1_r1952_bare_e', 'la1_r1952_bare2_e', 'la1_collide_e')],
     deps={'fxdep_la1': '{ path = "fxdep_la1" }'}, extra={'@dep/fxdep_la1/src/lib.rs': _la1_dep},
     chain=dict(kind='dep', dep_dir='fxdep_la1'), hdr=False, detail=True,
     note='lane L1a: the spellings beside R1952 and R1957-R1961 that the design matrix does not hold')
for _p, _, _ in CELLS['la1']['probes']:
    pred('la1', _p, 'record', 'record', 'record')


# bl1, bl1b, bl2, bl2b (Fable's merge review of L1a round 1, `~/candor-handoff/rust-bound-L1a/fable/fx/`): a SUPERTRAIT's origin is its written ROOT.
#   bl1   a DEPENDENCY supertrait (`trait LSub: fxdep::Sup {}`) sharing its leaf with a crate-local `m::Sup`; `_q`'s instantiation implements the dependency's
#         `Sup` purely, and a local `Noisy` implements the local `Sup` with a write (round 1 dispatched over it: the fabrication)
#   bl1b  the std twin (`trait LSub: std::default::Default` beside a local `m::Default`)
#   bl2   the dependency supertrait written by PATH with no `use` (`trait LSubD: fxdep::SupD {}`): an in-crate impl inheriting the dependency default, one declaring
#   bl2b  the same imported by `use`, with a pure declaring twin
cell('bl1', {'main.rs': _ALLOW + """pub mod m_bl1 { pub trait Sup { fn lsf(); } }
pub trait LSub_bl1: fxdep_bl1::Sup {}
pub struct Noisy_bl1; impl m_bl1::Sup for Noisy_bl1 { fn lsf() { EFF } }
pub struct Quiet_bl1; impl fxdep_bl1::Sup for Quiet_bl1 { fn lsf() {} } impl LSub_bl1 for Quiet_bl1 {}
pub struct Loud_bl1; impl fxdep_bl1::Sup for Loud_bl1 { fn lsf() { EFF } } impl LSub_bl1 for Loud_bl1 {}
fn g_bl1<T: LSub_bl1>() { T::lsf() }
fn g_bl1q<T: LSub_bl1>() { T::lsf() }
pub fn bl1_collide_q() { g_bl1q::<Quiet_bl1>() }
pub fn bl1_collide_e() { g_bl1::<Loud_bl1>() }
"""}, [('bl1_collide_q', 'q', None), ('bl1_collide_e', 'e', None)],
     deps={'fxdep_bl1': '{ path = "fxdep_bl1" }'}, extra={'@dep/fxdep_bl1/src/lib.rs': 'pub trait Sup { fn lsf(); }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_bl1'), hdr=False, detail=True,
     note='Fable L1a review: a dependency supertrait sharing its leaf with a crate-local trait')
cell('bl1b', {'main.rs': _ALLOW + """pub mod m_bl1b { pub trait Default { fn default() -> Self; } }
pub trait LSub_bl1b: std::default::Default {}
pub struct Noisy_bl1b; impl m_bl1b::Default for Noisy_bl1b { fn default() -> Self { EFF Noisy_bl1b } }
#[derive(Default)] pub struct Quiet_bl1b; impl LSub_bl1b for Quiet_bl1b {}
fn g_bl1b<T: LSub_bl1b>() { let _ = T::default(); }
pub fn bl1b_stdcollide_q() { g_bl1b::<Quiet_bl1b>() }
"""}, [('bl1b_stdcollide_q', 'q', None)], hdr=False, detail=True,
     note='Fable L1a review: a std supertrait sharing its leaf with a crate-local trait')
cell('bl2', {'main.rs': _ALLOW + """pub trait LSubD_bl2: fxdep_bl2::SupD {}
pub struct Inh_bl2; impl fxdep_bl2::SupD for Inh_bl2 {} impl LSubD_bl2 for Inh_bl2 {}
pub struct Decl_bl2; impl fxdep_bl2::SupD for Decl_bl2 { fn sf() { EFF } } impl LSubD_bl2 for Decl_bl2 {}
fn g_inh<T: LSubD_bl2>() { T::sf() }
fn g_decl<T: LSubD_bl2>() { T::sf() }
pub fn bl2_inherit_e() { g_inh::<Inh_bl2>() }
pub fn bl2_declares_e() { g_decl::<Decl_bl2>() }
"""}, [('bl2_inherit_e', 'e', None), ('bl2_declares_e', 'e', None)],
     deps={'fxdep_bl2': '{ path = "fxdep_bl2" }'}, extra={'@dep/fxdep_bl2/src/lib.rs': 'pub trait SupD { fn sf() { EFF } }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_bl2'), hdr=False, detail=True,
     note='Fable L1a review: a dependency supertrait written by path, no use')
cell('bl2b', {'main.rs': _ALLOW + """use fxdep_bl2b::SupD;
pub trait LSubD_bl2b: SupD {}
pub struct Inh_bl2b; impl SupD for Inh_bl2b {} impl LSubD_bl2b for Inh_bl2b {}
pub struct Decl_bl2b; impl SupD for Decl_bl2b { fn sf() { EFF } } impl LSubD_bl2b for Decl_bl2b {}
pub struct DeclQ_bl2b; impl SupD for DeclQ_bl2b { fn sf() {} } impl LSubD_bl2b for DeclQ_bl2b {}
fn g_inh<T: LSubD_bl2b>() { T::sf() }
fn g_decl<T: LSubD_bl2b>() { T::sf() }
fn g_declq<T: LSubD_bl2b>() { T::sf() }
pub fn bl2b_inherit_e() { g_inh::<Inh_bl2b>() }
pub fn bl2b_declares_e() { g_decl::<Decl_bl2b>() }
pub fn bl2b_declares_q() { g_declq::<DeclQ_bl2b>() }
"""}, [('bl2b_inherit_e', 'e', None), ('bl2b_declares_e', 'e', None), ('bl2b_declares_q', 'q', None)],
     deps={'fxdep_bl2b': '{ path = "fxdep_bl2b" }'}, extra={'@dep/fxdep_bl2b/src/lib.rs': 'pub trait SupD { fn sf() { EFF } }\n'},
     chain=dict(kind='dep', dep_dir='fxdep_bl2b'), hdr=False, detail=True,
     note='Fable L1a review: a dependency supertrait imported by use')
# la2 (round 2's own control, the object-0.37.3 corpus shape): a supertrait written through a path rooted at a module THIS module declares
# (`use id_private::IdPrivate; trait Id: IdPrivate {}`) is the crate's own trait, never a dependency's, though no `crate::` spells it
cell('la2', {'main.rs': _ALLOW + """mod id_private { pub trait IdPrivate { fn new_la2() -> Self; } }
use id_private::IdPrivate;
pub trait Id_la2: IdPrivate {}
pub trait Item_la2 { type Id: Id_la2; }
pub struct IdE_la2; impl IdPrivate for IdE_la2 { fn new_la2() -> Self { EFF IdE_la2 } } impl Id_la2 for IdE_la2 {}
pub struct ItE_la2; impl Item_la2 for ItE_la2 { type Id = IdE_la2; }
fn g_la2<T: Item_la2>() { let _ = T::Id::new_la2(); }
fn g2_la2<T: Id_la2>() { let _ = T::new_la2(); }
pub fn la2_localmod_proj_e() { g_la2::<ItE_la2>() }
pub fn la2_localmod_path_e() { g2_la2::<IdE_la2>() }
"""}, [('la2_localmod_proj_e', 'e', None), ('la2_localmod_path_e', 'e', None)], hdr=False, detail=True,
     note='lane L1a round 2: a supertrait rooted at a module this module declares is local')
for _c in ('bl1', 'bl1b', 'bl2', 'bl2b', 'la2'):
    for _p, _, _ in CELLS[_c]['probes']:
        pred(_c, _p, 'record', 'record', 'record')


# ----------------------------------------------------------------------------------------------- rendering
def dispatcher(c, crate_ref):
    arms = ''.join('        "%s" => %s%s(),\n' % (p, crate_ref, p) for p, _, _ in c['probes'])
    return 'fn main() {\n    match std::env::args().nth(1).unwrap().as_str() {\n%s        _ => panic!("unknown probe"),\n    }\n}\n' % arms


def cargo_toml(c, name, deps, lib_only=False, is_dep=False):
    s = '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "%s"\n\n' % (name, '2021' if is_dep else c.get('edition', '2021'))
    s += '[dependencies]\n' + ''.join('%s = %s\n' % (k, v) for k, v in deps.items())
    if not is_dep:
        s += '\n[workspace]\n'
    return s


def render(out):
    cells_meta = {}
    for cid, c in sorted(CELLS.items()):
        d = os.path.join(out, 'fx', cid)
        os.makedirs(os.path.join(d, 'src'), exist_ok=True)
        deps = dict(c['deps'])
        open(os.path.join(d, 'Cargo.toml'), 'w').write(cargo_toml(c, 'cf_' + cid, deps))
        files = dict(c['files'])
        body_name = 'lib.rs' if c['lib'] else 'main.rs'
        body = files[body_name]
        probes_def = ''
        for p, k, b in c['probes']:
            if b is not None:
                probes_def += 'pub fn %s() { %s }\n' % (p, b if b.rstrip().endswith((';', '}')) else b + ';')
        text = (HDR.replace('EFF', EFF) if c.get('hdr', True) else '') + body.replace('EFF', EFF) + '\n' + probes_def
        if c['lib']:
            open(os.path.join(d, 'src', 'lib.rs'), 'w').write(text)
            open(os.path.join(d, 'src', 'main.rs'), 'w').write(dispatcher(c, 'cf_%s::' % cid))
        else:
            open(os.path.join(d, 'src', 'main.rs'), 'w').write(text + '\n' + dispatcher(c, ''))
        for rel, txt in c['extra'].items():
            txt = txt.replace('EFF', EFF)
            if rel.startswith('@dep/'):
                parts = rel[len('@dep/'):].split('/', 1)
                dname, drel = parts[0], parts[1]
                dd = os.path.join(d, dname)
                os.makedirs(os.path.dirname(os.path.join(dd, drel)), exist_ok=True)
                open(os.path.join(dd, drel), 'w').write(txt)
                open(os.path.join(dd, 'Cargo.toml'), 'w').write(cargo_toml(c, dname, c.get('dep_deps', {}).get(dname, {}), is_dep=True))
            elif rel.startswith('@edit/'):
                p = os.path.join(d, 'edit', rel[len('@edit/'):])
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, 'w').write(txt)
            else:
                p = os.path.join(d, 'src', rel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, 'w').write(txt)
        ch = c['chain']
        if ch and ch['kind'] == 'consumer':
            cd = os.path.join(d, 'cons')
            os.makedirs(os.path.join(cd, 'src'), exist_ok=True)
            open(os.path.join(cd, 'Cargo.toml'), 'w').write(
                cargo_toml(c, 'cons_' + cid, {ch['dep_name']: '{ package = "%s", path = ".." }' % ch['dep_package']}))
            for rel, txt in ch['consumer_files'].items():
                open(os.path.join(cd, rel), 'w').write('#![allow(unused)]\n' + txt)
            open(os.path.join(cd, 'src', 'main.rs'), 'w').write(
                'fn main() {\n    match std::env::args().nth(1).unwrap().as_str() {\n        "%s" => cons_%s::%s(),\n        _ => panic!("unknown probe"),\n    }\n}\n'
                % (ch['consumer_probe'], cid, ch['consumer_probe']))
        meta = dict(id=cid, lib=c['lib'], probes=[dict(name=p, kind=k) for p, k, _ in c['probes']], chain=ch, note=c['note'],
                    deps=sorted(deps))
        if c.get('edition', '2021') != '2021':
            meta['edition'] = c['edition']
        if c.get('nocompile'):
            meta['nocompile_ok'] = True
        if c.get('detail'):
            meta['detail'] = True
        open(os.path.join(d, 'cell.json'), 'w').write(json.dumps(meta, indent=1, sort_keys=True) + '\n')
        open(os.path.join(d, 'probes.txt'), 'w').write('\n'.join(p for p, _, _ in c['probes']) + '\n')
        cells_meta[cid] = meta
    with open(os.path.join(out, 'predict.tsv'), 'w') as f:
        f.write('# cell\tprobe\tpub\tmain\tcand_accept   (generated by gen.py from MATRIX.md; `-` = absent or no effect/Unknown;\n'
                '# `|` alternatives; `U*` = U or a named residual; `=pub` = must equal the pub reading; `record` = no acceptance)\n')
        for r in PREDICT:
            f.write('\t'.join(r) + '\n')


def main():
    if '--check' in sys.argv:
        t = tempfile.mkdtemp(prefix='cfgtwin-check-')
        render(t)
        import subprocess
        r = subprocess.run(['diff', '-r', os.path.join(t, 'fx'), os.path.join(HERE, 'fx')], capture_output=True, text=True)
        r2 = subprocess.run(['diff', os.path.join(t, 'predict.tsv'), os.path.join(HERE, 'predict.tsv')], capture_output=True, text=True)
        shutil.rmtree(t)
        if r.returncode or r2.returncode:
            print((r.stdout + r2.stdout)[:2000])
            print('gen.py --check: checked-in fixtures differ from a fresh render')
            sys.exit(1)
        print('gen.py --check: %d cells identical' % len(CELLS))
        return
    shutil.rmtree(os.path.join(HERE, 'fx'), ignore_errors=True)
    render(HERE)
    print('rendered %d cells' % len(CELLS))


if __name__ == '__main__':
    main()
