use libunbound_sys::*;
use std::ffi::{c_int, CStr, CString};
use trust_dns_proto::error::ProtoResult;
use trust_dns_proto::op::response_code::ResponseCode;
use trust_dns_proto::rr::record_type::RecordType;
use trust_dns_proto::rr::{DNSClass, RData};
use trust_dns_proto::serialize::binary::{BinDecoder, Restrict};

#[derive(Debug, Clone, Copy)]
pub enum Error {
    Sys(ub_ctx_err),
    InvalidName,
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
}

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
    /// Is NULL if the result is not bogus.
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
