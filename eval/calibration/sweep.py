import os, glob, subprocess, json, re, sys
REG="/Users/tom/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f"
SCAN="/Users/tom/git/candor-rust/target/release/candor-scan"
SAMPLE = [
 ("reqwest","Net (HTTP client)"),("hyper","Net (HTTP)"),("ureq","Net (blocking HTTP)"),
 ("tokio","Net/Fs/Clock (async runtime)"),("mio","Net/Ipc (low-level IO)"),("tonic","Net (gRPC)"),
 ("rusqlite","Db (SQLite FFI)"),("redis","Db/Net"),("tempfile","Fs/Env/Rand"),
 ("walkdir","Fs (dir walk)"),("notify","Fs (file watch)"),("memmap2","Fs (mmap)"),
 ("git2","Fs/Net/Exec (libgit2)"),("flate2","Fs (compression)"),("tar","Fs (archives)"),
 ("zip","Fs (archives)"),("csv","Fs (csv io)"),("config","Fs/Env (config)"),
 ("clap","Env (arg parsing)"),("chrono","Clock"),("time","Clock"),("uuid","Rand/Clock"),
 ("rand","Rand"),("ring","Rand (crypto)"),("rustls","Net (TLS)"),("sha2","pure (hashing)"),
 ("regex","pure"),("serde_json","pure (parse)"),("toml","pure (parse)"),
 ("nix","Fs/Exec/Net/Ipc (syscalls)"),("which","Fs/Env/Exec"),("dirs","Fs/Env"),
 ("tracing","Log"),("log","Log"),("crossbeam-channel","Ipc (channels)"),
]
def newest(name):
    cands = glob.glob(f"{REG}/{name}-[0-9]*")
    cands = [c for c in cands if re.match(rf"{re.escape(name)}-\d", os.path.basename(c))]
    def ver(p):
        m = re.search(r'-(\d+)\.(\d+)\.(\d+)', os.path.basename(p))
        return tuple(int(x) for x in m.groups()) if m else (0,0,0)
    return max(cands, key=ver) if cands else None

rows=[]
for name, expect in SAMPLE:
    d = newest(name)
    if not d:
        rows.append({"name":name,"expect":expect,"missing":True}); continue
    nrs = sum(1 for _ in glob.glob(d+"/**/*.rs", recursive=True))
    try:
        out = subprocess.run([SCAN, d, "--json"], capture_output=True, text=True, timeout=120)
        doc = json.loads(out.stdout)
        fns = doc["functions"]
        from collections import Counter
        eff=Counter()
        for f in fns:
            for e in f["inferred"]: eff[e]+=1
        rows.append({"name":name,"expect":expect,"ver":os.path.basename(d).replace(name+"-",""),
                     "rs":nrs,"fns":len(fns),"eff":dict(eff.most_common())})
    except Exception as ex:
        rows.append({"name":name,"expect":expect,"error":str(ex)[:80]})
json.dump(rows, open("/tmp/sweep_results.json","w"), indent=1)
print("done:", len(rows), "crates")
