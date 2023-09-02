use libunbound::*;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::DNSClass;

fn main() {
    let ctx = Context::new().expect("Context to be created");
    ctx.load_resolv_conf(None).unwrap();
    ctx.set_debug_level(DebugLevel::Detailed).unwrap();
    let result = ctx
        .resolve("_25._tcp.do.havedane.net", RecordType::TLSA, DNSClass::IN)
        .unwrap();
    println!("{result:#?}");
    ctx.print_local_zones().unwrap();
}
