// The rustagent-liveness drop-counter fixture (68 cells), checked in by rustagent-v049. Executed by
// soundness/run_handoff.sh --lv: `probe` prints each cell's in-frame drop count; candor-scan must charge or
// disclose every cell that drops, except the stated residue in ../lv.expect.tsv.
use std::sync::atomic::{AtomicUsize, Ordering};
pub static DROPS: AtomicUsize = AtomicUsize::new(0);
pub struct H { pub tag: u32 }
impl H { pub fn new() -> H { H { tag: 0 } } }
impl Drop for H {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
        let _ = std::fs::write("/dev/null", b"x");
    }
}
pub fn gen(i: u32) -> Result<u32, ()> { if i == 99 { Err(()) } else { Ok(i) } }
pub fn opt(i: u32) -> Option<u32> { if i == 99 { None } else { Some(i) } }

// ===== R198 =====
pub fn a_tryfold(items: &[u32]) -> Result<Vec<H>, ()> {
    items.iter().try_fold(Vec::new(), |mut acc, i| { gen(*i)?; acc.push(H::new()); Ok(acc) })
}
pub fn a_tryfold_typed(items: &[u32]) -> Result<Vec<H>, ()> {
    items.iter().try_fold(Vec::new(), |mut acc: Vec<H>, i| { gen(*i)?; acc.push(H::new()); Ok(acc) })
}
pub fn a_collect(items: &[u32]) -> Result<Vec<H>, ()> {
    let v = items.iter().map(|i| { gen(*i)?; Ok(H::new()) }).collect::<Result<Vec<_>, ()>>()?;
    Ok(v)
}
pub fn a_collect_sep(items: &[u32]) -> Result<usize, ()> {
    let r = items.iter().map(|i| { gen(*i)?; Ok(H::new()) }).collect::<Result<Vec<_>, ()>>();
    let v = r?; let n = v.len(); std::mem::forget(v); Ok(n)
}
// loop twin of the try_fold accumulator: R187 should cover this
pub fn a_forloop(items: &[u32]) -> Result<Vec<H>, ()> {
    let mut acc = Vec::new();
    for i in items { gen(*i)?; acc.push(H::new()); }
    Ok(acc)
}
// over-charge controls: 0 drops
pub fn a_fold_ctl(items: &[u32]) -> Vec<H> {
    items.iter().fold(Vec::new(), |mut acc, _| { acc.push(H::new()); acc })
}
pub fn a_tryfold_ctl(items: &[u32]) -> Result<Vec<H>, ()> {
    items.iter().try_fold(Vec::new(), |mut acc, _| { acc.push(H::new()); Ok::<_, ()>(acc) })
}
pub fn a_collect_ctl(items: &[u32]) -> Vec<H> {
    items.iter().map(|_| H::new()).collect::<Vec<_>>()
}
// once-called closure with ? before the ctor: 0 drops (and_then runs it once)
pub fn a_andthen_ctl(n: u32) -> Option<H> {
    Some(n).and_then(|x| { opt(x)?; Some(H::new()) })
}

// ===== R200 =====
fn eat_opt(_o: Option<H>) {}
fn eat_pair(_p: (H, u32)) {}
fn eat_arr(_a: [H; 1]) {}
fn eat_vec(_v: Vec<H>) {}
fn eat_gen<T>(_t: T) {}
fn eat_into(h: impl Into<H>) { let _x: H = h.into(); }
fn eat_bare(_h: H) {}
pub fn b_opt() { eat_opt(Some(H::new())) }
pub fn b_pair() { eat_pair((H::new(), 1)) }
pub fn b_arr() { eat_arr([H::new()]) }
pub fn b_vec() { eat_vec(vec![H::new()]) }
pub fn b_gen() { eat_gen(H::new()) }
pub fn b_into() { eat_into(H::new()) }
pub fn b_bare_ctl() { eat_bare(H::new()) }
pub fn b_clos_bare() { let f = |_h: H| {}; f(H::new()) }
pub fn b_clos_opt() { let f = |_h: Option<H>| {}; f(Some(H::new())) }
pub fn b_and(n: u32) -> Result<u32, ()> { let v = gen(n).and(Ok::<u32, ()>(7)); let _ = Ok::<H, ()>(H::new()).and(v); v }
pub fn b_okor(n: u32) -> Result<u32, H> { let v = opt(n).ok_or(H::new())?; Ok(v) }
pub fn b_memdrop() { std::mem::drop(H::new()) }
pub fn b_memdrop_let() { let h = H::new(); drop(h) }
pub fn b_unwrapor(o: Option<H>) -> H { o.unwrap_or(H::new()) }
// R230 over-charge controls: a callee that STORES its by-value param — 0 drops anywhere
fn keep(h: H, v: &mut Vec<H>) { v.push(h); }
fn keep_opt(o: Option<H>, v: &mut Vec<Option<H>>) { v.push(o); }
pub fn c_keep(v: &mut Vec<H>) { keep(H::new(), v) }
pub fn c_keep_opt(v: &mut Vec<Option<H>>) { keep_opt(Some(H::new()), v) }
pub fn c_keep_only(h: H, v: &mut Vec<H>) { v.push(h); }

// ===== R323 =====
pub fn d_shadow() { let c = || H::new(); c(); let c = || H::new(); c(); }
pub fn d_sibling() { { let c = || H::new(); c(); } { let c = || H::new(); c(); } }
pub fn d_single() { let c = || H::new(); c(); }
pub fn d_twonames() { let c = || H::new(); c(); let e = || H::new(); e(); }
pub fn d_shadow_val() { let c = || H::new(); c(); let c = 5u32; let _ = c + 1; }
// over-charge controls: 0 in-frame drops
pub fn d_shadow_ret() -> H { let c = || H::new(); let _ = &c; let c = || H::new(); c() }
pub fn d_shadow_nocall() { let c = || H::new(); let _ = &c; let c = || H::new(); let _ = &c; }
// ===== extra spellings =====
pub fn b_and_row(n: u32) -> Result<H, ()> { let x = gen(n).and(Ok(H::new()))?; Ok(x) }
pub fn b_drop_bare() { drop(H::new()) }
pub fn b_memdrop_let2() { let h = H::new(); std::mem::drop(h) }
pub fn b_gen_let() { let h = H::new(); eat_gen(h) }
pub fn b_opt_let() { let o = Some(H::new()); eat_opt(o) }
pub fn b_clos_bare_let() { let f = |_h: H| {}; let h = H::new(); f(h) }
pub fn a_tryforeach(items: &[u32]) -> Result<(), ()> {
    let mut acc = Vec::new();
    items.iter().try_for_each(|i| { gen(*i)?; acc.push(H::new()); Ok(()) })?;
    std::mem::forget(acc); Ok(())
}
pub fn e_memdrop_use() { use std::mem; let h = H::new(); mem::drop(h) }
pub fn e_coredrop() { let h = H::new(); core::mem::drop(h) }
pub fn e_drop_let() { let h = H::new(); drop(h) }
// ===== R200 statement-position twins (one variable: the trailing `;`) =====
pub fn f_opt_semi() { eat_opt(Some(H::new())); }
pub fn f_pair_semi() { eat_pair((H::new(), 1)); }
pub fn f_vec_semi() { eat_vec(vec![H::new()]); }
pub fn f_gen_semi() { eat_gen(H::new()); }
pub fn f_into_semi() { eat_into(H::new()); }
pub fn f_clos_opt_semi() { let f = |_h: Option<H>| {}; f(Some(H::new())); }
pub fn f_memdrop_semi() { std::mem::drop(H::new()); }
pub fn f_tail_unit_ret() { let h = H::new(); std::mem::drop(h) }
// the `?` operand spine twin, as the row spells it
fn eat_opt_r(_o: Option<H>, n: u32) -> Result<u32, ()> { gen(n) }
pub fn f_opt_q(n: u32) -> Result<u32, ()> { let x = eat_opt_r(Some(H::new()), n)?; Ok(x) }
pub fn f_opt_q_split(n: u32) -> Result<u32, ()> { let r = eat_opt_r(Some(H::new()), n); let x = r?; Ok(x) }
pub fn c_keep_opt_semi(v: &mut Vec<Option<H>>) { keep_opt(Some(H::new()), v); }
// ===== R323 extra controls =====
pub fn g_inner_shadow() { let c = || H::new(); let _ = &c; { let c = || 5u32; c(); } }
pub fn g_match_pat() { let c = || H::new(); let _ = &c; match Some(1u32) { Some(c) => { let _ = c; } None => {} } }
pub fn g_sib_half() { { let c = || H::new(); c(); } { let c = || H::new(); let _ = &c; } }
pub fn g_shadow_after_use() { let c = || H::new(); c(); let c = || H::new(); let r = c(); std::mem::forget(r); }
pub fn g_macro_use() { let c = || H::new(); c(); let c = || H::new(); let _ = format!("{:p}", &c); }
// ===== R198 fold controls =====
// fresh value replaces the accumulator; ? before it — the carried one dies on the Err path (1 drop)
pub fn h_tryfold_replace(items: &[u32]) -> Result<Option<H>, ()> {
    items.iter().try_fold(None, |acc: Option<H>, i| { gen(*i)?; let _ = &acc; Ok(Some(H::new())) })
}
// accumulator returned unchanged except via push AFTER a `?` that never fires on these inputs: 0 drops
pub fn h_tryfold_ok(items: &[u32]) -> Result<Vec<H>, ()> {
    items.iter().try_fold(Vec::new(), |mut acc, i| { gen(*i)?; acc.push(H::new()); Ok(acc) })
}
// once-called and_then with the same body shape: 0 drops (must stay absent)
pub fn h_andthen_acc(n: u32) -> Result<Vec<H>, ()> {
    Ok::<Vec<H>, ()>(Vec::new()).and_then(|mut acc| { gen(n)?; acc.push(H::new()); Ok(acc) })
}
pub fn i_collect_vecres(items: &[u32]) -> Vec<Result<H, ()>> {
    items.iter().map(|i| { gen(*i)?; Ok(H::new()) }).collect::<Vec<Result<H, ()>>>()
}
pub fn i_collect_opt(items: &[u32]) -> Option<Vec<H>> {
    items.iter().map(|i| { opt(*i)?; Some(H::new()) }).collect::<Option<Vec<H>>>()
}
pub fn i_collect_let(items: &[u32]) -> Result<usize, ()> {
    let v: Result<Vec<H>, ()> = items.iter().map(|i| { gen(*i)?; Ok(H::new()) }).collect();
    let v = v?; let n = v.len(); std::mem::forget(v); Ok(n)
}
pub fn i_collect_tail(items: &[u32]) -> Result<Vec<H>, ()> {
    items.iter().map(|i| { gen(*i)?; Ok(H::new()) }).collect()
}
