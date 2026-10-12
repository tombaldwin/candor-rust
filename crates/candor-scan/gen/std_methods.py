#!/usr/bin/env python3
"""SOUNDNESS R1136 — generate (method name, non-self argument count) for every PUBLIC std/core/alloc method:
an inherent `pub fn` taking `self`, and every fn declared in a `pub trait` body that takes `self`.
usage: gen_std_methods.py <rust-src library dir>  -> `name<TAB>argc` lines, sorted, unique."""
import os, re, sys
lib = sys.argv[1]
pub_fn = re.compile(r'\bpub\s+(?:const\s+)?(?:unsafe\s+)?(?:async\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)')
any_fn = re.compile(r'\bfn\s+([A-Za-z_][A-Za-z0-9_]*)')
pub_trait = re.compile(r'\bpub\s+(?:const\s+)?(?:unsafe\s+)?(?:auto\s+)?trait\s+[A-Za-z_][A-Za-z0-9_]*')

def strip_comments(s):
    s = re.sub(r'//[^\n]*', '', s)
    return re.sub(r'/\*.*?\*/', '', s, flags=re.S)

def params_after(s, i):
    if i < len(s) and s[i] == '<':
        d = 0
        while i < len(s):
            if s[i] == '<': d += 1
            elif s[i] == '>' and s[i-1] != '-':
                d -= 1
                if d == 0: i += 1; break
            i += 1
    while i < len(s) and s[i].isspace(): i += 1
    if i >= len(s) or s[i] != '(':
        return None
    d = 0; j = i
    while j < len(s):
        c = s[j]
        if c in '([{': d += 1
        elif c in ')]}': d -= 1
        if d == 0: break
        j += 1
    params = s[i+1:j]
    parts = []; d = 0; cur = ''
    for k, c in enumerate(params):
        if c in '([{<': d += 1
        elif c in ')]}': d -= 1
        elif c == '>' and not (k > 0 and params[k-1] == '-'): d -= 1
        if c == ',' and d == 0:
            parts.append(cur); cur = ''
        else:
            cur += c
    if cur.strip(): parts.append(cur)
    return [p.strip() for p in parts if p.strip()]

def takes_self(parts):
    if not parts: return False
    first = re.sub(r'#\[[^\]]*\]\s*', '', parts[0]).strip()
    return re.match(r"^(&\s*('[A-Za-z_]+\s+)?)?(mut\s+)?self\b", first) is not None

out = set()
def unstable(s, m):
    # the attributes between the previous item boundary and this fn
    k = max(s.rfind(';', 0, m.start()), s.rfind('}', 0, m.start()), s.rfind('{', 0, m.start()))
    return '#[unstable(' in s[k + 1:m.start()]

def add(s, m):
    if unstable(s, m):
        return
    ps = params_after(s, m.end())
    if ps is not None and takes_self(ps):
        out.add((m.group(1), len(ps) - 1))

for crate in ('core', 'alloc', 'std'):
    for root, dirs, files in os.walk(os.path.join(lib, crate, 'src')):
        dirs[:] = [d for d in dirs if d not in ('tests', 'benches')]
        for f in files:
            if not f.endswith('.rs') or f in ('tests.rs', 'benches.rs'):
                continue
            s = strip_comments(open(os.path.join(root, f), errors='replace').read())
            for m in pub_fn.finditer(s):
                add(s, m)
            for t in pub_trait.finditer(s):
                b = s.find('{', t.end())
                if b < 0: continue
                d = 0; e = b
                while e < len(s):
                    if s[e] == '{': d += 1
                    elif s[e] == '}':
                        d -= 1
                        if d == 0: break
                    e += 1
                body = s[b:e]
                for m in any_fn.finditer(body):
                    add(body, m)
for n, a in sorted(out):
    print(f"{n}\t{a}")
