pub const _: () = {
    #[cfg(windows)]
    let _w: () = { #[macro_export] macro_rules! mk { () => { () } } };
};
#[macro_export] macro_rules! mknop { () => { () } }
