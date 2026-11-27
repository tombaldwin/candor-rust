#!/usr/bin/env python3
"""SOUNDNESS R1757 (round 2) — std's INHERENT associated items per type, and the names std's public TRAITS declare.

A path call `Ty::m(..)` on a std type runs Ty's INHERENT `m` whenever one exists: Rust resolves an inherent
associated item before any trait member (`Duration::new(1, 0)` is std's, whatever `impl dep::Load for Duration
{ fn new() }` a dependency writes). A dependency trait's member is what runs only where the type has NO inherent
of that name. That is a fact per TYPE (`Duration::new` exists, `Instant::new` and `u64::new` do not), so it is
read here from rust-src, never from a list of names.

usage: std_assoc.py <rust-src library dir>  -> lines, sorted, unique:
    I<TAB><type leaf><TAB><fn name>   a STABLE `pub fn` (any receiver) of an inherent `impl <Type>` block
    T<TAB><fn name>                   a fn declared in a `pub trait` body (any receiver)
    TS<TAB><fn name><TAB><trait><TAB><implementor leaves, or ->  a fn declared in a SEALED `pub impl(..) trait` body (R1913)
    P/TC/N/G rows                     the std trait scope facts (R1951/R1956, see the block before the output)
The integer types' inherent fns are written inside `int_impl!`/`uint_impl!`, so every `pub fn` in
`int_macros.rs` / `uint_macros.rs` is filed under every signed / unsigned integer leaf, and every `pub fn` in a
`macro_rules!` of `core/src/num/mod.rs` that writes an integer impl (`from_str_int_impl!`) under every integer leaf.

SOUNDNESS R1859 — the inherent rows are read OUTSIDE `std/src/sys` (and `sys_common`): those modules are std's private
platform layer, whose types reuse public leaves (`sys::fs::File` has `read_at`, `seek`, `fsync`; `sys::net::TcpStream` has
`socket`), so keyed by leaf they answered "std::fs::File has an inherent `read_at`" — false (it is `FileExt`'s), and `Self::
read_at` in an impl for `File` was read as std's instead of the impl's own member. The public types that wrap them define
their own inherents outside `sys`; the only `pub use crate::sys::…` re-exports are `fortanix_sgx`'s and `thread::local_impl`'s
(target-specific / unstable). The `T` rows (trait fns) are read everywhere, as before."""
import os, re, sys
lib = sys.argv[1]
pub_fn = re.compile(r'\bpub\s+(?:const\s+)?(?:unsafe\s+)?(?:async\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)')
any_fn = re.compile(r'\bfn\s+([A-Za-z_][A-Za-z0-9_]*)')
# SOUNDNESS R1913 — `pub impl(self) trait X` / `pub impl(crate) const unsafe trait X` is std's SEALED-trait syntax (the `impl(..)`
# restriction names who may implement it); the first cut matched only `pub [const] [unsafe] [auto] trait`, so every fn of ~27 public std
# traits (`CommandExt`, `OpenOptionsExt`, `PermissionsExt`, `OsStrExt`, `IsTerminal`, `SliceIndex`, ..) was missing from the table. Those
# fns are NOT filed as `T` rows (a `T` row withholds the dependency join BY NAME, which is right for `default`/`from_str`, traits std
# implements for its own types and which the prelude or a common import brings into scope): a sealed std trait's member is std's only
# where that trait is IMPORTED, and a dependency trait's member of the same name runs where only the dependency's is. They are filed as
# `TS<TAB>fn<TAB>trait` rows, which a consumer asks beside the module's imports. A sealed trait whose member (or the trait) is `#[unstable]` is dropped: rustc ignores
# an unstable candidate where a stable one applies, so std's member never runs through that name from user code (`SliceIndex::get`).
pub_trait = re.compile(r'\bpub\s+(impl\s*\([^)]*\)\s+)?(?:(?:const|unsafe|auto)\s+)*trait\s+([A-Za-z_][A-Za-z0-9_]*)')
impl_kw = re.compile(r'(?<![A-Za-z0-9_])impl(?![A-Za-z0-9_])')
SIGNED = ['i8', 'i16', 'i32', 'i64', 'i128', 'isize']
UNSIGNED = ['u8', 'u16', 'u32', 'u64', 'u128', 'usize']

def strip_comments(s):
    s = re.sub(r'//[^\n]*', '', s)
    return re.sub(r'/\*.*?\*/', '', s, flags=re.S)

def unstable(s, start):
    k = max(s.rfind(';', 0, start), s.rfind('}', 0, start), s.rfind('{', 0, start))
    return '#[unstable(' in s[k + 1:start]

def block_end(s, b):
    d = 0; e = b
    while e < len(s):
        if s[e] == '{': d += 1
        elif s[e] == '}':
            d -= 1
            if d == 0: return e
        e += 1
    return e

def depth1_pub_fns(body):
    """pub fns that are DIRECT items of a `{ .. }` body (depth 1), stable only."""
    out = []; d = 0; i = 0
    for m in pub_fn.finditer(body):
        d = body.count('{', 0, m.start()) - body.count('}', 0, m.start())
        if d == 1 and not unstable(body, m.start()):
            out.append(m.group(1))
    return out

def self_leaf(header):
    """the self type's leaf of an INHERENT impl header (`<T> Vec<T, A>` -> `Vec`), None for a trait impl."""
    h = header.strip()
    if h.startswith('<'):
        d = 0
        for i, c in enumerate(h):
            if c == '<': d += 1
            elif c == '>' and (i == 0 or h[i-1] != '-'):
                d -= 1
                if d == 0: h = h[i+1:]; break
    h = re.split(r'\bwhere\b', h)[0].strip()
    # a trait impl names `for` at generic depth 0
    d = 0
    for tok in re.finditer(r'<|>|\bfor\b', h):
        t = tok.group(0)
        if t == '<': d += 1
        elif t == '>': d -= 1
        elif d == 0: return None
    h = h.lstrip('&').strip()
    h = re.sub(r"^'[A-Za-z_]+\s+", '', h)
    h = re.sub(r'^(mut|const|dyn)\s+', '', h)
    base = h.split('<')[0].strip()
    leaf = base.split('::')[-1].strip()
    return leaf if re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', leaf) else None

SYS_DIRS = [os.path.join(lib, 'std', 'src', d) for d in ('sys', 'sys_common')]
def private_platform(root):
    return any(root == d or root.startswith(d + os.sep) for d in SYS_DIRS)

inh = set(); trait_fns = set(); sealed_fns = set(); impls_for = {}
# SOUNDNESS R1913 round 2 — `impl OsStrExt for OsStr`: the NOMINAL std types a trait is implemented for, so a sealed trait's member is
# std's for `Type::member` only where Type is one of them (`PathBuf::from_bytes` runs a dependency's impl: OsStrExt is not implemented for PathBuf).
trait_impl = re.compile(r"(?<![A-Za-z0-9_])impl\s*(?:<[^{};]*?>)?\s*(?:unsafe\s+)?(?:const\s+)?(?:[A-Za-z_]\w*::)*([A-Za-z_]\w*)\s*(?:<[^{};]*?>)?\s+for\s+&?\s*(?:'\w+\s+)?(?:mut\s+)?(?:[A-Za-z_]\w*::)*([A-Za-z_]\w*)")
for crate in ('core', 'alloc', 'std'):
    for root, dirs, files in os.walk(os.path.join(lib, crate, 'src')):
        dirs[:] = [d for d in dirs if d not in ('tests', 'benches')]
        for f in files:
            if not f.endswith('.rs') or f in ('tests.rs', 'benches.rs'):
                continue
            s = strip_comments(open(os.path.join(root, f), errors='replace').read())
            if private_platform(root):
                for t in pub_trait.finditer(s):
                    b = s.find('{', t.end())
                    if b < 0: continue
                    e = block_end(s, b)
                    for m in any_fn.finditer(s[b:e+1]):
                        (None if (t.group(1) and (unstable(s, t.start()) or unstable(s[b:e+1], m.start()))) else sealed_fns.add((m.group(1), t.group(2))) if t.group(1) else trait_fns.add(m.group(1)))
                continue
            if f in ('int_macros.rs', 'uint_macros.rs'):
                leaves = SIGNED if f == 'int_macros.rs' else UNSIGNED
                for m in pub_fn.finditer(s):
                    if not unstable(s, m.start()):
                        for l in leaves: inh.add((l, m.group(1)))
            if f == 'mod.rs' and root.endswith(os.path.join('core', 'src', 'num')):
                # `from_str_int_impl!` and its siblings: pub fns written inside a `macro_rules!` this file applies to
                # every integer type (`u64::from_str_radix`).
                for mr in re.finditer(r'\bmacro_rules!\s*[A-Za-z_][A-Za-z0-9_]*\s*\{', s):
                    e = block_end(s, mr.end() - 1)
                    body = s[mr.end() - 1:e + 1]
                    if '$int_ty' not in body and '$t' not in body:
                        continue
                    for m in pub_fn.finditer(body):
                        if not unstable(body, m.start()):
                            for l in SIGNED + UNSIGNED: inh.add((l, m.group(1)))
            for ti in trait_impl.finditer(s):
                impls_for.setdefault(ti.group(1), set()).add(ti.group(2))
            for m in impl_kw.finditer(s):
                b = s.find('{', m.end()); semi = s.find(';', m.end())
                if b < 0 or (0 <= semi < b): continue
                leaf = self_leaf(s[m.end():b])
                if leaf is None: continue
                e = block_end(s, b)
                for n in depth1_pub_fns(s[b:e+1]):
                    inh.add((leaf, n))
            for t in pub_trait.finditer(s):
                b = s.find('{', t.end())
                if b < 0: continue
                e = block_end(s, b)
                for m in any_fn.finditer(s[b:e+1]):
                    (None if (t.group(1) and (unstable(s, t.start()) or unstable(s[b:e+1], m.start()))) else sealed_fns.add((m.group(1), t.group(2))) if t.group(1) else trait_fns.add(m.group(1)))
# ROUND 3 — INHERENT IMPLS A `macro_rules!` WRITES. std writes many inherent impls inside macros whose self type is a
# macro parameter (`atomic_int!` -> `impl $atomic_type { pub const fn new(..) }`), and names the type only at the
# invocation (`atomic_int! { .., i8 AtomicI8 }`); an alias family is minted the same way (`nonzero_integer! { Self =
# NonZeroU8, .. }` -> `pub type $Ty = NonZero<$Int>`). The first cut read neither, so `AtomicUsize::new` had no row and
# the R1757 join fired where Rust picks std's inherent (EXECUTED). Each macro whose body holds an inherent impl files its
# fns — the `$`-typed blocks' own, plus every fn of a CONCRETE type its impl headers or `type` aliases name — under EVERY
# capitalised identifier any invocation of it passes. Over-inclusion (an identifier that is not a type) only withholds.
macro_def = re.compile(r'\bmacro_rules!\s*([A-Za-z_][A-Za-z0-9_]*)\s*\{')
defs = {}   # name -> (set of fns, set of concrete leaves)
invocations = {}   # name -> set of capitalised idents in its arguments
texts = []
for crate in ('core', 'alloc', 'std'):
    for root, dirs, files in os.walk(os.path.join(lib, crate, 'src')):
        dirs[:] = [d for d in dirs if d not in ('tests', 'benches')]
        for f in files:
            if f.endswith('.rs') and f not in ('tests.rs', 'benches.rs') and not private_platform(root):
                texts.append(strip_comments(open(os.path.join(root, f), errors='replace').read()))
for s in texts:
    for md in macro_def.finditer(s):
        e = block_end(s, md.end() - 1)
        body = s[md.end() - 1:e + 1]
        fns = set(); leaves = set(); any_impl = False
        for m in impl_kw.finditer(body):
            b = body.find('{', m.end()); semi = body.find(';', m.end())
            if b < 0 or (0 <= semi < b): continue
            hdr = body[m.end():b]
            if re.search(r'\bfor\b', re.sub(r'<[^<>]*>', '', hdr)):
                continue   # a trait impl
            any_impl = True
            raw = hdr.strip()
            leaf = self_leaf(hdr)
            if leaf:
                leaves.add(leaf)
            elif '$' in raw:
                eb = block_end(body, b)
                for n in depth1_pub_fns(body[b:eb + 1]):
                    fns.add(n)
        for ta in re.finditer(r'\btype\s+\$[A-Za-z_]+\s*=\s*([A-Za-z_][A-Za-z0-9_]*)\s*<', body):
            leaves.add(ta.group(1)); any_impl = True
        if any_impl:
            d = defs.setdefault(md.group(1), (set(), set()))
            d[0].update(fns); d[1].update(leaves)
inv_re = re.compile(r'(?<![A-Za-z0-9_$])([A-Za-z_][A-Za-z0-9_]*)!\s*([({\[])')
for s in texts:
    for m in inv_re.finditer(s):
        name = m.group(1)
        if name not in defs or name == 'macro_rules':
            continue
        o = m.end() - 1; close = {'(': ')', '{': '}', '[': ']'}[m.group(2)]
        d = 0; e = o
        while e < len(s):
            if s[e] in '({[': d += 1
            elif s[e] in ')}]':
                d -= 1
                if d == 0: break
            e += 1
        args = s[o + 1:e]
        invocations.setdefault(name, set()).update(re.findall(r'(?<![A-Za-z0-9_$"])([A-Z][A-Za-z0-9_]*)(?![A-Za-z0-9_"])', args))
by_leaf = {}
for l, n in inh:
    by_leaf.setdefault(l, set()).add(n)
for name, (fns, leaves) in defs.items():
    allf = set(fns)
    for l in leaves:
        allf |= by_leaf.get(l, set())
    for t in invocations.get(name, ()):
        for n in allf:
            inh.add((t, n))
# A top-level `pub type A = B<..>;` (outside any macro) — the alias leaf is what a consumer writes.
for s in texts:
    for ta in re.finditer(r'\bpub\s+type\s+([A-Z][A-Za-z0-9_]*)\s*(?:<[^=]*>)?\s*=\s*([A-Za-z_][A-Za-z0-9_:]*)\s*[<;]', s):
        tgt = ta.group(2).split('::')[-1]
        for n in by_leaf.get(tgt, ()):
            inh.add((ta.group(1), n))

# SELF-CHECK — a sibling family std generates from ONE macro must come out with ONE fn set. A family whose members
# disagree (or one missing outright) means a macro this generator does not read: refuse to write the table.
fam = {
    'atomic ints': ['AtomicI8', 'AtomicI16', 'AtomicI32', 'AtomicI64', 'AtomicIsize',
                    'AtomicU8', 'AtomicU16', 'AtomicU32', 'AtomicU64', 'AtomicUsize'],
    'nonzero': ['NonZeroI8', 'NonZeroI16', 'NonZeroI32', 'NonZeroI64', 'NonZeroI128', 'NonZeroIsize',
                'NonZeroU8', 'NonZeroU16', 'NonZeroU32', 'NonZeroU64', 'NonZeroU128', 'NonZeroUsize'],
    'signed ints': SIGNED, 'unsigned ints': UNSIGNED,
}
if os.environ.get('STD_ASSOC_SELFCHECK_DROP'):   # calibration: drop one leaf's rows, the check must refuse
    inh = {(l, n) for (l, n) in inh if l != os.environ['STD_ASSOC_SELFCHECK_DROP']}
cur = {}
for l, n in inh:
    cur.setdefault(l, set()).add(n)
bad = []
for fname, leaves in fam.items():
    sets = [frozenset(cur.get(l, ())) for l in leaves]
    core_set = frozenset.intersection(*sets)
    for l, st in zip(leaves, sets):
        if not st or len(st) < 0.8 * max(len(x) for x in sets) or not core_set:
            bad.append(f"{fname}: {l} has {len(st)} fns (family max {max(len(x) for x in sets)})")
# ROUND 4 — `Wrapping<T>` / `Saturating<T>`: their inherent impls are written by a macro whose header names a CONCRETE
# generic leaf (`impl Wrapping<$t>`), and Wrapping's macro-written inherents are `#[unstable]` while Saturating's are
# stable, so the two legitimately differ (1 row against 18) and the 0.8-of-max rule above does not apply. What must
# hold, and is checked rather than lucky: BOTH leaves have rows (a generator that stops reading the `Ty<$t>` header
# files none, and R1757 would then join a dependency impl where Rust picks std's inherent), and Wrapping's stable
# set is contained in Saturating's (a Wrapping-only fn means an unstable gate stopped filtering).
for l in ('Wrapping', 'Saturating'):
    if not cur.get(l):
        bad.append(f"wrapping family: {l} has no rows")
extra = cur.get('Wrapping', set()) - cur.get('Saturating', set())
if cur.get('Saturating') and extra:
    bad.append(f"wrapping family: Wrapping has fns Saturating lacks: {sorted(extra)}")
if bad:
    sys.stderr.write("std_assoc.py: SELF-CHECK FAILED — a sibling family is incomplete:\n  " + "\n  ".join(bad) + "\n")
    sys.exit(3)

# SOUNDNESS R1951 / R1956 (lane L3) — the facts the std-type path rule needs to decide which std traits are IN SCOPE, read from rust-src:
#   P<TAB><edition><TAB><leaf>          a name the edition's prelude imports that is a `pub trait` leaf somewhere in std (over-inclusion only
#                                       keeps a withholding: `Debug`/`Hash` are the derive macros there, and are kept)
#   TC<TAB><fn><TAB><trait><TAB><impls> a stable member of a stable SEALED trait, the trait by its CANONICAL path (the module that declares it:
#                                       unix's and windows' `OsStrExt` are different traits with different members), and the std types it is
#                                       implemented for by an UNGATED, stable impl in the declaring file (`-` for none)
#   N<TAB><path><TAB><trait>            a path a `use` can name that sealed trait by (`std::os::unix::prelude::OsStrExt`)
#   G<TAB><module><TAB><trait>          `use <module>::*` brings that sealed trait into scope on every target the module exists on (an ungated,
#                                       stable declaration or re-export, followed through named and glob re-exports)
# A missing TC/N/G row only leaves a dependency join in place (an over-charge); a WRONG one withholds a join where the dependency's member
# runs, so these rows are built from declarations only, never from names.
STD = os.path.join(lib, 'std', 'src')
trait_leaves = set()
for crate in ('core', 'alloc', 'std'):
    for root, dirs, files in os.walk(os.path.join(lib, crate, 'src')):
        dirs[:] = [d for d in dirs if d not in ('tests', 'benches')]
        for f in files:
            if f.endswith('.rs'):
                for t in pub_trait.finditer(strip_comments(open(os.path.join(root, f), errors='replace').read())):
                    trait_leaves.add(t.group(2))

def use_idents(text):
    """(leaf names, glob paths) of every `pub use ..;` statement in `text`."""
    names, globs = set(), []
    for m in re.finditer(r'\bpub\s+use\s+([^;]*);', text):
        body = re.sub(r'\s+', '', m.group(1))
        for leaf in re.findall(r'([A-Za-z_][A-Za-z0-9_]*)(?=[,}]|$)', body):
            if leaf not in ('self', 'super', 'crate'):
                names.add(leaf)
        if body.endswith('*'):
            globs.append(body)
    return names, globs

def read(path):
    return strip_comments(open(path, errors='replace').read())

prelude = {}
v1 = use_idents(read(os.path.join(lib, 'core', 'src', 'prelude', 'v1.rs')))[0] | use_idents(read(os.path.join(STD, 'prelude', 'v1.rs')))[0]
core_prelude = read(os.path.join(lib, 'core', 'src', 'prelude', 'mod.rs'))
for ed in ('2015', '2018', '2021', '2024'):
    mm = re.search(r'\bpub\s+mod\s+rust_%s\s*\{' % ed, core_prelude)
    extra = set()
    if mm:
        e = block_end(core_prelude, mm.end() - 1)
        extra = use_idents(core_prelude[mm.end() - 1:e + 1])[0] - {'v1'}
    prelude[ed] = {n for n in (v1 | extra) if n in trait_leaves}

def attrs_before(text, start):
    k = max(text.rfind(';', 0, start), text.rfind('}', 0, start), text.rfind('{', 0, start))
    return text[k + 1:start]

# the std module tree: module path -> text (a file, or an inline `pub mod x { .. }` block inside one)
mods = {}
def add_inline(mp, text):
    mods[mp] = text
    for mm in re.finditer(r'\bpub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{', text):
        if '#[unstable(' in attrs_before(text, mm.start()) or re.search(r'#\[cfg\(', attrs_before(text, mm.start())):
            continue
        e = block_end(text, mm.end() - 1)
        inner = text[mm.end():e]
        if text.count('{', 0, mm.start()) - text.count('}', 0, mm.start()) == 0:
            add_inline(mp + '::' + mm.group(1), inner)
for root, dirs, files in os.walk(STD):
    dirs[:] = [d for d in dirs if d not in ('tests', 'benches', 'sys', 'sys_common')]
    for f in files:
        if not f.endswith('.rs') or f in ('tests.rs', 'benches.rs'):
            continue
        rel = os.path.relpath(os.path.join(root, f), STD)[:-3].split(os.sep)
        segs = [x for x in rel if x not in ('mod', 'lib')]
        add_inline('::'.join(['std'] + segs), read(os.path.join(root, f)))

def top_level(text, pos):
    return text.count('{', 0, pos) - text.count('}', 0, pos) == 0

unstable_memo = {}
def module_unstable(mp):
    """the module, or a module above it, is `#![unstable]` (an inner attribute) or declared `#[unstable] pub mod x`"""
    if mp in unstable_memo: return unstable_memo[mp]
    r = '#![unstable(' in mods.get(mp, '')
    if not r and '::' in mp:
        par, leaf = mp.rsplit('::', 1)
        pt = mods.get(par, '')
        # every declaration unstable (a doc-only or unsupported-target arm of a `cfg_select!` declares `#[unstable] pub mod unix {}` beside
        # the real, stable `pub mod unix;`)
        decls = ['#[unstable(' in attrs_before(pt, mm.start()) for mm in re.finditer(r'\bpub\s+mod\s+%s\b' % re.escape(leaf), pt)]
        r = (bool(decls) and all(decls)) or module_unstable(par)
    unstable_memo[mp] = r
    return r

def decl_sealed(mp):
    """sealed traits DECLARED at the top level of module mp: name -> (stable, ungated)"""
    out = {}
    if module_unstable(mp):
        return out
    t = mods.get(mp, '')
    for m in pub_trait.finditer(t):
        if m.group(1) and top_level(t, m.start()):
            a = attrs_before(t, m.start())
            out[m.group(2)] = ('#[unstable(' not in a, not re.search(r'#\[cfg\(', a))
    return out

def resolve_mod(mp, segs):
    """the module a `use` path's module part names, from module mp; None outside std"""
    cur = mp.split('::')
    if not segs:
        return mp
    if segs[0] == 'crate' or segs[0] == 'std':
        cur, segs = ['std'], segs[1:]
    elif segs[0] == 'self':
        segs = segs[1:]
    elif segs[0] == 'super':
        while segs and segs[0] == 'super':
            cur, segs = cur[:-1], segs[1:]
    elif segs[0] in ('core', 'alloc'):
        return None
    for x in segs:
        cur = cur + [x]
    return '::'.join(cur)

def reexports(mp):
    """top-level `pub use` of module mp: (bound name, target module, target name, stable, ungated); a glob is name `*`. A renamed
    re-export (`pub use x::A as B`) binds `B`."""
    t = mods.get(mp, '')
    out = []
    for m in re.finditer(r'\bpub\s+use\s+([^;]*);', t):
        if not top_level(t, m.start()):
            continue
        a = attrs_before(t, m.start())
        st, ug = '#[unstable(' not in a, not re.search(r'#\[cfg\(', a)
        body = re.sub(r'\s+as\s+', ' as ', m.group(1).strip())
        body = re.sub(r'\s*(::|,|\{|\})\s*', r'\1', body)
        def expand(prefix, tree):
            if tree.startswith('{') and tree.endswith('}'):
                depth, cur, parts = 0, '', []
                for ch in tree[1:-1]:
                    if ch == ',' and depth == 0:
                        parts.append(cur); cur = ''; continue
                    depth += ch == '{'; depth -= ch == '}'; cur += ch
                if cur.strip(): parts.append(cur)
                for x in parts: expand(prefix, x.strip())
                return
            i = tree.find('::{')
            if i >= 0:
                expand(prefix + tree[:i].split('::'), tree[i + 2:]); return
            bound = None
            if ' as ' in tree:
                tree, bound = tree.split(' as ', 1)
            segs = prefix + tree.split('::')
            name = segs[-1]
            if name == 'self':
                return
            out.append((bound or name, resolve_mod(mp, segs[:-1]), name, st, ug))
        expand([], body)
    return out

canon_memo = {}
def canonical(mp, name, depth=0):
    """(canonical path, stable) of sealed trait `name` as nameable in module mp, or None"""
    key = (mp, name)
    if key in canon_memo: return canon_memo[key]
    canon_memo[key] = None
    if depth > 12 or mp is None: return None
    d = decl_sealed(mp)
    r = None
    if name in d:
        r = (mp + '::' + name, d[name][0], d[name][1])
    else:
        for b, tm, n, st, ug in reexports(mp):
            if n == '*' and tm:
                c = canonical(tm, name, depth + 1)
                if c: r = (c[0], c[1] and st, c[2] and ug); break
            elif b == name and tm:
                c = canonical(tm, n, depth + 1)
                if c: r = (c[0], c[1] and st, c[2] and ug); break
    canon_memo[key] = r
    return r

sealed_leaves = {tr for (_, tr) in sealed_fns}
N_rows, G_rows, canon_traits, N_ungated = set(), set(), set(), set()
for mp in sorted(mods):
    for leaf in sorted(sealed_leaves):
        c = canonical(mp, leaf)
        if c and c[1]:
            N_rows.add((mp + '::' + leaf, c[0]))
            canon_traits.add(c[0])
            if c[2]:
                G_rows.add((mp, c[0]))
                N_ungated.add((mp + '::' + leaf, c[0]))

TC_rows, expect_members = set(), {}
for ct in sorted(canon_traits):
    mp, leaf = ct.rsplit('::', 1)
    t = mods.get(mp, '')
    m = re.search(r'\bpub\s+(impl\s*\([^)]*\)\s+)(?:(?:const|unsafe|auto)\s+)*trait\s+%s\b' % re.escape(leaf), t)
    if not m: continue
    b = t.find('{', m.end()); e = block_end(t, b)
    body = t[b:e + 1]
    members = [fm.group(1) for fm in any_fn.finditer(body) if not unstable(body, fm.start())]
    impls = set()
    for im in re.finditer(r'(?<![A-Za-z0-9_])impl\s*(?:<[^{};]*?>)?\s*%s\s+for\s+&?\s*(?:[A-Za-z_]\w*::)*([A-Za-z_]\w*)' % re.escape(leaf), t):
        a = attrs_before(t, im.start())
        if top_level(t, im.start()) and '#[unstable(' not in a and not re.search(r'#\[cfg\(', a):
            impls.add(im.group(1))
    for n in members:
        TC_rows.add((n, ct, ','.join(sorted(impls)) or '-'))
    expect_members[ct] = set(members)

# SELF-CHECK (R1951 / R1956) — the scope rows must hold their structure, and the check must be able to fail on ANY dropped row:
# `STD_ASSOC_SELFCHECK_DROP_ROW='P|2024|Future'` (or `G|<module>|<trait>`, `N|<path>|<trait>`, `TC|<fn>|<trait>`) drops that row after it is
# derived and before the checks, and the generator must refuse (exit 3). Each row kind is checked against a second derivation: the prelude
# editions against each other and anchors; G against the ungated N bindings and N against G; TC against each trait's declared members.
P_rows = {(ed, n) for ed, ns in prelude.items() for n in ns}
drop = os.environ.get('STD_ASSOC_SELFCHECK_DROP_ROW')
if drop:
    k, a, b2 = drop.split('|', 2)
    if k == 'P': P_rows.discard((a, b2))
    elif k == 'G': G_rows.discard((a, b2))
    elif k == 'N': N_rows.discard((a, b2))
    elif k == 'TC': TC_rows = {r for r in TC_rows if (r[0], r[1]) != (a, b2)}
bad2 = []
ed_sets = {ed: {n for e2, n in P_rows if e2 == ed} for ed in ('2015', '2018', '2021', '2024')}
if ed_sets['2015'] != ed_sets['2018']:
    bad2.append('prelude: 2015 and 2018 differ')
if not (ed_sets['2018'] < ed_sets['2021'] < ed_sets['2024']):
    bad2.append('prelude: the editions are not strictly increasing (2018 < 2021 < 2024)')
for ed, need in (('2015', {'Clone', 'Default', 'Iterator', 'From', 'Into', 'Drop', 'ToString', 'ToOwned', 'IntoIterator', 'Extend'}),
                 ('2021', {'TryFrom', 'TryInto', 'FromIterator'}), ('2024', {'Future', 'IntoFuture'})):
    if not need <= ed_sets[ed]:
        bad2.append('prelude %s lacks %s' % (ed, sorted(need - ed_sets[ed])))
gmods = {}
for m_, t_ in G_rows:
    gmods.setdefault(m_, set()).add(t_)
for need_m, need_l in (('std::os::unix::prelude', ('OsStrExt', 'OsStringExt', 'CommandExt', 'ExitStatusExt')),
                       ('std::os::unix::ffi', ('OsStrExt', 'OsStringExt'))):
    have = {t_.rsplit('::', 1)[1] for t_ in gmods.get(need_m, ()) if t_.startswith('std::os::unix::')}
    if not set(need_l) <= have:
        bad2.append('glob %s lacks %s' % (need_m, sorted(set(need_l) - have)))
for m_, ts in gmods.items():   # a prelude module re-exports: every trait it brings is brought by some sibling module too
    if m_.endswith('::prelude'):
        par = m_.rsplit('::', 1)[0]
        sib = set().union(*[v for k2, v in gmods.items() if k2.startswith(par + '::') and k2 != m_])
        if not ts <= sib:
            bad2.append('glob %s brings %s that no sibling module declares or re-exports' % (m_, sorted(ts - sib)))
for _, t_ in G_rows | N_rows:
    if not any(r[1] == t_ for r in TC_rows):
        bad2.append('trait %s is nameable but has no TC member row' % t_)
for n_, t_, _ in TC_rows:
    if (t_.rsplit('::', 1)[0] + '::' + t_.rsplit('::', 1)[1], t_) not in N_rows and not any(x[1] == t_ for x in N_rows):
        bad2.append('trait %s has TC rows but no N path' % t_)
for pth, ct in N_ungated:
    if (pth.rsplit('::', 1)[0], ct) not in G_rows:
        bad2.append('glob %s lacks %s, which it binds ungated as %s' % (pth.rsplit('::', 1)[0], ct, pth))
for m_, ct in G_rows:
    if not any(p_.rsplit('::', 1)[0] == m_ and c_ == ct for p_, c_ in N_rows):
        bad2.append('glob %s brings %s but no N path names it there' % (m_, ct))
for ct, want in expect_members.items():
    have = {r[0] for r in TC_rows if r[1] == ct}
    if have != want:
        bad2.append('trait %s: TC members %s, declared %s' % (ct, sorted(have), sorted(want)))
if bad2:
    sys.stderr.write("std_assoc.py: SELF-CHECK FAILED — the scope rows are inconsistent:\n  " + "\n  ".join(sorted(set(bad2))) + "\n")
    sys.exit(3)

for l, n in sorted(inh):
    print(f"I\t{l}\t{n}")
for n in sorted(trait_fns):
    print(f"T\t{n}")
for n, tr in sorted(sealed_fns):
    print(f"TS\t{n}\t{tr}\t{','.join(sorted(impls_for.get(tr, ()))) or '-'}")

for ed, n in sorted(P_rows):
    print(f"P\t{ed}\t{n}")
for n, ct, im in sorted(TC_rows):
    print(f"TC\t{n}\t{ct}\t{im}")
for pth, ct in sorted(N_rows):
    print(f"N\t{pth}\t{ct}")
for m_, ct in sorted(G_rows):
    print(f"G\t{m_}\t{ct}")
