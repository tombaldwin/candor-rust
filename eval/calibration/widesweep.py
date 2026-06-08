import json, subprocess, glob, os, re, sys
from collections import Counter
REG="/Users/tom/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f"
SCAN="/Users/tom/git/candor/target/release/candor-scan"

# Curated HIGH-CONFIDENCE pure crates: encoders, data structures, text/parse, hashing-compute, math,
# token manipulation. These should report ~0 effects — any effect is a false-positive candidate.
PURE = set("""itoa ryu base64 base64ct hex percent-encoding form_urlencoded httparse httpdate mime
mime_guess smallvec arrayvec tinyvec indexmap hashbrown bytes bitflags either slab memchr aho-corasick
regex-syntax regex-automata nom unicode-segmentation unicode-width unicode-ident unicode-normalization
strsim textwrap heck sha1 sha2 sha3 md5 crc32fast digest generic-array block-buffer fnv siphasher adler
adler2 ahash crc typenum num-bigint num-traits num-integer num-complex num-rational ordered-float serde
quote proc-macro2 syn itertools semver lazy_static once_cell smol_str ryu-js encoding_rs cesu8 utf8parse
const-oid spki pem-rfc7468 subtle constant_time_eq universal-hash crypto-common cipher
unicode-xid pin-project-lite futures-core futures-sink anyhow thiserror""".split())

def newest(name):
    cands = glob.glob(f"{REG}/{name}-[0-9]*")
    cands = [c for c in cands if re.match(rf"{re.escape(name)}-\d", os.path.basename(c))]
    def ver(p):
        m = re.search(r'-(\d+)\.(\d+)\.(\d+)', os.path.basename(p))
        return tuple(int(x) for x in m.groups()) if m else (0,0,0)
    return max(cands, key=ver) if cands else None

names = [l.strip() for l in open("/tmp/crate_names.txt") if l.strip()]
rows=[]
errors=[]
for name in names:
    d = newest(name)
    if not d: continue
    try:
        out = subprocess.run([SCAN, d, "--json"], capture_output=True, text=True, timeout=60)
        if out.returncode != 0:
            errors.append((name, "exit"+str(out.returncode))); continue
        doc = json.loads(out.stdout)
    except subprocess.TimeoutExpired:
        errors.append((name,"timeout")); continue
    except Exception as e:
        errors.append((name, str(e)[:40])); continue
    fns = doc["functions"]
    eff = Counter()
    direct_src = Counter()  # distinct functions that DIRECTLY source each effect
    for f in fns:
        for e in f["inferred"]: eff[e]+=1
        for e in f.get("direct",[]): direct_src[e]+=1
    rows.append({"name":name,"ver":os.path.basename(d).replace(name+"-",""),
                 "fns":len(fns),"eff":dict(eff),"direct":dict(direct_src)})
json.dump({"rows":rows,"errors":errors,"pure":sorted(PURE)}, open("/tmp/wide_results.json","w"))
print(f"scanned {len(rows)} crates, {len(errors)} errors")
