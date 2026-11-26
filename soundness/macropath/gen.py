#!/usr/bin/env python3
"""gen.py — render the macro-path attack matrix group B (MATRIX.md, rust-macropath design) into checked-in fixture crates.

    python3 soundness/macropath/gen.py            # (re)render fx/ in place
    python3 soundness/macropath/gen.py --check    # render to a temp dir and diff against the checked-in tree

Same shape as soundness/cfgtwin: one BIN crate per cell, fx/<cell>/{Cargo.toml,src/*.rs,cell.json}; `main()` dispatches argv[1] to
ONE probe, so a marker file written (or not) under $MARK is that probe's executed ground truth. `run.py` (a thin wrapper over
cfgtwin's runner) reads fx/ and expect.tsv from THIS directory.

Group B = CALLER spellings of a name declared by a local item / impl-item / trait-item macro invocation that the engine leaves
UNEXPANDED (the path-qualified `self::mkfn!{ go }` is never bound, `Ctx::local_name` takes `get_ident()` only). Every writer
probe `<id>_e` has the SAME spelling over a pure twin `<id>_q` (unique names per cell). Readings: `-` ABSENT or no effect and no
Unknown, `E` concrete effect, `U` Unknown, `E+U` both.
"""
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
W = '::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap();'

CELLS = {}


def cell(cid, main, extra=None, probes=(), deps=None, chain=None, note=''):
    assert cid not in CELLS, cid
    CELLS[cid] = dict(id=cid, main=main, extra=extra or {}, probes=list(probes), deps=deps or {}, chain=chain, note=note)


def mk(cid, tmpl):
    return tmpl.replace('{id}', cid)


# defs.rs: the REFUSED path — a `pub(crate) use` re-export, reached by `self::mkfn!` from `inner` (not a bare ident, not a root head)
DEFS = '''macro_rules! mkfn_{id} { ($n:ident) => { pub fn $n() { W } } } pub(crate) use mkfn_{id};
macro_rules! mkfnp_{id} { ($n:ident) => { pub fn $n() {} } } pub(crate) use mkfnp_{id};
macro_rules! mkm_{id} { ($n:ident) => { pub fn $n(&self) { W } } } pub(crate) use mkm_{id};
macro_rules! mkmp_{id} { ($n:ident) => { pub fn $n(&self) {} } } pub(crate) use mkmp_{id};
macro_rules! mktr_{id} { ($n:ident) => { fn $n(&self) { W } } } pub(crate) use mktr_{id};
macro_rules! mktrp_{id} { ($n:ident) => { fn $n(&self) {} } } pub(crate) use mktrp_{id};
'''

FN_INNER = '''use crate::defs::{mkfn_{id}, mkfnp_{id}};
self::mkfn_{id}!{ go_{id} }
self::mkfnp_{id}!{ qgo_{id} }
'''


def root(cid, body, mods='mod inner; mod defs;'):
    return mods + '\n' + body


def fn_cell(cid, root_body, inner_extra='', extra=None, probes=None, note='', mods='mod inner; mod defs;'):
    ex = {'defs.rs': mk(cid, DEFS), 'inner.rs': mk(cid, FN_INNER + inner_extra)}
    ex.update(extra or {})
    cell(cid, mk(cid, root_body) + '\n', ex, probes, note=note)


# B1 path-qualified (R128 already hedges it): the control that must stay HEDGED
fn_cell('l1path', 'mod inner; mod defs;', probes=[('l1path_e', 'e', 'inner::go_l1path()'), ('l1path_q', 'q', 'inner::qgo_l1path()')],
        note='B1: path-qualified caller (R128).')
# B2 bare same-module caller (i1)
fn_cell('l1bare', 'mod inner; mod defs;',
        inner_extra='pub fn p_inner_{id}() { go_{id}() }\npub fn q_inner_{id}() { qgo_{id}() }\n'
                    'pub fn {id}_d() { go_{id}() }\npub fn {id}_dq() { qgo_{id}() }\n',
        probes=[('l1bare_e', 'e', 'inner::p_inner_l1bare()'), ('l1bare_q', 'q', 'inner::q_inner_l1bare()'),
                ('l1bare_d', 'e', None), ('l1bare_dq', 'q', None)],
        note='B2: bare same-module caller (design i1); l1bare_d is the direct caller inside the module.')
CELLS['l1bare']['main'] += 'pub use inner::{l1bare_d, l1bare_dq};\n'
# B3 `use`-imported caller (i2)
fn_cell('l1use', 'mod inner; mod defs;\nuse inner::{go_l1use, qgo_l1use};',
        probes=[('l1use_e', 'e', 'go_l1use()'), ('l1use_q', 'q', 'qgo_l1use()')], note='B3: `use inner::go; go()` (design i2).')
# B4 renamed import
fn_cell('l1ren', 'mod inner; mod defs;\nuse inner::go_l1ren as g_l1ren;\nuse inner::qgo_l1ren as qg_l1ren;',
        probes=[('l1ren_e', 'e', 'g_l1ren()'), ('l1ren_q', 'q', 'qg_l1ren()')], note='B4: `use inner::go as g; g()`.')
# B5 glob import
fn_cell('l1glob', 'mod inner; mod defs;\nuse inner::*;',
        probes=[('l1glob_e', 'e', 'go_l1glob()'), ('l1glob_q', 'q', 'qgo_l1glob()')], note='B5: `use inner::*; go()`.')
# B6 caller in ANOTHER file
fn_cell('l1file', 'mod inner; mod defs; mod other;',
        extra={'other.rs': 'use crate::inner::{go_l1file, qgo_l1file};\npub fn via_l1file() { go_l1file() }\npub fn qvia_l1file() { qgo_l1file() }\n'},
        probes=[('l1file_e', 'e', 'other::via_l1file()'), ('l1file_q', 'q', 'other::qvia_l1file()')], note='B6: caller in another file.')
# B7 inherent impl member
cell('l1impl', 'mod inner; mod defs;\n',
     {'defs.rs': mk('l1impl', DEFS),
      'inner.rs': mk('l1impl', 'use crate::defs::{mkm_{id}, mkmp_{id}};\npub struct S_{id};\nimpl S_{id} { self::mkm_{id}!{ go_{id} } }\n'
                               'pub struct SQ_{id};\nimpl SQ_{id} { self::mkmp_{id}!{ qgo_{id} } }\n')},
     [('l1impl_e', 'e', 'inner::S_l1impl.go_l1impl()'), ('l1impl_q', 'q', 'inner::SQ_l1impl.qgo_l1impl()')],
     note='B7: method on a type whose inherent impl holds an unexpanded impl-item invocation (design fx2 call_m_crate).')
# B8 UFCS over B7's impl
cell('l1ufcs', 'mod inner; mod defs;\n',
     {'defs.rs': mk('l1ufcs', DEFS),
      'inner.rs': mk('l1ufcs', 'use crate::defs::{mkm_{id}, mkmp_{id}};\npub struct S_{id};\nimpl S_{id} { self::mkm_{id}!{ go_{id} } }\n'
                               'pub struct SQ_{id};\nimpl SQ_{id} { self::mkmp_{id}!{ qgo_{id} } }\n')},
     [('l1ufcs_e', 'e', 'inner::S_l1ufcs::go_l1ufcs(&inner::S_l1ufcs)'), ('l1ufcs_q', 'q', 'inner::SQ_l1ufcs::qgo_l1ufcs(&inner::SQ_l1ufcs)')],
     note='B8: UFCS call over the B7 impl.')
# B9 trait-item
TRI = ('use crate::defs::{mktr_{id}, mktrp_{id}};\npub struct S_{id};\npub struct SQ_{id};\n'
       'pub trait T_{id} { self::mktr_{id}!{ go_{id} } }\nimpl T_{id} for S_{id} {}\n'
       'pub trait TQ_{id} { self::mktrp_{id}!{ qgo_{id} } }\nimpl TQ_{id} for SQ_{id} {}\n')
cell('l1trait', 'mod inner; mod defs;\n', {'defs.rs': mk('l1trait', DEFS), 'inner.rs': mk('l1trait', TRI)},
     [('l1trait_e', 'e', '<inner::S_l1trait as inner::T_l1trait>::go_l1trait(&inner::S_l1trait)'),
      ('l1trait_q', 'q', '<inner::SQ_l1trait as inner::TQ_l1trait>::qgo_l1trait(&inner::SQ_l1trait)')],
     note='B9: UFCS call of a trait-item-minted default method (design fx2 call_t_crate).')
cell('l1traitm', 'mod inner; mod defs;\n', {'defs.rs': mk('l1traitm', DEFS), 'inner.rs': mk('l1traitm', TRI)},
     [('l1traitm_e', 'e', '{ use inner::T_l1traitm; inner::S_l1traitm.go_l1traitm() }'),
      ('l1traitm_q', 'q', '{ use inner::TQ_l1traitm; inner::SQ_l1traitm.qgo_l1traitm() }')],
     note='B9: method-call spelling of the trait-item-minted default method.')
# B10 tlpath: a glob-supplied dependency macro invoked by PATH at the root (opaque arm; recorded, not this lane's)
DEP_TL = ('#[macro_export] macro_rules! thread_local { (static $n:ident : $t:ty = $e:expr ;) => { pub fn tl_touch() { $crate::emit() } } }\n'
          '#[macro_export] macro_rules! mkq { (static $n:ident : $t:ty = $e:expr ;) => { pub fn qtl_touch() {} } }\n')
DEP_LIB = 'pub fn emit() { ' + W + ' }\n'
cell('tlpath', 'use fxdeptlpath::*;\ncrate::thread_local! { static X_tlpath: u8 = 0; }\ncrate::mkq! { static Y_tlpath: u8 = 0; }\n',
     {'@dep/fxdeptlpath/src/lib.rs': DEP_LIB + DEP_TL},
     [('tlpath_e', 'e', 'tl_touch()'), ('tlpath_q', 'q', 'qtl_touch()')],
     deps={'fxdeptlpath': '{ path = "fxdeptlpath" }'}, chain=dict(kind='dep', dep_dir='fxdeptlpath'),
     note='B10: a glob-supplied DEPENDENCY macro minting the callee (R1908 vein; foreign-opaque arm, not L1(a)).')
# B11 dependency ITEM macro by path (L1(b2) counter cell; recorded, not this lane's)
DEP_MKFN = ('#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { $crate::emit() } } }\n'
            '#[macro_export] macro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n')
cell('l1dep', 'mod inner;\n',
     {'@dep/fxdepl1dep/src/lib.rs': DEP_LIB + DEP_MKFN,
      'inner.rs': 'fxdepl1dep::mkfn!{ go_l1dep }\nfxdepl1dep::mkfnp!{ qgo_l1dep }\npub fn p_inner_l1dep() { go_l1dep() }\npub fn q_inner_l1dep() { qgo_l1dep() }\n'},
     [('l1dep_e', 'e', 'inner::p_inner_l1dep()'), ('l1dep_q', 'q', 'inner::q_inner_l1dep()')],
     deps={'fxdepl1dep': '{ path = "fxdepl1dep" }'}, chain=dict(kind='dep', dep_dir='fxdepl1dep'),
     note='B11: dependency item macro by path; its minted name is unknowable (L1(b2): NOT this lane).')
# B12 let-bound closure of the same name beside hidden text (control: a local binding, not the hidden fn)
fn_cell('l1let', 'mod inner; mod defs;',
        inner_extra='pub fn p_inner_{id}() { let go_{id} = || (); go_{id}() }\n',
        probes=[('l1let_q', 'q', 'inner::p_inner_l1let()')], note='B12: a let-bound closure named like the hidden fn; PURE (executed).')
# B13 expanded control (design i3): a bare macro in textual scope IS expanded
cell('l1expanded', '#[macro_export] macro_rules! mkfn_l1expanded { ($n:ident) => { pub fn $n() { W } } }\n'
                   '#[macro_export] macro_rules! mkfnp_l1expanded { ($n:ident) => { pub fn $n() {} } }\nmod inner;\n'.replace('W', W),
     {'inner.rs': 'mkfn_l1expanded!{ go_l1expanded }\nmkfnp_l1expanded!{ qgo_l1expanded }\n'
                  'pub fn p_inner_l1expanded() { go_l1expanded() }\npub fn q_inner_l1expanded() { qgo_l1expanded() }\n'},
     [('l1expanded_e', 'e', 'inner::p_inner_l1expanded()'), ('l1expanded_q', 'q', 'inner::q_inner_l1expanded()')],
     note='B13 (design i3): bare `mkfn!{ go }` is expanded; CHARGED on every arm, no hedge.')

# ---- cells beyond MATRIX group B, written by the fix lane to attack its own change (executed like the rest) ----
# X1 the invocation at the crate ROOT (module qual ""), bare caller in the root
cell('l1root', 'mod defs;\ncrate::defs::mkfn_l1root! { go_l1root }\ncrate::defs::mkfnp_l1root! { qgo_l1root }\n',
     {'defs.rs': mk('l1root', DEFS)},
     [('l1root_e', 'e', 'go_l1root()'), ('l1root_q', 'q', 'qgo_l1root()')], note='X1: hidden text at the crate root, bare caller in the root.')
# X2 a path headed by a module declared in the SAME item list (`defs::mk!`)
cell('l1modhead', 'mod defs;\ndefs::mkfn_l1modhead! { go_l1modhead }\ndefs::mkfnp_l1modhead! { qgo_l1modhead }\n',
     {'defs.rs': mk('l1modhead', DEFS)},
     [('l1modhead_e', 'e', 'go_l1modhead()'), ('l1modhead_q', 'q', 'qgo_l1modhead()')], note='X2: invocation headed by a module of the same item list.')
# X3 nested hidden module, caller by relative and absolute path from the parent
cell('l1deep', 'mod inner; mod defs;\n', {'defs.rs': mk('l1deep', DEFS),
     'inner.rs': mk('l1deep', 'pub mod deep { use crate::defs::{mkfn_{id}, mkfnp_{id}}; self::mkfn_{id}!{ go_{id} } self::mkfnp_{id}!{ qgo_{id} } }\n'
                              'pub fn rel_{id}() { deep::go_{id}() }\npub fn relq_{id}() { deep::qgo_{id}() }\n')},
     [('l1deep_e', 'e', 'inner::rel_l1deep()'), ('l1deep_q', 'q', 'inner::relq_l1deep()'),
      ('l1deepabs_e', 'e', 'inner::deep::go_l1deep()'), ('l1deepabs_q', 'q', 'inner::deep::qgo_l1deep()')],
     note='X3: hidden text in a nested module; relative and absolute path callers.')
# X4 caller inside a closure and via a fn-pointer value
fn_cell('l1clos', 'mod inner; mod defs;\nuse inner::{go_l1clos, qgo_l1clos};',
        probes=[('l1clos_e', 'e', '(|| go_l1clos())()'), ('l1clos_q', 'q', '(|| qgo_l1clos())()'),
                ('l1fnref_e', 'e', '{ let f: fn() = go_l1clos; f() }'), ('l1fnref_q', 'q', '{ let f: fn() = qgo_l1clos; f() }')],
        note='X4: the hidden name called in a closure / taken as a fn pointer.')
# X5 trait IMPL member minted by hidden impl-item text (R1245 owns it)
cell('l1timpl', 'mod inner; mod defs;\n', {'defs.rs': mk('l1timpl', DEFS),
     'inner.rs': mk('l1timpl', 'use crate::defs::{mktr_{id}, mktrp_{id}};\npub trait Tr_{id} { fn go_{id}(&self); }\npub trait TrQ_{id} { fn qgo_{id}(&self); }\n'
                               'pub struct S_{id};\nimpl Tr_{id} for S_{id} { self::mktr_{id}!{ go_{id} } }\n'
                               'pub struct SQ_{id};\nimpl TrQ_{id} for SQ_{id} { self::mktrp_{id}!{ qgo_{id} } }\n')},
     [('l1timpl_e', 'e', '{ use inner::Tr_l1timpl; inner::S_l1timpl.go_l1timpl() }'), ('l1timpl_q', 'q', '{ use inner::TrQ_l1timpl; inner::SQ_l1timpl.qgo_l1timpl() }')],
     note='X5: trait IMPL whose member is minted by hidden impl-item text (R1245).')
# X6 hidden text in ANOTHER file's module, `use`-imported caller in the root
cell('l1hfile', 'mod defs; mod hid;\nuse hid::{go_l1hfile, qgo_l1hfile};\n', {'defs.rs': mk('l1hfile', DEFS),
     'hid.rs': mk('l1hfile', 'use crate::defs::{mkfn_{id}, mkfnp_{id}};\nself::mkfn_{id}!{ go_{id} }\nself::mkfnp_{id}!{ qgo_{id} }\n')},
     [('l1hfile_e', 'e', 'go_l1hfile()'), ('l1hfile_q', 'q', 'qgo_l1hfile()')], note='X6: hidden text in another file, `use`d by the root.')
# X7 foreign macro of the SAME LEAF as a local one: its names are unknowable, not read from the local body (opaque arm; not this lane's)
DEP_COLL = ('#[macro_export] macro_rules! mkfn_l1collide { ($n:ident) => { pub fn $n() { $crate::emit() } } }\n')
cell('l1collide', 'mod inner; mod defs;\n',
     {'@dep/fxdepl1collide/src/lib.rs': DEP_LIB + DEP_COLL, 'defs.rs': mk('l1collide', DEFS),
      'inner.rs': 'fxdepl1collide::mkfn_l1collide!{ go_l1collide }\npub fn p_inner_l1collide() { go_l1collide() }\n'},
     [('l1collide_e', 'e', 'inner::p_inner_l1collide()')], deps={'fxdepl1collide': '{ path = "fxdepl1collide" }'},
     chain=dict(kind='dep', dep_dir='fxdepl1collide'),
     note='X7: a dependency macro whose leaf equals a local macro (defs.rs): the local body must NOT be read for it (L1(b2), not this lane).')
# X8 a visible fn of the SAME NAME as the hidden one's twin in another module, called bare there (must not hedge)
cell('l1other', 'mod inner; mod defs; mod plain;\n', {'defs.rs': mk('l1other', DEFS), 'inner.rs': mk('l1other', FN_INNER),
     'plain.rs': 'pub fn go_l1other() { ' + W + ' }\npub fn call_l1other() { go_l1other() }\n'},
     [('l1other_e', 'e', 'plain::call_l1other()')],
     note='X8: a VISIBLE fn named like a hidden one in ANOTHER module; the caller there is CHARGED, not hedged.')

# ---- R1911 (rust-l1b-r1911): a dependency EXPRESSION macro whose expansion calls the dependency's effectful fn. A macro is not a
# unit of the dependency's report, so chaining that report must not delete the standalone `invisible:[D]` disclosure. Every cell is
# CHAINED (`@chained` rows); the writer `_e` and the pure twin `_q` carry the SAME bytes on every arm (the engine cannot tell them
# apart), so the reading is a disclosure, never a verdict: both read `I` (`[]` + `invisible`) standalone AND chained.
DEP_EXPR = (DEP_LIB + 'pub fn pure_{id}() -> u8 { 7 }\n'
            '#[macro_export] macro_rules! mk { () => { $crate::emit() } }\n'
            '#[macro_export] macro_rules! nop { () => { () } }\n')
# S1 `D::mk!()` by path
cell('s1path', 'mod inner;\n',
     {'@dep/fxdeps1path/src/lib.rs': DEP_EXPR.replace('{id}', 's1path'),
      'inner.rs': 'pub fn go_s1path() { fxdeps1path::mk!() }\npub fn qgo_s1path() { fxdeps1path::nop!() }\n'
                  'pub fn via_s1path() { go_s1path() }\npub fn qvia_s1path() { qgo_s1path() }\n'},
     [('s1path_e', 'e', 'inner::via_s1path()'), ('s1path_q', 'q', 'inner::qvia_s1path()')],
     deps={'fxdeps1path': '{ path = "fxdeps1path" }'}, chain=dict(kind='dep', dep_dir='fxdeps1path'),
     note='S1 (R1911): `D::mk!()` in expression position, one caller hop away. Chained, the disclosure must survive.')
# S2 `use D::mk; mk!()`
cell('s1use', 'mod inner;\n',
     {'@dep/fxdeps1use/src/lib.rs': DEP_EXPR.replace('{id}', 's1use'),
      'inner.rs': 'use fxdeps1use::{mk, nop};\npub fn go_s1use() { mk!() }\npub fn qgo_s1use() { nop!() }\n'},
     [('s1use_e', 'e', 'inner::go_s1use()'), ('s1use_q', 'q', 'inner::qgo_s1use()')],
     deps={'fxdeps1use': '{ path = "fxdeps1use" }'}, chain=dict(kind='dep', dep_dir='fxdeps1use'),
     note='S2 (R1911): `use D::mk; mk!()`.')
# S3 control: an ORDINARY pure fn of the same chained dependency is answered by the join and stays ABSENT (no over-disclosure),
# beside a macro caller in the same crate that is disclosed.
cell('s1fn', 'mod inner;\n',
     {'@dep/fxdeps1fn/src/lib.rs': DEP_EXPR.replace('{id}', 's1fn'),
      'inner.rs': 'pub fn go_s1fn() { fxdeps1fn::mk!() }\npub fn fnonly_s1fn() -> u8 { fxdeps1fn::pure_s1fn() }\n'},
     [('s1fn_e', 'e', 'inner::go_s1fn()'), ('s1fn_q', 'q', '{ inner::fnonly_s1fn(); }')],
     deps={'fxdeps1fn': '{ path = "fxdeps1fn" }'}, chain=dict(kind='dep', dep_dir='fxdeps1fn'),
     note='S3 (R1911 control): a pure dependency FN call is answered by the chained report and reads ABSENT; only the macro caller is disclosed.')


def cargo_toml(name, deps, is_dep=False):
    s = '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' % name
    s += ''.join('%s = %s\n' % (k, v) for k, v in deps.items())
    if not is_dep:
        s += '\n[workspace]\n'
    return s


def render(out):
    for cid, c in sorted(CELLS.items()):
        d = os.path.join(out, 'fx', cid)
        os.makedirs(os.path.join(d, 'src'), exist_ok=True)
        open(os.path.join(d, 'Cargo.toml'), 'w').write(cargo_toml('cf_' + cid, c['deps']))
        decl = ''.join('pub fn %s() { %s; }\n' % (p, b if b.rstrip().endswith('}') and False else b) for p, k, b in c['probes'] if b is not None)
        arms = ''.join('        "%s" => %s(),\n' % (p, p) for p, _, _ in c['probes'])
        disp = 'fn main() {\n    match std::env::args().nth(1).unwrap().as_str() {\n%s        _ => panic!("unknown probe"),\n    }\n}\n' % arms
        main = '#![allow(unused)]\n' + c['main'].replace('W;', W) + decl + disp
        main = main.replace('{ W }', '{ ' + W + ' }')
        open(os.path.join(d, 'src', 'main.rs'), 'w').write(main)
        for rel, txt in c['extra'].items():
            txt = txt.replace('{ W }', '{ ' + W + ' }')
            if rel.startswith('@dep/'):
                dname, drel = rel[len('@dep/'):].split('/', 1)
                p = os.path.join(d, dname, drel)
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, 'w').write(txt)
                open(os.path.join(d, dname, 'Cargo.toml'), 'w').write(cargo_toml(dname, {}, is_dep=True))
            else:
                p = os.path.join(d, 'src', rel)
                open(p, 'w').write('#![allow(unused)]\n' + txt if rel.endswith('.rs') and False else txt)
        meta = dict(id=cid, lib=False, probes=[dict(name=p, kind=k) for p, k, _ in c['probes']], chain=c['chain'], note=c['note'],
                    deps=sorted(c['deps']))
        open(os.path.join(d, 'cell.json'), 'w').write(json.dumps(meta, indent=1, sort_keys=True) + '\n')
        open(os.path.join(d, 'probes.txt'), 'w').write('\n'.join(p for p, _, _ in c['probes']) + '\n')


def main():
    if '--check' in sys.argv:
        import subprocess
        t = tempfile.mkdtemp(prefix='macropath-check-')
        render(t)
        r = subprocess.run(['diff', '-r', os.path.join(t, 'fx'), os.path.join(HERE, 'fx')], capture_output=True, text=True)
        shutil.rmtree(t)
        if r.returncode:
            print(r.stdout[:2000])
            print('gen.py --check: checked-in fixtures differ from a fresh render')
            sys.exit(1)
        print('gen.py --check: %d cells identical' % len(CELLS))
        return
    shutil.rmtree(os.path.join(HERE, 'fx'), ignore_errors=True)
    render(HERE)
    print('rendered %d cells' % len(CELLS))


if __name__ == '__main__':
    main()
