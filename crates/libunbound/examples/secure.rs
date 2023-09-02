use libunbound::*;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::DNSClass;

fn main() {
    let ctx = Context::new().expect("Context to be created");
    ctx.bootstrap_trust_anchor_file("/tmp/root.dnssec.anchor")
        .unwrap();
    /* alternatively, if you don't want to touch local files:
    for a in ROOT_TRUST_ANCHORS {
        ctx.add_trust_anchor(a).unwrap();
    }
    */
    let result = ctx
        .resolve("_25._tcp.do.havedane.net", RecordType::TLSA, DNSClass::IN)
        .unwrap();
    println!("{result:#?}");
}
