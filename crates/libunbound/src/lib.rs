use libunbound_sys::*;
use std::ffi::{c_int, CStr, CString};
use std::io::Write;
use std::net::IpAddr;
use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::oneshot::error::RecvError;
use tokio::sync::oneshot::{channel, Sender};
use trust_dns_proto::error::ProtoResult;
use trust_dns_proto::op::response_code::ResponseCode;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::{DNSClass, RData};
use trust_dns_proto::serialize::binary::{BinDecoder, Restrict};

/// These are the root trust anchors at the time of writing.
/// See <https://www.nlnetlabs.nl/documentation/unbound/howto-anchor/>
/// for more information on anchors.
/// The data for these comes from:
/// <https://data.iana.org/root-anchors/root-anchors.xml>
pub const ROOT_TRUST_ANCHORS: &[&str] = &[
    ". IN DS 19036 8 2 49AAC11D7B6F6446702E54A1607371607A1A41855200FD2CE1CDDE32F24E8FB5",
    ". IN DS 20326 8 2 E06D44B80B8F1D39A95C0B0D7C65D08458E880409BBC683457104237C7F8EC8D",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unbound error: {}", unbound_error_string(*.0))]
    Sys(ub_ctx_err),
    #[error("DNS name has an embedded NUL character")]
    InvalidName,
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error waiting for query result: {0}")]
    Recv(#[from] RecvError),
    #[error("Failed to create Context")]
    ContextCreation,
}

pub fn unbound_error_string(err: ub_ctx_err) -> String {
    let res = unsafe { ub_strerror(err) };
    if res.is_null() {
        format!("[{err}]: Unknown error")
    } else {
        let s = unsafe { CStr::from_ptr(res) };
        let s = s.to_string_lossy();
        format!("[{err}]: {s}")
    }
}

pub struct Context {
    ctx: *mut ub_ctx,
}

// Context is internally thread safe
unsafe impl Sync for Context {}
unsafe impl Send for Context {}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            ub_ctx_delete(self.ctx);
        }
    }
}

/// The validation context is created to hold the resolver status,
/// validation keys and a small cache (containing messages, rrsets,
/// roundtrip times, trusted keys, lameness information).
impl Context {
    /// Create a resolving and validation context.
    /// The information from /etc/resolv.conf and /etc/hosts is not utilised by
    /// default.
    /// Use ub_ctx_resolvconf and ub_ctx_hosts to read them.
    pub fn new() -> Result<Self, Error> {
        openssl::init();
        let ctx = unsafe { ub_ctx_create() };
        if ctx.is_null() {
            Err(Error::ContextCreation)
        } else {
            Ok(Self { ctx })
        }
    }

    /// Perform resolution and validation of the target name.
    /// The context is finalized, and can no longer accept config changes.
    /// @param name: domain name in text format (a zero terminated text string).
    /// @param rrtype: type of RR
    /// @param rrclass: class of RR
    pub fn resolve(
        &self,
        name: &str,
        rrtype: RecordType,
        rrclass: DNSClass,
    ) -> Result<Answer, Error> {
        let rrclass: u16 = rrclass.into();
        let rrtype: u16 = rrtype.into();
        let name = CString::new(name).map_err(|_| Error::InvalidName)?;
        let mut result = std::ptr::null_mut();
        let err = unsafe {
            ub_resolve(
                self.ctx,
                name.as_ptr(),
                rrtype as c_int,
                rrclass as c_int,
                &mut result,
            )
        };
        if err == ub_ctx_err_UB_NOERROR {
            assert!(!result.is_null());
            Ok(Answer { result })
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Set an option for the context
    /// @param opt: option name from the unbound.conf config file format.
    /// (not all settings applicable). The name includes the trailing ':'
    /// for example
    ///
    /// ```rust
    /// let ctx = libunbound::Context::new().unwrap();
    /// ctx.set_option("logfile:", "mylog.txt");
    /// ```
    ///
    /// This is a power-users interface that lets you specify all sorts
    /// of options.
    ///
    /// For some specific options, such as adding trust anchors, special
    /// routines exist.
    ///
    /// @param val: value of the option.
    pub fn set_option(&self, opt: &str, value: &str) -> Result<(), Error> {
        let opt = CString::new(opt).map_err(|_| Error::InvalidName)?;
        let value = CString::new(value).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_set_option(self.ctx, opt.as_ptr(), value.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Get an option from the context.
    /// @param opt: option name from the unbound.conf config file format.
    /// (not all settings applicable). The name excludes the trailing ':'
    /// for example:
    ///
    /// ```rust
    /// let ctx = libunbound::Context::new().unwrap();
    /// ctx.get_option("logfile");
    /// ```
    ///
    /// This is a power-users interface that lets you specify all sorts
    /// of options.
    /// In cases with multiple entries (auto-trust-anchor-file),
    /// a newline delimited list is returned in the string.
    pub fn get_option(&self, opt: &str) -> Result<String, Error> {
        let opt = CString::new(opt).map_err(|_| Error::InvalidName)?;
        let mut result = std::ptr::null_mut();
        let err = unsafe { ub_ctx_get_option(self.ctx, opt.as_ptr(), &mut result) };

        if err == ub_ctx_err_UB_NOERROR {
            assert!(!result.is_null());
            let s = unsafe { CStr::from_ptr(result) };
            let value = s.to_string_lossy().to_string();
            unsafe { libc::free(result as *mut libc::c_void) };
            Ok(value)
        } else {
            Err(Error::Sys(err))
        }
    }

    /// setup configuration for the given context.
    /// @param config_file_name: unbound config file (not all settings applicable).
    /// This is a power-users interface that lets you specify all sorts of options.
    /// For some specific options, such as adding trust anchors, special routines exist.
    pub fn load_unbound_config_file(&self, config_file_name: &str) -> Result<(), Error> {
        let fname = CString::new(config_file_name).map_err(|_| Error::InvalidName)?;

        let err = unsafe { ub_ctx_config(self.ctx, fname.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Set machine to forward DNS queries to, the caching resolver to use.
    /// IP4 or IP6 address. Forwards all DNS requests to that machine, which
    /// is expected to run a recursive resolver. If the proxy is not DNSSEC-capable,
    /// validation may fail. Can be called several times, in that case the addresses
    /// are used as backup servers.
    /// To read the list of nameservers from /etc/resolv.conf (from DHCP or so),
    /// use the call ub_ctx_resolvconf.
    /// At this time it is only possible to set configuration before the\n\tfirst resolve is done.
    /// If the addr is None, forwarding is disabled.
    pub fn set_forward(&self, addr: Option<IpAddr>) -> Result<(), Error> {
        let err = match addr {
            Some(addr) => {
                let addr = CString::new(addr.to_string()).map_err(|_| Error::InvalidName)?;
                unsafe { ub_ctx_set_fwd(self.ctx, addr.as_ptr()) }
            }
            None => unsafe { ub_ctx_set_fwd(self.ctx, std::ptr::null()) },
        };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Use DNS over TLS to send queries to machines set with set_forward().
    /// At this time it is only possible to set configuration before the first resolve is done.
    /// @param tls: enable or disable DNS over TLS
    pub fn set_tls(&self, tls: bool) -> Result<(), Error> {
        let err = unsafe { ub_ctx_set_tls(self.ctx, if tls { 1 } else { 0 }) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Add a stub zone, with given address to send to.  This is for custom
    /// root hints or pointing to a local authoritative dns server.
    /// For dns resolvers and the 'DHCP DNS' ip address, use set_forward.
    /// This is similar to a stub-zone entry in unbound.conf.
    ///
    /// It is only possible to set configuration before the first resolve is done.
    /// @param zone: name of the zone, string.
    /// @param addr: address, IP4 or IP6 in string format.
    /// The addr is added to the list of stub-addresses if the entry exists.
    /// If the addr is None the stub entry is removed.
    /// @param isprime: set to true to set stub-prime to yes for the stub.
    /// For local authoritative servers, people usually set it to false,
    /// For root hints it should be set to true.
    pub fn set_stub(&self, zone: &str, addr: Option<IpAddr>, is_prime: bool) -> Result<(), Error> {
        let zone = CString::new(zone).map_err(|_| Error::InvalidName)?;
        let is_prime = if is_prime { 1 } else { 0 };

        let err = match addr {
            Some(addr) => {
                let addr = CString::new(addr.to_string()).map_err(|_| Error::InvalidName)?;
                unsafe { ub_ctx_set_stub(self.ctx, zone.as_ptr(), addr.as_ptr(), is_prime) }
            }
            None => unsafe { ub_ctx_set_stub(self.ctx, zone.as_ptr(), std::ptr::null(), is_prime) },
        };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Read list of nameservers to use from the filename given.
    /// Usually `"/etc/resolv.conf"`. Uses those nameservers as caching proxies.
    /// If they do not support DNSSEC, validation may fail.
    /// Only nameservers are picked up, the searchdomain, ndots and other
    /// settings from resolv.conf(5) are ignored.
    /// At this time it is only possible to set configuration before the first resolve is done.
    /// n @param fname: file name string. If None `"/etc/resolv.conf"` is used.
    pub fn load_resolv_conf(&self, file_name: Option<&str>) -> Result<(), Error> {
        let err = match file_name {
            Some(name) => {
                let name = CString::new(name).map_err(|_| Error::InvalidName)?;
                unsafe { ub_ctx_resolvconf(self.ctx, name.as_ptr()) }
            }
            None => unsafe { ub_ctx_resolvconf(self.ctx, std::ptr::null()) },
        };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Read list of hosts from the filename given.
    /// Usually `"/etc/hosts"`
    /// These addresses are not flagged as DNSSEC secure when queried for.
    /// At this time it is only possible to set configuration before the first resolve is done.
    /// @param fname: file name string. If None `"/etc/hosts"` is used.
    pub fn load_hosts(&self, file_name: Option<&str>) -> Result<(), Error> {
        let err = match file_name {
            Some(name) => {
                let name = CString::new(name).map_err(|_| Error::InvalidName)?;
                unsafe { ub_ctx_hosts(self.ctx, name.as_ptr()) }
            }
            None => unsafe { ub_ctx_hosts(self.ctx, std::ptr::null()) },
        };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Add a trust anchor to the given context.
    /// The trust anchor is a string, on one line, that holds a valid DNSKEY or
    /// DS RR.
    /// tAt this time it is only possible to add trusted keys before the
    /// first resolve is done.
    ///
    /// @param ta: string, with zone-format RR on one line.
    ///
    /// `[domainname] [TTL optional] [type] [class optional] [rdata contents]`
    pub fn add_trust_anchor(&self, ta: &str) -> Result<(), Error> {
        let ta = CString::new(ta).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_add_ta(self.ctx, ta.as_ptr()) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Add the built-in trust anchors to the context.
    /// These anchors were correct at the time that this module was authored,
    /// however, for robust DNSSEC, you should consider bootstrapping an
    /// anchor file and loading it via load_trust_anchor_file() if you want
    /// stronger assertions that these anchors are correct and to keep them
    /// updated.
    pub fn add_builtin_trust_anchors(&self) -> Result<(), Error> {
        for a in ROOT_TRUST_ANCHORS {
            self.add_trust_anchor(a)?;
        }
        Ok(())
    }

    /// Add trust anchors to the given context.
    /// Pass name of a file with DS and DNSKEY records (like from dig or drill).
    /// At this time it is only possible to add trusted keys before the
    /// first resolve is done.
    /// @param fname: filename of file with keyfile with trust anchors.
    pub fn load_trust_anchor_file(&self, file_name: &str) -> Result<(), Error> {
        let fname = CString::new(file_name).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_add_ta_file(self.ctx, fname.as_ptr()) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Use file_name to maintain root trust anchors for DNSSEC.
    /// If the file doesn't already exist, it will be populated with
    /// the built-in ROOT_TRUST_ANCHORS defined by this module.
    pub fn bootstrap_trust_anchor_file(&self, file_name: &str) -> Result<(), Error> {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(file_name)
        {
            f.write_all(ROOT_TRUST_ANCHORS.join("\n").as_bytes())?;
        }

        self.load_trust_anchor_file(file_name)
    }

    /// Add trust anchor to the given context that is tracked with RFC5011
    /// automated trust anchor maintenance.
    /// The file is written to when the trust anchor is changed.
    /// Pass the name of a file that was output from eg. unbound-anchor,
    /// or you can start it by providing a trusted DNSKEY or DS record on one
    /// line in the file.
    /// At this time it is only possible to add trusted keys before the
    /// first resolve is done.
    /// @param fname: filename of file with trust anchor.
    pub fn load_trust_anchor_file_with_auto_update(&self, file_name: &str) -> Result<(), Error> {
        let fname = CString::new(file_name).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_add_ta_autr(self.ctx, fname.as_ptr()) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Set debug output (and error output) to the specified stream.
    /// Pass NULL to disable. Default is stderr.
    /// @param out: FILE* out file stream to log to.
    pub fn set_debug_output(&self, stream: *mut libc::FILE) -> Result<(), Error> {
        let err = unsafe { ub_ctx_debugout(self.ctx, stream as *mut _) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Set debug verbosity for the context
    /// Output is directed to stderr or whatever was configured via set_debug_output().
    /// @param d: debug level, 0 is off, 1 is very minimal, 2 is detailed, and 3 is lots.
    pub fn set_debug_level(&self, level: DebugLevel) -> Result<(), Error> {
        let err = unsafe { ub_ctx_debuglevel(self.ctx, level as _) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Debug routine.  Print the local zone information to debug output.
    /// Is finalized by the routine.
    pub fn print_local_zones(&self) -> Result<(), Error> {
        let err = unsafe { ub_ctx_print_local_zones(self.ctx) };
        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Add a new zone with the zonetype to the local authority info of the
    /// library.
    /// Is finalized by the routine.
    /// @param zone_name: name of the zone in text, `"example.com"`
    /// tIf it already exists, the type is updated.
    /// @param zone_type: type of the zone (like for unbound.conf) in text.
    pub fn zone_add(&self, zone_name: &str, zone_type: &str) -> Result<(), Error> {
        let zone_name = CString::new(zone_name).map_err(|_| Error::InvalidName)?;
        let zone_type = CString::new(zone_type).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_zone_add(self.ctx, zone_name.as_ptr(), zone_type.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Remove zone from local authority info of the library.
    /// Is finalized by the routine.
    /// @param zone_name: name of the zone in text, `"example.com\"`
    /// If it does not exist, nothing happens.
    pub fn zone_remove(&self, zone_name: &str) -> Result<(), Error> {
        let zone_name = CString::new(zone_name).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_zone_remove(self.ctx, zone_name.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Add localdata to the library local authority info.
    /// Similar to local-data config statement.
    /// @param data: the resource record in text format, for example
    /// `"www.example.com IN A 127.0.0.1"`
    pub fn add_local_data(&self, data: &str) -> Result<(), Error> {
        let data = CString::new(data).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_data_add(self.ctx, data.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Remove localdata from the library local authority info.
    /// @param data: the name to delete all data from, like `"www.example.com"
    pub fn remove_local_data(&self, data: &str) -> Result<(), Error> {
        let data = CString::new(data).map_err(|_| Error::InvalidName)?;
        let err = unsafe { ub_ctx_data_remove(self.ctx, data.as_ptr()) };

        if err == ub_ctx_err_UB_NOERROR {
            Ok(())
        } else {
            Err(Error::Sys(err))
        }
    }

    /// Convert the context into an async resolving context.
    /// A helper thread will be spawned to manage the context,
    /// and will automatically shut down when the returned
    /// AsyncContext is dropped.
    pub fn into_async(self) -> Result<AsyncContext, Error> {
        // Use a thread rather than forking a worker for unbound
        unsafe {
            ub_ctx_async(self.ctx, 1);
        }

        // Note that while libunbound does support exporting its
        // fd for monitoring by eg: mio or AsyncFd in tokio,
        // I've found that it is perpetually ready for read
        // when there are no pending queries, so we use an
        // old-school condvar and count of pending queries
        // to avoid busy waiting in the helper task.
        // As a result, we're using thread for that purpose.
        let inner = Arc::new(Inner {
            context: self,
            condvar: Condvar::new(),
            predicate: Mutex::new(0),
        });

        let processor = Arc::clone(&inner);

        std::thread::Builder::new()
            .name("libunbound worker".to_string())
            .spawn(move || {
                loop {
                    if Arc::strong_count(&processor) == 1 {
                        // We are the only remaining ref; no further
                        // queries are possible
                        break;
                    }

                    // Wait for there to be any queries
                    {
                        let mut pred_lock = processor.predicate.lock().unwrap();
                        while *pred_lock == 0 {
                            pred_lock = processor.condvar.wait(pred_lock).unwrap();
                        }
                    }

                    // Wait for and process result(s).
                    // This will satisfy multiple read answers
                    let err = unsafe { ub_wait(processor.context.ctx) };
                    if err != ub_ctx_err_UB_NOERROR {
                        // Most likely error is a broken pipe in the
                        // case where the context is being destroyed
                        break;
                    }
                }
            })?;

        Ok(AsyncContext { inner })
    }
}

struct Inner {
    context: Context,
    condvar: Condvar,
    predicate: Mutex<usize>,
}

impl Inner {
    pub fn increment(&self) {
        let mut pred_lock = self.predicate.lock().unwrap();
        *pred_lock += 1;
        self.condvar.notify_one();
    }

    pub fn decrement(&self) {
        let mut pred_lock = self.predicate.lock().unwrap();
        *pred_lock -= 1;
    }
}

pub struct AsyncContext {
    inner: Arc<Inner>,
}

impl Drop for AsyncContext {
    fn drop(&mut self) {
        // Wake up the helper so that it can wind itself down
        // when we are dropped
        self.inner.increment();
    }
}

impl AsyncContext {
    /// Asynchronously perform resolution and validation of the target name.
    /// @param name: domain name in text format (a zero terminated text string).
    /// @param rrtype: type of RR
    /// @param rrclass: class of RR
    pub async fn resolve(
        &self,
        name: &str,
        rrtype: RecordType,
        rrclass: DNSClass,
    ) -> Result<Answer, Error> {
        let rrclass: u16 = rrclass.into();
        let rrtype: u16 = rrtype.into();
        let name = CString::new(name).map_err(|_| Error::InvalidName)?;

        let (tx, rx) = channel::<Result<Answer, Error>>();

        // The outstanding query takes a strong ref to the context
        // to ensure that the helper thread isn't stopped before
        // completion
        let state = Box::new(AsyncQueryState {
            tx,
            inner: Arc::clone(&self.inner),
        });

        let mut id = 0;
        {
            let state_ptr = Box::into_raw(state);
            let err = unsafe {
                ub_resolve_async(
                    self.inner.context.ctx,
                    name.as_ptr(),
                    rrtype as c_int,
                    rrclass as c_int,
                    state_ptr as *mut _,
                    Some(Self::process_result),
                    &mut id,
                )
            };
            if err != ub_ctx_err_UB_NOERROR {
                // Reclaim ownership of the query state pointer so
                // that we don't leak it
                let state: Box<AsyncQueryState> = unsafe { Box::from_raw(state_ptr) };
                drop(state);

                return Err(Error::Sys(err));
            }
        }

        // Hold the id so that we can cancel on drop
        let mut id = AsyncId {
            id: Some(id),
            context: self,
        };

        self.inner.increment();

        let res = rx.await?;

        // Query completed; no need to cancel after this point
        id.disarm();

        res
    }

    /// This function is called by unbound when a query completes
    unsafe extern "C" fn process_result(
        my_arg: *mut ::std::os::raw::c_void,
        err: ::std::os::raw::c_int,
        result: *mut ub_result,
    ) {
        let state = Box::from_raw(my_arg as *mut AsyncQueryState);

        let result = if err == ub_ctx_err_UB_NOERROR {
            assert!(!result.is_null());
            Ok(Answer { result })
        } else {
            assert!(result.is_null());
            Err(Error::Sys(err))
        };
        state.tx.send(result).ok();

        state.inner.decrement();
    }
}

struct AsyncQueryState {
    tx: Sender<Result<Answer, Error>>,
    inner: Arc<Inner>,
}

/// Helper for cancelling a lookup
struct AsyncId<'a> {
    id: Option<c_int>,
    context: &'a AsyncContext,
}

impl<'a> AsyncId<'a> {
    /// Set the state to prevent cancelling the query
    fn disarm(&mut self) {
        self.id.take();
    }
}

impl<'a> Drop for AsyncId<'a> {
    fn drop(&mut self) {
        // Cancel the query on drop
        if let Some(id) = self.id.take() {
            unsafe {
                ub_cancel(self.context.inner.context.ctx, id);
            }
        }
    }
}

/// The validation and resolution results.
pub struct Answer {
    result: *mut ub_result,
}

// Answer is internally thread safe
unsafe impl Sync for Answer {}
unsafe impl Send for Answer {}

impl Drop for Answer {
    fn drop(&mut self) {
        unsafe {
            ub_resolve_free(self.result);
        }
    }
}

impl Answer {
    fn answer(&self) -> &ub_result {
        unsafe { &*self.result }
    }

    /// The original question, name text string.
    pub fn qname(&self) -> &str {
        let cstr = unsafe { CStr::from_ptr(self.answer().qname) };
        cstr.to_str()
            .expect("original resolve param was str, so this should be too")
    }

    /// the type asked for
    pub fn qtype(&self) -> RecordType {
        (self.answer().qtype as u16).into()
    }

    /// the class asked for
    pub fn qclass(&self) -> DNSClass {
        DNSClass::from_u16(self.answer().qclass as u16)
            .expect("original rrclass was DNSClass, so this should be too")
    }

    /// canonical name for the result (the final cname).
    pub fn canon_name(&self) -> Option<&str> {
        let ptr = self.answer().canonname;
        if ptr.is_null() {
            None
        } else {
            let cstr = unsafe { CStr::from_ptr(ptr) };
            Some(
                cstr.to_str()
                    .expect("DNS names should be ASCII, so this should be too"),
            )
        }
    }

    /// DNS RCODE for the result. May contain additional error code if
    /// there was no data due to an error, ResponseCode::NoError if OK.
    pub fn rcode(&self) -> ResponseCode {
        (self.answer().rcode as u16).into()
    }

    /// The DNS answer packet. Network formatted. Can contain DNSSEC types.
    pub fn answer_packet(&self) -> &[u8] {
        let answer = self.answer();
        unsafe {
            std::slice::from_raw_parts(
                answer.answer_packet as *const u8,
                answer.answer_len as usize,
            )
        }
    }

    /// If there is any data, this is true.
    /// If false, there was no data (nxdomain may be true, rcode can be set).
    pub fn have_data(&self) -> bool {
        self.answer().havedata != 0
    }

    /// If there was no data, and the domain did not exist, this is true.
    /// If it is false, and there was no data, then the domain name
    /// is purported to exist, but the requested data type is not available.
    pub fn nxdomain(&self) -> bool {
        self.answer().nxdomain != 0
    }

    /// True, if the result is validated securely.
    /// False, if validation failed or domain queried has no security info.
    /// It is possible to get a result with no data (havedata is false),
    /// and secure is true. This means that the non-existence of the data
    /// was cryptographically proven (with signatures).
    pub fn secure(&self) -> bool {
        self.answer().secure != 0
    }

    /// If the result was not secure (secure==0), and this result is due
    /// to a security failure, bogus is true.
    /// This means the data has been actively tampered with, signatures
    /// failed, expected signatures were not present, timestamps on
    /// signatures were out of date and so on.
    /// If !secure and !bogus, this can happen if the data is not secure
    /// because security is disabled for that domain name.
    /// This means the data is from a domain where data is not signed.
    pub fn bogus(&self) -> bool {
        self.answer().bogus != 0
    }

    /// If the result is bogus this contains a string
    /// that describes the failure.
    /// There may be other errors as well
    /// as the one described, the description may not be perfectly accurate.
    /// Is None if the result is not bogus.
    pub fn why_bogus(&self) -> Option<&str> {
        let ptr = self.answer().why_bogus;
        if ptr.is_null() {
            None
        } else {
            let cstr = unsafe { CStr::from_ptr(ptr) };
            Some(cstr.to_str().expect("expected UTF8"))
        }
    }

    /// If the query or one of its subqueries was ratelimited.  Useful if
    /// ratelimiting is enabled and answer to the client is SERVFAIL as a result.
    pub fn was_rate_limited(&self) -> bool {
        self.answer().was_ratelimited != 0
    }

    /// TTL for the result, in seconds.  If the security is bogus, then
    /// you also cannot trust this value.
    pub fn ttl(&self) -> u32 {
        self.answer().ttl as u32
    }

    /// Iterates over the rdata items
    pub fn rdata(&self) -> AnswerDataIter {
        AnswerDataIter {
            pos: 0,
            answer: self,
        }
    }
}

impl std::fmt::Debug for Answer {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_struct("Answer")
            .field("qname", &self.qname())
            .field("qtype", &self.qtype())
            .field("qclass", &self.qclass())
            .field("canon_name", &self.canon_name())
            .field("rcode", &self.rcode())
            .field("have_data", &self.have_data())
            .field("nxdomain", &self.nxdomain())
            .field("secure", &self.secure())
            .field("bogus", &self.bogus())
            .field("why_bogus", &self.why_bogus())
            .field("was_rate_limited", &self.was_rate_limited())
            .field("ttl", &self.ttl())
            .field("tdata", &self.rdata().collect::<Vec<_>>())
            .finish()
    }
}

pub struct AnswerDataIter<'a> {
    pos: isize,
    answer: &'a Answer,
}

impl<'a> std::iter::Iterator for AnswerDataIter<'a> {
    type Item = ProtoResult<RData>;

    fn next(&mut self) -> Option<ProtoResult<RData>> {
        let raw_data = unsafe {
            let answer = self.answer.answer();
            let offset = *answer.data.offset(self.pos);
            if offset.is_null() {
                return None;
            }
            let len = (*answer.len.offset(self.pos)) as usize;
            std::slice::from_raw_parts(offset as *const u8, len)
        };
        self.pos += 1;

        Some(RData::read(
            &mut BinDecoder::new(raw_data),
            self.answer.qtype(),
            Restrict::new(raw_data.len() as u16),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum DebugLevel {
    Off = 0,
    Minimal = 1,
    Detailed = 2,
    Lots = 3,
}
