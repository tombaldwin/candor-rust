//! SOUNDNESS R1528, the 2026-11-13 coverage-gate refresh at the 0.42.0 staging commit: nine rows grew from engine changes
//! (R1717 builder typing, R1657 stage 2 re-export resolver). Four are real and carry rules (the `default_provider` chain
//! constructors, the same `Net` as the free fn they await); five carry verdicts (R1732-R1734, evidence at each entry in `src/lib.rs`).
use candor_classify::{classify, OPEN_ENTRY_VERDICTS};

#[test]
fn coverage_gate_refresh_2026_11_13_rules() {
    for p in ["meta::credentials::chain::CredentialsProviderChain", "meta::credentials::CredentialsProviderChain", "CredentialsProviderChain"] {
        for m in ["default_provider", "or_default_provider"] {
            assert_eq!(classify("aws_config", &format!("aws_config::{p}::{m}")), Some("Net"), "{p}::{m}");
        }
    }
    for m in ["default_provider", "or_default_provider"] {
        for p in ["meta::token::TokenProviderChain", "TokenProviderChain"] {
            assert_eq!(classify("aws_config", &format!("aws_config::{p}::{m}")), Some("Net"), "{p}::{m}");
        }
    }
    // the neighbours that only compose a chain stay pure, and another crate's same-named type is not claimed
    for p in ["CredentialsProviderChain::first_try", "meta::credentials::chain::CredentialsProviderChain::first_try", "meta::credentials::chain::CredentialsProviderChain::or_else",
              "meta::token::TokenProviderChain::first_try", "meta::token::TokenProviderChain::or_else"] {
        assert_eq!(classify("aws_config", &format!("aws_config::{p}")), None, "{p}");
    }
    assert_eq!(classify("mycrate", "mycrate::meta::token::TokenProviderChain::default_provider"), None);
}

#[test]
fn coverage_gate_refresh_2026_11_13_verdicts() {
    for (krate, entry, verdict) in [
        ("reqwest", "reqwest::blocking::multipart::Form::text", "R1732:overcharge"),
        ("reqwest", "reqwest::blocking::multipart::Part::bytes", "R1732:overcharge"),
        ("reqwest", "reqwest::blocking::multipart::Part::text", "R1732:overcharge"),
        ("rustls", "rustls::server::server_conn::Accepted::into_connection", "R1733:conditional"),
        ("redis", "redis::cluster_handling::client::ClusterClient::new", "R1734:overcharge"),
    ] {
        assert!(OPEN_ENTRY_VERDICTS.contains(&(krate, entry, verdict)), "{entry}");
        assert_eq!(classify(krate, entry), None, "a verdict is not a rule: {entry}");
    }
    // the reqwest rows that really open a path keep their rules
    assert_eq!(classify("reqwest", "reqwest::blocking::multipart::Part::file"), Some("Fs"));
    assert_eq!(classify("reqwest", "reqwest::blocking::multipart::Form::file"), Some("Fs"));
}
