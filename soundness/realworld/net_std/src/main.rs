// CALIBRATED-Net recall probe: std::net connects to a marker IP (192.0.2.1, RFC5737 TEST-NET, non-
// routable — the connect() syscall fires regardless of whether it succeeds). candor must predict Net.
use std::net::TcpStream;
use std::time::Duration;

fn do_net() {
    // marker: 192.0.2.1
    let _ = TcpStream::connect_timeout(
        &"192.0.2.1:80".parse().unwrap(),
        Duration::from_millis(200),
    );
}

fn main() {
    do_net();
}
