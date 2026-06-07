mod billing;

fn main() {
    billing::charge_customer(1999);
    billing::record_activity("login");
}
