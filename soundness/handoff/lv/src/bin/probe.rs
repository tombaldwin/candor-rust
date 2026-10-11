use lvfx::*;
use std::sync::atomic::Ordering;
macro_rules! m { ($name:expr, $e:expr) => {{
    DROPS.store(0, Ordering::SeqCst);
    let r = $e;
    let d = DROPS.load(Ordering::SeqCst);
    std::mem::forget(r);
    println!("{}\t{}", $name, d);
}}; }
fn main() {
    m!("a_tryfold[1,99]", a_tryfold(&[1, 99]));
    m!("a_tryfold_typed[1,99]", a_tryfold_typed(&[1, 99]));
    m!("a_collect[1,99]", a_collect(&[1, 99]));
    m!("a_collect_sep[1,99]", a_collect_sep(&[1, 99]));
    m!("a_forloop[1,99]", a_forloop(&[1, 99]));
    m!("a_fold_ctl", a_fold_ctl(&[1, 2]));
    m!("a_tryfold_ctl", a_tryfold_ctl(&[1, 2]));
    m!("a_collect_ctl", a_collect_ctl(&[1, 2]));
    m!("a_andthen_ctl(99)", a_andthen_ctl(99));
    m!("b_opt", b_opt());
    m!("b_pair", b_pair());
    m!("b_arr", b_arr());
    m!("b_vec", b_vec());
    m!("b_gen", b_gen());
    m!("b_into", b_into());
    m!("b_bare_ctl", b_bare_ctl());
    m!("b_clos_bare", b_clos_bare());
    m!("b_clos_opt", b_clos_opt());
    m!("b_and", b_and(1));
    m!("b_okor(1)", b_okor(1));
    m!("b_memdrop", b_memdrop());
    m!("b_memdrop_let", b_memdrop_let());
    { let o = Some(H::new()); m!("b_unwrapor", b_unwrapor(o)); }
    { let mut v = Vec::new(); m!("c_keep", c_keep(&mut v)); std::mem::forget(v); }
    { let mut v = Vec::new(); m!("c_keep_opt", c_keep_opt(&mut v)); std::mem::forget(v); }
    { let mut v = Vec::new(); let h = H::new(); m!("c_keep_only", c_keep_only(h, &mut v)); std::mem::forget(v); }
    m!("b_and_row(99)", b_and_row(99));
    m!("e_memdrop_use", e_memdrop_use());
    m!("e_coredrop", e_coredrop());
    m!("e_drop_let", e_drop_let());
    m!("b_drop_bare", b_drop_bare());
    m!("b_memdrop_let2", b_memdrop_let2());
    m!("b_gen_let", b_gen_let());
    m!("b_opt_let", b_opt_let());
    m!("b_clos_bare_let", b_clos_bare_let());
    m!("a_tryforeach[1,99]", a_tryforeach(&[1, 99]));
    m!("f_opt_semi", f_opt_semi());
    m!("f_pair_semi", f_pair_semi());
    m!("f_vec_semi", f_vec_semi());
    m!("f_gen_semi", f_gen_semi());
    m!("f_into_semi", f_into_semi());
    m!("f_clos_opt_semi", f_clos_opt_semi());
    m!("f_memdrop_semi", f_memdrop_semi());
    m!("f_tail_unit_ret", f_tail_unit_ret());
    m!("f_opt_q(1)", f_opt_q(1));
    m!("f_opt_q_split(1)", f_opt_q_split(1));
    { let mut v = Vec::new(); m!("c_keep_opt_semi", c_keep_opt_semi(&mut v)); std::mem::forget(v); }
    m!("g_inner_shadow", g_inner_shadow());
    m!("g_match_pat", g_match_pat());
    m!("g_sib_half", g_sib_half());
    m!("g_shadow_after_use", g_shadow_after_use());
    m!("g_macro_use", g_macro_use());
    m!("h_tryfold_replace[1,99]", h_tryfold_replace(&[1, 99]));
    m!("h_tryfold_ok[1,2]", h_tryfold_ok(&[1, 2]));
    m!("h_andthen_acc(99)", h_andthen_acc(99));
    m!("i_collect_vecres[1,99]", i_collect_vecres(&[1, 99]));
    m!("i_collect_opt[1,99]", i_collect_opt(&[1, 99]));
    m!("i_collect_let[1,99]", i_collect_let(&[1, 99]));
    m!("i_collect_tail[1,99]", i_collect_tail(&[1, 99]));
    m!("d_shadow", d_shadow());
    m!("d_sibling", d_sibling());
    m!("d_single", d_single());
    m!("d_twonames", d_twonames());
    m!("d_shadow_val", d_shadow_val());
    m!("d_shadow_ret", d_shadow_ret());
    m!("d_shadow_nocall", d_shadow_nocall());
}
