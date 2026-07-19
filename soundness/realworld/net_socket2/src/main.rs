// HONESTY probe (UNCALIBRATED): socket2 raw TCP connect to a marker IP. Kernel shows connect(203.0.113.10)
// -> program does Net. candor must predict Net OR disclose Unknown/blind. Silent-pure = a real under-report.
use socket2::{Domain, Socket, Type};
fn dial() {
    // marker: 203.0.113.10
    if let Ok(s) = Socket::new(Domain::IPV4, Type::STREAM, None) {
        let addr: std::net::SocketAddr = "203.0.113.10:80".parse().unwrap();
        let _ = s.connect_timeout(&addr.into(), std::time::Duration::from_millis(300));
    }
}
fn main() { dial(); }
