//! STEP 0b — the HIR-typeck RESOLUTION ORACLE: a dump mode, off unless `CANDOR_ORACLE_DUMP` names a directory.
//!
//! It answers ONE question per call site the source writes, with rustc's own answer: which item does this call reach?
//! It feeds an out-of-tree comparator that sets candor-scan's per-call-site sidecar beside it (`CANDOR_SIDECAR`). Nothing
//! here changes what the lint reports: the lint's own `check_expr` runs exactly as before, this only READS typeck results,
//! and with the variable unset `Candor::oracle` is `None` and nothing below executes.
//!
//! One JSON line per call site, keyed by the MEMBER IDENTIFIER's span (`file:line:col`, 1-based col, chars), plus one
//! `crate` header line carrying the configuration the crate compiled under. Three truth levels per site:
//! - `named`  — the item the call names (`type_dependent_def_id` / the callee's `FnDef`): the TRAIT item for a trait method;
//! - `resolved` — `Instance::try_resolve` under the CALLER's typing env: the impl item where the caller's own types decide
//!   it (override vs provided default included), still the trait item where only a monomorphising caller or a vtable can;
//! - `mono` — the same with opaque types revealed (post-analysis), which is what codegen runs.
//!
//! Resolution is judged against `resolved`; effect truth against `mono`.
//!
//! Desugared calls (`for`, `?`, `.await`, operators) are LABELLED, not dropped. A site inside a macro expansion is keyed
//! to the OUTERMOST invocation in written code and marked `exp`.
use std::collections::{BTreeSet, HashMap, HashSet};

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, LocalDefId};
use rustc_hir::{Expr, ExprKind, HirId, Node, PatKind, QPath};
use rustc_lint::LateContext;
use rustc_middle::ty::{self, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::{ExpnKind, Span};

pub(crate) struct Oracle {
    dir: String,
    lines: Vec<String>,
    files: HashMap<String, String>,
    /// CENSUS — the expression `record` last keyed (its HirId and its `site` fragment), so the effects and edges the lint
    /// adds while analysing that same expression are keyed to the same site.
    cur: Option<(HirId, String)>,
    /// CENSUS — unit, contribution and post-edge lines, written to their own file (`.census`) beside the site dump.
    census: Vec<String>,
}

pub(crate) fn from_env() -> Option<Oracle> {
    let dir = candor_report::env::operator("CANDOR_ORACLE_DUMP").ok()?;
    if dir.is_empty() {
        return None;
    }
    Some(Oracle { dir, lines: Vec::new(), files: HashMap::new(), cur: None, census: Vec::new() })
}

fn esc(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// The type's HEAD: references peeled, an ADT by its item name, a parameter/`dyn`/opaque by kind.
fn ty_head<'tcx>(tcx: TyCtxt<'tcx>, t: Ty<'tcx>) -> String {
    let t = t.peel_refs();
    match t.kind() {
        ty::Adt(d, _) => tcx.item_name(d.did()).to_string(),
        ty::Param(p) => format!("param:{}", p.name),
        ty::Dynamic(preds, ..) => match preds.principal_def_id() {
            Some(d) => format!("dyn:{}", tcx.item_name(d)),
            None => "dyn".into(),
        },
        ty::Alias(..) => "alias".into(),
        ty::Str => "str".into(),
        ty::Slice(_) => "slice".into(),
        ty::Array(..) => "array".into(),
        ty::Closure(..) => "closure".into(),
        ty::FnDef(..) | ty::FnPtr(..) => "fn".into(),
        ty::Tuple(ts) if ts.is_empty() => "()".into(),
        _ => rustc_middle::ty::print::with_no_trimmed_paths!(t.to_string()),
    }
}

fn ty_str<'tcx>(t: Ty<'tcx>) -> String {
    rustc_middle::ty::print::with_no_trimmed_paths!(t.to_string())
}

impl Oracle {
    fn file_of(&mut self, cx: &LateContext<'_>, sp: Span) -> (String, usize, usize) {
        let sm = cx.tcx.sess.source_map();
        let loc = sm.lookup_char_pos(sp.lo());
        let key = format!("{:?}", loc.file.name);
        let path = self
            .files
            .entry(key)
            .or_insert_with(|| match &loc.file.name {
                rustc_span::FileName::Real(r) => match r.local_path() {
                    Some(p) => std::fs::canonicalize(p).map(|c| c.display().to_string()).unwrap_or_else(|_| p.display().to_string()),
                    None => format!("{:?}", loc.file.name),
                },
                other => format!("<{other:?}>"),
            })
            .clone();
        (path, loc.line, loc.col.0 + 1)
    }

    fn pos(&mut self, cx: &LateContext<'_>, sp: Span) -> String {
        let (f, l, c) = self.file_of(cx, sp);
        format!("{{\"file\":{},\"line\":{l},\"col\":{c}}}", esc(&f))
    }

    /// The key span, whether it came out of a macro (and which one, outermost), and any desugaring on the way.
    fn site(&mut self, cx: &LateContext<'_>, sp: Span) -> String {
        let mut s = sp;
        let mut kinds: Vec<String> = Vec::new();
        let mut in_macro = false;
        let mut desugar: Option<String> = None;
        let mut guard = 0;
        while s.from_expansion() && guard < 64 {
            guard += 1;
            let d = s.ctxt().outer_expn_data();
            match d.kind {
                ExpnKind::Macro(mk, name) => {
                    in_macro = true;
                    kinds.push(format!("{}:{name}", mk.descr()));
                }
                ExpnKind::Desugaring(k) => {
                    if desugar.is_none() {
                        desugar = Some(format!("{k:?}"));
                    }
                    kinds.push(format!("desugar:{k:?}"));
                }
                ExpnKind::AstPass(p) => kinds.push(format!("astpass:{p:?}")),
                ExpnKind::Root => break,
            }
            s = d.call_site;
        }
        let raw = self.pos(cx, sp);
        let outer = self.pos(cx, s);
        let key = if in_macro { outer.clone() } else { raw.clone() };
        let kinds = kinds.iter().map(|k| esc(k)).collect::<Vec<_>>().join(",");
        format!(
            "\"key\":{key},\"exp\":{in_macro},\"desugar\":{},\"raw\":{raw},\"outer\":{outer},\"expn\":[{kinds}]",
            desugar.as_deref().map(esc).unwrap_or_else(|| "null".into())
        )
    }

    fn def(&mut self, cx: &LateContext<'_>, did: DefId) -> String {
        let tcx = cx.tcx;
        let path = rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did));
        let verbose = tcx.def_path(did).to_string_no_crate_verbose();
        let krate = tcx.crate_name(did.krate).to_string();
        let dk = tcx.def_kind(did);
        let name = tcx.opt_item_name(did).map(|n| n.to_string()).unwrap_or_default();
        // A non-local item's span comes from its crate's metadata: for a sibling crate of the same package (a bin calling
        // its lib) it names the file the scanner read, which is what makes it a LOCAL target to the scanner.
        let ident = tcx.def_ident_span(did).map(|s| self.pos(cx, s)).unwrap_or_else(|| "null".into());
        let mut parent = "null".to_string();
        if let Some(p) = tcx.opt_parent(did) {
            let pk = tcx.def_kind(p);
            let ppath = rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(p));
            let pverb = tcx.def_path(p).to_string_no_crate_verbose();
            let pspan = if p.is_local() { self.pos(cx, tcx.def_span(p)) } else { "null".into() };
            let (of_trait, self_head) = match pk {
                DefKind::Impl { of_trait } => {
                    let tr = if of_trait {
                        let id = tcx.impl_trait_id(p);
                        esc(&rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(id)))
                    } else {
                        "null".into()
                    };
                    let st = tcx.type_of(p).instantiate_identity().skip_normalization();
                    (tr, esc(&ty_head(tcx, st)))
                }
                _ => ("null".into(), "null".into()),
            };
            parent = format!(
                "{{\"kind\":{},\"path\":{},\"verbose\":{},\"span\":{pspan},\"trait\":{of_trait},\"self\":{self_head}}}",
                esc(&format!("{pk:?}")),
                esc(&ppath),
                esc(&pverb)
            );
        }
        let trait_item = tcx.trait_of_assoc(did).is_some();
        let has_value = trait_item && tcx.defaultness(did).has_value();
        format!(
            "{{\"path\":{},\"verbose\":{},\"krate\":{},\"local\":{},\"kind\":{},\"name\":{},\"ident\":{ident},\"trait_item\":{trait_item},\"provided\":{has_value},\"exp\":{},\"parent\":{parent}}}",
            esc(&path),
            esc(&verbose),
            esc(&krate),
            did.is_local(),
            esc(&format!("{dk:?}")),
            esc(&name),
            tcx.def_span(did).from_expansion()
        )
    }

    /// `Instance::try_resolve` at one typing env: the instance's item, or why there is none.
    fn resolve<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        did: DefId,
        args: ty::GenericArgsRef<'tcx>,
        reveal: bool,
    ) -> String {
        if !matches!(cx.tcx.def_kind(did), DefKind::Fn | DefKind::AssocFn) {
            return "{\"status\":\"not-fn\"}".into();
        }
        if args.has_escaping_bound_vars() {
            return "{\"status\":\"escaping\"}".into();
        }
        let (env, args) = if reveal {
            let env = cx.typing_env().with_post_analysis_normalized(cx.tcx);
            let a = cx.tcx.try_normalize_erasing_regions(env, ty::Unnormalized::new(args)).unwrap_or(args);
            (env, a)
        } else {
            (cx.typing_env(), args)
        };
        match ty::Instance::try_resolve(cx.tcx, env, did, args) {
            Ok(Some(inst)) => {
                let kind = match inst.def {
                    ty::InstanceKind::Item(_) => "item",
                    ty::InstanceKind::Virtual(..) => "virtual",
                    ty::InstanceKind::Intrinsic(_) => "intrinsic",
                    _ => "shim",
                };
                let d = self.def(cx, inst.def_id());
                format!("{{\"status\":\"{kind}\",\"def\":{d}}}")
            }
            Ok(None) => "{\"status\":\"unresolved\"}".into(),
            Err(_) => "{\"status\":\"error\"}".into(),
        }
    }

    pub(crate) fn record<'tcx>(&mut self, cx: &LateContext<'tcx>, expr: &'tcx Expr<'tcx>) {
        let Some(typeck) = cx.maybe_typeck_results() else {
            return;
        };
        let tcx = cx.tcx;
        // (form, key span, named item, generic args, receiver expr, path self type)
        let mut recv: Option<&'tcx Expr<'tcx>> = None;
        let mut written_self: Option<String> = None;
        let (form, key_sp, named, args): (String, Span, Option<DefId>, Option<ty::GenericArgsRef<'tcx>>) = match expr.kind {
            ExprKind::MethodCall(seg, r, _, _) => {
                recv = Some(r);
                ("method".into(), seg.ident.span, typeck.type_dependent_def_id(expr.hir_id), Some(typeck.node_args(expr.hir_id)))
            }
            ExprKind::Call(callee, _) => {
                let (named, args) = match typeck.expr_ty(callee).kind() {
                    ty::FnDef(d, a) => (Some(*d), Some(*a)),
                    _ => (None, None),
                };
                match callee.kind {
                    ExprKind::Path(ref qp) => match qp {
                        QPath::Resolved(Some(_), p) => {
                            ("qualified".into(), p.segments.last().map(|s| s.ident.span).unwrap_or(callee.span), named, args)
                        }
                        QPath::Resolved(None, p) => {
                            let f = if p.segments.len() > 1 { "path" } else { "path1" };
                            (f.into(), p.segments.last().map(|s| s.ident.span).unwrap_or(callee.span), named, args)
                        }
                        QPath::TypeRelative(q, seg) => {
                            // The Self type AS WRITTEN (`AtomicUsize` where rustc's item sits on `Atomic<usize>`).
                            if let rustc_hir::TyKind::Path(QPath::Resolved(_, qp)) = q.kind {
                                written_self = qp.segments.last().map(|s| s.ident.to_string());
                            }
                            ("type_path".into(), seg.ident.span, named, args)
                        }
                    },
                    _ => ("callexpr".into(), callee.span, named, args),
                }
            }
            ExprKind::Binary(op, lhs, _) => {
                use rustc_hir::BinOpKind::*;
                let did = typeck.type_dependent_def_id(expr.hir_id);
                let did = if did.is_none() && matches!(op.node, Eq | Ne | Lt | Le | Gt | Ge) {
                    // `==`/`!=` record no type_dependent_def_id; an ADT operand still calls its `PartialEq` impl.
                    match crate::resolve_cmp_op(cx, typeck, op.node, lhs) {
                        Some(crate::Callee::Def { did, .. }) if !typeck.expr_ty(lhs).peel_refs().is_primitive() => Some(did),
                        _ => None,
                    }
                } else {
                    did
                };
                let Some(did) = did else { return };
                (format!("op:{:?}", op.node), op.span, Some(did), Some(typeck.node_args(expr.hir_id)))
            }
            ExprKind::AssignOp(op, _, _) => {
                let Some(did) = typeck.type_dependent_def_id(expr.hir_id) else { return };
                (format!("op:{:?}", op.node), op.span, Some(did), Some(typeck.node_args(expr.hir_id)))
            }
            ExprKind::Unary(op, _) => {
                let Some(did) = typeck.type_dependent_def_id(expr.hir_id) else { return };
                (format!("op:{op:?}"), expr.span.shrink_to_lo(), Some(did), Some(typeck.node_args(expr.hir_id)))
            }
            ExprKind::Index(_, _, br) => {
                let Some(did) = typeck.type_dependent_def_id(expr.hir_id) else { return };
                ("op:Index".into(), br.shrink_to_lo(), Some(did), Some(typeck.node_args(expr.hir_id)))
            }
            ExprKind::Path(ref qp) => {
                // A named fn used as a VALUE (a callback). Its callee position is the `Call` arm above.
                if let rustc_hir::Node::Expr(p) = tcx.parent_hir_node(expr.hir_id)
                    && let ExprKind::Call(c, _) = p.kind
                    && c.hir_id == expr.hir_id
                {
                    return;
                }
                let (named, args) = match typeck.expr_ty(expr).kind() {
                    ty::FnDef(d, a) => (Some(*d), Some(*a)),
                    _ => return,
                };
                let sp = match qp {
                    QPath::Resolved(_, p) => p.segments.last().map(|s| s.ident.span).unwrap_or(expr.span),
                    QPath::TypeRelative(_, seg) => seg.ident.span,
                };
                ("fnref".into(), sp, named, args)
            }
            _ => return,
        };
        let site = self.site(cx, key_sp);
        let named_s = match named {
            Some(d) => self.def(cx, d),
            None => "null".into(),
        };
        let (resolved, mono) = match (named, args) {
            (Some(d), Some(a)) => (self.resolve(cx, d, a, false), self.resolve(cx, d, a, true)),
            _ => ("null".into(), "null".into()),
        };
        // CENSUS — `x.into()` runs core's blanket `impl<T, U: From<T>> Into<U> for T`, whose body calls `U::from(x)`: the
        // item that really runs is the `From` impl. rustc's `resolved` names the blanket item; this names the impl.
        let via_into = match (named, args) {
            (Some(d), Some(a)) if tcx.opt_item_name(d).is_some_and(|n| n.as_str() == "into")
                && tcx.trait_of_assoc(d).is_some_and(|t| tcx.is_diagnostic_item(rustc_span::sym::Into, t))
                && a.len() == 2 =>
            {
                match tcx.get_diagnostic_item(rustc_span::sym::From).and_then(|ft| {
                    tcx.associated_item_def_ids(ft).iter().copied().find(|f| tcx.opt_item_name(*f).is_some_and(|n| n.as_str() == "from"))
                }) {
                    Some(from_fn) => {
                        let fa = tcx.mk_args(&[a[1], a[0]]);
                        self.resolve(cx, from_fn, fa, true)
                    }
                    None => "null".into(),
                }
            }
            _ => "null".into(),
        };
        // Receiver: unadjusted, adjusted, and every autoderef/autoref step between.
        let recv_s = match recv {
            Some(r) => {
                let un = typeck.expr_ty(r);
                let adj = typeck.expr_ty_adjusted(r);
                let mut chain: Vec<String> = vec![esc(&ty_head(tcx, un))];
                for a in typeck.expr_adjustments(r) {
                    chain.push(esc(&ty_head(tcx, a.target)));
                }
                format!(
                    "{{\"unadj\":{},\"adj\":{},\"unadj_head\":{},\"adj_head\":{},\"chain\":[{}]}}",
                    esc(&ty_str(un)),
                    esc(&ty_str(adj)),
                    esc(&ty_head(tcx, un)),
                    esc(&ty_head(tcx, adj)),
                    chain.join(",")
                )
            }
            None => match (named, args) {
                // A path call's "receiver" is its Self type, where it has one.
                (Some(d), Some(a)) if tcx.trait_of_assoc(d).is_some() && !a.is_empty() => {
                    match a[0].kind() {
                        ty::GenericArgKind::Type(t) => format!(
                            "{{\"unadj\":{},\"adj\":{},\"unadj_head\":{},\"adj_head\":{},\"chain\":[{}],\"self\":true}}",
                            esc(&ty_str(t)),
                            esc(&ty_str(t)),
                            esc(&ty_head(tcx, t)),
                            esc(&ty_head(tcx, t)),
                            esc(&ty_head(tcx, t))
                        ),
                        _ => "null".into(),
                    }
                }
                (Some(d), _) => match tcx.impl_of_assoc(d) {
                    Some(i) => {
                        let t = tcx.type_of(i).instantiate_identity().skip_normalization();
                        format!(
                            "{{\"unadj\":{},\"adj\":{},\"unadj_head\":{},\"adj_head\":{},\"chain\":[{}],\"self\":true}}",
                            esc(&ty_str(t)),
                            esc(&ty_str(t)),
                            esc(&ty_head(tcx, t)),
                            esc(&ty_head(tcx, t)),
                            esc(&ty_head(tcx, t))
                        )
                    }
                    None => "null".into(),
                },
                _ => "null".into(),
            },
        };
        // A caller that is itself MACRO-GENERATED (a derive's `clone`, an attribute's wrapper) is not written code even
        // where a call inside it carries a written token's span.
        let caller_exp = crate::enclosing_named_fn(tcx, expr.hir_id).is_some_and(|c| tcx.def_span(c.to_def_id()).from_expansion());
        let caller = crate::enclosing_named_fn(tcx, expr.hir_id)
            .map(|c| esc(&tcx.def_path(c.to_def_id()).to_string_no_crate_verbose()))
            .unwrap_or_else(|| "null".into());
        let member = match named {
            Some(d) => tcx.opt_item_name(d).map(|n| n.to_string()).unwrap_or_default(),
            None => String::new(),
        };
        self.cur = Some((expr.hir_id, site.clone()));
        let recv_origin = match recv {
            Some(r) => {
                let mut v = Vec::new();
                self.origin(cx, r, 0, &mut v);
                format!("[{}]", v.join(","))
            }
            None => "null".into(),
        };
        self.lines.push(format!(
            "{{\"kind\":\"site\",\"via_into\":{via_into},\"recv_origin\":{recv_origin},\"form\":{},\"member\":{},{site},\"named\":{named_s},\"resolved\":{resolved},\"mono\":{mono},\"recv\":{recv_s},\"written_self\":{},\"caller\":{caller},\"caller_exp\":{caller_exp}}}",
            esc(&form),
            esc(&member),
            written_self.as_deref().map(esc).unwrap_or_else(|| "null".into())
        ));
    }

    /// The `key` fragment `site` would give a span (outermost macro invocation, else the span itself).
    fn key_of(&mut self, cx: &LateContext<'_>, sp: Span) -> String {
        let mut s = sp;
        let mut in_macro = false;
        let mut guard = 0;
        while s.from_expansion() && guard < 64 {
            guard += 1;
            let d = s.ctxt().outer_expn_data();
            match d.kind {
                ExpnKind::Macro(..) => in_macro = true,
                ExpnKind::Root => break,
                _ => {}
            }
            s = d.call_site;
        }
        if in_macro { self.pos(cx, s) } else { self.pos(cx, sp) }
    }

    /// CENSUS — where a method receiver's VALUE came from, in rustc's own HIR: a chain of steps from the receiver to the
    /// call, binding or parameter that produced it. The comparator joins each call step to the scanner's verdict at that
    /// site, which is what splits an untyped receiver into "a name upstream did not resolve" and "the typer could not
    /// follow the value". Bounded; never consulted by the lint.
    fn origin<'tcx>(&mut self, cx: &LateContext<'tcx>, e: &'tcx Expr<'tcx>, depth: usize, out: &mut Vec<String>) {
        if depth > 6 || out.len() > 8 {
            return;
        }
        let tcx = cx.tcx;
        match e.kind {
            ExprKind::AddrOf(_, _, x) | ExprKind::DropTemps(x) | ExprKind::Cast(x, _) | ExprKind::Type(x, _) => {
                self.origin(cx, x, depth, out)
            }
            ExprKind::Unary(_, x) => {
                out.push("{\"k\":\"unary\"}".into());
                self.origin(cx, x, depth + 1, out)
            }
            ExprKind::Field(b, f) => {
                out.push(format!("{{\"k\":\"field\",\"name\":{}}}", esc(f.as_str())));
                self.origin(cx, b, depth + 1, out)
            }
            ExprKind::Index(b, _, _) => {
                out.push("{\"k\":\"index\"}".into());
                self.origin(cx, b, depth + 1, out)
            }
            ExprKind::MethodCall(seg, r, _, _) => {
                let key = self.key_of(cx, seg.ident.span);
                out.push(format!("{{\"k\":\"mcall\",\"member\":{},\"key\":{key}}}", esc(seg.ident.as_str())));
                // A method's result is typed from its receiver: the receiver's own origin is the next step.
                self.origin(cx, r, depth + 1, out)
            }
            // `x?` lowers to `match Try::branch(x) { .. }`: the value's producer is `x`, not the desugared `branch` call.
            ExprKind::Call(callee, args) if callee.span.desugaring_kind().is_some() && !args.is_empty() => {
                out.push("{\"k\":\"try\"}".into());
                self.origin(cx, &args[0], depth + 1, out)
            }
            ExprKind::Call(callee, _) => {
                let sp = match callee.kind {
                    ExprKind::Path(QPath::Resolved(_, p)) => p.segments.last().map(|s| s.ident.span).unwrap_or(callee.span),
                    ExprKind::Path(QPath::TypeRelative(_, seg)) => seg.ident.span,
                    _ => callee.span,
                };
                let key = self.key_of(cx, sp);
                out.push(format!("{{\"k\":\"call\",\"key\":{key}}}"));
            }
            ExprKind::Match(scrut, _, src) => {
                out.push(format!("{{\"k\":\"match\",\"src\":{}}}", esc(&format!("{src:?}"))));
                self.origin(cx, scrut, depth + 1, out)
            }
            ExprKind::Path(QPath::Resolved(None, p)) => match p.res {
                Res::Local(hid) => self.binding_origin(cx, hid, depth, out),
                Res::Def(dk, _) => out.push(format!("{{\"k\":\"item\",\"dk\":{}}}", esc(&format!("{dk:?}")))),
                ref r => out.push(format!("{{\"k\":\"res\",\"res\":{}}}", esc(&format!("{r:?}")))),
            },
            _ => {
                let k = format!("{:?}", e.kind);
                let k = k.split(['(', ' ', '{']).next().unwrap_or("?").to_string();
                let in_mac = e.span.from_expansion();
                out.push(format!("{{\"k\":\"expr\",\"expr\":{},\"exp\":{in_mac}}}", esc(&k)));
            }
        }
        let _ = tcx;
    }

    fn binding_origin<'tcx>(&mut self, cx: &LateContext<'tcx>, hid: HirId, depth: usize, out: &mut Vec<String>) {
        let tcx = cx.tcx;
        // Climb out of the binding pattern to the node that introduces it.
        let mut cur = hid;
        let mut nested = false;
        for _ in 0..16 {
            match tcx.parent_hir_node(cur) {
                Node::Pat(p) => {
                    nested = true;
                    cur = p.hir_id;
                }
                Node::PatField(f) => {
                    nested = true;
                    cur = f.hir_id;
                }
                Node::Param(param) => {
                    let owner = tcx.hir_enclosing_body_owner(param.hir_id);
                    let closure = matches!(tcx.def_kind(owner.to_def_id()), DefKind::Closure);
                    let annotated = param.ty_span != param.pat.span;
                    let simple = matches!(param.pat.kind, PatKind::Binding(..)) && !nested;
                    out.push(format!(
                        "{{\"k\":\"param\",\"closure\":{closure},\"annotated\":{annotated},\"destructured\":{}}}",
                        !simple
                    ));
                    return;
                }
                Node::LetStmt(l) => {
                    out.push(format!(
                        "{{\"k\":\"let\",\"annotated\":{},\"destructured\":{nested},\"init\":{}}}",
                        l.ty.is_some(),
                        l.init.is_some()
                    ));
                    if let Some(i) = l.init {
                        self.origin(cx, i, depth + 1, out);
                    }
                    return;
                }
                Node::Arm(_) => {
                    out.push("{\"k\":\"arm-binding\"}".into());
                    if let Node::Expr(m) = tcx.parent_hir_node(tcx.parent_hir_id(cur))
                        && let ExprKind::Match(scrut, ..) = m.kind
                    {
                        self.origin(cx, scrut, depth + 1, out);
                    }
                    return;
                }
                Node::Expr(e) => {
                    if let ExprKind::Let(le) = e.kind {
                        out.push("{\"k\":\"if-let-binding\"}".into());
                        self.origin(cx, le.init, depth + 1, out);
                    } else {
                        out.push("{\"k\":\"binding\",\"in\":\"expr\"}".into());
                    }
                    return;
                }
                other => {
                    let k = format!("{other:?}");
                    let k = k.split(['(', ' ', '{']).next().unwrap_or("?").to_string();
                    out.push(format!("{{\"k\":\"binding\",\"in\":{}}}", esc(&k)));
                    return;
                }
            }
        }
    }

    fn vpath(cx: &LateContext<'_>, d: LocalDefId) -> String {
        esc(&cx.tcx.def_path(d.to_def_id()).to_string_no_crate_verbose())
    }

    /// CENSUS — the direct effects and local edges the lint added while analysing ONE expression (see `check_expr`).
    pub(crate) fn note_expr<'tcx>(
        &mut self,
        cx: &LateContext<'tcx>,
        expr: &'tcx Expr<'tcx>,
        caller: LocalDefId,
        direct: Option<&BTreeSet<&'static str>>,
        calls: Option<&HashSet<LocalDefId>>,
        cross: Option<&BTreeSet<&'static str>>,
    ) {
        let empty = |n: usize| n == 0;
        if empty(direct.map_or(0, |d| d.len())) && empty(calls.map_or(0, |c| c.len())) && empty(cross.map_or(0, |c| c.len())) {
            return;
        }
        let (site, nosite) = match &self.cur {
            Some((h, s)) if *h == expr.hir_id => (s.clone(), false),
            _ => {
                // A path in CALLEE position (`f(..)`'s `f`) is its call's site: `record` keys the call by the path's last
                // segment, so the same key joins the edge the lint forms while visiting the path to that call.
                let callee_of_call = matches!(expr.kind, ExprKind::Path(_))
                    && matches!(cx.tcx.parent_hir_node(expr.hir_id), Node::Expr(p) if matches!(p.kind, ExprKind::Call(c, _) if c.hir_id == expr.hir_id));
                let sp = match expr.kind {
                    ExprKind::Path(QPath::Resolved(_, p)) if callee_of_call => p.segments.last().map(|s| s.ident.span).unwrap_or(expr.span),
                    ExprKind::Path(QPath::TypeRelative(_, seg)) if callee_of_call => seg.ident.span,
                    _ => expr.span,
                };
                let k = self.key_of(cx, sp);
                (format!("\"key\":{k}"), !callee_of_call)
            }
        };
        let eff = direct.map(|d| d.iter().map(|e| esc(e)).collect::<Vec<_>>().join(",")).unwrap_or_default();
        let crs = cross.map(|d| d.iter().map(|e| esc(e)).collect::<Vec<_>>().join(",")).unwrap_or_default();
        let mut edges: Vec<String> = calls.map(|c| c.iter().map(|d| Self::vpath(cx, *d)).collect()).unwrap_or_default();
        edges.sort();
        let ek = format!("{:?}", expr.kind);
        let ek = ek.split(['(', ' ', '{']).next().unwrap_or("?").to_string();
        self.census.push(format!(
            "{{\"kind\":\"contrib\",{site},\"nosite\":{nosite},\"expr\":{},\"caller\":{},\"eff\":[{eff}],\"cross\":[{crs}],\"edges\":[{}]}}",
            esc(&ek),
            Self::vpath(cx, caller),
            edges.join(",")
        ));
    }

    /// CENSUS — edges the lint adds after the walk (callback flow, implicit drops), which no single expression owns.
    pub(crate) fn note_post_edges(
        &mut self,
        cx: &LateContext<'_>,
        kind: &str,
        before: &HashMap<LocalDefId, HashSet<LocalDefId>>,
        after: &HashMap<LocalDefId, HashSet<LocalDefId>>,
    ) {
        let none = HashSet::new();
        for (f, ts) in after {
            let b = before.get(f).unwrap_or(&none);
            for t in ts.difference(b) {
                self.census.push(format!(
                    "{{\"kind\":\"postedge\",\"ekind\":{},\"caller\":{},\"target\":{}}}",
                    esc(kind),
                    Self::vpath(cx, *f),
                    Self::vpath(cx, *t)
                ));
            }
        }
    }

    /// CENSUS — every unit the lint analysed, with its final (transitive) effect set and its own direct set.
    pub(crate) fn units(
        &mut self,
        cx: &LateContext<'_>,
        eff: &HashMap<LocalDefId, BTreeSet<&'static str>>,
        direct: &HashMap<LocalDefId, BTreeSet<&'static str>>,
    ) {
        let tcx = cx.tcx;
        for (f, es) in eff {
            let did = f.to_def_id();
            let ident = tcx.def_ident_span(did).map(|s| self.pos(cx, s)).unwrap_or_else(|| "null".into());
            let span = self.pos(cx, tcx.def_span(did));
            let path = rustc_middle::ty::print::with_no_trimmed_paths!(tcx.def_path_str(did));
            let d = direct.get(f).map(|d| d.iter().map(|e| esc(e)).collect::<Vec<_>>().join(",")).unwrap_or_default();
            self.census.push(format!(
                "{{\"kind\":\"unit\",\"def\":{},\"path\":{},\"dk\":{},\"ident\":{ident},\"span\":{span},\"exp\":{},\"eff\":[{}],\"direct\":[{d}]}}",
                Self::vpath(cx, *f),
                esc(&path),
                esc(&format!("{:?}", tcx.def_kind(did))),
                tcx.def_span(did).from_expansion(),
                es.iter().map(|e| esc(e)).collect::<Vec<_>>().join(",")
            ));
        }
    }

    pub(crate) fn flush(&mut self, cx: &LateContext<'_>) {
        let tcx = cx.tcx;
        let krate = tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).to_string();
        let types = tcx.crate_types().iter().map(|t| format!("{t:?}")).collect::<Vec<_>>().join("-");
        let mut feats: BTreeSet<String> = BTreeSet::new();
        let mut cfgs: BTreeSet<String> = BTreeSet::new();
        for (name, val) in tcx.sess.config.iter() {
            if name.as_str() == "feature" {
                if let Some(v) = val {
                    feats.insert(v.to_string());
                }
            } else {
                cfgs.insert(match val {
                    Some(v) => format!("{name}={v}"),
                    None => name.to_string(),
                });
            }
        }
        let triple = tcx.sess.opts.target_triple.tuple().to_string();
        let feats = feats.iter().map(|f| esc(f)).collect::<Vec<_>>().join(",");
        let cfgs = cfgs.iter().map(|f| esc(f)).collect::<Vec<_>>().join(",");
        let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
        let header = format!(
            "{{\"kind\":\"crate\",\"crate\":{},\"types\":{},\"target\":{},\"test\":{},\"features\":[{feats}],\"cfg\":[{cfgs}],\"cwd\":{},\"sites\":{}}}",
            esc(&krate),
            esc(&types),
            esc(&triple),
            tcx.sess.is_test_crate(),
            esc(&cwd),
            self.lines.len()
        );
        let _ = std::fs::create_dir_all(&self.dir);
        let file = format!("{}/{krate}.{types}.{}.jsonl", self.dir, std::process::id());
        let mut body = header;
        body.push('\n');
        for l in &self.lines {
            body.push_str(l);
            body.push('\n');
        }
        let _ = std::fs::write(file, body);
        self.lines.clear();
        if !self.census.is_empty() {
            let mut c = self.census.join("\n");
            c.push('\n');
            let _ = std::fs::write(format!("{}/{krate}.{types}.{}.census", self.dir, std::process::id()), c);
            self.census.clear();
        }
    }
}
