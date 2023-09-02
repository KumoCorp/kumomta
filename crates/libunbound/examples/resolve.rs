use libunbound::*;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::DNSClass;

fn main() {
    let ctx = Context::new().expect("Context to be created");
    let result = ctx
        .resolve("_25._tcp.do.havedane.net", RecordType::TLSA, DNSClass::IN)
        .unwrap();
    println!("{result:#?}");
}
