import json
d=json.load(open("/tmp/wide_results.json"))
rows={r["name"]:r for r in d["rows"]}
pure=d["pure"]

print("="*70)
print("1. PURE-CRATE FALSE POSITIVES (these should report ~0 effects)")
print("="*70)
fp=[]
for name in pure:
    r=rows.get(name)
    if not r: continue
    if r["eff"]:
        fp.append((name, r["eff"], r["direct"]))
if not fp:
    print("  NONE — every curated-pure crate reported zero effects.")
else:
    for name,eff,direct in fp:
        print(f"  {name:<22} eff={eff}  direct_sources={direct}")
print(f"  ({len([p for p in pure if p in rows])} pure crates on disk checked)")

print("="*70)
print("2. EXPLOSION CANDIDATES (effect amplified from few direct sources)")
print("="*70)
# amplification = total effectful fns / total distinct direct-source fns
cand=[]
for r in d["rows"]:
    tot_eff=sum(r["eff"].values())
    tot_dir=sum(r["direct"].values())
    if r["fns"]>=40 and tot_dir>0:
        amp=r["fns"]/tot_dir  # effectful fns per direct source
        if amp>=8:
            cand.append((amp, r["name"], r["fns"], tot_dir, r["eff"]))
cand.sort(reverse=True)
for amp,name,fns,dir,eff in cand[:15]:
    print(f"  amp={amp:5.1f}  {name:<20} {fns:>4} effectful / {dir:>3} direct  {eff}")
if not cand: print("  none above threshold")

print("="*70)
print("3. OVERALL")
print("="*70)
total=len(d["rows"])
with_eff=sum(1 for r in d["rows"] if r["eff"])
print(f"  {total} crates scanned, {len(d['errors'])} errors/timeouts")
print(f"  {with_eff} report >=1 effect ({100*with_eff//total}%); {total-with_eff} report none")
from collections import Counter
alleff=Counter()
for r in d["rows"]:
    for e in r["eff"]: alleff[e]+=1
print("  crates touching each effect:", dict(alleff.most_common()))
