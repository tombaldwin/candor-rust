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
    TS<TAB><fn name><TAB><trait>      a fn declared in a SEALED `pub impl(..) trait` body (R1913)
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
# `TS<TAB>fn<TAB>trait` rows, which a consumer asks beside the module's imports.
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

inh = set(); trait_fns = set(); sealed_fns = set()
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
                        (sealed_fns.add((m.group(1), t.group(2))) if t.group(1) else trait_fns.add(m.group(1)))
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
                    (sealed_fns.add((m.group(1), t.group(2))) if t.group(1) else trait_fns.add(m.group(1)))
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

for l, n in sorted(inh):
    print(f"I\t{l}\t{n}")
for n in sorted(trait_fns):
    print(f"T\t{n}")
for n, tr in sorted(sealed_fns):
    print(f"TS\t{n}\t{tr}")
