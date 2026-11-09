//! SOUNDNESS R1604 / R1605: common verbs (`get`, `send`, `all`, `count`) charged on a type whose real effect differs.
//! Each fabrication removed here sits beside the twin that must stay charged, and each fabrication NOT removed is pinned
//! with the reason it stays. Executed evidence: ~/candor-handoff/rust-fab/ (ev/*.obs.txt, ev/*/gate-*.tsv).

use candor_classify::classify;

/// R1604 — header-map `get` hops (the scan attributes `resp.headers().get(..)` to the chain's root type) are in-memory.
#[test]
fn r1604_header_map_get_is_not_net() {
    for p in [
        "reqwest::header::HeaderMap::get",
        "reqwest::Response::get",
        "reqwest::blocking::Response::get",
        "reqwest::Request::get",
        "reqwest::blocking::Request::get",
    ] {
        assert_eq!(classify("reqwest", p), None, "{p}");
    }
    for p in [
        "isahc::Response::get",
        "isahc::http::Response::get",
        "isahc::http::HeaderMap::get",
        "isahc::http::header::HeaderMap::get",
    ] {
        assert_eq!(classify("isahc", p), None, "{p}");
    }
}

/// R1604 — the twins: every real dispatch and the URL-naming builders keep `Net`.
#[test]
fn r1604_http_dispatch_twins_stay_net() {
    for p in [
        "reqwest::Client::get",
        "reqwest::Client::head",
        "reqwest::Client::send",
        "reqwest::blocking::Client::get",
        "reqwest::blocking::Client::send",
        "reqwest::get",
        "reqwest::blocking::get",
        "reqwest::RequestBuilder::send",
    ] {
        assert_eq!(classify("reqwest", p), Some("Net"), "{p}");
    }
    // the body drain on the same Response type stays charged
    assert_eq!(classify("reqwest", "reqwest::Response::json"), Some("Net"));
    assert_eq!(classify("reqwest", "reqwest::Response::text"), Some("Net"));
    for p in ["isahc::get", "isahc::HttpClient::get", "isahc::HttpClient::post", "isahc::HttpClient::send"] {
        assert_eq!(classify("isahc", p), Some("Net"), "{p}");
    }
    // NOT removed: `isahc::Request::{get,post}` is an in-memory `http` builder, but in `Request::post(u).body(b)?.send()?` the
    // `.send()` hop reaches no rule, so this is the only `Net` on code that sends (EXECUTED). Over-charge kept on purpose.
    for p in ["isahc::Request::get", "isahc::Request::post", "isahc::http::Request::get"] {
        assert_eq!(classify("isahc", p), Some("Net"), "{p}");
    }
}

/// R1604 — lettre's stub transport does nothing; the SMTP, generic, File and Sendmail twins keep their effect.
#[test]
fn r1604_lettre_stub_send_is_not_net() {
    for p in [
        "lettre::transport::stub::StubTransport::send",
        "lettre::transport::stub::StubTransport::send_raw",
        "lettre::transport::stub::AsyncStubTransport::send",
        "lettre::transport::stub::AsyncStubTransport::send_raw",
    ] {
        assert_eq!(classify("lettre", p), None, "{p}");
    }
    for p in [
        "lettre::SmtpTransport::send",
        "lettre::SmtpTransport::send_raw",
        "lettre::AsyncSmtpTransport::send",
        "lettre::Transport::send",
        "lettre::AsyncTransport::send",
    ] {
        assert_eq!(classify("lettre", p), Some("Net"), "{p}");
    }
    assert_eq!(classify("lettre", "lettre::FileTransport::send"), Some("Fs"));
    assert_eq!(classify("lettre", "lettre::SendmailTransport::send"), Some("Exec"));
}

/// R1604 — tungstenite `WebSocket<S>` I/O keeps `Net` for every `S` (the scan cannot see `S`; a socket is the common case).
#[test]
fn r1604_tungstenite_stream_io_stays_net() {
    for m in ["send", "read", "write", "flush", "close"] {
        assert_eq!(classify("tungstenite", &format!("tungstenite::WebSocket::{m}")), Some("Net"), "{m}");
        assert_eq!(classify("tungstenite", &format!("tungstenite::protocol::WebSocket::{m}")), Some("Net"), "{m}");
    }
    assert_eq!(classify("tungstenite", "tungstenite::protocol::WebSocketContext::write"), Some("Net"));
}

/// R1605 — TRIPWIRE. The crate-root spellings of sea_query builders are charged `Db` for pure work, and that charge is the
/// only `Db` on `Entity::find().filter(Condition::all()..).all(db)` with a consumer-declared entity (EXECUTED: a SELECT runs;
/// without the charge the fn leaves `functions[]` in both arms). Remove these only together with a fix that charges the
/// untyped terminal; this test is the reminder.
#[test]
fn r1605_sea_orm_root_builders_keep_db_until_the_terminal_is_typed() {
    for p in [
        "sea_orm::Condition::all",
        "sea_orm::query::Condition::all",
        "sea_orm::entity::prelude::Expr::all",
        "sea_orm::entity::prelude::Expr::count",
        "sea_orm::ExprTrait::count",
    ] {
        assert_eq!(classify("sea_orm", p), Some("Db"), "{p}");
    }
    // the long spelling was already excluded (sweep [5]) and stays so
    assert_eq!(classify("sea_orm", "sea_orm::sea_query::Condition::all"), None);
    // the typed terminal
    assert_eq!(classify("sea_orm", "sea_orm::Select::all"), Some("Db"));
}
