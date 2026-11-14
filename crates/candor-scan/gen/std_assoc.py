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
The integer types' inherent fns are written inside `int_impl!`/`uint_impl!`, so every `pub fn` in
`int_macros.rs` / `uint_macros.rs` is filed under every signed / unsigned integer leaf, and every `pub fn` in a
`macro_rules!` of `core/src/num/mod.rs` that writes an integer impl (`from_str_int_impl!`) under every integer leaf."""
import os, re, sys
lib = sys.argv[1]
pub_fn = re.compile(r'\bpub\s+(?:const\s+)?(?:unsafe\s+)?(?:async\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)')
any_fn = re.compile(r'\bfn\s+([A-Za-z_][A-Za-z0-9_]*)')
pub_trait = re.compile(r'\bpub\s+(?:const\s+)?(?:unsafe\s+)?(?:auto\s+)?trait\s+[A-Za-z_][A-Za-z0-9_]*')
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

inh = set(); trait_fns = set()
for crate in ('core', 'alloc', 'std'):
    for root, dirs, files in os.walk(os.path.join(lib, crate, 'src')):
        dirs[:] = [d for d in dirs if d not in ('tests', 'benches')]
        for f in files:
            if not f.endswith('.rs') or f in ('tests.rs', 'benches.rs'):
                continue
            s = strip_comments(open(os.path.join(root, f), errors='replace').read())
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
                    trait_fns.add(m.group(1))
for l, n in sorted(inh):
    print(f"I\t{l}\t{n}")
for n in sorted(trait_fns):
    print(f"T\t{n}")
