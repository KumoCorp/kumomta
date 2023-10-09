use std::collections::HashSet;
use std::path::PathBuf;

fn new_build() -> cc::Build {
    let mut cfg = cc::Build::new();
    cfg.warnings(false);
    cfg.extra_warnings(false);
    cfg.flag_if_supported("-Wno-deprecated-declarations");

    println!("cargo:rerun-if-env-changed=DEP_OPENSSL_INCLUDE");
    if let Some(path) = std::env::var_os("DEP_OPENSSL_INCLUDE") {
        if let Some(path) = std::env::split_paths(&path).next() {
            if let Some(path) = path.to_str() {
                if !path.is_empty() {
                    cfg.include(path);
                }
            }
        }
    }

    if let Some(lib_root) = std::env::var_os("DEP_OPENSSL_ROOT") {
        println!(
            "cargo:rustc-link-search=native={}",
            PathBuf::from(lib_root).join("lib").to_str().unwrap()
        );
    }

    cfg
}

struct Probed {
    defined: HashSet<String>,
}

impl Probed {
    fn new() -> Self {
        Self {
            defined: HashSet::new(),
        }
    }

    fn try_compile(&self, code: &str) -> std::io::Result<bool> {
        let temp = tempfile::TempDir::new()?;
        let main_c = temp.path().join("main.c");
        std::fs::write(&main_c, format!("{code}"))?;

        let mut cfg = cc::Build::new();
        cfg.cargo_metadata(false);
        if let Some(path) = std::env::var_os("DEP_OPENSSL_INCLUDE") {
            eprintln!("DEP_OPENSSL_INCLUDE is {path:?}");
            if let Some(path) = std::env::split_paths(&path).next() {
                if let Some(path) = path.to_str() {
                    if !path.is_empty() {
                        eprintln!("adding {path} to includes");
                        cfg.include(path);
                    }
                }
            }
        }

        let mut cmd = cfg.get_compiler().to_command();
        cmd.current_dir(temp.path());

        // Source file first
        cmd.arg(&main_c);

        // then libraries
        if let Ok(lib_root) = std::env::var("DEP_OPENSSL_ROOT") {
            eprintln!("DEP_OPENSSL_ROOT is {lib_root:?}");
            cmd.arg(&format!("-L{lib_root}/lib"));
        }
        cmd.arg("-lssl");
        cmd.arg("-lcrypto");

        cmd.stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .output()
            .map(|o| o.status.success())
    }

    fn check_header(&mut self, cfg: &mut cc::Build, name: &str) -> std::io::Result<()> {
        let def = name.to_uppercase().replace(".", "_").replace("/", "_");
        let def = format!("HAVE_{def}");
        eprintln!("checking for <{name}>");
        if self.try_compile(&format!(
            r#"
#include <{name}>
int main(void) {{
    return 0;
}}
"#
        ))? {
            eprintln!("defining {def}");
            cfg.define(&def, Some("1"));
            self.defined.insert(def);
        }

        Ok(())
    }

    fn check_type(&mut self, cfg: &mut cc::Build, name: &str, header: &str) -> std::io::Result<()> {
        let def = name.to_uppercase().replace(".", "_").replace("/", "_");
        let def = format!("HAVE_{def}");
        eprintln!("checking for <{name}>");
        if self.try_compile(&format!(
            r#"
#include <{header}>
int main(void) {{
    return sizeof({name});
}}
"#
        ))? {
            eprintln!("defining {def}");
            cfg.define(&def, Some("1"));
            self.defined.insert(def);
        }

        Ok(())
    }

    fn check_func(&mut self, cfg: &mut cc::Build, func: &str) -> std::io::Result<bool> {
        let def = func.to_uppercase();
        let def = format!("HAVE_{def}");
        eprintln!("checking for function {func}");
        if self.try_compile(&format!(
            r#"
extern int {func}();
int main(void) {{
    return {func}();
}}
"#
        ))? {
            eprintln!("defining {def}");
            cfg.define(&def, Some("1"));
            self.defined.insert(def);
            Ok(true)
        } else {
            Ok(false)
        }
    }
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
        "compat/arc4_lock.c",
        "compat/arc4random.c",
        "compat/arc4random_uniform.c",
        // "compat/fake-rfc2553.c",
        //"compat/malloc.c",
        //"compat/memcmp.c",
        //"compat/memmove.c",
        //"compat/reallocarray.c",
        "compat/sha512.c",
    ] {
        cfg.file(&format!("unbound/{f}"));
    }

    let mut probe = Probed::new();

    for hdr in &[
        "TargetConditionals.h",
        "arpa/inet.h",
        "bsd/stdlib.h",
        "bsd/string.h",
        "dlfcn.h",
        "endian.h",
        "event.h",
        "expat.h",
        "getopt.h",
        "glob.h",
        "grp.h",
        "hiredis/hiredis.h",
        "ifaddrs.h",
        "inttypes.h",
        "iphlpapi.h",
        "libkern/OSByteOrder.h",
        "linux/net_tstamp.h",
        "login_cap.h",
        "memory.h",
        "net/if.h",
        "netdb.h",
        "netinet/in.h",
        "netinet/tcp.h",
        "netioapi.h",
        "nettle/dsa-compat.h",
        "nettle/eddsa.h",
        "nghttp2/nghttp2.h",
        "openssl/bn.h",
        "openssl/conf.h",
        "openssl/core_names.h",
        "openssl/dh.h",
        "openssl/dsa.h",
        "openssl/engine.h",
        "openssl/err.h",
        "openssl/param_build.h",
        "openssl/rand.h",
        "openssl/rsa.h",
        "openssl/ssl.h",
        "poll.h",
        "pwd.h",
        "stdarg.h",
        "stdbool.h",
        "stdint.h",
        "stdlib.h",
        "string.h",
        "strings.h",
        "sys/endian.h",
        "sys/ipc.h",
        "sys/param.h",
        "sys/resource.h",
        "sys/select.h",
        "sys/sha2.h",
        "sys/shm.h",
        "sys/socket.h",
        "sys/stat.h",
        "sys/sysctl.h",
        "sys/types.h",
        "sys/uio.h",
        "sys/un.h",
        "sys/wait.h",
        "syslog.h",
        "time.h",
        "unistd.h",
        "vfork.h",
        "windows.h",
        "winsock2.h",
        "ws2tcpip.h",
    ] {
        probe.check_header(&mut cfg, hdr).unwrap();
    }

    for func in &[
        "BIO_set_callback_ex",
        "CRYPTO_THREADID_set_callback",
        "CRYPTO_cleanup_all_ex_data",
        "DSA_SIG_set0",
        "ENGINE_cleanup",
        "ERR_free_strings",
        "ERR_load_crypto_strings",
        "EVP_DigestVerify",
        "EVP_EncryptInit_ex",
        "EVP_MAC_CTX_set_params",
        "EVP_MD_CTX_new",
        "EVP_aes_256_cbc",
        "EVP_cleanup",
        "EVP_default_properties_is_fips_enabled",
        "EVP_dss1",
        "EVP_sha1",
        "EVP_sha256",
        "EVP_sha512",
        "FIPS_mode",
        "HMAC_Init_ex",
        "OPENSSL_config",
        "OPENSSL_init_crypto",
        "OPENSSL_init_ssl",
        "OSSL_PARAM_BLD_new",
        "OpenSSL_add_all_digests",
        "RAND_cleanup",
        "SHA512_Update",
        "SSL_CTX_set_alpn_protos",
        "SSL_CTX_set_alpn_select_cb",
        "SSL_CTX_set_ciphersuites",
        "SSL_CTX_set_security_level",
        "SSL_CTX_set_tlsext_ticket_key_evp_cb",
        "SSL_get0_alpn_selected",
        "SSL_get0_peername",
        "SSL_get1_peer_certificate",
        "SSL_set1_host",
        "X509_VERIFY_PARAM_set1_host",
        "_beginthreadex",
        "accept4",
        "be64toh",
        "chown",
        "chroot",
        "daemon",
        "endprotoent",
        "endpwent",
        "endservent",
        "ev_default_loop",
        "ev_loop",
        "event_assign",
        "event_base_free",
        "event_base_get_method",
        "event_base_new",
        "event_base_once",
        "fcntl",
        "fork",
        "fsync",
        "getaddrinfo",
        "getauxval",
        "getentropy",
        "getifaddrs",
        "getpwnam",
        "getrlimit",
        "gettid",
        "glob",
        "htobe64",
        "if_nametoindex",
        "ioctlsocket",
        "initgroups",
        "kill",
        "localtime_r",
        "memmove",
        "poll",
        "reallocarray",
        "random",
        "recvmsg",
        "sendmsg",
        "setregid",
        "setresgid",
        "setresuid",
        "malloc",
        "setreuid",
        "setrlimit",
        "setsid",
        "setusercontext",
        "shmget",
        "sigprocmask",
        "sleep",
        "socketpair",
        "srandom",
        "strftime",
        "strptime",
        "tzset",
        "usleep",
        "vfork",
        "writev",
    ] {
        probe.check_func(&mut cfg, func).unwrap();
    }

    if cfg!(target_os = "macos") {
        cfg.define("HAVE_ISBLANK", Some("1"));
    } else {
        if !probe.check_func(&mut cfg, "isblank").unwrap() {
            cfg.file(&format!("unbound/compat/isblank.c"));
        }
    }

    for func in &[
        //"arc4random",
        //"arc4random_uniform",
        "ctime_r",
        "gmtime_r",
        "explicit_bzero",
        "strlcat",
        "strlcpy",
        "inet_ntop",
        "inet_pton",
        "inet_aton",
        "malloc",
        "memcmp",
        "memmove",
        "reallocarray",
        "strsep",
        "snprintf",
        "strptime",
    ] {
        if !probe.check_func(&mut cfg, func).unwrap() {
            cfg.file(&format!("unbound/compat/{func}.c"));
        }
    }

    for t in &["pthread_rwlock_t", "pthread_spinlock_t"] {
        probe.check_type(&mut cfg, t, "pthread.h").unwrap();
    }

    if !probe.defined.contains("HAVE_GETENTROPY") {
        let name = if cfg!(target_os = "macos") {
            "compat/getentropy_osx.c"
        } else {
            "compat/getentropy_linux.c"
        };
        //"compat/getentropy_freebsd.c",
        //"compat/getentropy_solaris.c",
        //"compat/getentropy_win.c",
        cfg.file(&format!("unbound/{name}"));
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
