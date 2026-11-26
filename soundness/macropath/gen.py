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


# ======================================================================================================================
# Groups C-G (MATRIX.md, rust-macropath design): L2 body / item positions, L3 toolchain names x crate kind, the must-not-compile
# cells. Same generator, same runner, same expect.tsv. Conventions of the design's section 0 are kept where this generator's
# probe shape allows: every writer probe `<id>_e` has a twin `<id>_q` over a PURE definition, names are unique per cell
# (`go_<id>`, `q_<id>_pure`), a cell's dependency is the nested path crate `fxdep<id>`.  The `cand` column stays empty.
# ======================================================================================================================
W0 = '::std::fs::write(::std::env::var("MARK").unwrap(), b"x").unwrap()'   # no trailing `;`: it is spliced into expression position


def sb(t, cid):
    return t.replace('@W@', W0).replace('@ID@', cid)


EXP_W = '#[macro_export] macro_rules! mk { () => { @W@ } }\n'
EXP_P = '#[macro_export] macro_rules! mk { () => { () } }\n'
EXP_NOP = '#[macro_export] macro_rules! mknop { () => { () } }\n'
WRAPS = ('#[macro_export] macro_rules! wrap { () => { $crate::mk!() } }\n'
         '#[macro_export] macro_rules! wrapnop { () => { $crate::mknop!() } }\n')
MKFN_W = '#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { @W@ } } }\n'
MKFN_P = '#[macro_export] macro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n'
MKFN_PN = '#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() {} } }\n'      # a PURE macro named like the writer
MKM_W = '#[macro_export] macro_rules! mkm { ($n:ident) => { pub fn $n(&self) { @W@ } } }\n'
MKM_P = '#[macro_export] macro_rules! mkmp { ($n:ident) => { pub fn $n(&self) {} } }\n'
MKM_PN = '#[macro_export] macro_rules! mkm { ($n:ident) => { pub fn $n(&self) {} } }\n'
MKTR_W = '#[macro_export] macro_rules! mktr { ($n:ident) => { fn $n(&self) { @W@ } } }\n'
MKTR_P = '#[macro_export] macro_rules! mktrp { ($n:ident) => { fn $n(&self) {} } }\n'
DEP_BASE = 'pub fn emit() { @W@ }\n#[macro_export] macro_rules! nop { () => { () } }\n'
DEP_MK_W = '#[macro_export] macro_rules! mk { () => { $crate::emit() } }\n'
DEP_MK_P = '#[macro_export] macro_rules! mk { () => { () } }\n'
DEP_MKFN_W = '#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() { $crate::emit() } } }\n'
DEP_MKM_W = '#[macro_export] macro_rules! mkm { ($n:ident) => { pub fn $n(&self) { $crate::emit() } } }\n'
DEP_TL2 = ('#[macro_export] macro_rules! thread_local { (static $n:ident : $t:ty = $e:expr ;) => { pub fn tl_touch() { $crate::emit() } } }\n')
DEP_WRITE = '#[macro_export] macro_rules! write { ($($t:tt)*) => { $crate::emit() } }\n'
DEP_EMIT = 'pub fn emit() { @W@ }\n'


def with_dep(cid, text):
    """-> (extra-files, deps, chain) for a nested path dependency `fxdep<cid>` whose lib.rs is `text`."""
    n = 'fxdep' + cid
    return {'@dep/%s/src/lib.rs' % n: sb(text, cid)}, {n: '{ path = "%s" }' % n}, dict(kind='dep', dep_dir=n)


def gcell(cid, root, files, probes, note, dep=None, lib=False, edition='2021', features=None):
    """Register a cell. `files` = {relpath-in-src: text}; `dep` = dependency lib.rs text (or None)."""
    extra = {k: sb(v, cid) for k, v in files.items()}
    deps, chain = {}, None
    if dep is not None:
        e2, deps, chain = with_dep(cid, dep)
        extra.update(e2)
    cell(cid, sb(root, cid) + '\n', extra, probes, deps=deps, chain=chain, note=note)
    CELLS[cid].update(lib=lib, edition=edition, features=features, nocompile_ok=True)


# ---------------------------------------------------------------------------------------------------- C: body position
def ccell(cid, note, *, imp='', defs=EXP_W + EXP_NOP, head='crate::mk!()', headq='crate::mknop!()', dep=DEP_BASE,
          mods='mod other;\nmod inner;', inner_pre='', files=None, root_probe=False, edition='2021', defs_name='other.rs', probes=None):
    f = {'inner.rs': inner_pre + ('' if root_probe else 'pub fn go_@ID@() { %s }\npub fn q_@ID@_pure() { %s }\n' % (head, headq))}
    if root_probe:
        f['inner.rs'] += 'pub fn unused_@ID@() {}\n'
    if defs_name:
        f[defs_name] = defs
    f.update(files or {})
    if probes is None:
        probes = ([(cid + '_e', 'e', head), (cid + '_q', 'q', headq)] if root_probe else
                  [(cid + '_e', 'e', 'inner::go_%s()' % cid), (cid + '_q', 'q', 'inner::q_%s_pure()' % cid)])
    gcell(cid, imp + '\n' + mods, f, probes, note, dep=dep, edition=edition)


R1 = 'use fxdep@ID@::*;'
R4 = 'mod prelude { pub use fxdep@ID@::*; }\nuse prelude::*;'
CD = 'C: '
ccell('c1', CD + 'R0 x H1 x EXP_W (b7).', dep=None)
ccell('c2', CD + 'R1 x H1 x EXP_W (b4x).', imp=R1)
ccell('c3', CD + 'R2 (dep has a PURE mk) x H1 x EXP_W (b4w).', imp=R1, dep=DEP_BASE + DEP_MK_P)
ccell('c3r', CD + 'R2w (dep mk WRITES) x H1 x EXP_P (b4): export beats the glob, pure.', imp=R1, defs=EXP_P + EXP_NOP, dep=DEP_BASE + DEP_MK_W)
ccell('c4', CD + 'R3 `pub use other::*;` x H1 x EXP_W (b4z). Dep present only so the chained arm can be run.', imp='pub use other::*;')
ccell('c5', CD + 'R4 prelude glob x H1 x EXP_W.', imp=R4)
ccell('c6', CD + 'R1 x H2 (wrap!) x EXP_W (b4v).', imp=R1, defs=EXP_W + EXP_NOP + WRAPS, head='crate::wrap!()', headq='crate::wrapnop!()')
ccell('c6r', CD + 'R2w x H2 x EXP_P: pure.', imp=R1, defs=EXP_P + EXP_NOP + WRAPS, head='crate::wrap!()', headq='crate::wrapnop!()', dep=DEP_BASE + DEP_MK_W)
ccell('c7', CD + 'R1 x H3 (super::mk!) x EXP_W.', imp=R1, head='super::mk!()', headq='super::mknop!()')
ccell('c7r', CD + 'R2w x H3 x EXP_P: pure.', imp=R1, defs=EXP_P + EXP_NOP, head='super::mk!()', headq='super::mknop!()', dep=DEP_BASE + DEP_MK_W)
ccell('c8', CD + 'R1 x H4: the call sits in a ROOT fn of main.rs, no inner call.', imp=R1, head='self::mk!()', headq='self::mknop!()', root_probe=True)
ccell('c8b', CD + 'R1 x H5 (super::super::mk! from inner::deep).', imp=R1, mods='mod other;\nmod inner;',
      inner_pre='pub mod deep;\n', files={'inner/deep.rs': 'pub fn go_@ID@() { super::super::mk!() }\npub fn q_@ID@_pure() { super::super::mknop!() }\n'},
      probes=[('c8b_e', 'e', 'inner::deep::go_c8b()'), ('c8b_q', 'q', 'inner::deep::q_c8b_pure()')])
# the file for H5 replaces inner.rs's own fns
CELLS['c8b']['extra']['inner.rs'] = 'pub mod deep;\n'

LIM_DEFS_L = ('#[macro_export(local_inner_macros)] macro_rules! wrap { () => { mk!() } }\n'
              '#[macro_export(local_inner_macros)] macro_rules! wrapnop { () => { mknop!() } }\n')
LIM_DEFS_NL = ('#[macro_export] macro_rules! wrap { () => { mk!() } }\n'
               '#[macro_export] macro_rules! wrapnop { () => { nop!() } }\n')
H7_INNER = 'use fxdep@ID@::{mk, nop};\npub fn go_@ID@() { crate::wrap!() }\npub fn q_@ID@_pure() { crate::wrapnop!() }\n'


def h7(cid, note, defs, dep):
    gcell(cid, '', {'inner.rs': H7_INNER, 'defs.rs': defs}, [(cid + '_e', 'e', 'inner::go_%s()' % cid), (cid + '_q', 'q', 'inner::q_%s_pure()' % cid)],
          note, dep=dep)
    CELLS[cid]['main'] = 'mod inner;\nmod defs;\n'


h7('c9', CD + 'H7: local_inner_macros wrap!{mk!()} -> the crate\'s writer export (lim1).', EXP_W + EXP_NOP + LIM_DEFS_L, DEP_BASE + DEP_MK_P)
h7('c9c', CD + 'H7 WITHOUT local_inner_macros: mk! resolves at the call site to the dependency\'s pure mk (lim0).', EXP_W + EXP_NOP + LIM_DEFS_NL, DEP_BASE + DEP_MK_P)
h7('c9r', CD + 'H7: local_inner_macros -> the crate\'s PURE export; the dependency\'s mk writes.', EXP_P + EXP_NOP + LIM_DEFS_L, DEP_BASE + DEP_MK_W)

NX = ('macro_rules! mk { () => { @W@ } }\nmacro_rules! mknop { () => { () } }\npub(crate) use mk;\npub(crate) use mknop;\n')
ccell('c10', CD + 'R1 + R6: NON-exported writer re-exported by `pub(crate) use` (ru).', imp=R1 + '\npub(crate) use other::{mk, mknop};', defs=NX)
ccell('c10r', CD + 'R2w + R6: non-exported PURE mk shadows the dependency\'s writing glob.', imp=R1 + '\npub(crate) use other::{mk, mknop};',
      defs=NX.replace('@W@', '()'), dep=DEP_BASE + DEP_MK_W)
DEFMK = ('macro_rules! defmk { () => { #[macro_export] macro_rules! mk { () => { @W@ } } #[macro_export] macro_rules! mknop { () => { () } } } }\ndefmk!();\n')
ALLOW = '#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]'
ccell('c11', CD + 'R1 + allow-lint: the exports are EMITTED by a local macro (e2g).', imp=ALLOW + '\n' + R1, defs=DEFMK)
ccell('c11a', CD + 'R0 + allow-lint, emitted exports (e2a): must not regress.', imp=ALLOW, defs=DEFMK, dep=None)
ccell('c12', CD + 'ANCESTOR cfg twins: `#[cfg(unix)] mod other` (writer) / `#[cfg(not(unix))] mod other_p` (pure export).', dep=None,
      mods='#[cfg(unix)] mod other;\n#[cfg(not(unix))] mod other_p;\nmod inner;', files={'other_p.rs': EXP_P + EXP_NOP})
ccell('c13', CD + 'item cfg twins on the export itself (e3).', dep=None,
      defs='#[cfg(unix)]\n' + EXP_W + '#[cfg(not(unix))]\n' + EXP_P + EXP_NOP)
ccell('c14', CD + 'exports defined INSIDE a fn body (fb).', dep=DEP_BASE,
      defs='pub fn setup() { ' + EXP_W + EXP_NOP + '}\n')
ccell('c14g', CD + 'as c14 with the R1 glob (fbg).', imp=R1, defs='pub fn setup() { ' + EXP_W + EXP_NOP + '}\n')
ccell('c15', CD + '`use dep::mk` (writes) + a textual-only PURE mk in other.rs (b1): crate::mk! is the dependency\'s.', imp='use fxdep@ID@::mk;\nuse fxdep@ID@::nop as mknop;',
      defs='macro_rules! mk { () => { () } }\nmacro_rules! mknop { () => { () } }\n', mods='#[macro_use]\nmod other;\nmod inner;', dep=DEP_BASE + DEP_MK_W)
ccell('c15r', CD + 'bodies swapped (b1r): the dependency\'s mk is PURE, the textual-only local mk writes.', imp='use fxdep@ID@::mk;\nuse fxdep@ID@::nop as mknop;',
      defs='macro_rules! mk { () => { @W@ } }\nmacro_rules! mknop { () => { () } }\n', mods='#[macro_use]\nmod other;\nmod inner;', dep=DEP_BASE + DEP_MK_P)
ccell('c16', CD + 'edition 2015, `extern crate`, `::mk!()` (e15): control.', imp='extern crate fxdep@ID@;', head='::mk!()', headq='::mknop!()', edition='2015')
ccell('c17', CD + 'R0; `dep::wrap!()` expands `$crate::mk!()` of the DEPENDENCY (pure) while a same-named local mk writes: must NOT charge.',
      defs=EXP_W + EXP_NOP, head='fxdep@ID@::wrap!()', headq='fxdep@ID@::nopwrap!()',
      dep=DEP_BASE + DEP_MK_P + '#[macro_export] macro_rules! wrap { () => { $crate::mk!() } }\n#[macro_export] macro_rules! nopwrap { () => { $crate::nop!() } }\n')
ccell('c18', CD + 'R1 + `#[macro_use] extern crate dep` (pure mk) beside the crate\'s writer export: crate::mk is the export.',
      imp='#[macro_use] extern crate fxdep@ID@;\n' + R1, dep=DEP_BASE + DEP_MK_P)
# c16 needs the `extern crate` at the root BEFORE `mod`s in 2015 and `::mk!()`; nothing else differs.

# ---------------------------------------------------------------------------------------------------- D: item / impl-item / trait-item
DNX = ('macro_rules! mkfn { ($n:ident) => { pub fn $n() { @W@ } } }\nmacro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n'
       'pub(crate) use mkfn;\npub(crate) use mkfnp;\n')


def dcell(cid, note, *, pos='item', pfx='crate::', imp='', defs=MKFN_W + MKFN_P, dep=DEP_BASE, mods='mod other;\nmod inner;', inner_pre='',
          wrap=None, files=None, defs_name='other.rs'):
    if pos == 'item':
        body = ('%smkfn!{ go_@ID@ }\n%smkfnp!{ qgo_@ID@ }\n' % (pfx, pfx)) if wrap is None else wrap
        body += 'pub fn p_inner_@ID@() { go_@ID@() }\npub fn q_inner_@ID@() { qgo_@ID@() }\n'
        probes = [('%s_e' % cid, 'e', 'inner::go_%s()' % cid), ('%s_q' % cid, 'q', 'inner::qgo_%s()' % cid),
                  ('%s_in_e' % cid, 'e', 'inner::p_inner_%s()' % cid), ('%s_in_q' % cid, 'q', 'inner::q_inner_%s()' % cid),
                  ('%s_use_e' % cid, 'e', 'g_%s()' % cid), ('%s_use_q' % cid, 'q', 'qg_%s()' % cid)]
        imp = imp + '\nuse inner::go_@ID@ as g_@ID@;\nuse inner::qgo_@ID@ as qg_@ID@;'
    elif pos == 'impl':
        body = ('pub struct S_@ID@;\nimpl S_@ID@ { %smkm!{ go_@ID@ } }\npub struct SQ_@ID@;\nimpl SQ_@ID@ { %smkmp!{ qgo_@ID@ } }\n' % (pfx, pfx))
        probes = [('%s_e' % cid, 'e', 'inner::S_%s.go_%s()' % (cid, cid)), ('%s_q' % cid, 'q', 'inner::SQ_%s.qgo_%s()' % (cid, cid)),
                  ('%s_ufcs_e' % cid, 'e', 'inner::S_%s::go_%s(&inner::S_%s)' % (cid, cid, cid)),
                  ('%s_ufcs_q' % cid, 'q', 'inner::SQ_%s::qgo_%s(&inner::SQ_%s)' % (cid, cid, cid))]
    else:
        body = ('pub struct S_@ID@;\npub struct SQ_@ID@;\npub trait T_@ID@ { %smktr!{ go_@ID@ } }\nimpl T_@ID@ for S_@ID@ {}\n'
                'pub trait TQ_@ID@ { %smktrp!{ qgo_@ID@ } }\nimpl TQ_@ID@ for SQ_@ID@ {}\n' % (pfx, pfx))
        probes = [('%s_e' % cid, 'e', '<inner::S_%s as inner::T_%s>::go_%s(&inner::S_%s)' % (cid, cid, cid, cid)),
                  ('%s_q' % cid, 'q', '<inner::SQ_%s as inner::TQ_%s>::qgo_%s(&inner::SQ_%s)' % (cid, cid, cid, cid)),
                  ('%s_m_e' % cid, 'e', '{ use inner::T_%s; inner::S_%s.go_%s() }' % (cid, cid, cid)),
                  ('%s_m_q' % cid, 'q', '{ use inner::TQ_%s; inner::SQ_%s.qgo_%s() }' % (cid, cid, cid))]
    f = {'inner.rs': inner_pre + body}
    if defs_name:
        f[defs_name] = defs
    f.update(files or {})
    gcell(cid, imp + '\n' + mods, f, probes, 'D: ' + note, dep=dep)


dcell('d1', 'item, H1, R0, MKFN_W (i1).', dep=None)
dcell('d2', 'item, H1, R1, MKFN_W.', imp=R1)
dcell('d3', 'item, H1, R2w-shaped: the dependency\'s mkfn WRITES, the crate\'s same-named export is PURE (g6).', imp=R1, defs=MKFN_PN + MKFN_P,
      dep=DEP_BASE + DEP_MKFN_W)
dcell('d4', 'item, H3 (super::mkfn!), R0.', pfx='super::', dep=None)
dcell('d5', 'item, H1, R3 `pub use other::*;`.', imp='pub use other::*;', dep=None)
dcell('d6', 'item, H1, R6: non-exported macros re-exported by `pub(crate) use`.', imp='pub(crate) use other::{mkfn, mkfnp};', defs=DNX, dep=None)
dcell('d7', 'impl-item, H1, R0: method + UFCS.', pos='impl', defs=MKM_W + MKM_P, dep=None)
dcell('d7r', 'impl-item, H1, R2w-shaped: the dependency\'s mkm writes, the crate\'s same-named export is PURE.', pos='impl', imp=R1,
      defs=MKM_PN + MKM_P, dep=DEP_BASE + DEP_MKM_W)
dcell('d8', 'trait-item, H1, R0.', pos='trait', defs=MKTR_W + MKTR_P, dep=None)
dcell('d9', 'item, H1, R0: ITEM cfg twins on the macro (writer on unix, pure otherwise).', dep=None,
      defs='#[cfg(unix)]\n' + MKFN_W + '#[cfg(not(unix))]\n' + MKFN_PN + MKFN_P)
dcell('d10', 'item, H1: the INVOCATION is cfg-gated, with a pure fn on the other arm.', dep=None,
      wrap=('#[cfg(unix)]\ncrate::mkfn!{ go_@ID@ }\n#[cfg(not(unix))]\npub fn go_@ID@() {}\n'
            '#[cfg(unix)]\ncrate::mkfnp!{ qgo_@ID@ }\n#[cfg(not(unix))]\npub fn qgo_@ID@() {}\n'))
dcell('d11', 'NONROOT control: `use crate::defs::mkfn; self::mkfn!{ go }` (= B2); L2 must NOT bind it.', pfx='self::', dep=None, mods='mod inner;\nmod defs;',
      defs=DNX, defs_name='defs.rs', inner_pre='use crate::defs::{mkfn, mkfnp};\n')
# d12: the glob supplies the DEPENDENCY's item macro to `self::mk!`; the crate's own exported mk is pure and sits at the root.
gcell('d12', 'mod prelude { pub use fxdepd12::*; }\n#[macro_export] macro_rules! mk { ($n:ident) => { pub fn $n() {} } }\n#[macro_export] macro_rules! mknop { ($n:ident) => { pub fn $n() {} } }\nmod inner;\n',
      {'inner.rs': 'use crate::prelude::*;\nself::mk!{ go_d12 }\nself::mknop!{ qgo_d12 }\npub fn p_inner_d12() { go_d12() }\npub fn q_inner_d12() { qgo_d12() }\n'},
      [('d12_e', 'e', 'inner::go_d12()'), ('d12_q', 'q', 'inner::qgo_d12()'),
       ('d12_in_e', 'e', 'inner::p_inner_d12()'), ('d12_in_q', 'q', 'inner::q_inner_d12()')],
      'D: item, `self::mk!` whose only binding is the glob chain to the DEPENDENCY (g1d); a pure root export of the same name sits elsewhere (and one for the twin\'s `mknop`, so both spellings have a same-leaf local macro).',
      dep=DEP_EMIT + '#[macro_export] macro_rules! mk { ($n:ident) => { pub fn $n() { $crate::emit() } } }\n'
                     '#[macro_export] macro_rules! mknop { ($n:ident) => { pub fn $n() {} } }\n')

# ---------------------------------------------------------------------------------------------------- E: toolchain names x crate kind
GATED = '#[cfg(unix)] pub fn gated_@ID@() {}\n#[cfg(not(unix))] pub fn gated_@ID@() {}\n'
TLINV = 'thread_local! { static X_@ID@: u8 = 0; }\n'
PGO_DEP = 'pub fn p_go_@ID@() { tl_touch() }\n'
PGO_STD = 'pub fn p_go_@ID@() { X_@ID@.with(|_| ()) }\n'


def ecell(cid, note, *, kind, inner, root='', dep=DEP_EMIT + DEP_TL2, files=None, pgo='dep', features=None, mods='mod inner;', ctr=''):
    """kind 'bin' (K1) or 'lib' (K2/K3/K4). Probes: the writer/pure `p_go` and the cfg-pair `gated` (read as its own unit)."""
    pg = PGO_DEP if pgo == 'dep' else PGO_STD if pgo == 'std' else pgo
    f = {'inner.rs': inner + TLINV * (1 if ctr == '' else 0) + GATED + pg}
    f.update(files or {})
    islib = kind == 'lib'
    if islib:
        probes = [('p_go_' + cid, 'q' if pgo == 'std' else 'e', None), ('gated_' + cid, 'g', None)]
    else:
        probes = [(cid + ('_q' if pgo == 'std' else '_e'), 'q' if pgo == 'std' else 'e', 'inner::p_go_%s()' % cid), ('gated_' + cid, 'g', None)]
        root = root + '\npub use inner::gated_@ID@;'
    gcell(cid, root + '\n' + mods, f, probes, 'E: ' + note, dep=dep, lib=islib, features=features)


E1 = 'use fxdep@ID@::*;\n'
ecell('e1', 'K1 x G1 glob x thread_local (t2): expected E0659.', kind='bin', inner=E1)
ecell('e2', 'K1 x G2 prelude glob x thread_local (t4): expected E0659.', kind='bin', inner='mod prelude { pub use fxdep@ID@::*; }\nuse prelude::*;\n')
ecell('e3', 'K1 x G5 macro_use extern crate x thread_local (t1): the macro_use prelude beats std\'s.', kind='bin', inner='', root='#[macro_use] extern crate fxdep@ID@;')
# e4: the core-prelude name `write`; p_go formats through it. The gated pair sits beside a STD thread_local (the dep supplies only `write`).
ecell('e4', 'K1 x G5 x `write` (a CORE name, tcore): macro_use beats the std prelude; p_go calls write! in BODY position.', kind='bin', inner='',
      root='#[macro_use] extern crate fxdep@ID@;', dep=DEP_EMIT + DEP_WRITE,
      pgo='pub fn p_go_@ID@() { use std::fmt::Write as _; let mut s = String::new(); let _ = write!(s, "x"); }\n')
ecell('e5', 'K1 x G6 explicit `use dep::thread_local` (t3).', kind='bin', inner='use fxdep@ID@::thread_local;\n')
# e6/e7: std\'s thread_local binds; p_go is PURE; the glob is not in scope of the invocation
ecell('e6', 'K1 x G8: `use dep::*;` in a module nobody globs (g4): std\'s binds, pure.', kind='bin', inner='', pgo='std', mods='mod other;\nmod inner;',
      files={'other.rs': 'use fxdep@ID@::*;\npub fn unrelated_@ID@() {}\n'})
ecell('e7', 'K1 x G9: `use dep::*;` inside an inline `mod x {}` of the invoking FILE (imports do not leak out of x): std\'s binds, pure.', kind='bin',
      inner='mod x { use fxdep@ID@::*; }\n', pgo='std')
ecell('e8', 'K2 (#![no_std] lib) x G2 (f4d).', kind='lib', root='#![no_std]', inner='mod prelude { pub use fxdep@ID@::*; }\nuse prelude::*;\n')
ecell('e9', 'K2 x G3: `use crate::prelude::*;` from a child module, prelude at the root (f4e).', kind='lib',
      root='#![no_std]\nmod prelude { pub use fxdep@ID@::*; }', inner='use crate::prelude::*;\n')
ecell('e10', 'K2 x G4: the glob is EMITTED by a local macro in prelude.rs (g7).', kind='lib', root='#![no_std]\nmod prelude;', inner='use crate::prelude::*;\n',
      files={'prelude.rs': 'macro_rules! reexp { () => { pub use fxdep@ID@::*; } }\nreexp!();\n'})
ecell('e11', 'K2 x G5 macro_use extern crate, invocation in another file (g5).', kind='lib', root='#![no_std]\n#[macro_use] extern crate fxdep@ID@;', inner='')
ecell('e12', 'K2 x G1 x `write` (gcore): expected E0659; no thread_local pair (no_std has none).', kind='lib', root='#![no_std]', dep=DEP_EMIT + DEP_WRITE,
      inner='use fxdep@ID@::*;\n', pgo='pub fn p_go_@ID@() { write!(1) }\n', ctr='x')
CELLS['e12']['extra']['inner.rs'] = CELLS['e12']['extra']['inner.rs'].replace(GATED.replace('@ID@', 'e12'), '')
CELLS['e12']['probes'] = [('p_go_e12', 'e', None)]
ecell('e13', 'K1 x G5 x `vec` at BODY position (neither L1 nor L3 reaches it).', kind='bin', inner='', root='#[macro_use] extern crate fxdep@ID@;',
      dep=DEP_EMIT + '#[macro_export] macro_rules! vec { ($($t:tt)*) => { $crate::emit() } }\n',
      pgo='pub fn p_go_@ID@() { let _ = vec![1]; }\n')
ecell('e14', 'K3 (cfg_attr(not(feature = "std"), no_std)) x G1: default build compiles (dep\'s binds); --features std: E0659.', kind='lib',
      root='#![cfg_attr(not(feature = "std"), no_std)]', inner=E1, features=['std'])
ecell('e15', 'K4 (#![no_std] + #[macro_use] extern crate std;) x G1: a guess until built.', kind='lib',
      root='#![no_std]\n#[macro_use] extern crate std;', inner=E1)
ecell('e16', 'K2 x G7: a LOCAL thread_local macro defined above the invocation (its body is readable).', kind='lib', root='#![no_std]',
      dep=DEP_EMIT, inner='macro_rules! thread_local { (static $n:ident : $t:ty = $e:expr ;) => { pub fn tl_touch() { fxdep@ID@::emit() } } }\n')
ecell('e17', 'K1 x G1 of a crate that RE-EXPORTS std\'s own thread_local: same definition, no ambiguity, std\'s binds (pure).', kind='bin', inner=E1,
      dep=DEP_EMIT + 'pub use std::thread_local;\n', pgo='std')

# ---------------------------------------------------------------------------------------------------- N: L2's second fixtures
# Lane rust-macropath-l2 (R-ROOT), written BEFORE the binding: the NON-export and non-root shapes R-ROOT must REFUSE, each with a
# reversed twin cell whose executed truth is the opposite, so a rule that binds what rustc does not (lane residue-C's D1/E1) fails
# one of the two in the fabrication direction or the silence direction.
TXT_MKFN = 'macro_rules! mkfn { ($n:ident) => { pub fn $n() {} } }\nmacro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n'
TXT_MKFN_W = 'macro_rules! mkfn { ($n:ident) => { pub fn $n() { @W@ } } }\nmacro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n'
DEP_MKFNP = '#[macro_export] macro_rules! mkfnp { ($n:ident) => { pub fn $n() {} } }\n'
DEP_MKFN_P = '#[macro_export] macro_rules! mkfn { ($n:ident) => { pub fn $n() {} } }\n'
DEP_MKNOP = '#[macro_export] macro_rules! mknop { () => { () } }\n'
ND = 'N (L2 second fixture): '
dcell('n1', ND + 'item, f3c2: root `use dep::{mkfn, mkfnp}` (dep mkfn WRITES) + a textual-only PURE local mkfn: `crate::mkfn!` is the dep\'s.',
      imp='use fxdep@ID@::{mkfn, mkfnp};', defs=TXT_MKFN, dep=DEP_BASE + DEP_MKFN_W + DEP_MKFNP)
dcell('n1r', ND + 'item, f3crev2: as n1 with the bodies swapped (dep mkfn PURE, textual-only local mkfn WRITES): must not charge.',
      imp='use fxdep@ID@::{mkfn, mkfnp};', defs=TXT_MKFN_W, dep=DEP_BASE + DEP_MKFN_P + DEP_MKFNP)
ccell('n2', ND + 'body: root `pub(crate) use other::{mk, mknop}` where other.rs RE-EXPORTS the dependency (writes) and a third file holds a '
      'textual-only pure mk: no local definition is the binding.', imp='pub(crate) use other::{mk, mknop};',
      defs='pub use fxdep@ID@::{mk, nop as mknop};\n', mods='mod other;\nmod third;\nmod inner;',
      files={'third.rs': 'macro_rules! mk { () => { () } }\nmacro_rules! mknop { () => { () } }\n'}, dep=DEP_BASE + DEP_MK_W)
ccell('n2r', ND + 'body: n2 with the bodies swapped (dependency mk PURE, the third file\'s textual-only mk WRITES): must not charge.',
      imp='pub(crate) use other::{mk, mknop};', defs='pub use fxdep@ID@::{mk, nop as mknop};\n', mods='mod other;\nmod third;\nmod inner;',
      files={'third.rs': 'macro_rules! mk { () => { @W@ } }\nmacro_rules! mknop { () => { () } }\n'}, dep=DEP_BASE + DEP_MK_P)
ccell('n4', ND + 'TWO crate roots: lib.rs owns the WRITING export; main.rs globs the dependency (pure mk) and its inner calls `crate::mk!()`, '
      'which is the BIN crate\'s root: must not charge.', imp=R1, mods='mod inner;', defs_name=None,
      files={'lib.rs': 'mod other;\n', 'other.rs': EXP_W + EXP_NOP}, dep=DEP_BASE + DEP_MK_P + DEP_MKNOP)
ccell('n5', ND + 'body: `self::mk!()` in a NON-root module that imports the dependency\'s mk (writes); the root export is pure.',
      defs=EXP_P + EXP_NOP, inner_pre='use fxdep@ID@::{mk, nop as mknop};\n', head='self::mk!()', headq='self::mknop!()', dep=DEP_BASE + DEP_MK_W)
ccell('n5r', ND + 'body: n5 reversed (root export WRITES, the module\'s imported dependency mk is PURE): must not charge.',
      defs=EXP_W + EXP_NOP, inner_pre='use fxdep@ID@::{mk, nop as mknop};\n', head='self::mk!()', headq='self::mknop!()', dep=DEP_BASE + DEP_MK_P)
ccell('n6', ND + 'body: `super::mk!()` from depth 2 reaches `inner` (which imports the dependency\'s PURE mk), not the root (export WRITES): '
      'must not charge.', mods='mod other;\nmod inner;', files={'inner/deep.rs': 'pub fn go_@ID@() { super::mk!() }\npub fn q_@ID@_pure() { super::mknop!() }\n'},
      probes=[('n6_e', 'e', 'inner::deep::go_n6()'), ('n6_q', 'q', 'inner::deep::q_n6_pure()')], dep=DEP_BASE + DEP_MK_P)
CELLS['n6']['extra']['inner.rs'] = 'use fxdepn6::{mk, nop as mknop};\npub mod deep;\n'
ccell('n7', ND + 'body: `self::mk!()` inside an INLINE `mod m` of the root file, which imports the dependency\'s PURE mk; the root export '
      'WRITES: `self` is `m`, not the root: must not charge.', imp='mod m_n7 { use fxdepn7::{mk, nop as mknop}; pub fn go() { self::mk!() } '
      'pub fn goq() { self::mknop!() } }', mods='mod other;\nmod inner;', probes=[('n7_e', 'e', 'm_n7::go()'), ('n7_q', 'q', 'm_n7::goq()')],
      dep=DEP_BASE + DEP_MK_P)
ccell('n8', ND + 'positive: `super::mk!()` inside an INLINE `mod m` of the root file reaches the root\'s WRITING export (R1 glob).',
      imp=R1 + '\nmod m_n8 { pub fn go() { super::mk!() } pub fn goq() { super::mknop!() } }', mods='mod other;\nmod inner;',
      probes=[('n8_e', 'e', 'm_n8::go()'), ('n8_q', 'q', 'm_n8::goq()')])
ccell('n10', ND + 'body: `self::mk!()` inside a module declared in a FN BODY (the block\'s own module) that imports the dependency\'s PURE mk; '
      'the root export WRITES: must not charge.', mods='mod other;\nmod inner;',
      imp='pub fn host_n10() { mod m { use fxdepn10::{mk, nop as mknop}; pub fn g() { self::mk!() } pub fn gq() { self::mknop!() } } m::g() }\n'
          'pub fn hostq_n10() { mod m { use fxdepn10::{mk, nop as mknop}; pub fn gq() { self::mknop!() } } m::gq() }',
      probes=[('n10_e', 'e', 'host_n10()'), ('n10_q', 'q', 'hostq_n10()')], dep=DEP_BASE + DEP_MK_P)
ccell('n9', ND +'positive: the root re-export RENAMES (`pub(crate) use other::{mk as mkr, mknop as mkrnop}`), R1 glob, `crate::mkr!()`.',
      imp=R1 + '\npub(crate) use other::{mk as mkr, mknop as mkrnop};', defs=NX, head='crate::mkr!()', headq='crate::mkrnop!()')

# ---------------------------------------------------------------------------------------------------- C (revision 1): Fable's L2 merge review
# Design revision 1 (~/candor-handoff/rust-macropath-design/DESIGN.md), from Fable's merge review of lane rust-macropath-l2 (D1, D2 and
# the attacks that held, ~/candor-handoff/rust-macropath-L2/FABLE-MERGE-REVIEW.md), rendered from its executed fixtures.
FD1 = 'C (rev 1): '
GATED_CONST = 'pub const _: () = {\n    #[cfg(windows)]\n    let _w: () = { #[macro_export] macro_rules! mk { () => { %s } } };\n};\n' + EXP_NOP
ccell('fconst', FD1 + 'D1: the W export sits under a statement `#[cfg(windows)]` in a CONST initializer; the root glob\'s dependency mk is '
      'PURE: crate::mk is the dependency\'s (executed pure). Must not charge.', imp=R1, defs=GATED_CONST % '@W@', dep=DEP_BASE + DEP_MK_P)
ccell('fconstr', FD1 + 'D1 mirror: the gated const-block export is PURE, the dependency mk WRITES: the `invisible` must stay.', imp=R1,
      defs=GATED_CONST % '()', dep=DEP_BASE + DEP_MK_W)
ccell('fconstw', FD1 + 'positive: an UNGATED const-block export, no glob: rustc exports it, R-ROOT must still bind (WRITES).',
      defs='pub const _: () = { ' + EXP_W.strip() + ' };\n' + EXP_NOP)
ccell('fimplc', FD1 + 'D1: as fconst, inside an impl\'s ASSOCIATED const initializer.', imp=R1,
      defs='pub struct S;\nimpl S { pub const C: () = { #[cfg(windows)] let _w: () = { ' + EXP_W.strip() + ' }; }; }\n' + EXP_NOP,
      dep=DEP_BASE + DEP_MK_P)
ccell('fstatic', FD1 + 'D1: as fconst, inside a STATIC initializer.', imp=R1,
      defs='pub static S: () = { #[cfg(windows)] let _w: () = { ' + EXP_W.strip() + ' }; };\n' + EXP_NOP, dep=DEP_BASE + DEP_MK_P)
LIMNEST = ('#[macro_export(local_inner_macros)]\n'
           'macro_rules! wrap { () => { macro_rules! q { () => { mk!() } } macro_rules! qn { () => { mknop!() } } } }\n')
ccell('flimnest', FD1 + 'D2: a local_inner_macros transcriber DEFINES `q`, whose bare mk! resolves at q!\'s call site (inner imports the '
      'dependency\'s PURE mk); the root export WRITES: must not charge.', mods='mod defs;\nmod other;\nmod inner;',
      files={'defs.rs': LIMNEST}, inner_pre='use fxdep@ID@::{mk, nop as mknop};\ncrate::wrap!();\n', head='q!()', headq='qn!()',
      dep=DEP_BASE + DEP_MK_P + DEP_MKNOP)
ccell('flimnestr', FD1 + 'D2 mirror: root export PURE, the imported dependency mk WRITES. A PRE-EXISTING silence (ABSENT on pub, main and '
      'cand): recorded, NOT acceptance.', mods='mod defs;\nmod other;\nmod inner;', defs=EXP_P + EXP_NOP,
      files={'defs.rs': LIMNEST}, inner_pre='use fxdep@ID@::{mk, nop as mknop};\ncrate::wrap!();\n', head='q!()', headq='qn!()',
      dep=DEP_BASE + DEP_MK_W + DEP_MKNOP)
gcell('fr1675', '#[macro_export]\nmacro_rules! select { () => { mod kq; use kq as sys; } }\ncrate::select!();\n'
      'pub struct Poller { poller: sys::Poller }\nimpl Poller {\n    pub fn new() -> Poller { Poller { poller: sys::Poller } }\n'
      '    pub fn wait(&self) -> u8 { self.poller.wait() }\n}\n',
      {'kq.rs': 'pub struct Poller;\nimpl Poller { pub fn wait(&self) -> u8 { @W@; 1 } }\n'},
      [('fr1675_e', 'e', 'let _ = Poller::new().wait()'), ('fr1675_q', 'q', 'let _ = Poller::new()')],
      FD1 + 'the moved r1675 test device at its ORIGINAL root spelling (`crate::select!()` of the crate\'s export): a resolution, pinned.')
gcell('fr1814', 'pub trait Fetch: Sized { fn fetch_all(self, p: &str) { @W@ } }\npub trait Quiet: Sized { fn hush_zz(self, p: &str) -> usize { p.len() } }\n'
      '#[macro_export] macro_rules! gen_impls { ($t:ty) => { impl crate::Fetch for $t {} impl crate::Quiet for $t {} impl $t { pub fn inh_zz(&self, p: &str) { @W@ } } }; }\n'
      'pub mod km { pub struct Kmac(pub String); pub fn kmac(s: &str) -> Kmac { Kmac(s.to_string()) } crate::gen_impls!(Kmac); }\n', {},
      [('fr1814_e', 'e', 'km::kmac("x").fetch_all("p")'), ('fr1814_inh_e', 'e', 'km::kmac("x").inh_zz("p")'),
       ('fr1814_q', 'q', 'let _ = km::kmac("x").hush_zz("p")')],
      FD1 + 'the moved world_gl device at its ORIGINAL root spelling (`crate::gen_impls!(Kmac)` in `pub mod km`): fr1814_e was SILENT on pub and '
            'main (the trait-default caller of an emitted impl); a resolution, pinned.')
ccell('fshadow', FD1 + 'root-namespace export mk WRITES, then a later TEXTUAL `macro_rules! mk {()}` in the same file: crate::mk is the export.',
      defs=EXP_W + 'macro_rules! mk { () => { () } }\n' + EXP_NOP)
ccell('fshadowr', FD1 + 'fshadow with the bodies swapped (export PURE, textual WRITES): pure in rustc. pub and main FABRICATE; R-ROOT fixes it.',
      defs=EXP_P + 'macro_rules! mk { () => { @W@ } }\n' + EXP_NOP)

# ---------------------------------------------------------------------------------------------------- F: cells that should NOT compile
FD = 'F: '
gcell('f1', 'use fxdepf1::mk;\nmod other;\nmod inner;\n', {'other.rs': EXP_P, 'inner.rs': 'pub fn go_f1() { crate::mk!() }\n'},
      [('f1_e', 'e', 'inner::go_f1()')], FD + 'root `use dep::mk;` + an exported crate mk (e1): expected E0255.', dep=DEP_EMIT + DEP_MK_W)
gcell('f2', 'mod other;\nmod inner;\n' + EXP_P, {'other.rs': 'macro_rules! defmk { () => { #[macro_export] macro_rules! mk { () => { @W@ } } } }\ndefmk!();\n',
      'inner.rs': 'pub fn go_f2() { crate::mk!() }\n'}, [('f2_e', 'e', 'inner::go_f2()')],
      FD + 'a source export beside an EMITTED export of the same name, lint allowed (e0428): expected E0428.')
CELLS['f2']['main'] = '#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]\n' + CELLS['f2']['main']
gcell('f4', 'macro_rules! imp { () => { use fxdepf4::mk; } }\nimp!();\nmod other;\nmod inner;\n', {'other.rs': EXP_W, 'inner.rs': 'pub fn go_f4() { crate::mk!() }\n'},
      [('f4_e', 'e', 'inner::go_f4()')], FD + 'LOCAL-macro-emitted root `use dep::mk;` beside an exported crate mk: expected E0255.', dep=DEP_EMIT + DEP_MK_P)
gcell('f5', 'fxdepf5::imp!();\nmod other;\nmod inner;\n', {'other.rs': EXP_W, 'inner.rs': 'pub fn go_f5() { crate::mk!() }\n'},
      [('f5_e', 'e', 'inner::go_f5()')], FD + 'DEPENDENCY-macro-emitted root `pub use $crate::mk;` beside an exported crate mk: expected E0255.',
      dep=DEP_EMIT + DEP_MK_P + '#[macro_export] macro_rules! imp { () => { pub use $crate::mk; } }\n')
gcell('f6', 'mod other;\nmod inner;\n', {'other.rs': EXP_W, 'inner.rs': '#[macro_use] extern crate fxdepf6;\npub fn go_f6() { crate::mk!() }\n'},
      [('f6_e', 'e', 'inner::go_f6()')], FD + '`#[macro_use] extern crate dep;` inside a NON-root module: expected E0468.', dep=DEP_EMIT + DEP_MK_P)
gcell('f7', 'pub(crate) use other::mk;\nmod other;\nmod inner;\n', {'other.rs': EXP_W + 'pub(crate) use mk;\n', 'inner.rs': 'pub fn go_f7() { crate::mk!() }\n'},
      [('f7_e', 'e', 'inner::go_f7()')], FD + 'an exported crate mk AND `pub(crate) use other::mk;` at the root (the export re-imported in-module): E0255 or compiles-as-one?')
gcell('f7b', 'pub(crate) use other::mk;\nmod other;\nmod inner;\n', {'other.rs': EXP_W, 'inner.rs': 'pub fn go_f7b() { crate::mk!() }\n'},
      [('f7b_e', 'e', 'inner::go_f7b()')], FD + 'extra (not in the matrix): f7 WITHOUT the in-module `pub(crate) use mk;`: `other::mk` does not exist (export lives at the root).')


def cargo_toml(name, deps, is_dep=False, edition='2021', features=None):
    s = '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "%s"\n\n[dependencies]\n' % (name, edition)
    s += ''.join('%s = %s\n' % (k, v) for k, v in deps.items())
    if features:
        s += '\n[features]\n' + ''.join('%s = []\n' % f for f in features)
    if not is_dep:
        s += '\n[workspace]\n'
    return s


def render(out):
    for cid, c in sorted(CELLS.items()):
        d = os.path.join(out, 'fx', cid)
        os.makedirs(os.path.join(d, 'src'), exist_ok=True)
        open(os.path.join(d, 'Cargo.toml'), 'w').write(cargo_toml('cf_' + cid, c['deps'], edition=c.get('edition') or '2021', features=c.get('features')))
        decl = ''.join('pub fn %s() { %s; }\n' % (p, b if b.rstrip().endswith('}') and False else b) for p, k, b in c['probes'] if b is not None)
        arms = ''.join('        "%s" => %s(),\n' % (p, p) for p, _, _ in c['probes'])
        disp = 'fn main() {\n    match std::env::args().nth(1).unwrap().as_str() {\n%s        _ => panic!("unknown probe"),\n    }\n}\n' % arms
        if c.get('lib'):
            main = '#![allow(unused)]\n' + c['main'].replace('W;', W)
            main = main.replace('{ W }', '{ ' + W + ' }')
            open(os.path.join(d, 'src', 'lib.rs'), 'w').write(main)
        else:
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
                os.makedirs(os.path.dirname(p), exist_ok=True)
                open(p, 'w').write('#![allow(unused)]\n' + txt if rel.endswith('.rs') and False else txt)
        meta = dict(id=cid, lib=False, probes=[dict(name=p, kind=k) for p, k, _ in c['probes']], chain=c['chain'], note=c['note'],
                    deps=sorted(c['deps']))
        for k in ('lib', 'edition', 'features', 'nocompile_ok'):
            if c.get(k) and not (k == 'edition' and c[k] == '2021'):
                meta[k] = c[k]
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
