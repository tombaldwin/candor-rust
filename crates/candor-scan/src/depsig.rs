//! SOUNDNESS R1165 — which std traits a CHAINED dependency's callee can run on the value a consumer hands it, read
//! from the dependency's own SOURCE SIGNATURE where the consumer can see it (a `path` dependency, or the registry
//! checkout Cargo.lock names).
//!
//! The consumer-side disclosure in `scan.rs` (`witness_sites`) fires whenever a local value with an effectful std-trait
//! impl is handed to a chained dependency, because the dependency's report cannot say whether its body runs that impl.
//! This module only ever NARROWS that disclosure, so every answer it is unsure of is `None` ("any trait may run"),
//! and the narrowing is a DENYLIST of proven-inert shapes, never an allowlist:
//!
//!   * the callee's candidates are found by its leaf: a free fn's are the source's free fns of that name; a method's
//!     (`Type::m`) are `Type`'s (under every alias the source gives it) inherent and trait-impl methods, every blanket
//!     impl's and every trait declaration's — and the type must have an INHERENT `m` the source shows (or the path names
//!     the trait), else the method that runs may be a foreign trait's; the answer is the UNION of what each allows;
//!   * a candidate's parameter at the argument's position allows the std traits its generic bounds (inline, `where`,
//!     `impl Trait`, `dyn Trait`, transitively through bound arguments) imply — a bound this module does not know
//!     (a dependency's own trait, `serde::Serialize`, anything with a supertrait it cannot see) is `None`;
//!   * a parameter of a CONCRETE type (no generic, no `impl`/`dyn`) allows nothing: the consumer's type cannot be
//!     passed there;
//!   * a parameter whose binding and generic names the body never mentions (identifier or format-string capture)
//!     allows nothing;
//!   * with no fn candidate at all, a tuple struct / variant of that name is a constructor and allows nothing; and a
//!     `proc-macro` crate exports no runtime item, so nothing it is credited with runs;
//!   * SOUNDNESS R1192 — opacity is judged per ITEM, not per crate: the callee is unreadable only where a definition of IT
//!     may be out of sight — a `pub use` of its leaf or type (or an external glob, for a free fn or a type the crate does
//!     not declare) from outside the crate (a root that is a dependency, std, or nothing the crate declares), a
//!     `macro_rules!` that can name it, a macro invocation naming it or its type, a file that does not parse and names
//!     it. A pass-through `macro_rules!` (`cfg_rt!`) and `cfg_if!` are read as the items they re-emit.
//!   * SOUNDNESS R1200 — the candidates are found by LEAF across the whole crate, not by the PATH the consumer wrote, so the
//!     union is sound only if every definition the path can reach is in sight. Four ways it was not, each now unreadable:
//!     a module-level FOREIGN item macro (any name; `macro_is_known`), an external glob whose OWN module does not declare
//!     the type (a declaration shadows a glob only there), a MODULE segment of the path brought in from outside (`pub use
//!     other::inner`, `pub extern crate other as inner`), and a METHOD-CALL site, where method probing may run a trait
//!     method out of sight over the inherent anchor. Chosen over resolving the path through the module tree because the
//!     coarse rules cost 59 newly-`Unknown` rows over the 981 chained consumers (rustagent-v070).
use crate::*;

/// The std-witness traits (`model::STD_WITNESS_TRAITS` leaves) a body bounded by `bound` can reach on the value.
/// `Some(empty)` for a MARKER bound that runs nothing; `None` for a bound this table does not know.
fn implied(bound: &str) -> Option<&'static [&'static str]> {
    Some(match bound {
        "Sized" | "Send" | "Sync" | "Unpin" | "Any" | "Fn" | "FnMut" | "FnOnce" | "UnwindSafe" | "RefUnwindSafe" => &[],
        "PartialEq" | "Eq" => &["PartialEq"],
        "PartialOrd" => &["PartialOrd", "PartialEq"],
        "Ord" => &["Ord", "PartialOrd", "PartialEq"],
        "Hash" => &["Hash"],
        "Hasher" => &["Hasher"],
        "Display" | "ToString" => &["Display"],
        "Debug" => &["Debug"],
        "LowerHex" => &["LowerHex"],
        "UpperHex" => &["UpperHex"],
        "Octal" => &["Octal"],
        "Binary" => &["Binary"],
        "LowerExp" => &["LowerExp"],
        "UpperExp" => &["UpperExp"],
        "Pointer" => &["Pointer"],
        "Clone" | "Copy" => &["Clone"],
        "Default" => &["Default"],
        "Iterator" => &["Iterator"],
        "DoubleEndedIterator" => &["Iterator", "DoubleEndedIterator"],
        "ExactSizeIterator" => &["Iterator", "ExactSizeIterator"],
        "IntoIterator" => &["IntoIterator", "Iterator", "DoubleEndedIterator", "ExactSizeIterator"],
        "Extend" => &["Extend"],
        "FromIterator" => &["FromIterator"],
        "Read" => &["Read"],
        "Write" => &["Write"],
        "BufRead" => &["BufRead", "Read"],
        "Seek" => &["Seek"],
        "AsRef" => &["AsRef"],
        "AsMut" => &["AsMut"],
        "Borrow" => &["Borrow"],
        "BorrowMut" => &["BorrowMut", "Borrow"],
        "Deref" => &["Deref"],
        "DerefMut" => &["DerefMut", "Deref"],
        "Into" => &["Into"],
        "TryInto" => &["TryInto"],
        "FromStr" => &["FromStr"],
        "Future" => &["Future"],
        "IntoFuture" => &["IntoFuture", "Future"], // R1181
        "ToSocketAddrs" => &["ToSocketAddrs"],
        "AsFd" => &["AsFd"],
        "AsRawFd" => &["AsRawFd"],
        "IntoRawFd" => &["IntoRawFd"],
        "AsHandle" => &["AsHandle"],
        "AsRawHandle" => &["AsRawHandle"],
        "IntoRawHandle" => &["IntoRawHandle"],
        "AsSocket" => &["AsSocket"],
        "AsRawSocket" => &["AsRawSocket"],
        "IntoRawSocket" => &["IntoRawSocket"],
        "Wake" => &["Wake"],
        "Error" => &["Error", "Display", "Debug"],
        "Index" => &["Index"],
        "IndexMut" => &["IndexMut", "Index"],
        "Add" => &["Add"],
        "Sub" => &["Sub"],
        "Mul" => &["Mul"],
        "Div" => &["Div"],
        "Rem" => &["Rem"],
        "Neg" => &["Neg"],
        "Not" => &["Not"],
        "BitAnd" => &["BitAnd"],
        "BitOr" => &["BitOr"],
        "BitXor" => &["BitXor"],
        "Shl" => &["Shl"],
        "Shr" => &["Shr"],
        "AddAssign" => &["AddAssign"],
        "SubAssign" => &["SubAssign"],
        "MulAssign" => &["MulAssign"],
        "DivAssign" => &["DivAssign"],
        "RemAssign" => &["RemAssign"],
        "BitAndAssign" => &["BitAndAssign"], // R1181
        "BitOrAssign" => &["BitOrAssign"],
        "BitXorAssign" => &["BitXorAssign"],
        "ShlAssign" => &["ShlAssign"],
        "ShrAssign" => &["ShrAssign"],
        _ => return None,
    })
}

/// Every identifier token in a token stream, and every identifier-shaped word inside its literals (a format string's
/// inline capture, `"{t}"`). Used only to ask "is this name mentioned at all", so over-collection is the safe side.
fn token_words(ts: &proc_macro2::TokenStream, out: &mut HashSet<String>) {
    for tt in ts.clone() {
        match tt {
            proc_macro2::TokenTree::Ident(i) => {
                out.insert(i.to_string());
            }
            proc_macro2::TokenTree::Group(g) => token_words(&g.stream(), out),
            proc_macro2::TokenTree::Literal(l) => lit_words(&l.to_string(), out),
            proc_macro2::TokenTree::Punct(_) => {}
        }
    }
}

fn lit_words(s: &str, out: &mut HashSet<String>) {
    for w in s.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if !w.is_empty() {
            out.insert(w.to_string());
        }
    }
}

/// The names a body mentions — identifiers anywhere (macro arguments included) and words inside string literals.
#[derive(Default)]
struct Words(HashSet<String>);

impl<'ast> Visit<'ast> for Words {
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        self.0.insert(i.to_string());
    }
    fn visit_lit_str(&mut self, l: &'ast syn::LitStr) {
        lit_words(&l.value(), &mut self.0);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        syn::visit::visit_macro(self, m);
        token_words(&m.tokens, &mut self.0);
    }
}

/// The identifier-ish names a type mentions (path segments, generic arguments, `impl`/`dyn` bound paths).
#[derive(Default)]
struct TyNames(HashSet<String>);

impl<'ast> Visit<'ast> for TyNames {
    fn visit_ident(&mut self, i: &'ast proc_macro2::Ident) {
        self.0.insert(i.to_string());
    }
}

/// The trait leaves of a bound list; `?Sized` and lifetimes are dropped (they run nothing).
fn bound_leaves<'a>(bounds: impl Iterator<Item = &'a syn::TypeParamBound>) -> Vec<String> {
    bounds
        .filter_map(|b| match b {
            syn::TypeParamBound::Trait(t) if matches!(t.modifier, syn::TraitBoundModifier::None) => {
                t.path.segments.last().map(|s| s.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

/// Where a candidate sits: what the callee's path must name for this definition to be the one that runs.
#[derive(Clone, PartialEq, Eq)]
enum Site {
    /// a free fn (any module)
    Free,
    /// an inherent `impl X` method; `X`'s outer leaf (`None` when the self type is not a plain path or is a generic)
    Inherent(Option<String>),
    /// a method of `impl Trait for X` (`trait` leaf, `X`'s outer leaf, `None` = a generic or unreadable self type)
    TraitImpl(String, Option<String>),
    /// a method declared (and maybe defaulted) in `trait T`
    TraitDecl(String),
}

/// One fn-like item of the dependency: its generics in scope (impl/trait + own), its inputs, and its body if any.
struct Cand<'a> {
    scope: Vec<&'a syn::Generics>,
    inputs: Vec<&'a syn::FnArg>,
    body: Option<&'a syn::Block>,
    site: Site,
}

/// SOUNDNESS R1192 — what one `macro_rules!` body can define, read from its tokens: the fixed fn names (`fn name`), whether
/// a fn takes its name from a metavariable (`fn $name`, the name then being some invocation's token), and whether a fn's
/// name is COMPUTED (`fn [<get_ $x>]`, paste-style: it can be any name at all). A `fn(..)` pointer type defines nothing.
#[derive(Default, Clone)]
struct FnNames {
    fixed: HashSet<String>,
    metavar: bool,
    computed: bool,
    /// SOUNDNESS R1200 — the macros this body invokes, as `(root, name)` (`root` `None` for a bare `name!`, `Some("$crate")`
    /// for `$crate::name!`): a FOREIGN one among them can define fns this reading never sees
    invokes: Vec<(Option<String>, String)>,
}

/// SOUNDNESS R1200 — std macros that define no fn a path outside them can reach: expression macros, and the item macros
/// whose items their own tokens name (`thread_local!`). A DENYLIST of proven-inert names; every other macro this crate does
/// not define is FOREIGN. `include!` is deliberately absent: it pastes a file (often `OUT_DIR`'s) of any items.
const INERT_STD_MACROS: &[&str] = &[
    "macro_rules", "thread_local", "compile_error", "global_asm", "asm", "naked_asm", "format", "format_args", "write",
    "writeln", "print", "println", "eprint", "eprintln", "panic", "unreachable", "todo", "unimplemented", "assert",
    "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne", "matches", "vec", "dbg", "concat",
    "stringify", "env", "option_env", "include_str", "include_bytes", "line", "column", "file", "module_path", "cfg",
    "ready", "addr_of", "addr_of_mut", "try", "const_format_args", "unreachable_unchecked", "debug_assert_matches",
    "assert_matches", "trace_macros", "log_syntax", "concat_idents", "pin", "offset_of",
];

/// `(root, name)` of a `name!` / `a::b::name!` / `$crate::name!` invocation starting at `toks[i]` (an ident followed by
/// `!`), walking back over `::`-joined path segments.
fn invoked_at(toks: &[proc_macro2::TokenTree], i: usize) -> (Option<String>, String) {
    use proc_macro2::TokenTree as T;
    let name = toks[i].to_string();
    let mut j = i;
    let mut root: Option<String> = None;
    // `seg :: name` — the `::` is two `:` puncts
    while j >= 3 {
        match (&toks[j - 1], &toks[j - 2], &toks[j - 3]) {
            (T::Punct(c1), T::Punct(c2), T::Ident(seg)) if c1.as_char() == ':' && c2.as_char() == ':' => {
                root = Some(seg.to_string());
                j -= 3;
            }
            _ => break,
        }
    }
    if root.as_deref() == Some("crate") && j >= 1 && matches!(&toks[j - 1], T::Punct(d) if d.as_char() == '$') {
        root = Some("$crate".to_string());
    }
    (root, name)
}

fn fn_names(ts: &proc_macro2::TokenStream, out: &mut FnNames) {
    let toks: Vec<proc_macro2::TokenTree> = ts.clone().into_iter().collect();
    for (i, tt) in toks.iter().enumerate() {
        match tt {
            proc_macro2::TokenTree::Ident(id) if id == "fn" => match toks.get(i + 1) {
                Some(proc_macro2::TokenTree::Ident(n)) => {
                    out.fixed.insert(n.to_string().trim_start_matches("r#").to_string());
                }
                Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '$' => out.metavar = true,
                Some(proc_macro2::TokenTree::Group(g)) if g.delimiter() == proc_macro2::Delimiter::Parenthesis => {} // `fn(..)` type
                None => {}
                // `fn [<..>]`, `fn #name` (a quote! template) — a name this reading cannot bound
                _ => out.computed = true,
            },
            proc_macro2::TokenTree::Ident(id)
                if matches!(toks.get(i + 1), Some(proc_macro2::TokenTree::Punct(b)) if b.as_char() == '!')
                    && matches!(toks.get(i + 2), Some(proc_macro2::TokenTree::Group(_))) =>
            {
                // `$m!(..)` takes its macro from an invocation: `invoked_at` names the metavariable, which no definition has
                let _ = id;
                out.invokes.push(invoked_at(&toks, i));
            }
            proc_macro2::TokenTree::Group(g) => fn_names(&g.stream(), out),
            _ => {}
        }
    }
}

/// SOUNDNESS R1192 — a `macro_rules!` every arm of which is `($($i:item)*) => { $( #[..]* $i )* }`: it re-emits the
/// items it is handed, so they are read as written instead of making the dependency unreadable (tokio's `cfg_rt!`).
fn is_passthrough(ts: &proc_macro2::TokenStream) -> bool {
    use proc_macro2::TokenTree as T;
    let toks: Vec<T> = ts.clone().into_iter().collect();
    let flat = |g: &proc_macro2::Group| -> Vec<T> { g.stream().into_iter().collect() };
    // `$ ( $ x : item ) *` → Some(x)
    let rep_item = |v: &[T]| -> Option<String> {
        match v {
            [T::Punct(d), T::Group(g), T::Punct(st)] if d.as_char() == '$' && st.as_char() == '*' && g.delimiter() == proc_macro2::Delimiter::Parenthesis => {
                match &flat(g)[..] {
                    [T::Punct(d2), T::Ident(x), T::Punct(c), T::Ident(k)] if d2.as_char() == '$' && c.as_char() == ':' && k == "item" => Some(x.to_string()),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    let mut i = 0;
    let mut arms = 0;
    while i < toks.len() {
        let (Some(T::Group(m)), Some(T::Punct(eq)), Some(T::Punct(gt)), Some(T::Group(t))) = (toks.get(i), toks.get(i + 1), toks.get(i + 2), toks.get(i + 3)) else {
            return false;
        };
        if eq.as_char() != '=' || gt.as_char() != '>' {
            return false;
        }
        let Some(x) = rep_item(&flat(m)) else { return false };
        // transcriber: `$ ( [# [..]]* $ x ) *`
        let tv = flat(t);
        let ok = match &tv[..] {
            [T::Punct(d), T::Group(g), T::Punct(st)] if d.as_char() == '$' && st.as_char() == '*' && g.delimiter() == proc_macro2::Delimiter::Parenthesis => {
                let inner = flat(g);
                let mut j = 0;
                while j + 1 < inner.len() && matches!((&inner[j], &inner[j + 1]), (T::Punct(h), T::Group(b)) if h.as_char() == '#' && b.delimiter() == proc_macro2::Delimiter::Bracket) {
                    j += 2;
                }
                matches!(&inner[j..], [T::Punct(d2), T::Ident(y)] if d2.as_char() == '$' && *y == x)
            }
            _ => false,
        };
        if !ok {
            return false;
        }
        arms += 1;
        i += 4;
        if matches!(toks.get(i), Some(T::Punct(p)) if p.as_char() == ';') {
            i += 1;
        }
    }
    arms > 0
}

/// A sequence of items, for re-reading a pass-through macro's tokens.
struct Items(Vec<syn::Item>);

impl syn::parse::Parse for Items {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut v = Vec::new();
        while !input.is_empty() {
            v.push(input.parse()?);
        }
        Ok(Items(v))
    }
}

/// SOUNDNESS R1192 — `cfg_if::cfg_if!`'s branches (`if #[cfg(..)] { items } else { items }`): every brace group at the top
/// level, each parsed as items. `None` when any does not parse.
fn cfg_if_items(ts: &proc_macro2::TokenStream) -> Option<Vec<syn::Item>> {
    let mut out = Vec::new();
    for tt in ts.clone() {
        if let proc_macro2::TokenTree::Group(g) = tt {
            if g.delimiter() == proc_macro2::Delimiter::Brace {
                out.extend(syn::parse2::<Items>(g.stream()).ok()?.0);
            }
        }
    }
    Some(out)
}

/// Replace every invocation of a pass-through macro (and `cfg_if!`) by the items it re-emits, to a fixpoint, in this item
/// list and every inline module's.
fn expand_items(items: &mut Vec<syn::Item>, pass: &HashSet<String>, local: &HashSet<String>, depth: usize) {
    let mut i = 0;
    while i < items.len() {
        let rep = match &items[i] {
            syn::Item::Macro(m) if m.ident.is_none() && depth < 16 => {
                let name = m.mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                if pass.contains(&name) {
                    syn::parse2::<Items>(m.mac.tokens.clone()).ok().map(|x| x.0)
                } else if name == "cfg_if" && !local.contains(&name) {
                    cfg_if_items(&m.mac.tokens)
                } else {
                    None
                }
            }
            _ => None,
        };
        match rep {
            Some(mut v) => {
                expand_items(&mut v, pass, local, depth + 1);
                let n = v.len();
                items.splice(i..=i, v);
                i += n;
            }
            None => {
                if let syn::Item::Mod(m) = &mut items[i] {
                    if let Some((_, c)) = &mut m.content {
                        expand_items(c, pass, local, depth);
                    }
                }
                i += 1;
            }
        }
    }
}

/// The outer path leaf of a type (`&'a Foo<T>` → `Foo`); `None` for anything else.
fn ty_leaf(t: &syn::Type) -> Option<String> {
    match t {
        syn::Type::Reference(r) => ty_leaf(&r.elem),
        syn::Type::Paren(p) => ty_leaf(&p.elem),
        syn::Type::Group(g) => ty_leaf(&g.elem),
        syn::Type::Path(p) if p.qself.is_none() => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

/// SOUNDNESS R1192 — the crate-wide facts the per-ITEM rules read, built once per dependency source.
struct SrcIndex {
    files: Vec<syn::File>,
    /// the files `syn` could not parse, as text with their word sets (any of their items may be the callee)
    unparsed: Vec<(String, HashSet<String>)>,
    /// `[lib] proc-macro = true`: the crate exports macros only, so no runtime call can land in it
    proc_macro: bool,
    /// the names a `use` path can start with to leave the crate: std's roots, the manifest's dependencies, `extern crate`s
    extern_roots: HashSet<String>,
    /// every name the crate declares (a module, a type, a trait, a fn, a const, a `use` binding)
    local_names: HashSet<String>,
    /// local `macro_rules!` names, and what their bodies can define (union over same-name definitions)
    macros: HashMap<String, FnNames>,
    /// every identifier in any macro invocation's tokens or `macro_rules!` body (a `fn $name` takes its name from these)
    macro_words: HashSet<String>,
    /// the crate's type-like names (struct, enum, union, trait, type alias)
    types: HashSet<String>,
    /// type-alias edges, both directions (`type A = B<..>`, `use ..::B as A`)
    aliases: HashMap<String, HashSet<String>>,
    /// SOUNDNESS R1200 — local `macro_rules!` names whose body invokes a FOREIGN macro (directly, or through another
    /// such local macro): an invocation of one may define any fn
    tainted: HashSet<String>,
}

/// SOUNDNESS R1200 — is a `root::name!` invocation one this reading can account for? A local `macro_rules!` (judged at its
/// definition) that invokes no foreign macro, or an inert std macro. Everything else is FOREIGN: its items are not here.
fn macro_is_known(root: Option<&str>, name: &str, local: &HashMap<String, FnNames>, tainted: &HashSet<String>) -> bool {
    match root {
        None if local.contains_key(name) => !tainted.contains(name),
        None => INERT_STD_MACROS.contains(&name),
        Some("crate" | "self" | "super" | "$crate") => local.contains_key(name) && !tainted.contains(name),
        Some("std" | "core" | "alloc") => INERT_STD_MACROS.contains(&name),
        Some(_) => false,
    }
}

impl SrcIndex {
    /// R1351 — does this source NAME `leaf` at all (a declaration, a `use` binding, a macro token, an unparsed file)?
    fn names_leaf(&self, leaf: &str) -> bool {
        self.local_names.contains(leaf) || self.macro_words.contains(leaf) || self.unparsed.iter().any(|(_, w)| w.contains(leaf))
    }

    /// SOUNDNESS R1351 — `DepSigs::trait_declares` for one source. See there for what is a proof and what is a doubt.
    fn trait_answer(&self, leaf: &str, member: &str, depth: usize) -> Option<bool> {
        if depth > 8 || self.unparsed.iter().any(|(_, w)| w.contains(leaf)) {
            return None;
        }
        // `trait <leaf>` or `trait $x` anywhere in a macro's tokens: the macro may declare (part of) this trait.
        fn macro_trait(ts: &proc_macro2::TokenStream, leaf: &str) -> bool {
            let toks: Vec<proc_macro2::TokenTree> = ts.clone().into_iter().collect();
            for (i, t) in toks.iter().enumerate() {
                match t {
                    proc_macro2::TokenTree::Ident(id) if id == "trait" => match toks.get(i + 1) {
                        Some(proc_macro2::TokenTree::Ident(n)) if n == leaf => return true,
                        Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == '$' => return true,
                        _ => {}
                    },
                    proc_macro2::TokenTree::Group(g) if macro_trait(&g.stream(), leaf) => return true,
                    _ => {}
                }
            }
            false
        }
        #[derive(Default)]
        struct Traits<'a>(Vec<&'a syn::ItemTrait>, String, bool);
        impl<'a> Visit<'a> for Traits<'a> {
            fn visit_item_trait(&mut self, t: &'a syn::ItemTrait) {
                if t.ident == self.1.as_str() {
                    self.0.push(t);
                }
                syn::visit::visit_item_trait(self, t);
            }
            fn visit_macro(&mut self, m: &'a syn::Macro) {
                if macro_trait(&m.tokens, &self.1) {
                    self.2 = true;
                }
            }
        }
        let mut v = Traits(Vec::new(), leaf.to_string(), false);
        for f in &self.files {
            v.visit_file(f);
        }
        if v.2 {
            return None; // a macro may declare this trait, or more of it
        }
        if v.0.is_empty() {
            return None; // re-exported from elsewhere, or made by a macro: not read here
        }
        let mut answer = Some(false);
        for t in &v.0 {
            for it in &t.items {
                match it {
                    syn::TraitItem::Fn(f) if f.sig.ident == member => return Some(true),
                    syn::TraitItem::Fn(_) | syn::TraitItem::Const(_) | syn::TraitItem::Type(_) => {}
                    _ => return None, // a macro item (or an unparsed one) may declare anything
                }
            }
            // supertraits: the inline list and `where Self: ..`
            let mut bounds: Vec<&syn::TypeParamBound> = t.supertraits.iter().collect();
            if let Some(w) = &t.generics.where_clause {
                for p in &w.predicates {
                    if let syn::WherePredicate::Type(pt) = p {
                        if matches!(&pt.bounded_ty, syn::Type::Path(tp) if tp.qself.is_none() && tp.path.is_ident("Self")) {
                            bounds.extend(pt.bounds.iter());
                        }
                    }
                }
            }
            for b in bounds {
                let syn::TypeParamBound::Trait(tb) = b else { continue };
                let segs: Vec<String> = tb.path.segments.iter().map(|s| s.ident.to_string()).collect();
                let Some(sl) = segs.last() else { continue };
                let root = if segs.len() > 1 { segs[0].as_str() } else { "" };
                let sub = match root {
                    "std" | "core" | "alloc" => std_trait_has(sl, member, 0),
                    "crate" | "self" | "super" => self.trait_answer(sl, member, depth + 1),
                    "" if self.types.contains(sl.as_str()) => self.trait_answer(sl, member, depth + 1),
                    "" => std_trait_has(sl, member, 0),
                    _ => None, // another package's trait: not read here
                };
                match sub {
                    Some(true) => return Some(true),
                    Some(false) => {}
                    None => answer = None,
                }
            }
        }
        answer
    }

    fn build(src: &Path) -> Option<SrcIndex> {
        let mut files = Vec::new();
        let mut unparsed = Vec::new();
        let walk = walkdir::WalkDir::new(src).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy();
            e.depth() == 0 || !(n.starts_with('.') || n == "target" || n == "tests" || n == "benches" || n == "examples")
        });
        for e in walk.flatten() {
            if e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "rs") {
                let Ok(text) = std::fs::read_to_string(e.path()) else { return None };
                match syn::parse_file(&text) {
                    Ok(f) => files.push(f),
                    Err(_) => {
                        let mut w = HashSet::new();
                        lit_words(&text, &mut w);
                        unparsed.push((text, w));
                    }
                }
            }
        }
        if files.is_empty() {
            return None;
        }
        // The manifest: proc-macro-ness and the dependency names a `use` root can leave the crate through.
        let mut extern_roots: HashSet<String> = ["std", "core", "alloc", "proc_macro", "test"].iter().map(|s| s.to_string()).collect();
        let doc = std::fs::read_to_string(src.join("Cargo.toml")).ok().and_then(|t| t.parse::<toml::Table>().ok())?;
        let proc_macro = doc.get("lib").and_then(|l| l.get("proc-macro").or_else(|| l.get("proc_macro"))).and_then(|v| v.as_bool()) == Some(true);
        let mut tables: Vec<&toml::Table> = Vec::new();
        for k in ["dependencies", "dev-dependencies", "build-dependencies", "dev_dependencies", "build_dependencies"] {
            tables.extend(doc.get(k).and_then(toml::Value::as_table));
        }
        if let Some(t) = doc.get("target").and_then(toml::Value::as_table) {
            for e in t.values().filter_map(toml::Value::as_table) {
                for k in ["dependencies", "dev-dependencies", "build-dependencies", "dev_dependencies", "build_dependencies"] {
                    tables.extend(e.get(k).and_then(toml::Value::as_table));
                }
            }
        }
        for t in tables {
            for (k, v) in t {
                extern_roots.insert(k.replace('-', "_"));
                if let Some(p) = v.as_table().and_then(|t| t.get("package")).and_then(|p| p.as_str()) {
                    extern_roots.insert(p.replace('-', "_"));
                }
            }
        }
        // Pass 1: local macro definitions.
        #[derive(Default)]
        struct Defs(HashMap<String, FnNames>, HashSet<String>, HashSet<String>);
        impl<'ast> Visit<'ast> for Defs {
            fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
                if let Some(id) = &m.ident {
                    let n = id.to_string();
                    let mut f = FnNames::default();
                    fn_names(&m.mac.tokens, &mut f);
                    let e = self.0.entry(n.clone()).or_default();
                    e.fixed.extend(f.fixed);
                    e.metavar |= f.metavar;
                    e.computed |= f.computed;
                    e.invokes.extend(f.invokes);
                    if is_passthrough(&m.mac.tokens) {
                        self.1.insert(n);
                    } else {
                        self.2.insert(n);
                    }
                }
                syn::visit::visit_item_macro(self, m);
            }
        }
        let mut defs = Defs::default();
        for f in &files {
            defs.visit_file(f);
        }
        // SOUNDNESS R1200 — a local macro invoking a foreign one (or a tainted local one) is tainted, to a fixpoint
        let mut tainted: HashSet<String> = HashSet::new();
        loop {
            let before = tainted.len();
            for (n, f) in &defs.0 {
                if !tainted.contains(n) && f.invokes.iter().any(|(r, m)| !macro_is_known(r.as_deref(), m, &defs.0, &tainted)) {
                    tainted.insert(n.clone());
                }
            }
            if tainted.len() == before {
                break;
            }
        }
        // a name with any NON-pass-through definition is not expanded
        let pass: HashSet<String> = defs.1.difference(&defs.2).cloned().collect();
        let local: HashSet<String> = defs.0.keys().cloned().collect();
        for f in &mut files {
            expand_items(&mut f.items, &pass, &local, 0);
        }
        // Pass 2: names, aliases, extern crates, macro words.
        #[derive(Default)]
        struct Facts {
            names: HashSet<String>,
            types: HashSet<String>,
            externs: HashSet<String>,
            words: HashSet<String>,
            alias: Vec<(String, String)>,
        }
        impl Facts {
            fn tokens(&mut self, ts: &proc_macro2::TokenStream) {
                token_words(ts, &mut self.words);
            }
            fn ty(&mut self, i: &syn::Ident) {
                self.names.insert(i.to_string());
                self.types.insert(i.to_string());
            }
        }
        fn use_names(t: &syn::UseTree, out: &mut Facts) {
            match t {
                syn::UseTree::Path(p) => use_names(&p.tree, out),
                syn::UseTree::Name(n) => {
                    out.names.insert(n.ident.to_string());
                }
                syn::UseTree::Rename(r) => {
                    out.names.insert(r.rename.to_string());
                    out.alias.push((r.rename.to_string(), r.ident.to_string()));
                }
                syn::UseTree::Glob(_) => {}
                syn::UseTree::Group(g) => g.items.iter().for_each(|i| use_names(i, out)),
            }
        }
        impl<'ast> Visit<'ast> for Facts {
            fn visit_item_mod(&mut self, i: &'ast syn::ItemMod) {
                self.names.insert(i.ident.to_string());
                syn::visit::visit_item_mod(self, i);
            }
            fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
                self.ty(&i.ident);
                syn::visit::visit_item_struct(self, i);
            }
            fn visit_item_enum(&mut self, i: &'ast syn::ItemEnum) {
                self.ty(&i.ident);
                syn::visit::visit_item_enum(self, i);
            }
            fn visit_item_union(&mut self, i: &'ast syn::ItemUnion) {
                self.ty(&i.ident);
                syn::visit::visit_item_union(self, i);
            }
            fn visit_item_trait(&mut self, i: &'ast syn::ItemTrait) {
                self.ty(&i.ident);
                syn::visit::visit_item_trait(self, i);
            }
            fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
                self.ty(&i.ident);
                if let Some(l) = ty_leaf(&i.ty) {
                    self.alias.push((i.ident.to_string(), l));
                }
                syn::visit::visit_item_type(self, i);
            }
            fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
                self.names.insert(i.sig.ident.to_string());
                syn::visit::visit_item_fn(self, i);
            }
            fn visit_item_const(&mut self, i: &'ast syn::ItemConst) {
                self.names.insert(i.ident.to_string());
                syn::visit::visit_item_const(self, i);
            }
            fn visit_item_static(&mut self, i: &'ast syn::ItemStatic) {
                self.names.insert(i.ident.to_string());
                syn::visit::visit_item_static(self, i);
            }
            fn visit_item_extern_crate(&mut self, i: &'ast syn::ItemExternCrate) {
                self.externs.insert(i.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| i.ident.to_string()));
                self.externs.insert(i.ident.to_string());
            }
            fn visit_item_use(&mut self, i: &'ast syn::ItemUse) {
                use_names(&i.tree, self);
            }
            fn visit_item_macro(&mut self, m: &'ast syn::ItemMacro) {
                if m.ident.is_some() {
                    // a `macro_rules!` body: its words may name a `fn $name`; its own fn names are judged per definition
                    token_words(&m.mac.tokens, &mut self.words);
                } else {
                    self.tokens(&m.mac.tokens);
                }
            }
            fn visit_macro(&mut self, m: &'ast syn::Macro) {
                self.tokens(&m.tokens);
                syn::visit::visit_macro(self, m);
            }
        }
        let mut facts = Facts::default();
        for f in &files {
            facts.visit_file(f);
        }
        extern_roots.extend(facts.externs.iter().cloned());
        let mut aliases: HashMap<String, HashSet<String>> = HashMap::new();
        for (a, b) in facts.alias {
            aliases.entry(a.clone()).or_default().insert(b.clone());
            aliases.entry(b).or_default().insert(a);
        }
        Some(SrcIndex {
            files,
            unparsed,
            proc_macro,
            extern_roots,
            local_names: facts.names,
            macros: defs.0,
            macro_words: facts.words,
            types: facts.types,
            aliases,
            tainted,
        })
    }

    /// The names the callee's type segment may be spelled as in this source (its alias component).
    fn type_names(&self, t: &str) -> HashSet<String> {
        let mut out: HashSet<String> = HashSet::new();
        let mut work = vec![t.to_string()];
        while let Some(n) = work.pop() {
            if out.insert(n.clone()) {
                work.extend(self.aliases.get(&n).into_iter().flatten().cloned());
            }
        }
        out
    }

    /// Does a `use` path rooted at `root` leave the crate? Only a root this source cannot place inside itself does.
    fn external_root(&self, root: &str) -> bool {
        match root {
            "crate" | "self" | "super" | "Self" => false,
            r => self.extern_roots.contains(r) || !self.local_names.contains(r),
        }
    }
}

/// SOUNDNESS R1192 — the callee's definition as the per-ITEM rules see it: its leaf, and the type segment the consumer's path
/// put in front of it (`LocalSet::spawn_local` → `LocalSet`), when that segment is a type.
struct Want {
    leaf: String,
    /// the callee's type, under every alias the source gives it; empty for a free fn
    tys: HashSet<String>,
    /// SOUNDNESS R1200 — the MODULE segments the path walks before the type / leaf (`inner::f` → `inner`): one brought in
    /// from outside the crate (`pub use other::inner`, `pub extern crate other as inner`) holds definitions not here
    mods: Vec<String>,
}

/// SOUNDNESS R1200 — does this item list (a file, or an inline module's body) itself bind one of `names` in the namespace
/// a glob would fill, with no `#[cfg]` that may remove it? Such a binding shadows every glob of the list, so an external glob
/// there cannot supply the name. `types`: a struct / enum / union / trait / type alias / module; else a fn. An explicit
/// `use` binding of the name counts too (an explicit import shadows a glob; where IT leaves the crate is judged on its own).
fn list_binds(items: &[syn::Item], names: &HashSet<String>, types: bool) -> bool {
    let cfgd = |a: &[syn::Attribute]| a.iter().any(|x| x.path().is_ident("cfg"));
    fn use_binds(t: &syn::UseTree, names: &HashSet<String>) -> bool {
        match t {
            syn::UseTree::Path(p) => use_binds(&p.tree, names),
            syn::UseTree::Name(n) => names.contains(&n.ident.to_string()),
            syn::UseTree::Rename(r) => names.contains(&r.rename.to_string()),
            syn::UseTree::Glob(_) => false,
            syn::UseTree::Group(g) => g.items.iter().any(|i| use_binds(i, names)),
        }
    }
    items.iter().any(|it| match it {
        syn::Item::Struct(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Enum(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Union(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Trait(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Type(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Mod(i) if types => !cfgd(&i.attrs) && names.contains(&i.ident.to_string()),
        syn::Item::Fn(i) if !types => !cfgd(&i.attrs) && names.contains(&i.sig.ident.to_string()),
        syn::Item::Use(u) => !cfgd(&u.attrs) && use_binds(&u.tree, names),
        _ => false,
    })
}

struct Collect<'a, 'i> {
    idx: &'i SrcIndex,
    want: &'i Want,
    stack: Vec<&'a syn::Generics>,
    /// the trait and self type of the `impl` being walked
    imp: Vec<(Option<String>, Option<String>)>,
    fns: Vec<Cand<'a>>,
    ctor: bool,
    /// a definition of the callee may be somewhere this reading cannot see; the reason, for the reach counter
    opaque: Option<String>,
    /// SOUNDNESS R1200 — the item list being walked (a file, or an inline module's body), innermost last
    lists: Vec<&'a [syn::Item]>,
    /// SOUNDNESS R1200 — inside a fn / const / static body, where an item is reachable by no path from outside
    body: usize,
    /// SOUNDNESS R1200 — the first module-level invocation of a FOREIGN macro (`macro_is_known`): it may define a free fn
    /// of any name, its own tokens naming none of them (`mmac::make_free!()`)
    foreign: Option<String>,
}

impl<'a, 'i> Collect<'a, 'i> {
    fn sig(&mut self, sig: &'a syn::Signature, body: Option<&'a syn::Block>, site: Site) {
        if sig.ident == self.want.leaf.as_str() {
            let mut scope = self.stack.clone();
            scope.push(&sig.generics);
            self.fns.push(Cand { scope, inputs: sig.inputs.iter().collect(), body, site });
        }
    }

    fn dark(&mut self, why: impl FnOnce() -> String) {
        if self.opaque.is_none() {
            self.opaque = Some(why());
        }
    }

    /// A macro invocation may make any fn its tokens name, a fn of a COMPUTED name (`fn [<..>]`), and any fn of a type its
    /// tokens name (`bitflags! { struct Flags }` makes `Flags::from_bits`); in an `impl` block it may add any method to
    /// that impl's self type. A local `macro_rules!` is judged again at its definition.
    fn invocation(&mut self, mac: &syn::Macro, impl_self: Option<Option<&str>>) {
        let mut w = HashSet::new();
        token_words(&mac.tokens, &mut w);
        let name = mac.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
        let mut f = FnNames::default();
        fn_names(&mac.tokens, &mut f);
        let tys = &self.want.tys;
        let names_ty = tys.iter().any(|t| w.contains(t));
        let in_ty = match impl_self {
            Some(Some(t)) => tys.contains(t),
            Some(None) => !tys.is_empty(), // an impl of a self type this reading cannot name
            None => false,
        };
        if w.contains(&self.want.leaf) || f.computed || names_ty || in_ty {
            self.dark(|| format!("mac:{name}"));
        }
    }
}

impl<'a> Visit<'a> for Collect<'a, '_> {
    fn visit_file(&mut self, f: &'a syn::File) {
        self.lists.push(&f.items);
        syn::visit::visit_file(self, f);
        self.lists.pop();
    }
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        match &m.content {
            Some((_, items)) => {
                self.lists.push(items);
                syn::visit::visit_item_mod(self, m);
                self.lists.pop();
            }
            None => syn::visit::visit_item_mod(self, m),
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        self.sig(&f.sig, Some(&f.block), Site::Free);
        self.body += 1;
        syn::visit::visit_item_fn(self, f);
        self.body -= 1;
    }
    fn visit_item_const(&mut self, c: &'a syn::ItemConst) {
        self.body += 1;
        syn::visit::visit_item_const(self, c);
        self.body -= 1;
    }
    fn visit_item_static(&mut self, c: &'a syn::ItemStatic) {
        self.body += 1;
        syn::visit::visit_item_static(self, c);
        self.body -= 1;
    }
    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        self.stack.push(&i.generics);
        let generic_self = ty_leaf(&i.self_ty).filter(|l| {
            !i.generics.params.iter().any(|p| matches!(p, syn::GenericParam::Type(t) if t.ident == l.as_str()))
        });
        let tr = i.trait_.as_ref().and_then(|t| t.1.segments.last()).map(|s| s.ident.to_string());
        self.imp.push((tr.clone(), generic_self.clone()));
        for it in &i.items {
            match it {
                syn::ImplItem::Fn(f) => {
                    let site = match &tr {
                        Some(t) => Site::TraitImpl(t.clone(), generic_self.clone()),
                        None => Site::Inherent(generic_self.clone()),
                    };
                    self.sig(&f.sig, Some(&f.block), site);
                }
                syn::ImplItem::Macro(m) => self.invocation(&m.mac, Some(generic_self.as_deref())),
                _ => {}
            }
        }
        syn::visit::visit_item_impl(self, i);
        self.imp.pop();
        self.stack.pop();
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        // the signature was taken by `visit_item_impl` with the impl's generics in scope; walk the body for nested items
        self.body += 1;
        syn::visit::visit_block(self, &f.block);
        self.body -= 1;
    }
    fn visit_impl_item_macro(&mut self, _m: &'a syn::ImplItemMacro) {
        // judged by `visit_item_impl`, which knows the self type
    }
    fn visit_item_trait(&mut self, t: &'a syn::ItemTrait) {
        self.stack.push(&t.generics);
        let tn = t.ident.to_string();
        for it in &t.items {
            match it {
                syn::TraitItem::Fn(f) => self.sig(&f.sig, f.default.as_ref(), Site::TraitDecl(tn.clone())),
                syn::TraitItem::Macro(m) => self.invocation(&m.mac, Some(Some(tn.as_str()))),
                _ => {}
            }
        }
        self.body += 1;
        for it in &t.items {
            if let syn::TraitItem::Fn(f) = it {
                if let Some(b) = &f.default {
                    syn::visit::visit_block(self, b);
                }
            }
        }
        self.body -= 1;
        self.stack.pop();
    }
    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        if s.ident == self.want.leaf.as_str() && matches!(s.fields, syn::Fields::Unnamed(_)) {
            self.ctor = true;
        }
    }
    fn visit_variant(&mut self, v: &'a syn::Variant) {
        if v.ident == self.want.leaf.as_str() {
            self.ctor = true;
        }
    }
    fn visit_item_use(&mut self, u: &'a syn::ItemUse) {
        if !matches!(u.vis, syn::Visibility::Inherited) {
            let idx = self.idx;
            let ext = |r: &str| idx.external_root(r);
            // An external glob cannot supply a method of a type this crate DECLARES (a local item shadows a glob, and the
            // other crate cannot implement for it what the anchor below does not see); a free fn, or a type the crate
            // does not declare, may come through it.
            // SOUNDNESS R1200 — but a declaration shadows a glob only in ITS OWN module: `pub use other::*` at the root beside
            // a `Foo` declared in a private module supplies the root's `Foo`, which is the other crate's. So the exemption
            // holds only where THIS glob's own item list binds the type, uncfg'd (`CANDOR_R1200_GLOB_OFF` restores R1192's
            // crate-wide reading, for falsification only).
            let list: &[syn::Item] = self.lists.last().copied().unwrap_or(&[]);
            let declared = self.want.tys.iter().any(|t| idx.types.contains(t));
            let globs = if std::env::var_os("CANDOR_R1200_GLOB_OFF").is_some() {
                !declared
            } else {
                !(declared && list_binds(list, &self.want.tys, true))
            };
            if use_may_import(&u.tree, &self.want.leaf, None, &ext, globs) {
                self.dark(|| "use:leaf".to_string());
            } else if self.want.tys.iter().any(|t| use_may_import(&u.tree, t, None, &ext, globs)) {
                self.dark(|| "use:type".to_string());
            } else if std::env::var_os("CANDOR_R1200_MOD_OFF").is_none()
                && self.want.mods.iter().any(|m| {
                    let one: HashSet<String> = std::iter::once(m.clone()).collect();
                    use_may_import(&u.tree, m, None, &ext, !list_binds(list, &one, true))
                })
            {
                // SOUNDNESS R1200 — a module the path walks through comes from outside (`pub use other::inner`, or a glob
                // of a list that does not itself declare it)
                self.dark(|| "use:mod".to_string());
            }
        }
    }
    fn visit_item_extern_crate(&mut self, e: &'a syn::ItemExternCrate) {
        // SOUNDNESS R1200 — `pub extern crate other as name;` makes `name::..` the other crate's
        if !matches!(e.vis, syn::Visibility::Inherited) && std::env::var_os("CANDOR_R1200_MOD_OFF").is_none() {
            let n = e.rename.as_ref().map(|r| r.1.to_string()).unwrap_or_else(|| e.ident.to_string());
            if n == self.want.leaf || self.want.tys.contains(&n) || self.want.mods.contains(&n) {
                self.dark(|| "extern:crate".to_string());
            }
        }
    }
    fn visit_item_macro(&mut self, m: &'a syn::ItemMacro) {
        match &m.ident {
            Some(id) => {
                // a `macro_rules!` definition: only a fn it can actually name hides the callee
                if let Some(d) = self.idx.macros.get(&id.to_string()) {
                    let leaf = &self.want.leaf;
                    if d.fixed.contains(leaf) || d.computed || d.metavar && self.idx.macro_words.contains(leaf) {
                        self.dark(|| format!("macdef:{id}"));
                    }
                }
            }
            None => {
                self.invocation(&m.mac, None);
                if self.body == 0 && self.foreign.is_none() {
                    let segs: Vec<String> = m.mac.path.segments.iter().map(|s| s.ident.to_string()).collect();
                    let name = segs.last().cloned().unwrap_or_default();
                    let root = (segs.len() > 1).then(|| segs[0].clone());
                    if !macro_is_known(root.as_deref(), &name, &self.idx.macros, &self.idx.tainted) {
                        self.foreign = Some(segs.join("::"));
                    }
                }
            }
        }
    }
}

/// Does this `use` tree bring `leaf` (or a glob) in from a path rooted outside the crate (`external` judges a root)?
fn use_may_import(t: &syn::UseTree, leaf: &str, root: Option<&str>, external: &dyn Fn(&str) -> bool, globs: bool) -> bool {
    let out = |r: Option<&str>| r.is_none_or(external);
    match t {
        syn::UseTree::Path(p) => {
            let r = root.map(str::to_string).unwrap_or_else(|| p.ident.to_string());
            use_may_import(&p.tree, leaf, Some(&r), external, globs)
        }
        syn::UseTree::Name(n) => n.ident == leaf && out(root),
        syn::UseTree::Rename(r) => r.rename == leaf && out(root.or(Some(&r.ident.to_string()))),
        syn::UseTree::Glob(_) => globs && out(root),
        syn::UseTree::Group(g) => g.items.iter().any(|i| use_may_import(i, leaf, root, external, globs)),
    }
}

/// What one candidate allows at input `pos`: `Some(set)` (possibly empty) or `None` (any). `Err(())` when the
/// candidate has no input at `pos` (it is not the callee of a call with that many arguments).
fn cand_allows(c: &Cand<'_>, pos: usize) -> Result<Option<BTreeSet<String>>, ()> {
    let arg = c.inputs.get(pos).ok_or(())?;
    let syn::FnArg::Typed(pt) = arg else { return Ok(None) }; // the receiver: the consumer's type implements this trait
    let generic_names: HashSet<String> = c
        .scope
        .iter()
        .flat_map(|g| g.params.iter())
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut tn = TyNames::default();
    tn.visit_type(&pt.ty);
    if tn.0.contains("Self") {
        return Ok(None);
    }
    // The bounds reachable from the parameter's type, closed over generics their own arguments name.
    let mut leaves: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut work: Vec<String> = tn.0.iter().filter(|n| generic_names.contains(*n)).cloned().collect();
    // `impl Trait` / `dyn Trait` in the parameter's own type.
    struct Inline<'b>(&'b mut Vec<String>, &'b mut Vec<String>, &'b HashSet<String>);
    impl<'ast> Visit<'ast> for Inline<'_> {
        fn visit_type_impl_trait(&mut self, t: &'ast syn::TypeImplTrait) {
            self.0.extend(bound_leaves(t.bounds.iter()));
            let mut n = TyNames::default();
            for b in &t.bounds {
                n.visit_type_param_bound(b);
            }
            self.1.extend(n.0.into_iter().filter(|x| self.2.contains(x)));
        }
        fn visit_type_trait_object(&mut self, t: &'ast syn::TypeTraitObject) {
            self.0.extend(bound_leaves(t.bounds.iter()));
            let mut n = TyNames::default();
            for b in &t.bounds {
                n.visit_type_param_bound(b);
            }
            self.1.extend(n.0.into_iter().filter(|x| self.2.contains(x)));
        }
    }
    let had_inline = {
        let before = leaves.len();
        Inline(&mut leaves, &mut work, &generic_names).visit_type(&pt.ty);
        leaves.len() != before
    };
    if work.is_empty() && !had_inline {
        return Ok(Some(BTreeSet::new())); // a concrete type: the consumer's value cannot be passed here
    }
    let mentioned_generics: Vec<String> = work.clone();
    while let Some(g) = work.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        for gens in &c.scope {
            for p in &gens.params {
                if let syn::GenericParam::Type(t) = p {
                    if t.ident == g.as_str() {
                        leaves.extend(bound_leaves(t.bounds.iter()));
                        let mut n = TyNames::default();
                        for b in &t.bounds {
                            n.visit_type_param_bound(b);
                        }
                        work.extend(n.0.into_iter().filter(|x| generic_names.contains(x) && !seen.contains(x)));
                    }
                }
            }
            if let Some(w) = &gens.where_clause {
                for pred in &w.predicates {
                    if let syn::WherePredicate::Type(pt2) = pred {
                        let mut bn = TyNames::default();
                        bn.visit_type(&pt2.bounded_ty);
                        if bn.0.contains(&g) {
                            leaves.extend(bound_leaves(pt2.bounds.iter()));
                            let mut n = TyNames::default();
                            for b in &pt2.bounds {
                                n.visit_type_param_bound(b);
                            }
                            n.0.extend(bn.0);
                            work.extend(n.0.into_iter().filter(|x| generic_names.contains(x) && !seen.contains(x)));
                        }
                    }
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    for l in &leaves {
        match implied(l) {
            Some(ts) => out.extend(ts.iter().map(|t| t.to_string())),
            None => return Ok(None),
        }
    }
    // A parameter neither whose binding nor whose generic names the body mentions cannot have a member run on it.
    if !out.is_empty() {
        if let Some(b) = c.body {
            let mut w = Words::default();
            w.visit_block(b);
            let mut binds = TyNames::default();
            binds.visit_pat(&pt.pat);
            let used = binds.0.iter().any(|n| w.0.contains(n)) || mentioned_generics.iter().any(|g| w.0.contains(g));
            if !used {
                return Ok(Some(BTreeSet::new()));
            }
        }
    }
    Ok(Some(out))
}

/// SOUNDNESS R1186 — `(kind, target leaf)` pairs a callee can convert a value into; `None` = any.
pub(crate) type ConvTargets = Option<BTreeSet<(String, String)>>;

/// Per-scan cache of parsed dependency sources, keyed by the source directory.
#[derive(Default)]
pub(crate) struct DepSigs {
    idx: HashMap<std::path::PathBuf, Option<SrcIndex>>,
    dirs: HashMap<String, Vec<std::path::PathBuf>>,
    /// SOUNDNESS R1186 — `conversion_targets` answers, per (alias, callee, position).
    conv: HashMap<(String, String, usize), ConvTargets>,
    /// SOUNDNESS R1351 — `trait_declares` answers.
    tdecl: HashMap<(String, String, usize), Option<bool>>,
    /// R1351 — source trees over `TRAIT_SRC_CAP`, which `trait_declares` does not parse.
    too_big: HashMap<std::path::PathBuf, bool>,
}

/// SOUNDNESS R1351 — the CLOSED method lists of the std traits a package trait commonly names as a supertrait, and their
/// own supertraits. A trait absent from this table is unknown (`None`), which is what keeps it a denylist of proofs:
/// `Iterator`, `Future`, `Read`, `Hasher`, … have members this table does not list, so a supertrait naming one proves
/// nothing.
fn std_trait_members(leaf: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    Some(match leaf {
        "Sized" | "Send" | "Sync" | "Unpin" | "UnwindSafe" | "RefUnwindSafe" | "Fn" | "FnMut" | "FnOnce" => (&[], &[]),
        "Copy" => (&[], &["Clone"]),
        "Clone" => (&["clone", "clone_from"], &[]),
        "PartialEq" => (&["eq", "ne"], &[]),
        "Eq" => (&[], &["PartialEq"]),
        "PartialOrd" => (&["partial_cmp", "lt", "le", "gt", "ge"], &["PartialEq"]),
        "Ord" => (&["cmp", "max", "min", "clamp"], &["Eq", "PartialOrd"]),
        "Hash" => (&["hash", "hash_slice"], &[]),
        "Debug" | "Display" | "LowerHex" | "UpperHex" | "Octal" | "Binary" | "Pointer" | "LowerExp" | "UpperExp" => (&["fmt"], &[]),
        "Default" => (&["default"], &[]),
        "Add" => (&["add"], &[]),
        "Sub" => (&["sub"], &[]),
        "Mul" => (&["mul"], &[]),
        "Div" => (&["div"], &[]),
        "Rem" => (&["rem"], &[]),
        "Neg" => (&["neg"], &[]),
        "Not" => (&["not"], &[]),
        "BitAnd" => (&["bitand"], &[]),
        "BitOr" => (&["bitor"], &[]),
        "BitXor" => (&["bitxor"], &[]),
        "Shl" => (&["shl"], &[]),
        "Shr" => (&["shr"], &[]),
        "AddAssign" => (&["add_assign"], &[]),
        "SubAssign" => (&["sub_assign"], &[]),
        "MulAssign" => (&["mul_assign"], &[]),
        "DivAssign" => (&["div_assign"], &[]),
        "RemAssign" => (&["rem_assign"], &[]),
        "BitAndAssign" => (&["bitand_assign"], &[]),
        "BitOrAssign" => (&["bitor_assign"], &[]),
        "BitXorAssign" => (&["bitxor_assign"], &[]),
        "ShlAssign" => (&["shl_assign"], &[]),
        "ShrAssign" => (&["shr_assign"], &[]),
        "From" => (&["from"], &[]),
        "Into" => (&["into"], &[]),
        "TryFrom" => (&["try_from"], &[]),
        "TryInto" => (&["try_into"], &[]),
        "AsRef" => (&["as_ref"], &[]),
        "AsMut" => (&["as_mut"], &[]),
        "Borrow" => (&["borrow"], &[]),
        "BorrowMut" => (&["borrow_mut"], &["Borrow"]),
        "Deref" => (&["deref"], &[]),
        "DerefMut" => (&["deref_mut"], &["Deref"]),
        "ToString" => (&["to_string"], &[]),
        "ToOwned" => (&["to_owned", "clone_into"], &[]),
        "FromStr" => (&["from_str"], &[]),
        "Sum" => (&["sum"], &[]),
        "Product" => (&["product"], &[]),
        "Drop" => (&["drop"], &[]),
        "Error" => (&["source", "description", "cause", "provide"], &["Debug", "Display"]),
        _ => return None,
    })
}

/// R1351 — the largest dependency source `trait_declares` parses (bytes of `.rs`).
const TRAIT_SRC_CAP: u64 = 24 * 1024 * 1024;

fn src_bytes(d: &Path) -> u64 {
    walkdir::WalkDir::new(d)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "rs"))
        .filter_map(|e| e.metadata().ok().map(|m| m.len()))
        .sum()
}

/// R1351 — whether a std trait (by leaf) has `member`, through its own closed list and its std supertraits. `None` when
/// the table does not know the trait.
fn std_trait_has(leaf: &str, member: &str, depth: usize) -> Option<bool> {
    if depth > 8 {
        return None;
    }
    let (ms, sup) = std_trait_members(leaf)?;
    if ms.contains(&member) {
        return Some(true);
    }
    for s in sup {
        if std_trait_has(s, member, depth + 1)? {
            return Some(true);
        }
    }
    Some(false)
}

impl DepSigs {
    /// The source directories of the dependency the consumer at `dir` spells `alias` (real package `real`): a manifest
    /// `path` (own manifest, then the workspace root's `[workspace.dependencies]`), else every registry checkout the
    /// lockfile names. Empty when none is found.
    fn source_dirs(&mut self, dir: &str, alias: &str, real: &str) -> Vec<std::path::PathBuf> {
        if let Some(v) = self.dirs.get(alias) {
            return v.clone();
        }
        let norm = |s: &str| s.replace('-', "_");
        let mut out: Vec<std::path::PathBuf> = Vec::new();
        let root = crate::deps::find_workspace_root(Path::new(dir));
        let mut manifests: Vec<std::path::PathBuf> = vec![Path::new(dir).to_path_buf()];
        if let Some(r) = &root {
            manifests.push(r.clone());
        }
        let mut reqs: Vec<String> = Vec::new();
        'm: for m in &manifests {
            let Ok(text) = std::fs::read_to_string(m.join("Cargo.toml")) else { continue };
            let Ok(doc) = text.parse::<toml::Table>() else { continue };
            for table in crate::deps::manifest_dependency_tables(&doc) {
                for (k, v) in table {
                    if norm(k) != alias {
                        continue;
                    }
                    if let Some(p) = v.as_table().and_then(|t| t.get("path")).and_then(|p| p.as_str()) {
                        out.push(m.join(p));
                        break 'm;
                    }
                    if let Some(r) = v.as_str().or_else(|| v.as_table().and_then(|t| t.get("version")).and_then(|r| r.as_str())) {
                        reqs.push(r.to_string());
                    }
                }
            }
        }
        if out.is_empty() {
            let regs = crate::deps::dirs_cargo_registry_src();
            let mut lock_dirs = vec![Path::new(dir).to_path_buf()];
            if let Some(r) = &root {
                lock_dirs.push(r.clone());
            }
            for ld in lock_dirs {
                let Ok(lock) = std::fs::read_to_string(ld.join("Cargo.lock")) else { continue };
                let mut name = String::new();
                for line in lock.lines() {
                    let l = line.trim();
                    if l.starts_with('[') {
                        name.clear();
                    } else if let Some(v) = l.strip_prefix("name = ") {
                        name = v.trim_matches('"').to_string();
                    } else if let Some(v) = l.strip_prefix("version = ") {
                        if norm(&name) == real {
                            let v = v.trim_matches('"');
                            if let Some(p) = regs.iter().map(|r| r.join(format!("{name}-{v}"))).find(|p| p.is_dir()) {
                                if !out.contains(&p) {
                                    out.push(p);
                                }
                            }
                        }
                    }
                }
                if !out.is_empty() {
                    break;
                }
            }
        }
        // No lockfile entry (a library tree, a registry checkout): every registry checkout of the package that is
        // semver-compatible with a manifest requirement — a UNION, so the narrowing must hold for each of them.
        if out.is_empty() && !reqs.is_empty() {
            let compat = |v: &[u64]| -> Vec<u64> {
                match v {
                    [0, 0, p, ..] => vec![0, 0, *p],
                    [0, m, ..] => vec![0, *m],
                    [x, ..] => vec![*x],
                    _ => vec![],
                }
            };
            let nums = |s: &str| -> Vec<u64> {
                let s = s.trim_start_matches(|c: char| "^=~> ".contains(c));
                let core = s.split(['-', '+', ',', ' ']).next().unwrap_or("");
                let mut v: Vec<u64> = core.split('.').map_while(|x| x.parse().ok()).collect();
                v.resize(3, 0);
                v
            };
            for r in &reqs {
                if r.contains('*') || r.contains('<') || r.contains(',') {
                    continue; // a range this reading cannot bound: no narrowing source from it
                }
                let want = compat(&nums(r));
                for reg in crate::deps::dirs_cargo_registry_src() {
                    for e in std::fs::read_dir(&reg).into_iter().flatten().flatten() {
                        let n = e.file_name().to_string_lossy().to_string();
                        let Some((name, ver)) = n.rsplit_once('-').filter(|(_, v)| v.chars().next().is_some_and(|c| c.is_ascii_digit())) else { continue };
                        if norm(name) == real && !ver.contains('-') && compat(&nums(ver)) == want && !out.contains(&e.path()) {
                            out.push(e.path());
                        }
                    }
                }
            }
        }
        self.dirs.insert(alias.to_string(), out.clone());
        out
    }

    fn index(&mut self, src: &Path) -> Option<&SrcIndex> {
        if !self.idx.contains_key(src) {
            let v = SrcIndex::build(src);
            self.idx.insert(src.to_path_buf(), v);
        }
        self.idx.get(src).and_then(|o| o.as_ref())
    }

    /// SOUNDNESS R1192 — the callee's definitions in every source of the dependency, judged per ITEM, then `judge` asked of
    /// each: `Some(answer)` (the union; empty is a proof that nothing runs) or `None` ("any"), with the reason for the reach
    /// counter. The shared rules of `allowed` and `conversion_targets`:
    ///   * a `proc-macro` crate exports no runtime item, so no call lands in it (a consumer's misread path, `use
    ///     thiserror::Error` beside a local `Error`): nothing runs;
    ///   * the callee is unreadable when a definition of it may be somewhere this reading cannot see — a file that does not
    ///     parse and names it, a `pub use` of it (or its type, or a glob) from outside the crate, a `macro_rules!` that can
    ///     name it (a fixed `fn leaf`, a `fn $name` whose name some macro token supplies, a computed name), a macro
    ///     invocation naming it or its type, or one in an `impl` of its type;
    ///   * a callee on a TYPE (`LocalSet::spawn_local`) must have an INHERENT definition the source shows on that type (or
    ///     the path names the trait, `Trait::m`): otherwise the method that runs may be any trait's of that name in scope
    ///     at the consumer, through a blanket impl (`Digest::update`), or derived. Its candidates are that type's methods, every
    ///     blanket impl's, and every local trait's declaration; a free fn's are the free fns of that name;
    ///   * a tuple struct / variant of that name with no fn of it is a constructor: nothing runs.
    fn judged<T: Ord + Clone>(
        &mut self,
        dir: &str,
        alias: &str,
        real: &str,
        callee: &str,
        pos: usize,
        judge: impl Fn(&Cand<'_>, usize) -> Result<Option<BTreeSet<T>>, ()>,
    ) -> (Option<BTreeSet<T>>, String) {
        // SOUNDNESS R1200 — a `.` before the callee marks a METHOD-CALL site (`scan.rs`)
        let (method_site, callee) = match callee.strip_prefix('.') {
            Some(c) => (true, c),
            None => (false, callee),
        };
        let segs: Vec<&str> = callee.split("::").collect();
        let leaf = segs.last().copied().unwrap_or(callee).to_string();
        let srcs = self.source_dirs(dir, alias, real);
        if srcs.is_empty() {
            return (None, "nosrc".into());
        }
        let mut out = BTreeSet::new();
        for s in srcs {
            let Some(idx) = self.index(&s) else { return (None, "unreadable".into()) };
            if idx.proc_macro {
                continue;
            }
            let ty_seg = (segs.len() >= 2).then(|| segs[segs.len() - 2]).filter(|t| {
                idx.types.contains(*t) || t.chars().next().is_some_and(char::is_uppercase) || !idx.local_names.contains(*t)
            });
            let mods: Vec<String> = segs[..segs.len() - 1 - usize::from(ty_seg.is_some())].iter().map(|s| s.to_string()).collect();
            let want = Want { leaf: leaf.clone(), tys: ty_seg.map(|t| idx.type_names(t)).unwrap_or_default(), mods };
            for (text, w) in &idx.unparsed {
                if w.contains(&want.leaf) || want.tys.iter().any(|t| w.contains(t)) || w.contains("macro_rules") || text.contains("::*") {
                    return (None, "parse".into());
                }
            }
            let mut c = Collect {
                idx, want: &want, stack: Vec::new(), imp: Vec::new(), fns: Vec::new(), ctor: false, opaque: None,
                lists: Vec::new(), body: 0, foreign: None,
            };
            for f in &idx.files {
                c.visit_file(f);
            }
            if let Some(why) = c.opaque {
                return (None, why);
            }
            // SOUNDNESS R1200 — a FREE fn's candidates are the free fns of its name, and a foreign item macro may have made one
            // this reading never sees, in any module (`mmac::make_free!()` at the root beside a visible `twin::f`); its tokens
            // name nothing, so `invocation` cannot see it. Methods too: a foreign macro can write `impl crate::Foo<B> { fn m }`
            // beside a visible `impl Foo<A> { fn m }` (distinct instantiations are not duplicates). Measured: applying it to
            // methods as well costs 1 more newly-`Unknown` row over the chained corpus, so the duplicate-definition argument
            // is not leaned on.
            if std::env::var_os("CANDOR_R1200_FMAC_OFF").is_none() {
                if let Some(m) = &c.foreign {
                    return (None, format!("fmac:{m}"));
                }
            }
            if c.fns.is_empty() && c.ctor {
                continue;
            }
            let tys = &want.tys;
            let (cands, anchored): (Vec<&Cand<'_>>, bool) = if tys.is_empty() {
                let v: Vec<&Cand<'_>> = c.fns.iter().filter(|x| x.site == Site::Free).collect();
                let a = !v.is_empty();
                (v, a)
            } else {
                let on = |o: &Option<String>| o.as_ref().is_some_and(|t| tys.contains(t));
                let v: Vec<&Cand<'_>> = c
                    .fns
                    .iter()
                    .filter(|x| match &x.site {
                        Site::Free => false,
                        Site::Inherent(t) => on(t),
                        Site::TraitImpl(tr, t) => t.is_none() || on(t) || tys.contains(tr),
                        Site::TraitDecl(_) => true,
                    })
                    .collect();
                let a = v.iter().any(|x| match &x.site {
                    Site::Inherent(t) => on(t),
                    // a trait impl on the type does not anchor: another trait of the same method name, in scope at the
                    // consumer and blanket-implemented, may be the one that runs
                    Site::TraitImpl(tr, _) => tys.contains(tr),
                    Site::TraitDecl(tr) => tys.contains(tr),
                    Site::Free => false,
                });
                (v, a)
            };
            if !anchored {
                return (None, format!("noanchor:{}", c.fns.len()));
            }
            // SOUNDNESS R1200 — at a METHOD-CALL site the anchor does not settle which `m` runs. Method resolution probes the
            // receiver by value, then `&`, then `&mut`, and takes the FIRST step any method matches; an inherent method wins only
            // against a trait method found at the SAME step. So a by-value `fn m(self, ..)` of any trait in scope at the consumer
            // (a blanket `impl<T> Ext for T` in a third crate, never in this source) beats an inherent `fn m(&self, ..)` on a
            // value receiver — and which step the inherent sits at depends on the receiver's reference-ness, which the site does
            // not carry. A path call (`Type::m(..)`) has no such probe: the inherent is the callee.
            if method_site && std::env::var_os("CANDOR_R1200_METH_OFF").is_none() {
                return (None, "meth:probe".into());
            }
            let mut fitted = 0usize;
            for cand in cands {
                match judge(cand, pos) {
                    Ok(Some(set)) => {
                        fitted += 1;
                        out.extend(set);
                    }
                    Ok(None) => return (None, "bound".into()),
                    Err(()) => {}
                }
            }
            if fitted == 0 {
                return (None, "arity".into());
            }
        }
        (Some(out), "read".into())
    }

    fn say(&self, what: &str, alias: &str, callee: &str, pos: usize, why: &str) {
        if std::env::var_os("CANDOR_DEPSIG_INSTR").is_some() {
            eprintln!("DSWHY\t{what}\t{alias}\t{callee}\t{pos}\t{why}"); // §E1 REACH COUNTER
        }
    }

    /// The std-witness traits the dependency's callee (`callee`, the path after the crate) can run on the value at
    /// argument position `pos` (the receiver is 0 for a method). `None` = cannot narrow.
    pub(crate) fn allowed(&mut self, dir: &str, alias: &str, real: &str, callee: &str, pos: usize) -> Option<BTreeSet<String>> {
        let (v, why) = self.judged(dir, alias, real, callee, pos, cand_allows);
        self.say("allo", alias, callee, pos, &why);
        v
    }

    /// SOUNDNESS R1351 / R1353 — does the dependency's trait `leaf` (as the dependency spelled `alias`, real package `real`)
    /// declare a METHOD named `member`, itself or through a supertrait? Read from the trait's own declaration in the
    /// dependency's SOURCE — the authority, rather than a list of member names (§G). `Some(false)` is a PROOF: every
    /// declaration of that leaf in every source of the package was read, none declares the member, and every supertrait
    /// was either read the same way (a trait of this package) or is a std trait whose member list is closed and known
    /// (`std_trait_members`). Every doubt is `None`: no source, no declaration of the leaf (re-exported from elsewhere, or
    /// macro-made), a file that does not parse and names it, a macro anywhere naming it, a macro item in the trait body, a
    /// supertrait of another package or an unlisted std trait. A caller may narrow on `Some(false)` only.
    pub(crate) fn trait_declares(&mut self, dir: &str, alias: &str, real: &str, leaf: &str, member: &str) -> Option<bool> {
        let key = (format!("\u{1}tdecl\u{1}{alias}\u{1}{leaf}"), member.to_string(), 0usize);
        if let Some(v) = self.tdecl.get(&key) {
            return *v;
        }
        let dirs = self.source_dirs(dir, alias, real);
        // A source in which the leaf is not even NAMED is a version of the package that does not have the trait; the key came
        // from code that compiled against one that does, so that source says nothing. All such (or none) is a doubt.
        let mut answer: Option<bool> = None;
        let mut read_one = false;
        for d in &dirs {
            // A COST bound, and a doubt rather than a proof: a source tree this large (the `windows` bindings) is not
            // parsed for a reason string — the key is named.
            let big = match self.too_big.get(d) {
                Some(b) => *b,
                None => {
                    let b = !self.idx.contains_key(d.as_path()) && src_bytes(d) > TRAIT_SRC_CAP;
                    self.too_big.insert(d.clone(), b);
                    b
                }
            };
            if big {
                answer = None;
                break;
            }
            let a = match self.index(d) {
                Some(ix) if !ix.names_leaf(leaf) => continue,
                Some(ix) => ix.trait_answer(leaf, member, 0),
                None => None,
            };
            match a {
                Some(true) => {
                    answer = Some(true);
                    break;
                }
                Some(false) if !read_one || answer == Some(false) => answer = Some(false),
                _ => answer = None,
            }
            read_one = true;
            if answer.is_none() {
                break;
            }
        }
        if std::env::var_os("CANDOR_DEPSIG_INSTR").is_some() {
            eprintln!("DSTRAIT\t{alias}\t{leaf}\t{member}\t{answer:?}"); // §E1 REACH COUNTER
        }
        self.tdecl.insert(key, answer);
        answer
    }

    /// SOUNDNESS R1186 — what the dependency's callee can CONVERT the value at `pos` into, as `(kind, target leaf)`:
    /// kind `From` for an `Into<X>` bound on the parameter's generic or an `X: From<..>` bound naming it, `TryFrom` for
    /// `TryInto`/`TryFrom`; target `*` when `X` is (or names) a generic parameter or has no nominal leaf. `Some(empty)`
    /// is a PROOF that no conversion runs (a concrete parameter, bounds that are all known non-conversion std traits, a
    /// parameter the body never mentions, a constructor); every doubt — an unknown bound, which may carry a conversion
    /// as a supertrait or an associated type, a receiver, `Self`, an unreadable source — is `None`. The same DENYLIST
    /// discipline as `allowed`, over the same candidates.
    pub(crate) fn conversion_targets(&mut self, dir: &str, alias: &str, real: &str, callee: &str, pos: usize) -> ConvTargets {
        let key = (alias.to_string(), callee.to_string(), pos);
        if let Some(v) = self.conv.get(&key) {
            return v.clone();
        }
        let (v, why) = self.judged(dir, alias, real, callee, pos, cand_converts);
        self.say("conv", alias, callee, pos, &why);
        self.conv.insert(key, v.clone());
        v
    }
}

/// SOUNDNESS R1186 — the conversion target a bound's type argument names: its outer nominal leaf, or `*` when it is
/// (or names) one of the candidate's generic parameters, `Self`, or no plain path at all.
fn conv_target(t: &syn::Type, generic_names: &HashSet<String>) -> String {
    let mut n = TyNames::default();
    n.visit_type(t);
    if n.0.iter().any(|x| generic_names.contains(x) || x == "Self") {
        return "*".to_string();
    }
    let mut t = t;
    while let syn::Type::Reference(r) = t {
        t = &r.elem;
    }
    match t {
        syn::Type::Path(p) if p.qself.is_none() => p.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_else(|| "*".to_string()),
        _ => "*".to_string(),
    }
}

/// SOUNDNESS R1186 — the first type argument of a bound's last path segment (`Into<String>` → `String`).
fn bound_type_arg(b: &syn::TraitBound) -> Option<&syn::Type> {
    match &b.path.segments.last()?.arguments {
        syn::PathArguments::AngleBracketed(a) => a.args.iter().find_map(|g| match g {
            syn::GenericArgument::Type(t) => Some(t),
            _ => None,
        }),
        _ => None,
    }
}

/// SOUNDNESS R1186 — `cand_allows`'s twin for conversions: `Some(targets)` (possibly empty), `None` (any), `Err(())`
/// when the candidate has no input at `pos`.
fn cand_converts(c: &Cand<'_>, pos: usize) -> Result<Option<BTreeSet<(String, String)>>, ()> {
    let arg = c.inputs.get(pos).ok_or(())?;
    let syn::FnArg::Typed(pt) = arg else { return Ok(None) };
    let generic_names: HashSet<String> = c
        .scope
        .iter()
        .flat_map(|g| g.params.iter())
        .filter_map(|p| match p {
            syn::GenericParam::Type(t) => Some(t.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut tn = TyNames::default();
    tn.visit_type(&pt.ty);
    if tn.0.contains("Self") {
        return Ok(None);
    }
    let mut out: BTreeSet<(String, String)> = BTreeSet::new();
    // One bound on a type the value flows through: a conversion it runs, a known std trait that runs none, or doubt.
    let judge = |b: &syn::TraitBound, out: &mut BTreeSet<(String, String)>| -> bool {
        let Some(l) = b.path.segments.last().map(|s| s.ident.to_string()) else { return true };
        match l.as_str() {
            "Into" | "TryInto" => {
                let t = bound_type_arg(b).map(|t| conv_target(t, &generic_names)).unwrap_or_else(|| "*".to_string());
                out.insert((if l == "Into" { "From" } else { "TryFrom" }.to_string(), t));
                true
            }
            _ => implied(&l).is_some(),
        }
    };
    let mut work: Vec<String> = tn.0.iter().filter(|n| generic_names.contains(*n)).cloned().collect();
    // `impl Trait` / `dyn Trait` in the parameter's own type.
    let mut inline: Vec<&syn::TraitBound> = Vec::new();
    struct Inline<'b, 'ast>(&'b mut Vec<&'ast syn::TraitBound>);
    impl<'ast> Visit<'ast> for Inline<'_, 'ast> {
        fn visit_type_impl_trait(&mut self, t: &'ast syn::TypeImplTrait) {
            for b in &t.bounds {
                if let syn::TypeParamBound::Trait(tb) = b {
                    self.0.push(tb);
                }
            }
            syn::visit::visit_type_impl_trait(self, t);
        }
        fn visit_type_trait_object(&mut self, t: &'ast syn::TypeTraitObject) {
            for b in &t.bounds {
                if let syn::TypeParamBound::Trait(tb) = b {
                    self.0.push(tb);
                }
            }
            syn::visit::visit_type_trait_object(self, t);
        }
    }
    Inline(&mut inline).visit_type(&pt.ty);
    if work.is_empty() && inline.is_empty() {
        return Ok(Some(BTreeSet::new())); // a concrete type: the consumer's value cannot be passed here
    }
    for b in &inline {
        if matches!(b.modifier, syn::TraitBoundModifier::None) && !judge(b, &mut out) {
            return Ok(None);
        }
        let mut n = TyNames::default();
        n.visit_trait_bound(b);
        work.extend(n.0.into_iter().filter(|x| generic_names.contains(x)));
    }
    let mentioned_generics: Vec<String> = work.clone();
    // The generics the value flows through: the parameter's own, closed over the generics their bounds name.
    let mut seen: HashSet<String> = HashSet::new();
    while let Some(g) = work.pop() {
        if !seen.insert(g.clone()) {
            continue;
        }
        for gens in &c.scope {
            let mut preds: Vec<(Vec<String>, Vec<&syn::TraitBound>)> = Vec::new();
            for p in &gens.params {
                if let syn::GenericParam::Type(t) = p {
                    let bs = t.bounds.iter().filter_map(|b| match b {
                        syn::TypeParamBound::Trait(tb) => Some(tb),
                        _ => None,
                    });
                    preds.push((vec![t.ident.to_string()], bs.collect()));
                }
            }
            if let Some(w) = &gens.where_clause {
                for pred in &w.predicates {
                    if let syn::WherePredicate::Type(pt2) = pred {
                        let mut bn = TyNames::default();
                        bn.visit_type(&pt2.bounded_ty);
                        let bs = pt2.bounds.iter().filter_map(|b| match b {
                            syn::TypeParamBound::Trait(tb) => Some(tb),
                            _ => None,
                        });
                        preds.push((bn.0.into_iter().collect(), bs.collect()));
                    }
                }
            }
            for (bounded, bs) in preds {
                for b in bs {
                    if !matches!(b.modifier, syn::TraitBoundModifier::None) {
                        continue;
                    }
                    let mut an = TyNames::default();
                    an.visit_trait_bound(b);
                    let on_g = bounded.contains(&g);
                    // a bound whose ARGUMENTS name `g` (`U: From<T>`, `DErr: From<E>`, `S: Sink<T>`)
                    let names_g = an.0.contains(&g) && !on_g;
                    if on_g {
                        if !judge(b, &mut out) {
                            return Ok(None);
                        }
                        work.extend(an.0.iter().filter(|x| generic_names.contains(*x) && !seen.contains(*x)).cloned());
                        if bounded.len() > 1 {
                            work.extend(bounded.iter().filter(|x| generic_names.contains(*x) && !seen.contains(*x)).cloned());
                        }
                    } else if names_g {
                        let l = b.path.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
                        match l.as_str() {
                            "From" | "TryFrom" => {
                                let t = if bounded.iter().any(|x| generic_names.contains(x) || x == "Self") {
                                    "*".to_string()
                                } else {
                                    // the bounded type's own outer leaf, as the predicate wrote it
                                    gens.where_clause
                                        .as_ref()
                                        .and_then(|w| w.predicates.iter().find_map(|p| match p {
                                            syn::WherePredicate::Type(pt2) if pt2.bounds.iter().any(|x| matches!(x, syn::TypeParamBound::Trait(tb) if std::ptr::eq(tb, b))) => Some(conv_target(&pt2.bounded_ty, &generic_names)),
                                            _ => None,
                                        }))
                                        .unwrap_or_else(|| "*".to_string())
                                };
                                out.insert((l.clone(), t));
                            }
                            // a std trait taking `g` as an argument runs no conversion of it (`F: Fn(T)`, `PartialEq<T>`)
                            _ if implied(&l).is_some() => {}
                            _ => return Ok(None),
                        }
                    }
                }
            }
        }
    }
    // A parameter neither whose binding nor whose generic names the body mentions cannot be converted.
    if !out.is_empty() {
        if let Some(b) = c.body {
            let mut w = Words::default();
            w.visit_block(b);
            let mut binds = TyNames::default();
            binds.visit_pat(&pt.pat);
            let used = binds.0.iter().any(|n| w.0.contains(n)) || mentioned_generics.iter().any(|g| w.0.contains(g));
            if !used {
                return Ok(Some(BTreeSet::new()));
            }
        }
    }
    Ok(Some(out))
}
