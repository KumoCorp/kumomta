use libunbound_sys::*;
use std::ffi::{c_int, CStr, CString};
use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::oneshot::error::RecvError;
use tokio::sync::oneshot::{channel, Sender};
use trust_dns_proto::error::ProtoResult;
use trust_dns_proto::op::response_code::ResponseCode;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::{DNSClass, RData};
use trust_dns_proto::serialize::binary::{BinDecoder, Restrict};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unbound error code {0}")]
    Sys(ub_ctx_err),
    #[error("DNS name has an embedded NUL character")]
    InvalidName,
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Error waiting for query result: {0}")]
    Recv(#[from] RecvError),
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
            eprintln!("deleting Context");
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
    pub fn new() -> Option<Self> {
        openssl::init();
        let ctx = unsafe { ub_ctx_create() };
        if ctx.is_null() {
            None
        } else {
            Some(Self { ctx })
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

    /// Convert the context into an async resolving context.
    /// A helper thread will be spawned to manage the context,
    /// and will automatically shut down when the returned
    /// AsyncContext is dropped.
    pub fn into_async(self) -> Result<AsyncContext, Error> {
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
