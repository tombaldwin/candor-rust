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
     An item's module path is the FILE's path plus any enclosing inline `mod NAME { }` — see
     `inline_mod_spans`, added 2026-09-02 after both directions of ignoring inline mods were measured
     live (a `pub fn` discarded, and an `impl` correlated under a key candor-scan does not have).
  3. Self-scan the crate's own source with `candor-scan --json`, correlating each candidate to its
     self-scan report entry by the module-qualified path candor-scan itself derives. A candidate that
     fails to correlate is INDISTINGUISHABLE from a genuinely pure one — both are simply absent from
     the manifests — so a key-construction bug here is silent by construction. That is why the module
     path must be the one candor-scan derives, not an approximation of it.
  4. For every candidate whose self-scan `inferred` set contains a CONCRETE effect — the full vocabulary
     Fs/Net/Db/Exec/Clipboard/Ipc/Env/Clock/Rand/Log, widened 2026-08-28 from just Fs/Net/Db/Exec after
     `arboard::{Get,Set}::file_list` shipped a real Clipboard gap structurally invisible to the narrower
     trigger (see classify_check's header for the full before/after and why a bare `invisible` or
     `Unknown` still don't qualify) — guess the consumer-facing spelling(s) a caller would actually use
     (the raw module-qualified path AND, UNCONDITIONALLY, the shorter crate-root alias form —
     `ignore::Walk::new`, not just `ignore::walk::Walk::new`; unconditionally, because an alias guess
     that no `pub use` actually creates can only fail to match a rule, whereas gating it on a `pub use`
     scan would drop guesses for every crate that re-exports through an intermediate module) and hand
     them to `classify_check` (a tiny Rust binary — the ONLY thing that calls the real
     `candor_classify::classify`, so this script never re-derives its rules).
  5. `classify_check` writes covered.tsv (every candidate classify() DOES recognize — the HARD,
     regression-proof list `crates/candor-classify/tests/coverage_gate.rs` asserts every push) and
     open.tsv (the RATCHET — everything self-scan found effectful that no rule recognizes yet AND a
     consumer can name; SOUNDNESS R1528 step 2 routes the rest to unreachable.tsv — see `Reachability`).
     All three carry an `entry` column: the candidate's IDENTITY, which is what the refresh workflow
     diffs on. A row may enter open.tsv only with a verdict in `candor_classify::OPEN_ENTRY_VERDICTS`
     or as pre-ruling backlog in grandfathered.tsv (R1528 step 3; checked on push and weekly).
     `consumer_path` is a GUESS, picked by a different rule in each file (covered: the first guess
     classify() resolves; open: the shortest), so one candidate has two possible spellings and crossing
     between the lists rewrites the string with no change in what is covered — which the drift diff,
     keyed on that string, read as a regression and a growth at once.
  6. `--diff-manifests` (SOUNDNESS R214) compares a fresh regeneration against the checked-in manifests
     and is what `.github/workflows/coverage-gate-refresh.yml` now shells out to, rather than
     reimplementing the comparison in bash. See `diff_manifests()`'s docstring for why a checked-in
     `covered.tsv` row that a fresh run no longer sees covered splits into TWO different alarms, only
     one of which is a real regression, and for the open, unresolved question of whether either alarm
     is trustworthy across the macOS/Linux generation split (R212).

  Run `python3 eval/coverage-gate/generate.py --selftest` for this script's own tests (no cargo, no
  registry, no network); `cargo test --manifest-path eval/coverage-gate/classify_check/Cargo.toml` for
  the other half. Both run in CI.

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


def version_key(tail):
    """A SEMVER sort key for a registry directory's version tail (`8.19.0-alpha.1`, `1.2.3`, `0.9.0`).

    This was a plain `hits.sort()` — a LEXICOGRAPHIC string sort, with a comment saying it "is good
    enough to pick 'a' newest, not necessarily THE newest". It is not good enough, and the failure is
    silent: `"8.19.0-alpha.1" < "8.5.0-alpha.1"` as strings (`'1' < '5'` at the third character), so a
    machine with both cached — this one, measured 2026-09-02 — analyses **elasticsearch 8.5.0-alpha.1**
    while every other part of the pipeline (the fixture Cargo.toml, `VERSION_OVERRIDE`, the workflow's
    `cargo fetch`) is talking about 8.19.0. The manifests then record entry points from a version nobody
    asked for, and no output says which version was read. CI happens to be unaffected only because its
    registry is a one-shot fresh fetch holding exactly one version per crate — i.e. the bug is invisible
    in the place it is checked and live in the place it is reproduced, which is why it survived.

    Ordering rules (semver 2.0, the parts that matter here): numeric release components compare
    numerically; a version WITH a prerelease sorts below the same version without one; prerelease
    identifiers compare left to right, numeric ones numerically and below alphanumeric ones. Build
    metadata (`+...`) is ignored, as semver requires. Anything that does not parse as a leading numeric
    release sorts below everything that does, rather than raising — a registry directory is not
    guaranteed to be semver, and the caller only wants a total order.
    """
    tail = tail.split('+', 1)[0]
    core, _, pre = tail.partition('-')
    nums = []
    for part in core.split('.'):
        if not part.isdigit():
            return (0, (), 1, ())
        nums.append(int(part))
    if not nums:
        return (0, (), 1, ())
    pre_key = []
    for ident in (pre.split('.') if pre else []):
        # (0, n, "") for a numeric identifier, (1, 0, s) for an alphanumeric one — numeric sorts lower.
        pre_key.append((0, int(ident), "") if ident.isdigit() else (1, 0, ident))
    return (1, tuple(nums), 0 if pre else 1, tuple(pre_key))


def find_crate_dir(registry_src, name):
    pkg = RENAME.get(name, name)

    def exact(pkgname):
        out = []
        for h in glob.glob(os.path.join(registry_src, pkgname + "-*")):
            tail = os.path.basename(h)[len(pkgname) + 1:]
            if VERSION_TAIL.match(tail):
                out.append((version_key(tail), h))
        return out

    hits = exact(pkg) or exact(pkg.replace("_", "-"))
    if not hits:
        return None
    hits.sort()
    return hits[-1][1]


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


INLINE_MOD_RE = re.compile(r'\bmod\s+(\w+)\s*\{')


def inline_mod_spans(s):
    """Every INLINE `mod NAME { ... }` in a file, as (name, open_brace_index, close_brace_index).

    `module_path_for` derives a module path from the FILE path alone, so before this existed the
    generator was blind to the whole `mod NAME { ... }` shape in two different directions, both silent:

      - a free `pub fn` inside an inline mod was DISCARDED, because the old candidate test was
        `depth == 0` and an inline mod puts it at depth 1. Measured over the 74 calibrated crates:
        138 such functions, 106 of them inside a bare-`pub` inline mod. `fs_err::os::unix::fs::
        {symlink,chown,lchown,chroot}` and `async_fs::unix::symlink` are in that set — real,
        consumer-callable, and self-scan reports every one of them `['Fs']`.
      - an `impl` inside an inline mod was KEPT but MIS-KEYED: `IMPL_RE` never checked depth, so the
        entry got the file's module path and correlation then looked up a key the scanner does not
        have. `aws_config::ConfigLoader::load` is the measured instance — `mod loader {` at lib.rs:219,
        so the generator asked for `ConfigLoader::load` while candor-scan reports
        `loader::ConfigLoader::load` `['Log']`. The candidate came back `self_scan_found: false`, which
        is the SAME state as a genuinely pure function: absence, again, being the failure's signature.

    A regex plus `find_matching_brace` is enough here for the same reason the rest of this file is not
    a real parser: `strip_comments` has already removed comments and the pattern is anchored on the
    `mod` keyword. `mod NAME;` (a file module) has no brace and is correctly not matched.
    """
    spans = []
    for m in INLINE_MOD_RE.finditer(s):
        open_brace = m.end() - 1
        close = find_matching_brace(s, open_brace)
        if close != -1:
            spans.append((m.group(1), open_brace, close))
    return spans


def module_prefix_at(spans, pos):
    """The inline-mod names enclosing `pos`, outermost first — or None if `pos` sits inside a brace
    that is NOT an inline mod (a function body, an impl block, a struct literal). `None` is the "this
    is not a module-level item" answer the old `depth == 0` test was approximating."""
    enclosing = sorted((sp for sp in spans if sp[1] < pos < sp[2]), key=lambda sp: sp[1])
    return [sp[0] for sp in enclosing]


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


# TRIED AND REJECTED (2026-08-28): a `restricted_top_modules()` pass that treated a `pub fn`/`pub
# struct` inside a top-level `mod NAME;` with no bare `pub` as unreachable, one level up the tree from
# what `restricted_types()` already covers for types. It correctly diagnosed the widened trigger's two
# new `Ipc` false positives (dialoguer's `mod paging;` containing `pub struct Paging`/`pub fn
# render_prompt`, and mysql's `mod io;` containing `pub enum Stream`/`pub fn connect_socket` — neither
# re-exported anywhere, confirmed against real source) but was FAR too aggressive as a general rule:
# `mod internal; pub use internal::Thing;` (re-exporting a public item OUT of an otherwise-private
# module) is a common, idiomatic Rust organization pattern, not a rare exception — measured, applying
# the check across all 74 calibrated crates dropped total candidate entries 19985 -> 13662 and
# `covered.tsv` 1018 -> 680 rows, i.e. it silently discarded ~338 GENUINELY reachable, already-verified
# rows as a side effect of catching 2 real ones. Unlike `root_reexports`'s "loses recall, never
# fabricates" trade, this one actively shrinks the HARD gate itself — the risk profile is backwards.
# The two known instances are handled individually instead (see `REVIEWED_PURE_ENTRIES`, "unreachable"
# rows use the same escape hatch as "no effect" ones — either way, no consumer can ever observe it).


# `root_reexports()` USED TO LIVE HERE, and it was dead code from the gate's first commit to
# 2026-09-02: `process_crate` computed it into a `# noqa: F841` local and never read it. Deleted rather
# than wired up, because wiring it up could not have changed a single guess — the guess builder below
# already emits the crate-root alias (`{crate}::{Type}::{fn}` / `{crate}::{fn}`) UNCONDITIONALLY, for
# every candidate, re-exported or not. That is the deliberately over-approximate direction: an alias
# guess that no `pub use` actually creates can only fail to match a classify() rule, while gating the
# alias on a one-level, non-chain-following `pub use` scan of lib.rs would have REMOVED guesses for
# every crate that re-exports through an intermediate module. So the dead code was, by luck, the safe
# half of the trade its own docstring described. Its stale twin — a `root_reexport_prefixes` docstring
# that `classify_check/src/main.rs` cites for a measurement — never existed in this file at all.
#
# What it did NOT do, and what nothing does today, is guess an INTERMEDIATE-module alias:
# `crossterm::terminal::supports_keyboard_enhancement` (`pub use sys::supports_keyboard_enhancement`
# inside `terminal.rs`) and `sqlx_core::migrate::resolve_blocking` (`migrate/source.rs`, re-exported one
# level up) are spellings this generator cannot produce for the candidate that owns them, and both are
# spellings that classify() rules are keyed on. That is a RECALL gap — the entry lands in open.tsv under
# a spelling no rule matches instead of in covered.tsv — and it is why the `entry` column added in this
# commit, not the guessed path, is what the refresh workflow now diffs on.


# --- SOUNDNESS R1528 step 2: CONSUMER REACHABILITY ---------------------------------------------------
# A row in open.tsv claims "a consumer can call this and is told nothing". Phase 1's independent reader
# found 7 of 39 sampled rows UNREACHABLE — the item sits in a module no consumer can name (lettre's
# `mod net;`, tempfile's `mod imp;` / `mod util;`, sqlx-core's `mod tls_rustls;`, tungstenite's
# `mod mask;`) and nothing re-exports it. Those rows inflate the silent population and, worse, make a
# grown row's verdict cost a body read that cannot matter.
#
# The 2026-08-28 attempt (`restricted_top_modules`, see the TRIED AND REJECTED note above) failed in the
# OTHER direction: "private module => unreachable" ignores `mod internal; pub use internal::Thing;`, and
# dropped ~338 genuinely reachable rows. So this is not a module-privacy test, it is a small RESOLVER:
# which modules are nameable from outside, and which (module, name) items are re-exported into a
# nameable place, followed to a fixpoint through `pub use` chains, globs, `#[path]` renames and `self`.
#
# EVERY APPROXIMATION HERE FAILS TOWARD "REACHABLE" — i.e. toward keeping the row, which is the
# direction the ratchet already lives in. A false "unreachable" would hide a live silence from the
# register; a false "reachable" only keeps a row a reader then marks. Concretely: an unresolvable
# `pub use` path (an extern crate), a type with no in-crate definition, a type MENTIONED in any public
# signature / associated type / pub field (the "unnameable but obtainable" shape: a private-module type a
# public fn returns is callable), and `mod` / `pub use` text inside macro bodies (`cfg_if!`) all count as
# reachable. Only the classify()-unresolved rows are affected: a covered row stays covered whatever this
# says, so the HARD gate (`coverage_gate.rs`) does not shrink — the exact failure that sank the 08-28 try.
MOD_DECL_RE = re.compile(r'(pub(?:\s*\([^)]*\))?\s+)?\bmod\s+(\w+)\s*([;{])')
PUB_USE_RE = re.compile(r'(?<![\w)])pub\s+use\s+([^;]+);')
PATH_ATTR_RE = re.compile(r'\bpath\s*=\s*"([^"]+)"')
ITEM_DECL_RE = re.compile(r'\b(pub(?:\s*\([^)]*\))?\s+)?(?:struct|enum|union|trait|type)\s+(\w+)')
ASSOC_TYPE_RE = re.compile(r'\btype\s+\w+(?:<[^=;]*>)?\s*=\s*([^;]+);')
PUB_FIELD_RE = re.compile(r'\bpub\s+\w+\s*:\s*([^,}]+)')
PUB_TUPLE_FIELD_RE = re.compile(r'[(,]\s*pub\s+([^,)]+)')
PUB_BODY_DECL_RE = re.compile(r'\bpub\s+(?:enum|trait)\s+\w+[^{;]*\{')
FN_TRAIT_ARGS_RE = re.compile(r'\bFn(?:Mut|Once)?\s*\(')
WORD_RE = re.compile(r'[A-Za-z_]\w*')


def obtainable_words(head):
    """The words in a public fn signature through which a CONSUMER can come to HOLD a value: the
    return type, and the arguments of a callback it is handed (`impl FnOnce(&mut Hidden)`). A plain
    parameter type is not one — a consumer cannot construct an unnameable type to pass in (lettre's
    `SmtpConnection::set_stream(NetworkStream)` is the measured instance)."""
    words = set()
    if '->' in head:
        words.update(WORD_RE.findall(head.split('->', 1)[1]))
    for m in FN_TRAIT_ARGS_RE.finditer(head):
        close = head.find(')', m.end())
        words.update(WORD_RE.findall(head[m.end():close if close != -1 else len(head)]))
    return words


def parse_use_tree(text):
    """`a::b::{C, d::E as F, g::*, self}` -> [(segments, leaf, alias)], leaf `*` for a glob."""
    toks = re.findall(r'::|[{}*,]|\w+', text)
    out, pos = [], 0

    def tree(prefix):
        nonlocal pos
        segs = list(prefix)
        while pos < len(toks):
            t = toks[pos]
            if t == '{':
                pos += 1
                while pos < len(toks) and toks[pos] != '}':
                    if toks[pos] == ',':
                        pos += 1
                        continue
                    before = pos
                    tree(segs)
                    if pos == before:
                        pos += 1
                pos += 1
                return
            if t == '*':
                pos += 1
                out.append((segs, '*', None))
                return
            if t == '::':
                pos += 1
                continue
            if t in (',', '}'):
                return
            pos += 1
            if pos < len(toks) and toks[pos] == '::':
                segs = segs + [t]
                continue
            alias = None
            if pos + 1 < len(toks) and toks[pos] == 'as':
                alias = toks[pos + 1]
                pos += 2
            if t == 'self':
                if segs:
                    out.append((segs[:-1], segs[-1], alias))
            else:
                out.append((segs, t, alias))
            return

    while pos < len(toks):
        if toks[pos] == ',':
            pos += 1
            continue
        before = pos
        tree([])
        if pos == before:
            pos += 1
    return out


def _fn_body_spans(s):
    """(open, close) of every fn body in `s` — a `mod`/`pub use` inside one is not a module item."""
    spans = []
    for m in FN_HEAD_RE.finditer(s):
        i, n, depth = m.end(), len(s), 0
        while i < n:
            c = s[i]
            if c == '(':
                depth += 1
            elif c == ')':
                depth -= 1
            elif depth <= 0 and c in '{;':
                break
            i += 1
        if i < n and s[i] == '{':
            close = find_matching_brace(s, i)
            if close != -1:
                spans.append((i, close))
    return spans


def fn_head_at(s, start):
    """The signature text of the fn whose `fn` keyword match starts at `start`: up to its body or `;`."""
    i, n, depth = start, len(s), 0
    while i < n:
        c = s[i]
        if c == '(':
            depth += 1
        elif c == ')':
            depth -= 1
        elif depth <= 0 and c in '{;' and i > start:
            break
        i += 1
    return s[start:i]


class Reachability:
    """Which (module, name) items of one crate a consumer can name. See the block comment above."""

    def __init__(self, texts, srcdir):
        self.decls = {}    # module -> [(name, bare_pub, [target modules])]
        self.uses = {}     # module -> [(segs, leaf, alias)]
        self.types = {}    # type name -> [(module, bare_pub)]
        self.mods = set()
        self.mentioned = set()  # words in assoc-type / pub-field text (unconditional: over-approximate)
        for f, s in texts.items():
            fk = module_path_for(f, srcdir)
            self.mods.add(fk)
            spans = inline_mod_spans(s)
            bodies = _fn_body_spans(s)

            def owner(pos, fk=fk, spans=spans, bodies=bodies):
                if any(a < pos < b for a, b in bodies):
                    return None
                names = [sp[0] for sp in sorted((sp for sp in spans if sp[1] < pos < sp[2]),
                                                key=lambda sp: sp[1])]
                return "::".join(p for p in ([fk] if fk else []) + names)

            for m in MOD_DECL_RE.finditer(s):
                mk = owner(m.start(2))
                if mk is None:
                    continue
                bare = (m.group(1) or "").strip() == "pub"
                name, kind = m.group(2), m.group(3)
                child_default = f"{mk}::{name}" if mk else name
                targets = []
                if kind == ';':
                    stmt_lo = max(s.rfind(';', 0, m.start()), s.rfind('}', 0, m.start()),
                                  s.rfind('{', 0, m.start()))
                    for p in PATH_ATTR_RE.findall(s[stmt_lo + 1:m.start()]):
                        tf = os.path.normpath(os.path.join(os.path.dirname(f), p))
                        targets.append(module_path_for(tf, srcdir))
                if not targets:
                    targets = [child_default]
                self.mods.update(targets)
                self.decls.setdefault(mk, []).append((name, bare, targets))
            for m in PUB_USE_RE.finditer(s):
                mk = owner(m.start())
                if mk is None:
                    continue
                self.uses.setdefault(mk, []).extend(parse_use_tree(m.group(1)))
            for m in ITEM_DECL_RE.finditer(s):
                mk = owner(m.start())
                if mk is None:
                    continue
                self.types.setdefault(m.group(2), []).append(
                    (mk, (m.group(1) or "").strip() == "pub"))
            for rx in (ASSOC_TYPE_RE, PUB_FIELD_RE, PUB_TUPLE_FIELD_RE):
                for m in rx.finditer(s):
                    self.mentioned.update(WORD_RE.findall(m.group(1)))
            # A pub enum's variant fields are public, and a pub trait's method signatures hand values
            # to a consumer's impl: every word in either body counts (over-approximate, keeps rows).
            for m in PUB_BODY_DECL_RE.finditer(s):
                close = find_matching_brace(s, m.end() - 1)
                if close != -1:
                    self.mentioned.update(WORD_RE.findall(s[m.end():close]))
        self.visible = {""}
        self.items = set()
        self._fixpoint()

    @staticmethod
    def _parent(mk):
        return mk.rsplit("::", 1)[0] if "::" in mk else ""

    def _child(self, mk, name):
        out = [t for (n, _pub, ts) in self.decls.get(mk, []) if n == name for t in ts]
        if not out:
            k = f"{mk}::{name}" if mk else name
            if k in self.mods:
                out = [k]
        return out

    def _resolve(self, mk, segs):
        """The module(s) a use-path prefix names, or None when it leaves the crate (unresolvable)."""
        if not segs:
            return {mk}
        first, rest = segs[0], segs[1:]
        if first == "crate":
            cur = {""}
        elif first == "self":
            cur = {mk}
        elif first == "super":
            cur = {self._parent(mk)}
        else:
            kids = self._child(mk, first) or self._child("", first)
            if not kids:
                return None
            cur = set(kids)
        for sg in rest:
            if sg == "self":
                continue
            if sg == "super":
                cur = {self._parent(x) for x in cur}
                continue
            nxt = set()
            for x in cur:
                nxt.update(self._child(x, sg))
            if not nxt:
                return None
            cur = nxt
        return cur

    def _fixpoint(self):
        changed = True
        while changed:
            changed = False
            before = (len(self.visible), len(self.items))
            for mk in list(self.visible):
                for (_n, pub, ts) in self.decls.get(mk, []):
                    if pub:
                        self.visible.update(ts)
            for mk, uses in self.uses.items():
                vis = mk in self.visible
                exported = {n for (m, n) in self.items if m == mk}
                for segs, leaf, alias in uses:
                    if not vis and leaf != '*' and (alias or leaf) not in exported:
                        continue
                    xs = self._resolve(mk, segs)
                    if xs is None:
                        continue
                    if leaf == '*':
                        if vis:
                            self.visible.update(xs)
                        else:
                            for x in xs:
                                self.items.update((x, n) for n in exported)
                        continue
                    kids = [k for x in xs for k in self._child(x, leaf)]
                    if kids:
                        self.visible.update(kids)
                    self.items.update((x, leaf) for x in xs)
            for (x, n) in list(self.items):
                self.visible.update(self._child(x, n))
            changed = (len(self.visible), len(self.items)) != before

    def item_reachable(self, mk, name):
        return mk in self.visible or (mk, name) in self.items

    def type_reachable(self, name, extra_mentions=()):
        defs = self.types.get(name)
        if not defs:
            return True  # no in-crate definition we can see: cannot prove unreachable
        if name in self.mentioned or name in extra_mentions:
            return True
        return any(pub and self.item_reachable(mk, name) for mk, pub in defs)


def mark_reachability(reach, public_entries):
    """Set `reachable` on each candidate, iterating because a reachable fn's SIGNATURE can make a
    private-module type obtainable (and so its methods callable) — to a fixpoint."""
    mentions = set()
    state = {}
    changed = True
    while changed:
        changed = False
        for i, (mp, ty, fn, _f, head) in enumerate(public_entries):
            if state.get(i):
                continue
            ok = reach.type_reachable(ty, mentions) if ty else reach.item_reachable(mp, fn)
            if ok:
                state[i] = True
                new = obtainable_words(head) - mentions
                if new:
                    mentions |= new
                    changed = True
    return [bool(state.get(i)) for i in range(len(public_entries))]


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

    public_entries = []  # (module_path, type_name_or_None, fn_name, file, signature_text)
    for f, s in texts.items():
        file_mp = module_path_for(f, srcdir)
        spans = inline_mod_spans(s)

        def module_at(pos):
            """The module path an item at `pos` really lives in — the file's path plus any enclosing
            inline `mod NAME { }` names — or None if `pos` is inside a non-module brace (a fn body, an
            impl block), which is not a module-level item at all. This is what candor-scan itself
            derives, so it is what correlation must ask for."""
            mods = module_prefix_at(spans, pos)
            if s[:pos].count('{') - s[:pos].count('}') != len(mods):
                return None
            return "::".join(p for p in ([file_mp] if file_mp else []) + mods)

        for start, pub_kw, is_async, fn_name in find_fns(s):
            mp = module_at(start)
            if mp is not None and is_bare_pub(pub_kw):
                public_entries.append((mp, None, fn_name, f, fn_head_at(s, start)))
        for m in IMPL_RE.finditer(s):
            mp = module_at(m.start())
            if mp is None:
                continue  # an `impl` nested inside a function body is not a public entry point
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
                    public_entries.append((mp, ty_name, fn_name, f, fn_head_at(body, start)))

    reach = Reachability(texts, srcdir)
    reachable = mark_reachability(reach, public_entries)
    out = []
    for (mp, ty, fn, f, _head), ok in zip(public_entries, reachable):
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
                     "file": os.path.relpath(f, d), "guesses": sorted(guesses), "reachable": ok})
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


def selftest():
    """The generator's OWN test surface — it had none until 2026-09-02, which is why three
    candidate-extraction defects survived in a script whose entire job is to notice missing coverage.
    Pure Python: no cargo, no candor-scan, no registry, so it can run on every push rather than weekly.
    Each case below is red if the fix it pins is reverted; run with `--selftest`.
    """
    import tempfile, shutil
    fails = []

    def check(name, cond, detail=""):
        if cond:
            print(f"  ok    {name}")
        else:
            print(f"  FAIL  {name}  {detail}")
            fails.append(name)

    # --- version_key / find_crate_dir: SEMVER, not lexicographic -------------------------------------
    # The live instance: with both elasticsearch alphas cached, the old `hits.sort()` picked 8.5.0
    # because "8.19.0-alpha.1" < "8.5.0-alpha.1" as strings.
    order = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.2", "1.0.0-alpha.11", "1.0.0-beta",
             "1.0.0", "1.2.3", "1.9.0", "1.10.0", "8.5.0-alpha.1", "8.19.0-alpha.1", "8.19.0"]
    check("version_key orders semver, not strings",
          sorted(order, key=version_key) == order,
          f"got {sorted(order, key=version_key)}")
    # The docstring's other claim: a tail that is not a numeric release sorts BELOW everything that
    # is, and never raises. A registry directory is not guaranteed to be semver, and a comparison
    # that throws would take the whole run down on one odd directory name.
    odd = ["", "x", "1.2.x", "0.0.0", "1.0.0"]
    try:
        got = sorted(odd, key=version_key)
        ok = got[-2:] == ["0.0.0", "1.0.0"] and set(got[:3]) == {"", "x", "1.2.x"}
    except TypeError as e:
        got, ok = f"TypeError: {e}", False
    check("version_key is TOTAL: non-semver tails sort below, and never raise", ok, f"got {got}")

    tmp = tempfile.mkdtemp(prefix="cg-selftest-")
    try:
        for v in ("8.5.0-alpha.1", "8.19.0-alpha.1"):
            os.makedirs(os.path.join(tmp, "elasticsearch-" + v, "src"))
        os.makedirs(os.path.join(tmp, "elasticsearch-macros-1.0.0", "src"))  # must not be selected
        picked = find_crate_dir(tmp, "elasticsearch")
        check("find_crate_dir picks the newest by SEMVER",
              os.path.basename(picked or "") == "elasticsearch-8.19.0-alpha.1",
              f"picked {picked!r}")

        # --- inline `mod NAME {}`: module path, and what is NOT an entry point ------------------------
        crate = os.path.join(tmp, "fixture-1.0.0")
        src = os.path.join(crate, "src")
        os.makedirs(os.path.join(src, "os"))
        # `pub fn` inside a nested inline mod, in a NON-lib.rs file: fs_err::os::unix::fs::symlink.
        open(os.path.join(src, "os", "unix.rs"), "w").write(
            "pub mod fs {\n    pub fn symlink(a: &str) -> u8 { 0 }\n}\n")
        # an `impl` inside a private inline mod, in lib.rs: aws_config's `mod loader { impl ConfigLoader }`.
        # Plus two items that must stay INVISIBLE: a `pub fn` and an `impl` inside a function body.
        open(os.path.join(src, "lib.rs"), "w").write(
            "pub struct ConfigLoader;\n"
            "mod loader {\n"
            "    use super::ConfigLoader;\n"
            "    impl ConfigLoader {\n        pub fn load(&self) -> u8 { 0 }\n    }\n"
            "}\n"
            "pub struct Buried;\n"
            "pub fn outer() {\n"
            "    pub fn nested_should_not_count() {}\n"
            "    impl Buried { pub fn also_not(&self) {} }\n"
            "}\n")
        _, entries = process_crate(tmp, "fixture")
        by = {(e["module"], e["type"], e["fn"]): e for e in entries}
        seen = sorted(f'{e["module"]}|{e["type"]}|{e["fn"]}' for e in entries)

        check("a `pub fn` inside a nested inline mod is a candidate, keyed os::unix::fs",
              ("os::unix::fs", None, "symlink") in by,
              f"candidates seen: {seen}")
        check("an `impl` inside an inline mod is keyed with the mod, not the file",
              ("loader", "ConfigLoader", "load") in by and ("", "ConfigLoader", "load") not in by,
              f"candidates seen: {seen}")
        check("a `pub fn` inside a FUNCTION BODY is not an entry point",
              not any(e["fn"] == "nested_should_not_count" for e in entries))
        check("an `impl` inside a FUNCTION BODY is not an entry point",
              not any(e["fn"] == "also_not" for e in entries))
        g_load = by.get(("loader", "ConfigLoader", "load"), {}).get("guesses", [])
        g_sym = by.get(("os::unix::fs", None, "symlink"), {}).get("guesses", [])
        check("the crate-root alias is guessed unconditionally (no `pub use` scan gates it)",
              "fixture::ConfigLoader::load" in g_load and "fixture::symlink" in g_sym,
              f"{g_load} / {g_sym}")
        check("the module-qualified guess is the candidate identity",
              "fixture::os::unix::fs::symlink" in g_sym, f"{g_sym}")

        # --- SOUNDNESS R1528 step 2: consumer REACHABILITY, both directions ---------------------------
        # Each shape below is a measured instance: lettre's private `mod net;` (NetworkStream, named only
        # as a PARAMETER of a public fn), tempfile's `mod imp;` + cfg_attr-path `mod platform;` +
        # `pub use self::platform::*` inside the private module, tungstenite's `mod mask;`; and on the
        # keep side the idiom that sank the 2026-08-28 try (`mod internal; pub use internal::Thing;`),
        # a glob re-export, a `cfg_if!`-wrapped private module re-exported by glob, and a private-module
        # type a public fn RETURNS (unnameable, but a consumer holds one and can call its methods).
        rc = os.path.join(tmp, "reach-1.0.0")
        rsrc = os.path.join(rc, "src")
        os.makedirs(os.path.join(rsrc, "file", "imp"))
        os.makedirs(os.path.join(rsrc, "glob"))
        open(os.path.join(rsrc, "lib.rs"), "w").write(
            "mod internal;\npub use internal::Thing;\n"
            "mod net;\nmod mask;\nmod file;\nmod glob;\npub use glob::*;\n"
            "cfg_if::cfg_if! { if #[cfg(unix)] { mod sys; pub use sys::*; } }\n"
            "mod hidden;\n"
            "pub struct Conn;\n"
            "impl Conn {\n"
            "    pub fn set_stream(&mut self, s: net::NetworkStream) {}\n"
            "    pub fn handle(&self) -> hidden::Handle { hidden::Handle }\n"
            "}\n"
            "pub fn tempfile() -> u8 { file::imp::reopen() }\n")
        open(os.path.join(rsrc, "internal.rs"), "w").write(
            "pub struct Thing;\nimpl Thing { pub fn go(&self) {} }\npub struct Inner;\nimpl Inner { pub fn no(&self) {} }\n")
        open(os.path.join(rsrc, "net.rs"), "w").write(
            "pub struct NetworkStream;\nimpl NetworkStream { pub fn connect() -> NetworkStream { NetworkStream } }\n")
        open(os.path.join(rsrc, "mask.rs"), "w").write("pub fn generate_mask() -> u8 { 0 }\n")
        open(os.path.join(rsrc, "file", "mod.rs"), "w").write("mod imp;\n")
        open(os.path.join(rsrc, "file", "imp", "mod.rs"), "w").write(
            '#[cfg_attr(unix, path = "unix.rs")]\n#[cfg_attr(windows, path = "windows.rs")]\nmod platform;\n'
            "pub use self::platform::*;\n")
        open(os.path.join(rsrc, "file", "imp", "unix.rs"), "w").write("pub fn reopen() -> u8 { 0 }\n")
        open(os.path.join(rsrc, "glob", "mod.rs"), "w").write("pub fn globbed() {}\n")
        open(os.path.join(rsrc, "sys.rs"), "w").write("pub fn from_cfg_if() {}\n")
        open(os.path.join(rsrc, "hidden.rs"), "w").write(
            "pub struct Handle;\nimpl Handle { pub fn flush(&self) {} }\n")
        _, rents = process_crate(tmp, "reach")
        rby = {(e["module"], e["type"], e["fn"]): e["reachable"] for e in rents}
        for key, want, why in [
            (("internal", "Thing", "go"), True, "`mod internal; pub use internal::Thing;` keeps Thing"),
            (("internal", "Inner", "no"), False, "a sibling NOT re-exported out of the private module is unreachable"),
            (("net", "NetworkStream", "connect"), False, "a type named only as a PARAMETER is not obtainable"),
            (("mask", None, "generate_mask"), False, "a free fn in a private module, never re-exported"),
            (("file::imp::unix", None, "reopen"), False, "a cfg_attr-path module glob-re-exported only inside a "
                                                          "PRIVATE module stays unreachable"),
            (("glob", None, "globbed"), True, "`pub use glob::*` from the root reaches it"),
            (("sys", None, "from_cfg_if"), True, "a `cfg_if!`-wrapped private module re-exported by glob"),
            (("hidden", "Handle", "flush"), True, "a private-module type a public fn RETURNS is obtainable"),
            (("", "Conn", "set_stream"), True, "a root type's method"),
        ]:
            check(f"reachability: {why}", rby.get(key) is want, f"{key} -> {rby.get(key)!r}; all: {sorted(rby.items(), key=str)}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    # --- diff_manifests: distinguish 'classify() lost a rule' from 'oracle stopped nominating' -------
    # SOUNDNESS R214. Exactly the three cases the row demands: a real classify() regression (must
    # FAIL), an entry the self-scan candidate oracle stopped nominating (must REPORT, must NOT fail),
    # and a clean run (must PASS). Case 2 uses REAL data — this repo's own checked-in covered.tsv/
    # open.tsv, and the actual instance from the job's first-ever run (33872987298): `e9cdd23` (R173)
    # correctly withdrew a drop-glue over-charge from `async_nats::jetstream::context::Context::
    # publish`/`publish_message`/`publish_with_headers`, so a real fresh run drops all three from BOTH
    # manifests at once — self-scan's `inferred` set for each went from `['Log']` to `[]`.
    tmp2 = tempfile.mkdtemp(prefix="cg-selftest-diff-")
    try:
        def write(name, lines):
            p = os.path.join(tmp2, name)
            with open(p, "w") as fh:
                fh.write("\n".join(lines) + "\n")
            return p

        cov_hdr = "# crate\tconsumer_path\tself_scan_effects\tclassified_as\tentry"
        open_hdr = "# crate\tconsumer_path\teffects\tsource_file\tentry"
        devnull = open(os.devnull, "w")

        # Case 1: classify() lost a rule. `acme::mod1::Foo::bar` was covered; the fresh run no longer
        # resolves any guess for it, but self-scan STILL nominates it — it reappears in the fresh
        # open.tsv, same identity, with a CORE effect. A real regression: must fail.
        c1_checked_cov = write("c1_checked_cov.tsv",
                                [cov_hdr, "acme\tacme::Foo::bar\tNet\tNet\tacme::mod1::Foo::bar"])
        c1_fresh_cov = write("c1_fresh_cov.tsv", [cov_hdr])
        c1_checked_open = write("c1_checked_open.tsv", [open_hdr])
        c1_fresh_open = write("c1_fresh_open.tsv",
                               [open_hdr, "acme\tacme::Foo::bar\tNet,Unknown\tsrc/mod1.rs\t"
                                          "acme::mod1::Foo::bar"])
        r1 = diff_manifests(c1_checked_cov, c1_fresh_cov, c1_checked_open, c1_fresh_open)
        check("case 1 (real regression): lands in `regressed`, not `oracle_dropped`",
              len(r1["regressed"]) == 1 and len(r1["oracle_dropped"]) == 0
              and r1["regressed"][0][0] == ("acme", "acme::mod1::Foo::bar"),
              f"regressed={r1['regressed']} oracle_dropped={r1['oracle_dropped']}")
        exit1 = diff_manifests_cli(c1_checked_cov, c1_fresh_cov, c1_checked_open, c1_fresh_open,
                                    out=devnull)
        check("case 1: diff_manifests_cli exits 1 — a real classify() regression must FAIL",
              exit1 == 1, f"exit={exit1}")

        # Case 2: the self-scan candidate oracle stopped nominating — REAL data, the async-nats
        # instance. checked-in covered.tsv/open.tsv are this repo's real, currently-committed
        # manifests; the simulated "fresh" covered.tsv is the real one with the three publish* rows
        # removed, and the "fresh" open.tsv is the real checked-in open.tsv UNCHANGED (they do not
        # reappear there either — that absence from BOTH lists is the whole defect).
        real_covered = os.path.join(HERE, "covered.tsv")
        real_open = os.path.join(HERE, "open.tsv")
        dropped_entries = {
            "async_nats::jetstream::context::Context::publish",
            "async_nats::jetstream::context::Context::publish_message",
            "async_nats::jetstream::context::Context::publish_with_headers",
        }
        with open(real_covered) as fh:
            real_cov_lines = fh.read().splitlines()
        fresh_cov_lines = [ln for ln in real_cov_lines if ln.split("\t")[-1] not in dropped_entries]
        check("case 2 fixture: the real checked-in covered.tsv carries all 3 async_nats rows (else "
              "this test is not exercising the real R214 instance)",
              len(fresh_cov_lines) == len(real_cov_lines) - 3,
              f"{len(real_cov_lines)} -> {len(fresh_cov_lines)}")
        c2_fresh_cov = write("c2_fresh_cov.tsv", fresh_cov_lines)
        r2 = diff_manifests(real_covered, c2_fresh_cov, real_open, real_open)
        r2_dropped_keys = {k for k, *_ in r2["oracle_dropped"]}
        r2_regressed_entries = {k[1] for k, *_ in r2["regressed"]}
        check("case 2 (oracle stopped nominating): all 3 real async_nats rows land in "
              "`oracle_dropped`, none in `regressed`",
              {("async_nats", e) for e in dropped_entries} <= r2_dropped_keys
              and not (dropped_entries & r2_regressed_entries),
              f"oracle_dropped(async_nats)={sorted(k for k in r2_dropped_keys if k[0] == 'async_nats')}")
        exit2 = diff_manifests_cli(real_covered, c2_fresh_cov, real_open, real_open, out=devnull)
        check("case 2: diff_manifests_cli exits 0 — oracle-dropped ALONE must NOT fail the job",
              exit2 == 0, f"exit={exit2}")

        # Case 3: clean run — identical manifests. Nothing regressed, nothing dropped, nothing grown.
        c3_cov = write("c3_cov.tsv", [cov_hdr, "acme\tacme::Foo::bar\tNet\tNet\tacme::mod1::Foo::bar"])
        c3_open = write("c3_open.tsv",
                         [open_hdr, "acme\tacme::Baz::qux\tFs\tsrc/baz.rs\tacme::mod2::Baz::qux"])
        r3 = diff_manifests(c3_cov, c3_cov, c3_open, c3_open)
        check("case 3 (clean run): nothing regressed, nothing oracle-dropped, nothing grown",
              not r3["regressed"] and not r3["oracle_dropped"] and not r3["grown"], f"{r3}")
        c3_gf = {("acme", "acme::mod2::Baz::qux")}  # pre-ruling backlog: needs no verdict
        exit3 = diff_manifests_cli(c3_cov, c3_cov, c3_open, c3_open, out=devnull, verdicts={},
                                   grandfathered=c3_gf)
        check("case 3: diff_manifests_cli exits 0 — a clean run must PASS", exit3 == 0, f"exit={exit3}")

        # Case 4: ENGINE CHANGE vs DRIFT. Two grown rows; the baseline (manifest's engine on the same
        # fetch) produces only one of them, so it is drift and the other is engine change. A covered row
        # the fresh run lost while the baseline still covers it is an ENGINE regression.
        c4_checked_cov = write("c4_checked_cov.tsv", [cov_hdr, "acme\tacme::Foo::bar\tNet\tNet\tacme::m::Foo::bar"])
        c4_fresh_cov = write("c4_fresh_cov.tsv", [cov_hdr])
        c4_checked_open = write("c4_checked_open.tsv", [open_hdr])
        c4_fresh_open = write("c4_fresh_open.tsv", [open_hdr,
                              "acme\tacme::Foo::bar\tNet\tsrc/m.rs\tacme::m::Foo::bar",
                              "acme\tacme::A::up\tFs\tsrc/a.rs\tacme::a::A::up",
                              "acme\tacme::B::eng\tDb\tsrc/b.rs\tacme::b::B::eng"])
        c4_base_cov = write("c4_base_cov.tsv", [cov_hdr, "acme\tacme::Foo::bar\tNet\tNet\tacme::m::Foo::bar"])
        c4_base_open = write("c4_base_open.tsv", [open_hdr, "acme\tacme::A::up\tFs\tsrc/a.rs\tacme::a::A::up"])
        r4 = diff_manifests(c4_checked_cov, c4_fresh_cov, c4_checked_open, c4_fresh_open, c4_base_cov, c4_base_open)
        check("case 4 (attribution): A::up is drift, B::eng is engine change, Foo::bar an ENGINE regression",
              [k[1] for k, *_ in r4["grown_drift"]] == ["acme::a::A::up"]
              and [k[1] for k, *_ in r4["grown_engine"]] == ["acme::b::B::eng"]
              and [k[1] for k, *_ in r4["regressed_engine"]] == ["acme::m::Foo::bar"]
              and not r4["regressed_drift"], f"{r4}")
        r4u = diff_manifests(c4_checked_cov, c4_fresh_cov, c4_checked_open, c4_fresh_open)
        check("case 4: without a baseline nothing is attributed (never labelled drift by default)",
              not r4u["baseline"] and "grown_drift" not in r4u, f"{r4u}")
        # Case 6 (SOUNDNESS R1528 step 3): THE VERDICT GATE CAN FAIL, and clears only on a verdict.
        c6_open = write("c6_open.tsv", [open_hdr, "acme\tacme::Old::row\tFs\tsrc/o.rs\tacme::o::Old::row"])
        c6_fresh = write("c6_fresh.tsv", [open_hdr, "acme\tacme::Old::row\tFs\tsrc/o.rs\tacme::o::Old::row",
                                           "acme\tacme::New::row\tNet\tsrc/n.rs\tacme::n::New::row"])
        gf6 = {("acme", "acme::o::Old::row")}
        new6 = ("acme", "acme::n::New::row")
        e_none = diff_manifests_cli(c3_cov, c3_cov, c6_open, c6_fresh, out=devnull, verdicts={}, grandfathered=gf6)
        r6 = diff_manifests(c3_cov, c3_cov, c6_open, c6_fresh, verdicts={}, grandfathered=gf6)
        check("case 6: a grown row with NO verdict fails the job and is named",
              e_none == 1 and [k for k, *_ in r6["grown_unverdicted"]] == [new6], f"exit={e_none} {r6['grown_unverdicted']}")
        e_ok = diff_manifests_cli(c3_cov, c3_cov, c6_open, c6_fresh, out=devnull,
                                  verdicts={new6: "R1528:silent"}, grandfathered=gf6)
        check("case 6: the SAME grown row with a recorded verdict clears (exit 0)", e_ok == 0, f"exit={e_ok}")
        e_prose = diff_manifests_cli(c3_cov, c3_cov, c6_open, c6_fresh, out=devnull,
                                     verdicts={new6: "read it, looks fine"}, grandfathered=gf6)
        check("case 6: a PROSE verdict is not a verdict (exit 1)", e_prose == 1, f"exit={e_prose}")
        e_splice = diff_manifests_cli(c3_cov, c3_cov, c6_fresh, c6_fresh, out=devnull, verdicts={},
                                      grandfathered=gf6)
        r6s = diff_manifests(c3_cov, c3_cov, c6_fresh, c6_fresh, verdicts={}, grandfathered=gf6)
        check("case 6: SPLICING the row into the checked-in open.tsv does not clear it (unregistered, exit 1)",
              e_splice == 1 and [k for k, *_ in r6s["unregistered"]] == [new6], f"exit={e_splice}")
        vsrc = os.path.join(tmp2, "lib.rs")
        with open(vsrc, "w") as fh:
            fh.write('pub const OPEN_ENTRY_VERDICTS: &[(&str, &str, &str)] = &[\n'
                     '    ("acme", "acme::n::New::row", "R1528:silent"),\n'
                     '    // ("acme", "acme::commented::out", "R1:silent"),\n];\n')
        check("case 6: read_open_verdicts parses entries and ignores a commented-out one",
              read_open_verdicts(vsrc) == {new6: "R1528:silent"}, f"{read_open_verdicts(vsrc)}")
        real_v = read_open_verdicts()
        check("case 6: the real OPEN_ENTRY_VERDICTS parses, every entry well-formed",
              isinstance(real_v, dict) and all(verdict_is_well_formed(v) for v in real_v.values()), f"{real_v}")
        # R1507: the engine stamp round-trips, replaces (never duplicates) itself, and `unrecorded` reads as absent
        # so refresh.sh falls back to its history walk instead of trusting a sha no commit made.
        sp = os.path.join(tmp2, "stamp.tsv")
        with open(sp, "w") as fh:
            fh.write("# header\nrow\n")
        stamp_engine(sp, "a" * 40)
        stamp_engine(sp, "b" * 40)
        n_stamps = sum(1 for l in open(sp) if l.startswith(ENGINE_HEADER))
        check("case 5 (R1507): engine stamp round-trips and is replaced, not duplicated",
              manifest_engine(sp) == "b" * 40 and n_stamps == 1 and open(sp).read().endswith("row\n"),
              open(sp).read())
        stamp_engine(sp, "unrecorded")
        check("case 5: an `unrecorded` stamp reads as absent", manifest_engine(sp) == "", manifest_engine(sp))
    finally:
        shutil.rmtree(tmp2, ignore_errors=True)

    if fails:
        raise SystemExit(f"generate.py --selftest: {len(fails)} FAILED: {', '.join(fails)}")
    print("generate.py --selftest: all checks passed")


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


# --- SOUNDNESS R214 --------------------------------------------------------------------------------
# `.github/workflows/coverage-gate-refresh.yml`'s weekly job re-runs this generator against a fresh
# `cargo fetch` and diffs the result against the checked-in covered.tsv/open.tsv. Its first-ever
# completed run (33872987298) called this "5 regressed" — but a covered.tsv row that drops out of a
# fresh run means one of TWO very different things, and the workflow only ever asked one of them:
#
#   (a) classify() lost a rule.  The candidate is STILL effectful (self-scan still nominates it — it
#       reappears in the fresh open.tsv with the same identity) but no classify() rule resolves it any
#       more. This is a real coverage regression: `crates/candor-classify/tests/coverage_gate.rs`
#       already asserts the rule on every push, so if this fires here, that push-time gate has ALSO
#       gone red, or is about to the moment anyone edits the rule again. FAIL LOUDLY.
#
#   (b) the self-scan CANDIDATE ORACLE stopped nominating it.  The entry is absent from BOTH fresh
#       manifests: not covered, not open. classify() was never even asked the question, because the
#       candidate's self-scan `inferred` set no longer contains a CORE effect at all (it went empty,
#       or narrowed to something classify_check's CORE filter excludes) — the oracle got MORE precise,
#       not the checker less complete. Measured instance: `e9cdd23` (R173) correctly withdrew a
#       drop-glue over-charge from `async_nats::Context::publish`/`publish_message`/`publish_with_headers`
#       — self-scan's `inferred` set for each dropped from `['Log']` to `[]`, so all three vanished from
#       covered.tsv AND open.tsv at once. classify()'s `Context::publish -> Net` rule never moved.
#
# The old diff conflated these because it only ever asked "is the key still in covered.fresh?" and
# never checked whether it had moved to open.fresh instead of vanishing outright. `diff_manifests`
# asks that second question. The decision (Tom, R214, and open to being overruled by whoever reads
# this next): (b) is reported PROMINENTLY — a shrinking oracle is exactly the kind of change this gate
# exists to surface, it is a real fact about the generator's own recall — but it does NOT fail the job,
# because failing it would mean the coverage gate now polices classify()'s decisions about a candidate
# list the OTHER instrument (the self-scan engine itself) chose to shrink, and a real fix to
# candor-scan's precision (an over-charge withdrawal, exactly like R173) would then read as a coverage
# regression forever, training whoever reads the weekly issue to stop trusting it — the same "cries
# wolf" failure mode this row exists to close. (a) still fails: nothing about the oracle getting more
# precise excuses classify() actually losing a rule.
#
# SECOND, UNRESOLVED HALF OF R214: the checked-in manifest is generated on whatever machine last ran
# `python3 eval/coverage-gate/generate.py` and committed the result (macOS, historically) and diffed
# here against a FRESH run on `ubuntu-latest`. The same commit `f991ff1` measured 3 regressed rows
# locally (macOS) and 5 in CI (Linux) — R212 ruled out crate version, source bytes, gate code,
# classify(), RAYON_NUM_THREADS, HashMap seeding and rustc sort tie-breaks as the cause, but COULD NOT
# rule out the host platform itself (Docker was unavailable to test a Linux self-scan against the same
# source on the machine that did the ruling-out). This function does not attempt to establish that
# cause — doing so needs a same-platform-vs-cross-platform A/B of a candor-scan build, which is out of
# this change's scope — so it says so, loudly, in its own output, every time it has something to
# report: a diffed row here is only as trustworthy as the assumption that macOS and Linux self-scan the
# same source identically, and that assumption is UNPROVEN, not confirmed. A reader chasing a reported
# row should regenerate the checked-in manifest on the SAME platform the job just ran on before trusting
# that the row is real, not an artifact of the split.
PLATFORM_CAVEAT = (
    "NOTE (SOUNDNESS R212, unresolved): the checked-in manifest and this fresh run are not proven to "
    "be generated on the same platform (checked-in: whatever machine last committed it, historically "
    "macOS; this run: the CI runner). A same-commit run has measured a DIFFERENT row count on macOS "
    "vs Linux before, and the host-platform cause was never ruled out — Docker was unavailable to test "
    "it. Treat every row below as unconfirmed until it reproduces from a same-platform regeneration."
)


VERSION_HEADER = "# resolved-versions:"


def stamp_versions(path, resolved):
    """Insert `# resolved-versions: name=dir …` into a manifest's comment header, replacing any
    previous stamp. A COMMENT, so `read_tsv_rows` skips it and no join key changes."""
    stamp = VERSION_HEADER + " " + " ".join(
        "%s=%s" % (k, resolved[k]) for k in sorted(resolved)) + "\n"
    with open(path) as fh:
        lines = [l for l in fh if not l.startswith(VERSION_HEADER)]
    out, placed = [], False
    for l in lines:
        out.append(l)
        if not placed and l.startswith("#"):
            out.append(stamp)
            placed = True
    if not placed:
        out.insert(0, stamp)
    with open(path, "w") as fh:
        fh.writelines(out)


# R1507 — the ENGINE that generated a manifest, recorded as a comment (same channel as the version stamp).
# refresh.sh needs "the engine that last REGENERATED these files"; `git log -1 -- open.tsv covered.tsv`
# cannot say, because a splice commit (hand-added rows, no regeneration) moves it, and every engine change
# since the real regeneration is then labelled crates.io drift. The stamp survives a splice untouched.
ENGINE_HEADER = "# engine-sha:"
ENGINE_PATHS = ["crates/candor-scan", "crates/candor-classify", "crates/candor-report", "Cargo.lock",
                "eval/coverage-gate/generate.py", "eval/coverage-gate/classify_check"]


def current_engine_stamp():
    """HEAD's sha when the engine paths are clean against HEAD, else `unrecorded`: a dirty-tree
    generation was NOT made by any commit, and a sha there would be a false claim the baseline run
    would then trust. `unrecorded` makes refresh.sh fall back to its splice-skipping history walk."""
    try:
        root = subprocess.check_output(["git", "rev-parse", "--show-toplevel"], cwd=HERE, text=True).strip()
        dirty = subprocess.check_output(["git", "status", "--porcelain", "--"] + ENGINE_PATHS,
                                        cwd=root, text=True).strip()
        if dirty:
            return "unrecorded"
        return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    except (subprocess.CalledProcessError, OSError):
        return "unrecorded"


def stamp_engine(path, sha):
    with open(path) as fh:
        lines = [l for l in fh if not l.startswith(ENGINE_HEADER)]
    out, placed = [], False
    for l in lines:
        out.append(l)
        if not placed and l.startswith("#"):
            out.append("%s %s\n" % (ENGINE_HEADER, sha))
            placed = True
    if not placed:
        out.insert(0, "%s %s\n" % (ENGINE_HEADER, sha))
    with open(path, "w") as fh:
        fh.writelines(out)


def manifest_engine(path):
    """The stamped engine sha, or '' when absent (a manifest written before R1507) or `unrecorded`."""
    try:
        with open(path) as fh:
            for line in fh:
                if line.startswith(ENGINE_HEADER):
                    v = line[len(ENGINE_HEADER):].strip()
                    return "" if v == "unrecorded" else v
    except OSError:
        pass
    return ""


def manifest_versions(path):
    """crate -> resolved registry directory, from a manifest's R311 stamp. Empty for a manifest
    written before the stamp existed, which is NOT the same as 'no crates' — callers must treat an
    empty map as UNKNOWN and say so rather than reporting every crate as changed."""
    try:
        with open(path) as fh:
            for line in fh:
                if line.startswith(VERSION_HEADER):
                    body = line[len(VERSION_HEADER):].strip()
                    out = {}
                    for tok in body.split():
                        if "=" in tok:
                            k, v = tok.split("=", 1)
                            out[k] = v
                    return out
                if not line.startswith("#") and line.strip():
                    break
    except OSError:
        pass
    return {}


def read_tsv_rows(path):
    """The data rows (as column lists) of a covered.tsv/open.tsv-shaped file: strip the leading
    `#`-comment header block `classify_check` writes and blank lines. Both manifests carry the
    candidate's IDENTITY as their LAST column (`entry`) and its owning crate as their FIRST (`crate`)
    — see classify_check's own header comments — despite differing middle columns between the two
    files, so column 0 and column -1 are the stable join key regardless of which file this reads."""
    rows = []
    with open(path) as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            rows.append(line.split("\t"))
    return rows


def _manifest_key_map(rows):
    """(crate, entry) -> full row, for rows that actually carry both columns."""
    return {(r[0], r[-1]): r for r in rows if len(r) >= 2}


def _require_entry_column(path, rows):
    if rows and len(rows[0]) < 5:
        raise SystemExit(
            f"::error::{path} has no `entry` column — the checked-in manifest predates the identity "
            f"column and cannot be diffed. Regenerate it (python3 eval/coverage-gate/generate.py) "
            f"and commit, then re-run this job."
        )


GRANDFATHERED_PATH = os.path.join(HERE, "grandfathered.tsv")
VERDICT_KINDS = ("silent", "overcharge", "conditional")


def read_open_verdicts(src_path=CLASSIFY_SRC):
    """`candor_classify::OPEN_ENTRY_VERDICTS` as {(crate, entry): verdict}, read from the source the way
    `calibrated_crates_from_source` reads CALIBRATED_CRATES — one copy of the truth. Raises when the
    const is missing: a verdict gate that silently loaded nothing would clear nothing and look red for
    the wrong reason, or (worse, if the logic ever inverts) clear everything."""
    src = open(src_path).read()
    m = re.search(r'pub const OPEN_ENTRY_VERDICTS:\s*&\[\(&str, &str, &str\)\]\s*=\s*&\[(.*?)\];', src, re.DOTALL)
    if not m:
        raise SystemExit("could not find OPEN_ENTRY_VERDICTS in " + src_path)
    body = strip_comments(m.group(1))
    return {(c, e): v for c, e, v in re.findall(r'\(\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([^"]+)"\s*\)', body)}


def read_grandfathered(path=GRANDFATHERED_PATH):
    """The pre-ruling backlog's (crate, entry) keys. Absent file = empty set (every row then needs a
    verdict), never 'everything is grandfathered'."""
    if not os.path.exists(path):
        return set()
    return {(r[0], r[-1]) for r in read_tsv_rows(path) if len(r) >= 2}


def verdict_is_well_formed(v):
    row, _, kind = v.partition(":")
    return len(row) > 1 and row[0] == "R" and row[1:].isdigit() and kind in VERDICT_KINDS


def diff_manifests(checked_covered_path, fresh_covered_path, checked_open_path, fresh_open_path,
                   baseline_covered_path=None, baseline_open_path=None, verdicts=None, grandfathered=None,
                   checked_unreachable_path=None, fresh_unreachable_path=None):
    """Compare a fresh regeneration against the checked-in manifests and return a dict with three
    DISTINCT findings, per the R214 comment block above:

      regressed        -- classify() lost a rule. Real regression. The job must FAIL.
      oracle_dropped    -- the self-scan candidate oracle stopped nominating the entry (its `inferred`
                        set no longer clears classify_check's CORE bar). NOT a classify() regression;
                        reported, does not fail.
      grown          -- a genuinely new open.tsv entry (crates.io drift introduced an uncovered public
                        entry point) — unchanged from the pre-R214 behaviour.
      crate_covered_totals -- {crate: count of checked-in covered.tsv rows}, so a caller can tell
                        whether ALL of a crate's covered entries just landed in oracle_dropped at once
                        (more consistent with that crate's self-scan failing outright than with a
                        precision gain on one entry) from a partial, plausible drop.

    Each finding value is a list of ((crate, entry), checked_in_row, fresh_row_or_None) tuples, sorted
    by key, so a caller can print whatever detail it wants without re-reading the files.

    ENGINE CHANGE vs CRATES.IO DRIFT (2026-10-08). `grown` and `regressed` used to be reported as
    "crates.io drift" whatever moved them, and on run 37608278047 that was wrong for all 74 rows: the
    generator at the engine that wrote the manifest, over the SAME fresh fetch, reproduced the checked-in
    open.tsv exactly — every row came from engine work committed after it. When the caller supplies
    `baseline_*` (a fresh regeneration with the manifest's OWN engine — the commit that last wrote it —
    over the same fetch), each finding is split on that run: a row the baseline engine ALSO produces is
    DRIFT (the sources moved under an engine that did not), one it does not is ENGINE (the same sources
    read differently by the engine that did). Both still fail the job — the ratchet may not grow without
    review either way — but the report names which review is needed.
    """
    checked_cov_rows = read_tsv_rows(checked_covered_path)
    fresh_cov_rows = read_tsv_rows(fresh_covered_path)
    checked_open_rows = read_tsv_rows(checked_open_path)
    fresh_open_rows = read_tsv_rows(fresh_open_path)
    for p, rows in ((checked_covered_path, checked_cov_rows), (fresh_covered_path, fresh_cov_rows),
                    (checked_open_path, checked_open_rows), (fresh_open_path, fresh_open_rows)):
        _require_entry_column(p, rows)

    checked_cov = _manifest_key_map(checked_cov_rows)
    fresh_cov = _manifest_key_map(fresh_cov_rows)
    checked_open = _manifest_key_map(checked_open_rows)
    fresh_open = _manifest_key_map(fresh_open_rows)

    crate_covered_totals = {}
    for (crate, _entry) in checked_cov:
        crate_covered_totals[crate] = crate_covered_totals.get(crate, 0) + 1

    # A checked-in covered.tsv row the fresh run no longer sees covered at all.
    lost = sorted(set(checked_cov) - set(fresh_cov))
    # Split on whether the fresh run STILL nominates it (in open.fresh, meaning self-scan's `inferred`
    # set still clears CORE) — that split is exactly (a) vs (b) above.
    regressed = [(k, checked_cov[k], fresh_open[k]) for k in lost if k in fresh_open]
    oracle_dropped = [(k, checked_cov[k], None) for k in lost if k not in fresh_open]

    # GROWTH is unchanged from the pre-R214 diff: a fresh open.tsv entry absent from BOTH checked-in
    # lists (genuinely new — not a row that simply got fixed elsewhere and moved out of open.tsv).
    grown_raw = set(fresh_open) - set(checked_open)
    grown = sorted((k, None, fresh_open[k]) for k in grown_raw if k not in checked_cov)

    # SOUNDNESS R1528 step 3 — THE VERDICT GATE (Tom, 2026-11-08: a grown row clears the red ONLY
    # with a recorded verdict). `grown` stays the full list and keeps its engine/drift split; what
    # FAILS is the part of it with no verdict. A grown row with a well-formed `OPEN_ENTRY_VERDICTS`
    # entry is cleared (the lane splices it into open.tsv beside the verdict). A rule or a
    # REVIEWED_PURE_ENTRIES read never reaches `grown` at all — the fresh run already put the row in
    # covered.tsv or dropped it — so this is the only verdict that has to be looked up here.
    # `unregistered` is the same rule applied to the CHECKED-IN open.tsv: a row that is neither
    # pre-ruling backlog nor verdicted got in by a splice. The push-time test refuses that too; it is
    # repeated here so the weekly verdict does not depend on that test having run.
    if verdicts is None:
        verdicts = read_open_verdicts()
    if grandfathered is None:
        grandfathered = read_grandfathered()
    good = {k for k, v in verdicts.items() if verdict_is_well_formed(v)}
    grown_verdicted = [g for g in grown if g[0] in good]
    grown_unverdicted = [g for g in grown if g[0] not in good]
    unregistered = sorted((k, checked_open[k], None) for k in checked_open
                          if k not in grandfathered and k not in good)
    newly_unreachable = []
    if checked_unreachable_path and fresh_unreachable_path:
        checked_un = _manifest_key_map(read_tsv_rows(checked_unreachable_path)) \
            if os.path.exists(checked_unreachable_path) else {}
        fresh_un = _manifest_key_map(read_tsv_rows(fresh_unreachable_path))
        newly_unreachable = sorted((k, checked_open.get(k), fresh_un[k]) for k in set(fresh_un) - set(checked_un))

    result = {
        "regressed": regressed, "oracle_dropped": oracle_dropped, "grown": grown,
        "crate_covered_totals": crate_covered_totals, "baseline": False,
        "grown_verdicted": grown_verdicted, "grown_unverdicted": grown_unverdicted,
        "unregistered": unregistered, "verdicts": verdicts, "newly_unreachable": newly_unreachable,
    }
    if baseline_open_path and baseline_covered_path:
        base_open_rows = read_tsv_rows(baseline_open_path)
        base_cov_rows = read_tsv_rows(baseline_covered_path)
        _require_entry_column(baseline_open_path, base_open_rows)
        _require_entry_column(baseline_covered_path, base_cov_rows)
        base_open = _manifest_key_map(base_open_rows)
        base_cov = _manifest_key_map(base_cov_rows)
        # grown: drift iff the manifest's engine also leaves it open on these sources.
        result["grown_drift"] = [g for g in grown if g[0] in base_open]
        result["grown_engine"] = [g for g in grown if g[0] not in base_open]
        # regressed: drift iff the manifest's engine ALSO fails to cover it on these sources (the source
        # moved out from under the rule); engine iff that engine still covers it (HEAD lost the rule).
        result["regressed_drift"] = [r for r in regressed if r[0] not in base_cov]
        result["regressed_engine"] = [r for r in regressed if r[0] in base_cov]
        result["baseline"] = True
    return result


def _fmt_key(k):
    crate, entry = k
    return f"{crate}\t{entry}"


def format_diff_report(result):
    """Render `diff_manifests`'s result as the job-log text a human reads. Returns the text; the
    caller decides what to do with exit codes and CI outputs."""
    lines = []
    regressed, oracle_dropped, grown = result["regressed"], result["oracle_dropped"], result["grown"]
    crate_covered_totals = result["crate_covered_totals"]

    lines.append(f"--- REGRESSED: classify() lost a rule ({len(regressed)}) ---")
    lines.append("A candidate self-scan STILL finds effectful (it reappears in the fresh open.tsv) "
                  "that no classify() rule resolves any more. This is a real coverage loss.")
    for k, old, new in regressed:
        lines.append(f"  {_fmt_key(k)}  was classified_as={old[3]!r}  now self-scan effects={new[2]!r} "
                      f"(open.tsv:{new[3]})")

    lines.append("")
    lines.append(f"--- ORACLE-DROPPED: self-scan stopped nominating this entry ({len(oracle_dropped)}) ---")
    lines.append("Absent from BOTH fresh manifests: the entry's self-scan `inferred` set no longer "
                  "clears the CORE bar (went empty, or narrowed to a non-CORE effect). classify() was "
                  "never asked. NOT a regression by itself; reported for visibility, does not fail this "
                  "job.")
    dropped_per_crate = {}
    for k, old, _ in oracle_dropped:
        dropped_per_crate[k[0]] = dropped_per_crate.get(k[0], 0) + 1
        lines.append(f"  {_fmt_key(k)}  was classified_as={old[3]!r} self_scan_effects={old[2]!r}")
    whole_crate_vanished = sorted(
        crate for crate, n in dropped_per_crate.items() if n == crate_covered_totals.get(crate, -1)
    )
    if whole_crate_vanished:
        lines.append("  WARNING: every checked-in covered.tsv row for "
                      f"{', '.join(whole_crate_vanished)} landed here at once — that is more consistent "
                      "with that crate's self-scan FAILING outright this run (check the earlier "
                      "'self-scan FAILED' log lines from the regenerate step) than with a precision "
                      "gain on one entry. Investigate before assuming benign.")

    lines.append("")
    if result.get("baseline"):
        for label, key, why in (
            ("ENGINE CHANGE", "grown_engine",
             "the manifest's own engine, over this SAME fetch, does not produce these rows: engine/classify "
             "work since the manifest was written made self-scan find them. Regenerate and review."),
            ("CRATES.IO DRIFT", "grown_drift",
             "the manifest's own engine produces these too over this fetch: upstream source moved."),
        ):
            lines.append(f"--- GROWN — {label} ({len(result[key])}) ---")
            lines.append(why)
            for k, _, new in result[key]:
                lines.append(f"  {_fmt_key(k)}  effects={new[2]!r}  file={new[3]}")
            lines.append("")
        lines.append(f"--- REGRESSED split: engine {len(result['regressed_engine'])}, "
                     f"drift {len(result['regressed_drift'])} ---")
    else:
        lines.append(f"--- GROWN: newly-uncovered ({len(grown)}) — NOT attributed: no baseline-engine run "
                     "was supplied, so engine change and crates.io drift cannot be told apart ---")
        for k, _, new in grown:
            lines.append(f"  {_fmt_key(k)}  effects={new[2]!r}  file={new[3]}")

    lines.append("")
    verdicts = result.get("verdicts", {})
    lines.append(f"--- GROWN WITHOUT A VERDICT ({len(result['grown_unverdicted'])}) — FAILS (SOUNDNESS R1528, "
                 "Tom 2026-11-08) ---")
    lines.append("Each needs ONE of: a classify() rule; a REVIEWED_PURE_ENTRIES read; or a filed SOUNDNESS row "
                 "recorded in candor_classify::OPEN_ENTRY_VERDICTS as `R<id>:silent|overcharge|conditional`. "
                 "Copying the row into open.tsv is not a verdict, and the push-time test refuses it.")
    for k, _, new in result["grown_unverdicted"]:
        lines.append(f"  {_fmt_key(k)}  effects={new[2]!r}  file={new[3]}")
    lines.append(f"--- GROWN WITH A VERDICT ({len(result['grown_verdicted'])}) — cleared; splice into open.tsv ---")
    for k, _, new in result["grown_verdicted"]:
        lines.append(f"  {_fmt_key(k)}  verdict={verdicts.get(k)}  effects={new[2]!r}")
    lines.append(f"--- UNREGISTERED checked-in open.tsv rows: no verdict, not pre-ruling backlog "
                 f"({len(result['unregistered'])}) — FAILS ---")
    for k, old, _ in result["unregistered"]:
        lines.append(f"  {_fmt_key(k)}  effects={old[2]!r}")
    if result.get("newly_unreachable"):
        lines.append(f"--- NEWLY UNREACHABLE ({len(result['newly_unreachable'])}) — reported, not failing: no "
                     "consumer can name these (generate.py `Reachability`). A wrong verdict here hides a row; "
                     "spot-check any that a public caller does not already carry ---")
        for k, old, new in result["newly_unreachable"]:
            lines.append(f"  {_fmt_key(k)}  effects={new[2]!r}  file={new[3]}"
                         + ("  (was in open.tsv)" if old else ""))

    lines.append("")
    if regressed or oracle_dropped:
        lines.append(PLATFORM_CAVEAT)
    return "\n".join(lines)


def diff_manifests_cli(checked_covered_path, fresh_covered_path, checked_open_path, fresh_open_path,
                        github_output_path=None, out=sys.stdout, baseline_covered_path=None,
                        baseline_open_path=None, verdicts=None, grandfathered=None,
                        checked_unreachable_path=None, fresh_unreachable_path=None):
    """Run `diff_manifests`, print the report to `out`, write GitHub Actions step outputs if
    `github_output_path` is given, and return the process exit code the caller should use: 1 if
    `regressed` or `grown` is non-empty (the two FAILING findings), 0 otherwise. `oracle_dropped` is
    never, by itself, part of the exit-code decision — see the R214 comment block for why."""
    result = diff_manifests(checked_covered_path, fresh_covered_path, checked_open_path, fresh_open_path,
                            baseline_covered_path, baseline_open_path, verdicts=verdicts,
                            grandfathered=grandfathered, checked_unreachable_path=checked_unreachable_path,
                            fresh_unreachable_path=fresh_unreachable_path)
    print(format_diff_report(result), file=out)

    # SOUNDNESS R311 — report a CRATE VERSION CHANGE before the row counts, because it EXPLAINS them.
    # The calibrated fixture pins `"*"`, so an upstream release moves the ledger's reviewed-purity claim
    # onto source nobody reviewed; the only previous symptom was a pile of anonymous newly-uncovered
    # rows, and it cost two wrong diagnoses (R302) before the versions were compared by hand.
    v_old = manifest_versions(checked_covered_path)
    v_new = manifest_versions(fresh_covered_path)
    if not v_old or not v_new:
        print("\n  VERSION STAMP: absent from %s — cannot say which upstream versions produced these "
              "rows.\n  A manifest written before SOUNDNESS R311 carries no stamp; regenerate to get one. "
              "This is UNKNOWN, not 'unchanged'."
              % ("the checked-in manifest" if not v_old else "the fresh manifest"), file=out)
    else:
        moved = sorted(k for k in set(v_old) & set(v_new) if v_old[k] != v_new[k])
        gone = sorted(set(v_old) - set(v_new))
        added = sorted(set(v_new) - set(v_old))
        if moved or gone or added:
            print("\n--- CRATE VERSIONS CHANGED (R311) — these explain rows below, and a MAJOR bump means "
                  "the reviewed-purity claim now covers source no rule was written against ---", file=out)
            for k in moved:
                print("  %-24s %s  ->  %s" % (k, v_old[k], v_new[k]), file=out)
            for k in gone:
                print("  %-24s %s  ->  (no source found)" % (k, v_old[k]), file=out)
            for k in added:
                print("  %-24s (absent)  ->  %s" % (k, v_new[k]), file=out)
        else:
            print("\n  crate versions: identical in both manifests (%d stamped)" % len(v_new), file=out)

    if github_output_path:
        with open(github_output_path, "a") as fh:
            fh.write(f"regressed={len(result['regressed'])}\n")
            fh.write(f"oracle_dropped={len(result['oracle_dropped'])}\n")
            fh.write(f"grown={len(result['grown'])}\n")
            fh.write(f"attributed={'true' if result['baseline'] else 'false'}\n")
            fh.write(f"grown_engine={len(result.get('grown_engine', []))}\n")
            fh.write(f"grown_drift={len(result.get('grown_drift', []))}\n")
            fh.write(f"regressed_engine={len(result.get('regressed_engine', []))}\n")
            fh.write(f"regressed_drift={len(result.get('regressed_drift', []))}\n")
            fh.write(f"grown_unverdicted={len(result['grown_unverdicted'])}\n")
            fh.write(f"grown_verdicted={len(result['grown_verdicted'])}\n")
            fh.write(f"unregistered={len(result['unregistered'])}\n")
            fh.write(f"newly_unreachable={len(result['newly_unreachable'])}\n")

    # R1528 step 3: a grown row with a verdict no longer fails; one without still does, and so does a
    # checked-in row that got into open.tsv with neither a verdict nor a backlog slot.
    if result["regressed"] or result["grown_unverdicted"] or result["unregistered"]:
        print(f"::error::coverage-gate drift: {len(result['regressed'])} regressed "
              f"(classify() lost a rule), {len(result['oracle_dropped'])} oracle-dropped (self-scan "
              f"stopped nominating, reported not failed), {len(result['grown'])} newly-uncovered"
              + (f" ({len(result['grown_engine'])} engine change, {len(result['grown_drift'])} crates.io "
                 f"drift)" if result["baseline"] else " (not attributed)")
              + f" of which {len(result['grown_unverdicted'])} have NO VERDICT, "
              f"{len(result['unregistered'])} checked-in row(s) unregistered"
              + " — see job log above", file=out)
        return 1
    if result["grown_verdicted"]:
        print(f"::notice::coverage-gate: {len(result['grown_verdicted'])} grown row(s) carry a recorded verdict "
              f"and do not fail the job — splice them into open.tsv.", file=out)
    if result["oracle_dropped"]:
        print(f"::warning::coverage-gate: {len(result['oracle_dropped'])} entries dropped out of both "
              f"manifests (self-scan's candidate oracle got more precise, or a crate's self-scan "
              f"failed) — reported, not failing. See job log above.", file=out)
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--registry", default=None,
                    help="registry src dir (default: ~/.cargo/registry/src/index.crates.io-*, newest)")
    ap.add_argument("--scan-bin", default=SCAN_BIN, help="path to a built candor-scan binary")
    ap.add_argument("--selftest", action="store_true",
                    help="run the generator's own extraction tests (no cargo, no registry) and exit")
    ap.add_argument("--print-fixture-toml", action="store_true",
                    help="print a fresh fixture Cargo.toml to stdout (for `cargo fetch`) and exit")
    ap.add_argument("--diff-manifests", nargs=4, default=None,
                    metavar=("CHECKED_COVERED", "FRESH_COVERED", "CHECKED_OPEN", "FRESH_OPEN"),
                    help="diff a fresh regeneration against the checked-in manifests (SOUNDNESS R214) "
                         "and exit with the job's verdict; see diff_manifests()'s docstring")
    ap.add_argument("--unreachable", nargs=2, default=None, metavar=("CHECKED_UNREACHABLE", "FRESH_UNREACHABLE"),
                    help="with --diff-manifests: the checked-in and fresh unreachable.tsv (SOUNDNESS R1528), so "
                         "newly-unreachable rows are reported")
    ap.add_argument("--baseline", nargs=2, default=None, metavar=("BASELINE_COVERED", "BASELINE_OPEN"),
                    help="with --diff-manifests: a fresh regeneration by the engine that WROTE the checked-in "
                         "manifests, over the same fetch, so growth splits into engine change vs drift")
    ap.add_argument("--print-engine-paths", action="store_true",
                    help="print the engine-relevant paths (one per line) and exit; refresh.sh's single source")
    ap.add_argument("--manifest-engine", default=None, metavar="MANIFEST",
                    help="print the engine sha stamped in MANIFEST (empty if none/unrecorded) and exit")
    ap.add_argument("--github-output", default=None,
                    help="with --diff-manifests, append regressed/oracle_dropped/grown counts to this "
                         "GITHUB_OUTPUT file")
    args = ap.parse_args()

    if args.selftest:
        selftest()
        return
    if args.print_engine_paths:
        print("\n".join(ENGINE_PATHS))
        return
    if args.manifest_engine is not None:
        print(manifest_engine(args.manifest_engine))
        return

    if args.print_fixture_toml:
        print_fixture_toml()
        return

    if args.diff_manifests:
        b = args.baseline or (None, None)
        u = args.unreachable or (None, None)
        code = diff_manifests_cli(*args.diff_manifests, github_output_path=args.github_output,
                                  baseline_covered_path=b[0], baseline_open_path=b[1],
                                  checked_unreachable_path=u[0], fresh_unreachable_path=u[1])
        sys.exit(code)

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
    # SOUNDNESS R311 — record WHICH VERSION of each crate was actually read. The fixture pins `"*"`, so
    # a major upstream release silently moves the ledger onto a version nobody reviewed, and until this
    # existed the only symptom was a pile of anonymous "newly-uncovered" rows. `find_crate_dir` already
    # knows the answer; nothing was writing it down.
    resolved_versions = {}
    for c in calibrated:
        if c in EXCLUDE:
            continue
        d, entries = process_crate(registry_src, c)
        if d is not None:
            resolved_versions[c] = os.path.basename(d)
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
    unreachable_path = os.path.join(HERE, "unreachable.tsv")
    subprocess.run(
        ["cargo", "run", "--quiet", "--release", "--manifest-path", CLASSIFY_CHECK_MANIFEST,
         "--", entries_path, covered_path, open_path, unreachable_path],
        check=True,
    )
    # R311 — stamp the resolved versions into both manifests, as a comment so no data row moves and the
    # join key is untouched. `diff_manifests` reads it back and reports a version change as its own
    # labelled alarm, because "sea-orm went 1.x -> 2.x" and "43 entries appeared" are the same event and
    # only the first one is diagnosable.
    for p in (covered_path, open_path, unreachable_path):
        stamp_versions(p, resolved_versions)
    eng = current_engine_stamp()
    for p in (covered_path, open_path, unreachable_path):
        stamp_engine(p, eng)


if __name__ == "__main__":
    main()
