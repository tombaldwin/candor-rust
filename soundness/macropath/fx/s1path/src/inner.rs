pub fn go_s1path() { fxdeps1path::mk!() }
pub fn qgo_s1path() { fxdeps1path::nop!() }
pub fn via_s1path() { go_s1path() }
pub fn qvia_s1path() { qgo_s1path() }
