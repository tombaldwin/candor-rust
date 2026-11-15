//! STEP 0b — the PER-CALL-SITE SIDECAR: a measuring instrument, off unless `CANDOR_SIDECAR` names a directory.
//!
//! For each call site the collector handles it writes one record, keyed by the MEMBER IDENTIFIER's position
//! (`file:line:col`, 1-based col in chars — the key the rustc oracle in the dylint crate uses), carrying what the scan did
//! there: the calls the collector recorded (and so the receiver type the route resolved against), the local edges, effects,
//! `Unknown` reasons and post-fixpoint deferrals that site's calls produced, or nothing.
//!
//! It must not move the report. Nothing here is read by any route; the attribution in `scan.rs` takes a function's own
//! sets out of the maps before each call and puts their union back after the last one, and the loop body never reads them
//! for the function it is filling (checked: no `get`/`contains` on those maps inside the loop). With the variable unset
//! every entry point below returns at its first line.
//!
//! WHY POSITIONS NEED A TABLE. proc-macro2's fallback span is an offset into a THREAD-LOCAL source map, and candor walks
//! a file on a different thread from the one that parsed it (`model::SendFile`), so `span.start()` in the collector would
//! answer against an unrelated file, or abort. The span's raw offset survives the move (its `Debug` prints it), so the
//! parse worker records offset → (line, col) for every token of the file it parsed (`index_file`), and the collector
//! looks offsets up in that table. A token the table does not hold (re-spanned macro input, a splice from another file)
//! has no position, and is said to have none.
//!
//! CACHE (the R1634 rule). Positions are not cached (`#[serde(skip)]`), so a replayed entry would have none; and an entry
//! written by an instrumented run must not be replayed by an ordinary one. `scan.rs` therefore turns `--incremental` off
//! for a run with the variable set: it neither reads nor writes the cache.
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::model::Call;

type Table = HashMap<usize, (u32, u32)>;

static DIR: OnceLock<Option<String>> = OnceLock::new();

/// Read `CANDOR_SIDECAR` ONCE, at the start of a run (`scan::scan_main`), so the rest of the engine asks a flag rather than
/// the environment. An operator input (a sink), not a switch: nothing it selects reaches the report.
pub(crate) fn init() {
    let _ = DIR.set(candor_report::env::operator("CANDOR_SIDECAR").ok().filter(|s| !s.is_empty()));
}

/// The sidecar directory, when the instrument is on.
pub(crate) fn dir() -> Option<&'static str> {
    DIR.get().and_then(|d| d.as_deref())
}

pub(crate) fn on() -> bool {
    matches!(DIR.get(), Some(Some(_)))
}

fn tables() -> &'static Mutex<HashMap<String, Arc<Table>>> {
    static T: OnceLock<Mutex<HashMap<String, Arc<Table>>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The raw source-map offset of a span, read without consulting any thread's source map.
fn lo_of(s: proc_macro2::Span) -> Option<usize> {
    let d = format!("{s:?}");
    let inner = d.strip_prefix("bytes(")?;
    let lo = inner.split("..").next()?;
    lo.parse().ok()
}

/// On the PARSE worker: offset → (line, col) for every token of `file` (before any splice, so every token is this file's).
pub(crate) fn index_file(rel: &str, file: &syn::File) {
    if !on() {
        return;
    }
    use quote::ToTokens;
    fn walk(ts: proc_macro2::TokenStream, out: &mut Table) {
        let mut put = |s: proc_macro2::Span| {
            if let Some(lo) = lo_of(s) {
                if lo != 0 {
                    let p = s.start();
                    out.entry(lo).or_insert((p.line as u32, p.column as u32 + 1));
                }
            }
        };
        let mut groups = Vec::new();
        for tt in ts {
            match tt {
                proc_macro2::TokenTree::Group(g) => {
                    put(g.span_open());
                    put(g.span_close());
                    groups.push(g.stream());
                }
                proc_macro2::TokenTree::Ident(i) => put(i.span()),
                proc_macro2::TokenTree::Punct(p) => put(p.span()),
                proc_macro2::TokenTree::Literal(l) => put(l.span()),
            }
        }
        for g in groups {
            walk(g, out);
        }
    }
    let mut t = Table::new();
    let ts = file.to_token_stream();
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| walk(ts, &mut t)));
    if let Ok(mut m) = tables().lock() {
        m.insert(rel.to_string(), Arc::new(t));
    }
}

thread_local! {
    static CUR: RefCell<Option<Arc<Table>>> = const { RefCell::new(None) };
    static OUTER: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    static NOTES: RefCell<Vec<(String, Option<[u32; 3]>)>> = const { RefCell::new(Vec::new()) };
    static DEPHITS: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
}

/// Pass B: the file being walked, for the lifetime of the guard.
pub(crate) struct FileScope(bool);

impl FileScope {
    pub(crate) fn enter(rel: &str) -> FileScope {
        if !on() {
            return FileScope(false);
        }
        let t = tables().lock().ok().and_then(|m| m.get(rel).cloned());
        CUR.with(|c| *c.borrow_mut() = t);
        FileScope(true)
    }
}

impl Drop for FileScope {
    fn drop(&mut self) {
        if self.0 {
            CUR.with(|c| *c.borrow_mut() = None);
            OUTER.with(|o| o.set(None));
        }
    }
}

fn pos(s: proc_macro2::Span) -> Option<(u32, u32)> {
    let lo = lo_of(s)?;
    if lo == 0 {
        return None;
    }
    CUR.with(|c| c.borrow().as_ref().and_then(|t| t.get(&lo).copied()))
}

/// A note the collector made at a site (a refusal, a dispatch, an unresolved callable).
pub(crate) fn note(kind: &str, detail: &str) {
    if on() {
        NOTES.with(|n| n.borrow_mut().push((format!("{kind}|{detail}"), None)));
    }
}

/// A dependency-report entry a route looked up and found (`deps::DepMap::get`), and whether it carries an effect.
pub(crate) fn dep_hit(key: String, effectful: bool) {
    DEPHITS.with(|h| h.borrow_mut().push((key, effectful)));
}

/// The dependency entries looked up since the last call boundary in `scan.rs`'s call loop.
pub(crate) fn take_dep_hits() -> Vec<(String, bool)> {
    if !on() {
        return Vec::new();
    }
    DEPHITS.with(|h| std::mem::take(&mut *h.borrow_mut()))
}

/// Where the next body's notes will start (a nested body walked inside another takes only its own).
pub(crate) fn notes_mark() -> usize {
    if !on() {
        return 0;
    }
    NOTES.with(|n| n.borrow().len())
}

/// The notes made since `mark`, removed from the buffer.
pub(crate) fn take_notes_from(mark: usize) -> Vec<(String, Option<[u32; 3]>)> {
    if !on() {
        return Vec::new();
    }
    NOTES.with(|n| {
        let mut v = n.borrow_mut();
        let m = mark.min(v.len());
        v.split_off(m)
    })
}

/// Inside a macro invocation: the OUTERMOST invocation's position keys every site within it, for the guard's lifetime.
/// Only the guard that set it clears it.
pub(crate) struct MacroScope(bool);

impl MacroScope {
    pub(crate) fn enter(m: &syn::Macro) -> MacroScope {
        if !on() {
            return MacroScope(false);
        }
        let start = m.path.segments.first().map(|s| s.ident.span()).and_then(pos);
        let close = match &m.delimiter {
            syn::MacroDelimiter::Paren(p) => pos(p.span.close()),
            syn::MacroDelimiter::Brace(b) => pos(b.span.close()),
            syn::MacroDelimiter::Bracket(b) => pos(b.span.close()),
        };
        let name = m.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>().join("::");
        if OUTER.with(|o| o.get()).is_none() {
            if let (Some(s), Some(e)) = (start, close) {
                NOTES.with(|n| {
                    n.borrow_mut().push((format!("macgroup|{name}|{}:{}|{}:{}", s.0, s.1, e.0, e.1), Some([s.0, s.1, 1])))
                });
                OUTER.with(|o| o.set(Some(s)));
                return MacroScope(true);
            }
        }
        MacroScope(false)
    }
}

impl Drop for MacroScope {
    fn drop(&mut self) {
        if self.0 {
            OUTER.with(|o| o.set(None));
        }
    }
}

fn binop_span(op: &syn::BinOp) -> Option<proc_macro2::Span> {
    use syn::BinOp::*;
    Some(match op {
        Add(t) => t.spans[0],
        Sub(t) => t.spans[0],
        Mul(t) => t.spans[0],
        Div(t) => t.spans[0],
        Rem(t) => t.spans[0],
        And(t) => t.spans[0],
        Or(t) => t.spans[0],
        BitXor(t) => t.spans[0],
        BitAnd(t) => t.spans[0],
        BitOr(t) => t.spans[0],
        Shl(t) => t.spans[0],
        Shr(t) => t.spans[0],
        Eq(t) => t.spans[0],
        Lt(t) => t.spans[0],
        Le(t) => t.spans[0],
        Ne(t) => t.spans[0],
        Ge(t) => t.spans[0],
        Gt(t) => t.spans[0],
        AddAssign(t) => t.spans[0],
        SubAssign(t) => t.spans[0],
        MulAssign(t) => t.spans[0],
        DivAssign(t) => t.spans[0],
        RemAssign(t) => t.spans[0],
        BitXorAssign(t) => t.spans[0],
        BitAndAssign(t) => t.spans[0],
        BitOrAssign(t) => t.spans[0],
        ShlAssign(t) => t.spans[0],
        ShrAssign(t) => t.spans[0],
        _ => return None,
    })
}

/// The FIRST token of an expression, without `Span::join` (which consults the walking thread's source map).
fn first_span(e: &syn::Expr) -> Option<proc_macro2::Span> {
    use syn::Expr::*;
    match e {
        Path(p) => match &p.qself {
            Some(q) => Some(q.lt_token.span),
            None => p.path.leading_colon.map(|c| c.spans[0]).or_else(|| p.path.segments.first().map(|s| s.ident.span())),
        },
        MethodCall(m) => first_span(&m.receiver),
        Call(c) => first_span(&c.func),
        Field(f) => first_span(&f.base),
        Index(i) => first_span(&i.expr),
        Await(a) => first_span(&a.base),
        Try(t) => first_span(&t.expr),
        Cast(c) => first_span(&c.expr),
        Binary(b) => first_span(&b.left),
        Reference(r) => Some(r.and_token.span),
        Unary(u) => match u.op {
            syn::UnOp::Deref(t) => Some(t.spans[0]),
            syn::UnOp::Not(t) => Some(t.spans[0]),
            syn::UnOp::Neg(t) => Some(t.spans[0]),
            _ => None,
        },
        Lit(l) => Some(l.lit.span()),
        Paren(p) => Some(p.paren_token.span.open()),
        Tuple(t) => Some(t.paren_token.span.open()),
        Array(a) => Some(a.bracket_token.span.open()),
        Macro(m) => m.mac.path.segments.first().map(|s| s.ident.span()),
        Struct(s) => s.path.segments.first().map(|s| s.ident.span()),
        Range(r) => match &r.start {
            Some(s) => first_span(s),
            None => None,
        },
        Block(b) => Some(b.block.brace_token.span.open()),
        _ => None,
    }
}

/// The site an expression's calls are keyed to, and its form; `None` form = not a call site (`aux`).
fn site(e: &syn::Expr) -> (Option<proc_macro2::Span>, Option<&'static str>) {
    use syn::Expr::*;
    match e {
        MethodCall(m) => (Some(m.method.span()), Some("method")),
        Call(c) => match &*c.func {
            Path(p) => {
                let f = if p.qself.is_some() {
                    "qualified"
                } else if p.path.segments.len() > 1 {
                    "path"
                } else {
                    "path1"
                };
                (p.path.segments.last().map(|s| s.ident.span()), Some(f))
            }
            other => (first_span(other), Some("callexpr")),
        },
        Binary(b) => (binop_span(&b.op), Some("op")),
        Unary(u) => (first_span(e).filter(|_| matches!(u.op, syn::UnOp::Deref(_) | syn::UnOp::Not(_) | syn::UnOp::Neg(_))), Some("op")),
        Index(i) => (Some(i.bracket_token.span.open()), Some("op:Index")),
        Path(p) => (p.path.segments.last().map(|s| s.ident.span()), Some("fnref")),
        Await(a) => (Some(a.await_token.span), Some("await")),
        Try(t) => (Some(t.question_token.spans[0]), Some("try")),
        ForLoop(f) => (first_span(&f.expr), Some("for")),
        Macro(m) => (m.mac.path.segments.first().map(|s| s.ident.span()), Some("macro")),
        _ => (first_span(e), None),
    }
}

/// Before an expression's walk: where its calls and notes start.
pub(crate) fn mark(calls: &[Call]) -> Option<(usize, usize)> {
    if !on() {
        return None;
    }
    Some((calls.len(), NOTES.with(|n| n.borrow().len())))
}

/// After an expression's walk: every call and note made inside it and not already claimed by an inner expression
/// belongs to this one.
pub(crate) fn claim(e: &syn::Expr, calls: &mut [Call], m: Option<(usize, usize)>) {
    let Some((c0, n0)) = m else { return };
    let (sp, form) = site(e);
    let outer = OUTER.with(|o| o.get());
    let p = sp.and_then(pos);
    let key: Option<[u32; 3]> = match (outer, form) {
        (Some(o), _) => Some([o.0, o.1, 1]),
        (None, Some(_)) => p.map(|p| [p.0, p.1, 0]),
        (None, None) => p.map(|p| [p.0, p.1, 2]),
    };
    if let Some(f) = form {
        // The collector VISITED a call of this form here, whether or not it recorded anything.
        let inner = p.map(|p| format!("{}:{}", p.0, p.1)).unwrap_or_default();
        NOTES.with(|n| n.borrow_mut().push((format!("site|{f}|{inner}"), key)));
    }
    for c in calls.iter_mut().skip(c0) {
        if c.sp.is_none() {
            c.sp = key;
        }
    }
    NOTES.with(|n| {
        for x in n.borrow_mut().iter_mut().skip(n0) {
            if x.1.is_none() {
                x.1 = key;
            }
        }
    });
}

/// Per-call attribution inside `scan.rs`'s call loop: take what a call added to its function's set, and fold it into the
/// stash that is put back after the loop. The stash starts as the function's ORIGINAL entry (`m.remove(k)` before the
/// loop), so after `restore` an entry exists exactly when one existed before or a call made one, with the same members.
pub(crate) fn drain<S: Ord + Clone + ToString>(
    m: &mut HashMap<String, std::collections::BTreeSet<S>>,
    k: &str,
    stash: &mut Option<std::collections::BTreeSet<S>>,
) -> Vec<String> {
    match m.remove(k) {
        Some(s) => {
            let v = s.iter().map(ToString::to_string).collect();
            stash.get_or_insert_with(Default::default).extend(s);
            v
        }
        None => Vec::new(),
    }
}

pub(crate) fn restore<S: Ord>(m: &mut HashMap<String, std::collections::BTreeSet<S>>, k: &str, stash: Option<std::collections::BTreeSet<S>>) {
    if let Some(s) = stash {
        m.insert(k.to_string(), s);
    }
}

pub(crate) fn call_json(c: &Call) -> serde_json::Value {
    serde_json::json!({
        "path": c.path, "leaf": c.leaf, "typed": c.typed, "method": c.method, "dot": c.dot, "macro": c.is_macro,
        "exact_ty": c.exact_ty, "argc": c.argc,
    })
}

/// Write the sidecar for one scanned crate.
pub(crate) fn write(scan_dir: &str, fns: &[crate::model::FnInfo], records: Vec<serde_json::Value>) {
    let Some(d) = dir() else { return };
    let root = std::fs::canonicalize(scan_dir).map(|p| p.display().to_string()).unwrap_or_else(|_| scan_dir.to_string());
    let abs = |rel: &str| -> String {
        let p = std::path::Path::new(&root).join(rel);
        std::fs::canonicalize(&p).map(|c| c.display().to_string()).unwrap_or_else(|_| p.display().to_string())
    };
    let mut files: HashMap<String, String> = HashMap::new();
    let mut fn_rows = Vec::new();
    let mut note_rows = Vec::new();
    for f in fns {
        let (file, rest) = match f.loc.split_once(':') {
            Some((a, b)) => (a.to_string(), b.to_string()),
            None => (f.loc.clone(), String::new()),
        };
        let af = files.entry(file.clone()).or_insert_with(|| abs(&file)).clone();
        fn_rows.push(serde_json::json!({
            "qual": f.qual, "leaf": f.leaf, "file": af, "at": rest, "impl_trait": f.impl_trait,
            "impl_trait_path": f.impl_trait_path, "has_self": f.has_self, "assoc": f.assoc,
        }));
        for (n, sp) in &f.sc_notes {
            note_rows.push(serde_json::json!({ "caller": f.qual, "file": af, "sp": sp, "note": n }));
        }
    }
    let mut recs = records;
    for r in recs.iter_mut() {
        if let Some(rel) = r.get("rel").and_then(|v| v.as_str()).map(str::to_string) {
            let af = files.entry(rel.clone()).or_insert_with(|| abs(&rel)).clone();
            r["file"] = serde_json::Value::String(af);
        }
    }
    let doc = serde_json::json!({ "root": root, "fns": fn_rows, "notes": note_rows, "calls": recs });
    let _ = std::fs::create_dir_all(d);
    let name: String = root.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let _ = std::fs::write(format!("{d}/{name}.json"), serde_json::to_string(&doc).unwrap_or_default());
}
