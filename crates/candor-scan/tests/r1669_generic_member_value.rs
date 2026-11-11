//! SOUNDNESS R1669 — a trait member named as a FUNCTION VALUE through a generic parameter (`T::from_der` handed to a
//! dependency, rustls-webpki's `der::read_all`) is a dispatch over the trait's implementors, exactly as the call `T::from_der(x)`
//! is. It formed no edge, so the function read only its `invisible` ledger line over every implementor's body. EXECUTED in the
//! lane (`rustagent-r1537c/fx/r1669`: the implementor's write ran, `deny Fs zz` read 0 on 730eefa3). R1537 (c)'s split exposed it:
//! the merged inherent/trait unit had carried the trait body itself. `CANDOR_R1669_OFF=1` restores the miss.

use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_candor-scan")
}

fn w(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn zz_effects(off: bool, value: &str) -> Vec<String> {
    let root = std::env::temp_dir().join(format!("candor-r1669-{}-{off}-{}", std::process::id(), value.len()));
    let _ = std::fs::remove_dir_all(&root);
    w(&root.join("dapply/Cargo.toml"), "[package]\nname = \"dapply\"\nversion = \"0.0.0\"\nedition = \"2021\"\n");
    w(&root.join("dapply/src/lib.rs"), "pub fn apply<T>(f: impl Fn(u8) -> T) -> T { f(1) }\n");
    w(&root.join("app/Cargo.toml"), "[package]\nname = \"app\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[dependencies]\ndapply = { path = \"../dapply\" }\n");
    w(&root.join("app/src/lib.rs"), &format!(
        "pub trait FromDer: Sized {{ fn from_der(x: u8) -> Self; }}\npub struct A;\nimpl FromDer for A {{ fn from_der(_x: u8) -> Self {{ let _ = std::fs::write(\"/tmp/r1669-never\", b\"x\"); A }} }}\npub fn read_all<T: FromDer>() -> T {{ dapply::apply({value}) }}\npub fn zz() -> A {{ read_all::<A>() }}\n"));
    let mut cmd = Command::new(bin());
    cmd.arg(root.join("app").to_string_lossy().as_ref()).arg("--json").env_remove("CANDOR_DEPS").env_remove("CANDOR_R1669_OFF");
    if off {
        cmd.env("CANDOR_R1669_OFF", "1");
    }
    let o = cmd.output().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("report json");
    v["functions"].as_array().unwrap().iter().filter(|f| f["fn"] == "zz")
        .flat_map(|f| f["inferred"].as_array().unwrap().iter().map(|e| e.as_str().unwrap().to_string()).collect::<Vec<_>>())
        .collect()
}

#[test]
fn r1669_a_generic_member_passed_as_a_value_dispatches() {
    assert_eq!(zz_effects(false, "T::from_der"), vec!["Fs"]);
    // the fixture reaches the code: without the rule the same program reads no effect at `zz`.
    assert!(!zz_effects(true, "T::from_der").contains(&"Fs".to_string()));
}
