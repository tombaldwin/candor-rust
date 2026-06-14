#!/usr/bin/env python3
"""Fabrication probe for candor-rust — a precision regression guard (sibling of the soundness fuzzer).

candor's CARDINAL SIN is FABRICATION: classifying a PURE function as effectful. Several crate rules in
`candor-classify::classify()` are (or once were) WHOLE-CRATE — they paint one effect onto every path of
an effect-bearing crate, including its pure accessors/factories/inert data types, which perform no I/O /
read no entropy / read no clock. Those rules have since been narrowed to VERB-PRECISE: the effect stays
on the spawn/draw/read/dispatch surface, and the proven-pure members return None. This probe pins that
narrowing down so it can never silently regress to whole-crate.

For each effect-bearing crate it emits two kinds of fixture function:
  PURE  — calls a member that is PROVABLY free of I/O / entropy / clock-read (a length read over an
          already-mapped region, a builder setter, a cached accessor, a deterministic seeded ctor, a
          data-type ctor). candor MUST report it pure (omitted from the report / empty `inferred`).
          If it reports an effect => FABRICATION.
  CTRL  — calls a genuinely-effectful member. candor MUST still report the effect. If it goes pure =>
          a LOST CONTROL (an under-report), the OTHER failure direction this probe also guards.

candor-scan is a SYNTACTIC (syn-based) scanner: it resolves a method-call receiver's type from the
fixture's `use` imports + parameter type annotations WITHOUT compiling against the real crate. So a
fixture like `use memmap2::Mmap; pub fn f(m: &Mmap) -> usize { m.len() }` classifies correctly with NO
memmap2 dependency — the probe ships zero third-party deps. It runs candor-scan on each fixture and
checks the JSON: a function inferred-effect-free is OMITTED from `functions`, so "absent" == pure.

DISCIPLINE (why this probe has no false alarms):
  * Every PURE call is a member whose semantics are verified pure (rationale in the comment beside it).
    When in doubt a method is left OUT entirely (asserted neither pure nor effectful) — never asserted
    pure on a guess.
  * Every fixture body is a SINGLE bare call on a PARAMETER (or an associated-fn call). No method
    chaining: chaining a `.map()`/`.to_owned()` onto a call's result makes the syntactic scanner
    re-resolve the trailing method against the SAME receiver type — an inference artifact unrelated to
    the classifier rule under test. A single bare call tests the classifier rule and nothing else.

Usage:  fabrication_probe.py            # build candor-scan (if needed), run all cases, gate
        CANDOR_SCAN=/path/to/candor-scan fabrication_probe.py   # use a prebuilt scanner binary
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

# Each case: (id, use_lines, receiver_decl, pure_calls, ctrl_calls, expect_effect)
#   use_lines     : the `use` imports the fixture needs (so the scanner can resolve receiver types)
#   receiver_decl : how to name a receiver WITHOUT performing the effect — a function PARAMETER, so no
#                   handle is ever opened/mapped/connected; the only classified call is the probe call.
#                   "" => the case has only associated-fn calls (no receiver).
#   pure_calls    : (stmt, why) — each MUST classify pure (its own fixture fn)
#   ctrl_calls    : (stmt, why) — each MUST classify <expect_effect> (its own fixture fn)
#   expect_effect : the effect the controls must report
#
# Each statement is the FULL body of a `pub fn`. `{r}` is substituted with the receiver name.
CASES = [
    # ---- memmap2: map/flush/protect issue the syscall (Fs); reads over the mapped region are pure ----
    ("memmap2", ["use memmap2::{Mmap, MmapOptions};"], "m: &Mmap",
     [
        ("let _ = {r}.len();",      "length of the already-mapped region — plain memory, no syscall"),
        ("let _ = {r}.is_empty();", "len()==0 — plain memory read"),
        ("let _ = {r}.as_ptr();",   "the base pointer of the mapping — no syscall"),
        ("let _ = MmapOptions::new();", "the request BUILDER — maps nothing yet"),
     ],
     [
        ("let _ = MmapOptions::new().map(&f);", "the mmap(2) syscall (f: &std::fs::File)", "f: &std::fs::File"),
        ("let _ = {r}.flush();",               "msync(2) flush of the mapping to disk"),
     ], "Fs"),

    # ---- tracing: emit/span-lifecycle dispatch is Log; the Level/Span data accessors are pure ----
    ("tracing", ["use tracing::{Level, Span};"], "s: &Span",
     [
        ("let _ = Level::INFO.as_str();", "formats the level name — pure data read"),
        ("let _ = {r}.is_disabled();",    "reads the span's enabled flag — no output"),
        ("let _ = {r}.metadata();",       "returns the span's cached metadata — no output"),
        ("let _ = {r}.id();",             "returns the span's id — no output"),
     ],
     [
        ("let _ = {r}.enter();", "drives the subscriber's span-enter dispatch — program output"),
     ], "Log"),

    # ---- arboard: the Clipboard handle's verbs talk to the OS clipboard; Error formatting is pure ----
    ("arboard", ["use arboard::{Clipboard, Error};"], "c: &mut Clipboard",
     [
        ("let _: String = {e}.to_string();", "Display formatting of the error — pure", "e: &Error"),
     ],
     [
        ("let _ = {r}.get_text();", "reads the OS clipboard contents"),
        ("let _ = {r}.set_text(String::new());", "writes the OS clipboard contents"),
     ], "Clipboard"),

    # ---- fastrand: value draws + entropy-seeded ctors are Rand; deterministic ctor/split/copy are pure ----
    ("fastrand", ["use fastrand::Rng;"], "r: &Rng",
     [
        ("let _ = Rng::with_seed(42);", "DETERMINISTIC seeded ctor — same seed, same stream; no entropy"),
        ("let _ = {r}.fork();",         "splits existing RNG state — draws no fresh entropy"),
        ("let _ = {r}.clone();",        "copies existing RNG state — draws no fresh entropy"),
     ],
     [
        ("let _ = {r}.u32(0..10);", "draws a value from the generator — consumes entropy state"),
        ("let _ = {r}.usize(0..10);", "draws a value from the generator"),
     ], "Rand"),

    # ---- portable_pty: spawn/openpty/wait are Exec; config getters + pure data types are pure ----
    ("portable_pty", ["use portable_pty::{CommandBuilder, PtySize};"], "b: &CommandBuilder",
     [
        ("let _ = {r}.get_argv();", "reads back the configured argv — no spawn"),
        ("let _ = {r}.get_cwd();",  "reads back the configured cwd — no spawn"),
        ("let _ = PtySize::default();", "pure data type — describes a size, spawns nothing"),
        ("let _ = CommandBuilder::new(\"echo\");", "builds the command spec — spawns nothing"),
     ],
     [
        # concrete-typed receiver (NOT `&dyn …`: the syntactic scanner resolves a named type, not a
        # trait object) so the call resolves to `portable_pty::SlavePty::spawn_command`.
        ("let _ = {r}.spawn_command(CommandBuilder::new(\"echo\"));",
         "spawns the child process through the pty", "s: &mut portable_pty::SlavePty"),
     ], "Exec"),

    # ---- chrono: Utc::now/Local::now read the wall clock; Datelike accessors are pure ----
    ("chrono", ["use chrono::{Utc, Local, DateTime, Datelike, Timelike};"], "d: &DateTime<Utc>",
     [
        ("let _ = {r}.year();",   "reads the year field of an existing timestamp — no clock read"),
        ("let _ = {r}.month();",  "reads the month field — no clock read"),
        ("let _ = {r}.hour();",   "reads the hour field — no clock read"),
        ("let _ = {r}.timestamp();", "converts the stored instant to epoch seconds — no clock read"),
     ],
     [
        ("let _ = Utc::now();",   "reads the system wall clock"),
        ("let _ = Local::now();", "reads the system wall clock"),
     ], "Clock"),

    # ---- time: now_utc/now_local read the wall clock; Date/Time accessors are pure ----
    ("time", ["use time::{OffsetDateTime, Date};"], "d: &Date",
     [
        ("let _ = {r}.year();",    "reads the year field of an existing date — no clock read"),
        ("let _ = {r}.ordinal();", "reads the day-of-year of an existing date — no clock read"),
     ],
     [
        ("let _ = OffsetDateTime::now_utc();", "reads the system wall clock"),
     ], "Clock"),

    # ---- tempfile: create/persist touch the disk; the Builder setters + TempDir::path are pure ----
    ("tempfile", ["use tempfile::{TempDir, Builder};"], "d: &TempDir",
     [
        ("let _ = {r}.path();", "returns the cached temp-dir pathname — no FS access"),
        ("let _ = Builder::new().prefix(\"p\");", "a Builder SETTER — configures, creates nothing"),
     ],
     [
        ("let _ = TempDir::new();",  "mkdir(2)s a fresh temp directory on disk"),
        ("let _ = tempfile::tempfile();", "creates an anonymous temp file on disk"),
     ], "Fs"),

    # ---- reqwest: the dispatch (send/execute) is Net; the whole builder chain is pure ----
    ("reqwest", ["use reqwest::{Client, RequestBuilder};"], "rb: reqwest::RequestBuilder",
     [
        ("let _ = Client::new();",        "constructs the client — opens no connection"),
        ("let _ = {r}.header(\"k\", \"v\");", "a request-builder SETTER — sends nothing"),
        ("let _ = {r}.query(&[(\"a\", \"b\")]);", "a request-builder SETTER — sends nothing"),
     ],
     [
        ("let _ = {r}.send();", "dispatches the HTTP request over the network"),
     ], "Net"),

    # ---- url: pure URL parsing/inspection. url has NO effectful surface candor models, so NO control:
    # asserting a control here would be a LOST-CONTROL false alarm. Pure-only is the correct shape — the
    # whole point is that a URL crate must never fabricate Net (parsing a URL contacts nothing). ----
    ("url", ["use url::Url;"], "u: &Url",
     [
        ("let _ = Url::parse(\"http://example.com/a\");", "parses a URL string — contacts nothing"),
        ("let _ = {r}.host_str();", "returns the parsed host substring — contacts nothing"),
        ("let _ = {r}.path();",     "returns the parsed path substring — contacts nothing"),
        ("let _ = {r}.scheme();",   "returns the parsed scheme substring — contacts nothing"),
     ],
     [], None),

    # ---- std unix domain sockets: connect/bind ARE local IPC; SocketAddr::from_pathname is a pure
    # address ctor that opens no socket. The `std::os::unix::net` PREFIX rule would paint Ipc onto it
    # unless carved out. (Found sweeping socket2: `SockAddr::as_unix` → `from_pathname` fabricated Ipc.)
    ("std_unix_net", ["use std::os::unix::net::{UnixStream, SocketAddr};"], "_p: &str",
     [
        ("let _ = SocketAddr::from_pathname(\"/tmp/s\");",
         "builds a unix-socket ADDRESS struct from a path — opens no socket"),
     ],
     [
        ("let _ = UnixStream::connect(\"/tmp/s\");", "opens a unix-domain socket connection"),
     ], "Ipc"),
]


def emit_fixture(case):
    """Return (rust_source, {fn_name: ('pure'|'ctrl', expect_effect, stmt, why)})."""
    cid, uses, recv, pure, ctrl, eff = case
    rname = recv.split(":", 1)[0].strip() if recv else ""
    lines = ["// GENERATED by soundness/fabrication_probe.py — do not edit.", "#![allow(unused)]"]
    lines += uses
    meta = {}
    for kind, calls in (("pure", pure), ("ctrl", ctrl)):
        for idx, item in enumerate(calls):
            # A pure/ctrl entry may override its receiver decl as a 3rd tuple element (e.g. an
            # associated-fn case whose effect lives on a DIFFERENT type than the case's default receiver).
            if len(item) == 3:
                stmt, why, recv_override = item
            else:
                stmt, why, recv_override = item[0], item[1], None
            this_recv = recv_override if recv_override is not None else recv
            this_rname = this_recv.split(":", 1)[0].strip() if this_recv else ""
            name = f"{kind}{idx}"
            body = stmt.replace("{r}", this_rname).replace("{e}", this_rname)
            params = this_recv if this_recv else ""
            meta[name] = (kind, eff, body, why)
            lines.append(f"pub fn {name}({params}) {{ {body} }}")
    return "\n".join(lines) + "\n", meta


def run(scanner, case, workdir):
    cid = case[0]
    src, meta = emit_fixture(case)
    cdir = os.path.join(workdir, cid)
    os.makedirs(os.path.join(cdir, "src"), exist_ok=True)
    with open(os.path.join(cdir, "Cargo.toml"), "w") as f:
        f.write(f'[package]\nname = "probe_{cid}"\nversion = "0.0.0"\nedition = "2021"\n')
    with open(os.path.join(cdir, "src", "lib.rs"), "w") as f:
        f.write(src)
    proc = subprocess.run([scanner, cdir, "--json"], capture_output=True, text=True)
    if proc.returncode != 0 and not proc.stdout.strip():
        return [f"SCAN FAILED for {cid}: {proc.stderr.strip()[:400]}"], 0
    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return [f"BAD REPORT for {cid}: {proc.stdout[:200]!r}"], 0
    inferred = {e["fn"]: e.get("inferred", []) for e in report.get("functions", [])}

    failures = []
    checked = 0
    for fn, (kind, eff, stmt, why) in sorted(meta.items()):
        inf = inferred.get(fn)  # None => omitted => candor judged it pure
        checked += 1
        if kind == "pure":
            if inf:  # any non-empty inferred set on a pure method is a fabrication
                failures.append(f"FABRICATION {cid}::{fn} [{stmt}] -> {inf}  (provably pure: {why})")
        else:  # control
            if not inf or eff not in inf:
                got = inf if inf else "pure/omitted"
                failures.append(f"LOST CONTROL {cid}::{fn} [{stmt}] -> {got}  (must report {eff}: {why})")
    return failures, checked


# RAW multi-module fixtures — a structured (uses/recv/pure/ctrl) case is one module, but some fabrications
# need MORE than one module to reproduce (a name resolving across modules). Each raw case is a full lib.rs
# plus a map `fn-qual -> must_be_pure` (True = candor MUST report it pure; False = MUST report effectful).
RAW_CASES = [
    # ---- primitive-alias / struct name collision (the sled IVec::inline/subslice fabrication) ----
    # `type Buf = [u8; N]` (module `arr`) shares its NAME with `struct Buf` (module `cfg`) whose `Default`
    # reads the clock. A call `Buf::default()` in `arr` means the ARRAY's default (pure, std) — it must NOT
    # resolve to `cfg::Buf`'s effectful `Default`. The struct's own `default` body stays the effect control.
    ("prim_alias_collision", """
mod arr {
    type Buf = [u8; 4];
    pub fn make() -> Buf { Buf::default() }            // PURE — the array's std default, not cfg::Buf's
}
mod cfg {
    pub struct Buf { x: u64 }
    impl Default for Buf {
        fn default() -> Self { let _ = std::time::SystemTime::now(); Buf { x: 0 } }   // Clock (own body)
    }
}
""", {
        "arr::make": True,            # the alias caller must be pure (no fabricated Clock)
        "cfg::Buf::default": False,   # the struct's real Default keeps Clock (control)
    }, ""),
    # ---- cfg-feature block gating (the winnow debug-trace Env over-approximation) ----
    # A `#[cfg(feature="off")]` BLOCK inside an active fn is compiled out under default features, so its
    # effect is NOT the crate's behaviour. The matching ACTIVE-feature block IS (the control). Only inactive
    # BLOCKS are skipped — a whole cfg-gated capability FUNCTION is kept (so an opt-in capability surface is
    # not silently under-reported).
    ("cfg_block_gating", """
mod m {
    pub fn active_fn() {
        #[cfg(feature = "off")]
        { let _ = std::env::var("X"); }   // 'off' not in default → compiled out → active_fn stays PURE
    }
    pub fn on_fn() {
        #[cfg(feature = "on")]
        { let _ = std::env::var("X"); }   // 'on' IS default → block kept → Env (control)
    }
}
""", {
        "m::active_fn": True,   # inactive-feature block skipped → pure
        "m::on_fn": False,      # active-feature block kept → Env
    }, '[features]\ndefault = ["on"]\non = []\noff = []\n'),
]


def run_raw(scanner, cid, src, expect, manifest_extra, workdir):
    cdir = os.path.join(workdir, cid)
    os.makedirs(os.path.join(cdir, "src"), exist_ok=True)
    with open(os.path.join(cdir, "Cargo.toml"), "w") as f:
        f.write(f'[package]\nname = "probe_{cid}"\nversion = "0.0.0"\nedition = "2021"\n{manifest_extra}')
    with open(os.path.join(cdir, "src", "lib.rs"), "w") as f:
        f.write(src)
    proc = subprocess.run([scanner, cdir, "--json"], capture_output=True, text=True)
    if proc.returncode != 0 and not proc.stdout.strip():
        return [f"SCAN FAILED for {cid}: {proc.stderr.strip()[:400]}"], 0
    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return [f"BAD REPORT for {cid}: {proc.stdout[:200]!r}"], 0
    inferred = {e["fn"]: e.get("inferred", []) for e in report.get("functions", [])}
    failures, checked = [], 0
    for fn, must_be_pure in sorted(expect.items()):
        checked += 1
        inf = inferred.get(fn)  # None => omitted => pure
        if must_be_pure and inf:
            failures.append(f"FABRICATION {cid}::{fn} -> {inf}  (must be pure)")
        if not must_be_pure and not inf:
            failures.append(f"LOST CONTROL {cid}::{fn} -> pure/omitted  (must be effectful)")
    return failures, checked


def main():
    scanner = os.environ.get("CANDOR_SCAN")
    if not scanner:
        scanner = os.path.join(ROOT, "target", "debug", "candor-scan")
        if not os.path.exists(scanner):
            print("fabrication-probe: building candor-scan…")
            b = subprocess.run(["cargo", "build", "-q", "-p", "candor-scan"], cwd=ROOT)
            if b.returncode != 0:
                print("FAIL: candor-scan did not build")
                sys.exit(1)
    if not os.path.exists(scanner):
        print(f"FAIL: no scanner at {scanner}")
        sys.exit(1)

    all_failures = []
    total = 0
    with tempfile.TemporaryDirectory() as work:
        for case in CASES:
            fails, checked = run(scanner, case, work)
            total += checked
            all_failures += fails
        for cid, src, expect, manifest_extra in RAW_CASES:
            fails, checked = run_raw(scanner, cid, src, expect, manifest_extra, work)
            total += checked
            all_failures += fails

    print(f"fabrication-probe: {total} probe functions checked across {len(CASES) + len(RAW_CASES)} crates")
    if all_failures:
        print(f"fabrication-probe: {len(all_failures)} FAILURE(S):")
        for f in all_failures:
            print("  " + f)
        sys.exit(1)
    print("fabrication-probe: OK — no fabrication, no lost control")


if __name__ == "__main__":
    main()
