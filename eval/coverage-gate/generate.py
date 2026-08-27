#!/usr/bin/env python3
"""The completeness-gate GENERATOR for CALIBRATED_CRATES (crates/candor-classify/src/lib.rs:2458-2479's
coverage ledger treats an unmatched path in a calibrated crate as a CLAIM of reviewed purity — no
disclosure. Calibrating a crate converts "no rule" into "checked, and it's pure"; nothing previously
enforced that the verb list backing that claim was actually complete. This script is the enforcement.

THE METHOD — self-scan as the oracle, not a hand-written "looks effectful" heuristic. `candor-scan` is a
syntactic, no-rustc engine: point it at a crate's OWN vendored source (no cargo build, no type-checking,
just `syn`-parse + local call-graph propagation) and it computes each function's effect via REAL reachability
to std/FFI primitives and to classify()'s OWN existing rules for that crate — independent of whether the
public entry point ITSELF has a top-level rule. Proven empirically: self-scanning `ignore` with the
classify.rs commit BEFORE `ignore::Walk::new` was ever given its own rule still reports `Walk::new` as
`Fs` (via local propagation through `.build()` -> the pre-existing `WalkBuilder::build` rule) — the exact
defect fixed in 19ce144, found by an oracle that needs no per-crate hand tuning.

THE PIPELINE, per calibrated crate:
  1. Locate the crate's real source in the local cargo registry cache (`~/.cargo/registry/src/*`).
  2. Extract candidate PUBLIC entry points: `pub fn`s (including trait-impl methods where the trait is
     declared `pub trait` in the SAME crate — catches diesel's `establish`, which the `Connection` trait
     declares but no impl marks `pub`), excluding types declared ONLY as `pub(crate)`/`pub(super)`/private
     (a real false-positive class found and fixed during the first run: diesel's `RawConnection` is
     `pub(super)`, so its inherent `pub fn exec` is not actually reachable by any external consumer).
  3. Self-scan the crate's own source with `candor-scan --json`, correlating each candidate to its
     self-scan report entry by the module-qualified path candor-scan itself derives.
  4. For every candidate whose self-scan `inferred` set contains Fs/Net/Db/Exec (see classify_check's
     header for why this is the trigger, not a bare `invisible` or `Unknown`), guess the consumer-facing
     spelling(s) a caller would actually use (the raw module path AND, if the type/fn is re-exported at
     the crate root via `pub use`, the shorter root-alias form — `ignore::Walk::new`, not
     `ignore::walk::Walk::new`) and hand them to `classify_check` (a tiny Rust binary — the ONLY thing
     that calls the real `candor_classify::classify`, so this script never re-derives its rules).
  5. `classify_check` writes covered.tsv (every candidate classify() DOES recognize — the HARD,
     regression-proof list `crates/candor-classify/tests/coverage_gate.rs` asserts every push) and
     open.tsv (the RATCHET — everything self-scan found effectful that no rule recognizes yet).

SCOPE — CALIBRATED_CRATES minus 8 exclusions, each a DIFFERENT kind of "this gate's method doesn't apply
here" rather than "not worth checking":
  - tokio_tcp, tokio_udp, async_net: classified BLANKET (crate_name match alone, any path -> Net) —
    immune to a verb-list gap by construction; there is no allowlist to audit.
  - rustc_lint, rustc_errors: compiler-internal crates (rustc_private), not published to crates.io — no
    independently-versioned "real vendored source" to fetch or self-scan.
  - libc, nix, rustix: exhaustive raw-syscall FFI NAME tables, not "wrapper crate with a verb allowlist".
    Nearly every symbol in these crates IS a syscall — auditing them the same way as, say, rusqlite would
    either flood (thousands of true positives) or require re-deriving libc's own syscall completeness,
    a different, already-differently-addressed question (see soundness/oracle.sh's strace ground truth).

MEASURED, this run (2026-08-27): 987 self-scan-confirmed core-effect candidates across 74 crates, 669
already covered by an existing rule (checked in as covered.tsv), 260 not yet recognized by any rule under
any guessed spelling (checked in as open.tsv, the ratchet). Historical recall against the pre-19ce144
classify.rs (git show 19ce144~1): the differential correctly flags `ignore::Walk::new`,
`git2::Submodule::clone`/`::update`, `mongodb::Client::with_options`, `mysql_async::Conn::new`, and all
five of last night's rusqlite entries (`open_in_memory_with_flags`/`open_with_flags_and_vfs`/
`open_in_memory_with_flags_and_vfs`/`blob_open`/`Blob::reopen`) as newly-covered-only-after-the-fix — i.e.
this gate WOULD have caught 5 of the ten incident groups outright. It MISSES diesel's `establish`,
`mysql::Conn::new` (sync), `sea_orm::connect_proxy`, `tokio_postgres::connect_raw`, and 8 of tungstenite's
9 sites: each of those reaches its real effect by crossing into a DIFFERENT, uncalibrated external crate
(libsqlite3-sys, mysql_common, hyper-adjacent transports, …), which self-scan can only report as `Unknown`
(the deep resolver hit a boundary it can't see through) rather than a concrete Fs/Net/Db/Exec — and
tungstenite's handshake functions are transport-GENERIC (`Read + Write` bounds, not a concrete
`std::net` type), so there is no literal syscall in tungstenite's own source to propagate from at all; the
effect there is a judgment call about protocol semantics, not something reachable analysis can derive.
Including `Unknown` as a qualifying signal was measured and rejected: it roughly triples the candidate
count (987 -> 2423) and the open list (260 -> 1323) for a modest recall gain — a flood, by this project's
own standard. See REPORT (the commit that added this file) for the full measurement and a worked example
of the inverse failure mode: self-scanning `async_process` flagged its `Command::args`/`::env`/`::arg0`
builder setters as "Exec" purely because they delegate to `std::process::Command`'s OWN (separately,
pre-existing) coarse whole-type rule, even though `async_process`'s OWN hand-tuned exclusion list already
treats them as pure correctly — those 8 were removed from open.tsv after being traced to that std-rule
cross-talk, not to a real async_process gap.

USAGE:
  python3 eval/coverage-gate/generate.py --print-fixture-toml > /tmp/fixture/Cargo.toml
    Emits a throwaway Cargo.toml listing every in-scope crate as a `"*"` dep, derived from the real
    CALIBRATED_CRATES const (never hand-copied). `cd /tmp/fixture && cargo fetch` then downloads fresh
    crates.io source for all of them into the local registry cache — the ONLY step that needs network
    (see .github/workflows/coverage-gate-refresh.yml, the only place this repo runs it automatically).

  python3 eval/coverage-gate/generate.py
    Regenerates covered.tsv and open.tsv in this directory from local `~/.cargo/registry/src` sources.
    Requires: `cargo build --release -p candor-scan` already run; the in-scope crates already fetched
    (the `--print-fixture-toml` step above, if not already in the local registry cache from routine use).

  python3 eval/coverage-gate/generate.py --registry /path/to/registry/src/index.crates.io-HASH
    Point at a specific registry source dir (the refresh workflow's fresh-fetch cache, not `~/.cargo`).
"""
import re, glob, os, sys, json, subprocess, argparse

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
CLASSIFY_SRC = os.path.join(REPO_ROOT, "crates", "candor-classify", "src", "lib.rs")
SCAN_BIN = os.path.join(REPO_ROOT, "target", "release", "candor-scan")
CLASSIFY_CHECK_MANIFEST = os.path.join(HERE, "classify_check", "Cargo.toml")

# See the module docstring for why each of these is a DIFFERENT kind of out-of-scope.
EXCLUDE = {"tokio_tcp", "tokio_udp", "async_net", "rustc_lint", "rustc_errors", "libc", "nix", "rustix"}

# Cargo.toml `package = "..."` renames this gate needs to know about to find the real crates.io source
# (CALIBRATED_CRATES uses the Rust IDENTIFIER a consumer writes, which is not always the registry name).
# `find_crate_dir` also tries an underscore->hyphen fallback when LOCATING an already-fetched directory
# on disk, so most of these are redundant there — but `print_fixture_toml` needs the exact real name
# up front (no fallback: cargo's dependency resolution has no "try the other spelling" step), so every
# crate whose registry name uses hyphens where the Rust identifier uses underscores is listed here.
RENAME = {
    "native_tls_crate": "native-tls",
    "aws_config": "aws-config", "async_nats": "async-nats", "tokio_postgres": "tokio-postgres",
    "sea_orm": "sea-orm", "deadpool_postgres": "deadpool-postgres", "fs_err": "fs-err",
    "async_fs": "async-fs", "password_hash": "password-hash", "portable_pty": "portable-pty",
    "async_process": "async-process", "tokio_native_tls": "tokio-native-tls",
    "sqlx_core": "sqlx-core", "terminal_colorsaurus": "terminal-colorsaurus",
    "grep_cli": "grep-cli", "tracing_subscriber": "tracing-subscriber",
}


def calibrated_crates_from_source():
    """The single source of truth is `crates/candor-classify/src/lib.rs`'s own `CALIBRATED_CRATES`
    const — read it at generation time rather than hand-copying the list here, so this script cannot
    silently drift from the real one (the exact failure mode several other parts of this codebase have
    been bitten by; see MEMORY.md's `candor-changelogs-releases-page` "ONE spelling drifts" pattern)."""
    src = open(CLASSIFY_SRC).read()
    m = re.search(r'pub const CALIBRATED_CRATES:\s*\[&str;\s*\d+\]\s*=\s*\[(.*?)\];', src, re.DOTALL)
    if not m:
        raise SystemExit("could not find CALIBRATED_CRATES in " + CLASSIFY_SRC)
    return re.findall(r'"([A-Za-z0-9_]+)"', m.group(1))


VERSION_TAIL = re.compile(r'^\d')


def find_crate_dir(registry_src, name):
    pkg = RENAME.get(name, name)

    def exact(pkgname):
        out = []
        for h in glob.glob(os.path.join(registry_src, pkgname + "-*")):
            tail = os.path.basename(h)[len(pkgname) + 1:]
            if VERSION_TAIL.match(tail):
                out.append(h)
        return out

    hits = exact(pkg) or exact(pkg.replace("_", "-"))
    if not hits:
        return None
    hits.sort()  # lexicographic version sort is good enough to pick "a" newest, not necessarily THE newest
    return hits[-1]


def strip_comments(src):
    """Crude but sufficient for THIS purpose (finding fn/impl/use syntax, not full parsing): strips
    `//` and `/* */` comments while respecting string-literal boundaries so a `//` inside a doc example
    string doesn't eat the rest of the file."""
    out = []
    i, n, in_str, in_block = 0, len(src), False, False
    while i < n:
        c = src[i]
        if in_block:
            if src[i:i + 2] == "*/":
                in_block = False
                i += 2
                continue
            i += 1
            continue
        if in_str:
            out.append(c)
            if c == '\\' and i + 1 < n:
                out.append(src[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if src[i:i + 2] == "//":
            j = src.find("\n", i)
            if j == -1:
                break
            i = j
            continue
        if src[i:i + 2] == "/*":
            in_block = True
            i += 2
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


TRAIT_DECL_RE = re.compile(r'\bpub\s+trait\s+(\w+)')
IMPL_RE = re.compile(r'\bimpl\b')
FN_HEAD_RE = re.compile(r'(pub(?:\([^)]*\))?\s+)?(async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+(\w+)\s*')
BORING_TRAITS = {
    "Debug", "Clone", "Default", "Display", "PartialEq", "Eq", "Hash", "PartialOrd", "Ord",
    "From", "Into", "TryFrom", "TryInto", "Drop", "Deref", "DerefMut", "AsRef", "AsMut",
    "Serialize", "Deserialize", "Send", "Sync", "Iterator", "IntoIterator", "FromIterator", "Extend",
}
TYPE_DECL_RE = re.compile(r'\b(pub(?:\([^)]*\))?\s+)?(?:struct|enum)\s+(\w+)')


def is_bare_pub(pub_kw):
    """True only for a BARE `pub` — `pub(crate)`/`pub(super)`/`pub(in ...)` are restricted visibility,
    not reachable by an external consumer at all, and must not count as a public entry point. Mirrors
    `restricted_types()`'s treatment of types (found via diesel's `RawConnection`), but that check never
    covered plain FUNCTIONS: `sea_orm::DatabaseTransaction::begin`/`::run` (transaction.rs:32,96) and
    every one of ureq's module-private `connect`/`connect_host`/`connect_http`/`connect_https`
    (unit.rs:155, stream.rs:326,334,347) are `pub(crate) fn`, syntactically matched `pub(?:\\(...\\))?`
    by the old regex just like a bare `pub fn` — so the coverage-gate ratchet carried them as open
    "gaps" a real consumer can never trigger (calling them from outside the crate does not compile).
    FN_HEAD_RE's capture group includes the parenthesised qualifier verbatim when present, so the fix is
    a plain string check: only whitespace may follow the `pub` token for it to be bare."""
    return pub_kw is not None and pub_kw.strip() == "pub"


PUB_USE_RE = re.compile(r'pub\s+use\s+(?:crate::)?([\w:]+)::\{([^}]*)\}\s*;')
PUB_USE_SINGLE_RE = re.compile(r'pub\s+use\s+(?:crate::)?([\w:]+)::(\w+)(?:\s+as\s+(\w+))?\s*;')


def find_fns(body):
    """Yield (start, pub_kw, is_async, fn_name) for each `fn` item at the top of `body`, skipping an
    arbitrarily-NESTED generic parameter list (`<...>`) before requiring `(`. A first version of this
    used a single regex with `<[^>(]*>` for the generics, which cannot match `<P: AsRef<Path>>` (it stops
    at the FIRST `>`, closing on the wrong one) — silently dropping `ignore::Walk::new` itself, the
    flagship example this gate exists to catch. Fixed by scanning forward with real depth-tracking."""
    for fm in FN_HEAD_RE.finditer(body):
        pub_kw, is_async, fn_name = fm.groups()
        i, n = fm.end(), len(body)
        while i < n and body[i].isspace():
            i += 1
        if i < n and body[i] == '<':
            depth = 0
            while i < n:
                if body[i] == '<':
                    depth += 1
                elif body[i] == '>':
                    depth -= 1
                    if depth == 0:
                        i += 1
                        break
                i += 1
            while i < n and body[i].isspace():
                i += 1
        if i < n and body[i] == '(':
            yield fm.start(), pub_kw, is_async, fn_name


def find_matching_brace(s, start):
    depth, i, n = 0, start, len(s)
    while i < n:
        if s[i] == '{':
            depth += 1
        elif s[i] == '}':
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


def parse_impl_header(s, impl_kw_pos):
    brace_pos = s.find('{', impl_kw_pos)
    if brace_pos == -1:
        return None
    return s[impl_kw_pos:brace_pos], brace_pos


def extract_type_and_trait(header):
    h = header[len('impl'):].strip()
    if h.startswith('<'):
        depth, i = 0, 0
        for i, c in enumerate(h):
            if c == '<':
                depth += 1
            elif c == '>':
                depth -= 1
            if depth == 0:
                break
        h = h[i + 1:].strip()
    if ' for ' in h:
        trait_part, ty_part = h.split(' for ', 1)
        trait_name = re.split(r'[<\s]', trait_part.strip())[0].rsplit('::', 1)[-1]
    else:
        trait_name, ty_part = None, h
    ty_part = ty_part.split(' where')[0].strip()
    ty_name = re.split(r'[<\s(]', ty_part)[0].rsplit('::', 1)[-1]
    if not re.match(r'^[A-Za-z_]\w*$', ty_name):
        return None, trait_name
    return ty_name, trait_name


def module_path_for(file_path, crate_src_root):
    rel = os.path.relpath(file_path, crate_src_root)
    rel = rel[:-3] if rel.endswith(".rs") else rel
    parts = rel.split(os.sep)
    if parts[-1] == "mod":
        parts = parts[:-1]
    if parts and parts[0] in ("lib", "main"):
        parts = parts[1:]
    return "::".join(p for p in parts if p)


def restricted_types(concat_src):
    """Types declared ONLY as `pub(crate)`/`pub(super)`/`pub(in ...)`/private — never a bare `pub` —
    are not reachable by an external consumer at all, so a `pub fn` inside their inherent impl is not a
    real public entry point (found via diesel's `RawConnection`, `pub(super) struct RawConnection`)."""
    seen_public, seen_any = set(), set()
    for m in TYPE_DECL_RE.finditer(concat_src):
        vis, name = m.groups()
        seen_any.add(name)
        if vis is not None and not vis.startswith('pub('):
            seen_public.add(name)
    return seen_any - seen_public


def root_reexports(lib_rs_src):
    """Best-effort, ONE level: leaf names `pub use`-exported at the crate root of `src/lib.rs`. Lets the
    candidate-path guesser also try the SHORT alias form (`ignore::Walk::new`) alongside the raw
    module-qualified one (`ignore::walk::Walk::new`) — several of last night's fixes are exact-equality
    rules keyed on the short form specifically (`path == "ignore::Walk::new"`), which a module-qualified
    guess alone would never match. Does not follow re-export CHAINS or glob (`pub use foo::*`) — a
    documented limitation, not a correctness bug: missing an alias only loses recall (an entry that IS
    covered gets mis-reported as open), never produces a false "covered" claim.
    """
    leaves = set()
    for m in PUB_USE_RE.finditer(lib_rs_src):
        for leaf in m.group(2).split(','):
            leaf = leaf.strip()
            if not leaf or leaf == '*':
                continue
            leaf = leaf.split(' as ')[-1].strip().split('::')[-1].strip()
            if leaf:
                leaves.add(leaf)
    for m in PUB_USE_SINGLE_RE.finditer(lib_rs_src):
        leaves.add(m.group(3) or m.group(2))
    return leaves


def process_crate(registry_src, crate_name):
    d = find_crate_dir(registry_src, crate_name)
    if d is None:
        return None, None
    srcdir = os.path.join(d, "src")
    if not os.path.isdir(srcdir):
        return d, None
    files = glob.glob(os.path.join(srcdir, "**", "*.rs"), recursive=True)
    texts = {}
    for f in files:
        try:
            raw = open(f, errors="ignore").read()
        except OSError:
            continue
        texts[f] = strip_comments(raw)
    concat = "\n".join(texts.values())
    pub_traits = set(TRAIT_DECL_RE.findall(concat))
    restricted = restricted_types(concat)
    reexported = root_reexports(texts.get(os.path.join(srcdir, "lib.rs"), ""))  # noqa: F841 (kept for guess step)

    public_entries = []  # (module_path, type_name_or_None, fn_name, file)
    for f, s in texts.items():
        mp = module_path_for(f, srcdir)
        for start, pub_kw, is_async, fn_name in find_fns(s):
            depth = s[:start].count('{') - s[:start].count('}')
            if depth == 0 and is_bare_pub(pub_kw):
                public_entries.append((mp, None, fn_name, f))
        for m in IMPL_RE.finditer(s):
            res = parse_impl_header(s, m.start())
            if not res:
                continue
            header, brace_pos = res
            if len(header) > 400:  # a runaway/garbage match; real impl headers are short
                continue
            end = find_matching_brace(s, brace_pos)
            if end == -1:
                continue
            body = s[brace_pos + 1:end]
            ty_name, trait_name = extract_type_and_trait(header)
            if not ty_name or ty_name in restricted:
                continue
            if trait_name in BORING_TRAITS:
                continue
            require_pub_kw = True
            if trait_name is not None:
                if trait_name not in pub_traits:
                    continue  # a foreign or non-public-in-this-crate trait: skip, avoid noise
                require_pub_kw = False  # a pub trait's impl methods are public with no `pub` keyword
            for start, pub_kw, is_async, fn_name in find_fns(body):
                d2 = body[:start].count('{') - body[:start].count('}')
                if d2 == 0 and (is_bare_pub(pub_kw) or not require_pub_kw):
                    public_entries.append((mp, ty_name, fn_name, f))

    out = []
    for mp, ty, fn, f in public_entries:
        guesses = set()
        if ty:
            if mp:
                guesses.add(f"{crate_name}::{mp}::{ty}::{fn}")
            guesses.add(f"{crate_name}::{ty}::{fn}")  # crate-root re-export alias guess
        else:
            if mp:
                guesses.add(f"{crate_name}::{mp}::{fn}")
            guesses.add(f"{crate_name}::{fn}")
        out.append({"crate": crate_name, "module": mp, "type": ty, "fn": fn,
                     "file": os.path.relpath(f, d), "guesses": sorted(guesses)})
    return d, out


def self_scan(scan_bin, d):
    try:
        p = subprocess.run([scan_bin, d, "--json"], capture_output=True, text=True, timeout=180)
    except Exception as e:
        return None, str(e)
    try:
        return json.loads(p.stdout), None
    except Exception as e:
        return None, f"json parse failed: {e}: stdout[:200]={p.stdout[:200]!r} stderr[:200]={p.stderr[:200]!r}"


# `"*"` excludes prereleases by cargo's own semver rule — fine for the other 73, but `elasticsearch` has
# never published a stable version (only `-alpha.N`s), so `"*"` cannot resolve it at all. Pinned to a
# real prerelease explicitly; the weekly workflow bumping this occasionally (when a newer alpha ships)
# is a much smaller, self-contained staleness question than the gate's own.
VERSION_OVERRIDE = {"elasticsearch": "8.19.0-alpha.1"}


def print_fixture_toml():
    """Emit a throwaway Cargo.toml listing every in-scope crate as a `"*"` (always-latest) dependency, so
    `cargo fetch --manifest-path <this>` pulls fresh crates.io source for the weekly staleness check.
    Generated from `calibrated_crates_from_source()` (never hand-copied) so it cannot drift from
    CALIBRATED_CRATES; RENAME'd to the real crates.io package name where the Rust identifier differs."""
    print("[package]\nname = \"coverage-gate-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n")
    print("[dependencies]")
    for c in calibrated_crates_from_source():
        if c in EXCLUDE:
            continue
        pkg = RENAME.get(c, c)
        version = VERSION_OVERRIDE.get(c, "*")
        if pkg != c:
            print(f'{c} = {{ package = "{pkg}", version = "{version}" }}')
        else:
            print(f'{c} = "{version}"')


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--registry", default=None,
                    help="registry src dir (default: ~/.cargo/registry/src/index.crates.io-*, newest)")
    ap.add_argument("--scan-bin", default=SCAN_BIN, help="path to a built candor-scan binary")
    ap.add_argument("--print-fixture-toml", action="store_true",
                    help="print a fresh fixture Cargo.toml to stdout (for `cargo fetch`) and exit")
    args = ap.parse_args()

    if args.print_fixture_toml:
        print_fixture_toml()
        return

    registry_src = args.registry
    if registry_src is None:
        cands = sorted(glob.glob(os.path.expanduser("~/.cargo/registry/src/index.crates.io-*")))
        if not cands:
            raise SystemExit("no ~/.cargo/registry/src/index.crates.io-* found; run "
                              "`--print-fixture-toml` + `cargo fetch` first (see USAGE above), "
                              "or pass --registry")
        registry_src = cands[-1]
    if not os.path.exists(args.scan_bin):
        raise SystemExit(f"{args.scan_bin} not built; run: cargo build --release -p candor-scan")

    calibrated = calibrated_crates_from_source()
    all_entries, scan_errors = {}, {}
    for c in calibrated:
        if c in EXCLUDE:
            continue
        d, entries = process_crate(registry_src, c)
        if d is None:
            print(f"WARN no source for {c} under {registry_src}", file=sys.stderr)
            continue
        if entries is None:
            print(f"WARN no src/ dir for {c} at {d}", file=sys.stderr)
            continue
        doc, err = self_scan(args.scan_bin, d)
        if doc is None:
            scan_errors[c] = err
            print(f"WARN self-scan failed for {c}: {err}", file=sys.stderr)
            continue
        by_fn = {fe["fn"]: fe for fe in doc.get("functions", [])}
        for e in entries:
            key = "::".join(p for p in [e["module"], e["type"], e["fn"]] if p)
            fe = by_fn.get(key)
            e["self_scan_key"] = key
            e["self_scan_found"] = fe is not None
            e["inferred"] = fe.get("inferred", []) if fe else None
            e["invisible"] = fe.get("invisible", []) if fe else None
        all_entries[c] = entries

    entries_path = os.path.join(HERE, "entries.json")
    with open(entries_path, "w") as fh:
        json.dump(all_entries, fh, indent=1)
    if scan_errors:
        with open(os.path.join(HERE, "scan_errors.json"), "w") as fh:
            json.dump(scan_errors, fh, indent=1)

    total = sum(len(v) for v in all_entries.values())
    print(f"crates processed: {len(all_entries)} (of {len(calibrated) - len(EXCLUDE)} in scope)")
    print(f"total public-ish candidate entries: {total}")
    if scan_errors:
        print(f"self-scan FAILED for {len(scan_errors)} crates (see scan_errors.json): {sorted(scan_errors)}")

    covered_path = os.path.join(HERE, "covered.tsv")
    open_path = os.path.join(HERE, "open.tsv")
    subprocess.run(
        ["cargo", "run", "--quiet", "--release", "--manifest-path", CLASSIFY_CHECK_MANIFEST,
         "--", entries_path, covered_path, open_path],
        check=True,
    )


if __name__ == "__main__":
    main()
