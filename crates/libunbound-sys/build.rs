fn new_build() -> cc::Build {
    let mut cfg = cc::Build::new();
    cfg.warnings(false);
    cfg.extra_warnings(false);
    cfg
}

fn unbound() {
    let mut cfg = new_build();
    for f in [
        "services/cache/dns.c",
        "services/cache/infra.c",
        "services/cache/rrset.c",
        "util/as112.c",
        "util/data/dname.c",
        "util/data/msgencode.c",
        "util/data/msgparse.c",
        "util/data/msgreply.c",
        "util/data/packed_rrset.c",
        "iterator/iterator.c",
        "iterator/iter_delegpt.c",
        "iterator/iter_donotq.c",
        "iterator/iter_fwd.c",
        "iterator/iter_hints.c",
        "iterator/iter_priv.c",
        "iterator/iter_resptype.c",
        "iterator/iter_scrub.c",
        "iterator/iter_utils.c",
        "services/listen_dnsport.c",
        "services/localzone.c",
        "services/mesh.c",
        "services/modstack.c",
        "services/view.c",
        "services/rpz.c",
        "util/rfc_1982.c",
        "services/outbound_list.c",
        "services/outside_network.c",
        "util/alloc.c",
        "util/config_file.c",
        "util/configlexer.c",
        "util/configparser.c",
        // "util/shm_side/shm_main.c",
        "services/authzone.c",
        "util/fptr_wlist.c",
        "util/locks.c",
        "util/log.c",
        "util/mini_event.c",
        "util/module.c",
        "util/netevent.c",
        "util/net_help.c",
        "util/random.c",
        "util/rbtree.c",
        "util/regional.c",
        "util/rtt.c",
        "util/siphash.c",
        "util/edns.c",
        "util/storage/dnstree.c",
        "util/storage/lookup3.c",
        "util/storage/lruhash.c",
        "util/storage/slabhash.c",
        "util/tcp_conn_limit.c",
        "util/timehist.c",
        "util/tube.c",
        "util/proxy_protocol.c",
        "util/timeval_func.c",
        "util/winsock_event.c",
        "validator/autotrust.c",
        "validator/val_anchor.c",
        "validator/validator.c",
        "validator/val_kcache.c",
        "validator/val_kentry.c",
        "validator/val_neg.c",
        "validator/val_nsec3.c",
        "validator/val_nsec.c",
        "validator/val_secalgo.c",
        "validator/val_sigcrypt.c",
        "validator/val_utils.c",
        "dns64/dns64.c",
        /* subnet option
        "edns-subnet/edns-subnet.c",
        "edns-subnet/subnetmod.c",
        "edns-subnet/addrtree.c",
        "edns-subnet/subnet-whitelist.c",
        */
        "respip/respip.c",
        "libunbound/context.c",
        "libunbound/libunbound.c",
        "libunbound/libworker.c",
        "util/ub_event_pluggable.c",
        "sldns/keyraw.c",
        "sldns/sbuffer.c",
        "sldns/wire2str.c",
        "sldns/parse.c",
        "sldns/parseutil.c",
        "sldns/rrdef.c",
        "sldns/str2wire.c",
        "compat/strlcpy.c",
        "compat/arc4random.c",
        "compat/arc4_lock.c",
        "compat/arc4random_uniform.c",
    ] {
        cfg.file(&format!("unbound/{f}"));
    }

    let ptr_width_bits: usize = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH")
        .unwrap()
        .parse()
        .unwrap();
    let ptr_width_bytes = format!("{}", ptr_width_bits / 8);
    cfg.define("SIZEOF_SIZE_T", Some(ptr_width_bytes.as_str()));
    cfg.define("SIZEOF_TIME_T", Some(ptr_width_bytes.as_str()));
    cfg.include(".");
    cfg.include("unbound");

    cfg.compile("unbound");
}

fn main() {
    unbound();
}
