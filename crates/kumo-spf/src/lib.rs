use crate::record::MacroName;
use crate::eval::EvalContext;
use crate::record::Record;
use crate::error::SpfError;
use std::net::IpAddr;

pub mod dns;
pub mod error;
pub mod eval;
pub mod record;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpfDisposition {
    /// A result of "none" means either (a) no syntactically valid DNS domain
    /// name was extracted from the SMTP session that could be used as the
    /// one to be authorized, or (b) no SPF records were retrieved from
    /// the DNS.
    None,

    /// A "neutral" result means the ADMD has explicitly stated that it is
    /// not asserting whether the IP address is authorized.
    Neutral,

    /// A "pass" result is an explicit statement that the client is
    /// authorized to inject mail with the given identity.
    Pass,

    /// A "fail" result is an explicit statement that the client is not
    /// authorized to use the domain in the given identity.
    Fail,

    /// A "softfail" result is a weak statement by the publishing ADMD that
    /// the host is probably not authorized.  It has not published a
    /// stronger, more definitive policy that results in a "fail".
    SoftFail,

    /// A "temperror" result means the SPF verifier encountered a transient
    /// (generally DNS) error while performing the check.  A later retry may
    /// succeed without further DNS operator action.
    TempError,

    /// A "permerror" result means the domain's published records could not
    /// be correctly interpreted.  This signals an error condition that
    /// definitely requires DNS operator intervention to be resolved.
    PermError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpfResult {
    pub disposition: SpfDisposition,
    pub context: String,
}

pub struct CheckHostParams {
    /// the IP address of the SMTP client that is emitting the mail,
    /// either IPv4 or IPv6.
    pub client_ip: IpAddr,

    /// the domain that provides the sought-after authorization
    /// information; initially, the domain portion of the
    /// "MAIL FROM" or "HELO" identity.
    pub domain: String,

    /// the "MAIL FROM" or "HELO" identity.
    pub sender: String,
}

pub async fn check_host(resolver: &dyn dns::Lookup, params: &CheckHostParams) -> SpfResult {
    let initial_txt = match resolver.lookup_txt(&params.domain).await {
        Ok(parts) => parts.join(""),
        Err(err @ SpfError::DnsRecordNotFound(_)) => {
            return SpfResult {
                disposition: SpfDisposition::None,
                context: format!("{err}"),
            };
        }
        Err(err) => {
            return SpfResult {
                disposition: SpfDisposition::TempError,
                context: format!("{err}"),
            };
        }
    };

    let record = match Record::parse(&initial_txt) {
        Err(context) => {
            return SpfResult {
                disposition: SpfDisposition::PermError,
                context: format!("Failed to parse spf record: {context}"),
            };
        }
        Ok(r) => r,
    };

    let mut context = EvalContext::new();
    context.set_ip(params.client_ip);
    if let Err(err) = context.set_sender(&params.sender) {
        return SpfResult {
            disposition: SpfDisposition::TempError,
            context: format!("input sender parameter '{}' is malformed", params.sender),
        };
    }
    context.set_var(MacroName::Domain, &params.domain);

    let mut checker = HostChecker {
        stack: vec![record],
        context,
    };

    checker.run().await
}

struct HostChecker {
    stack: Vec<Record>,
    context: EvalContext,
}

impl HostChecker {
    async fn run(&mut self) -> SpfResult {
        if let Err(err) = self.process_stack().await {
            return err;
        }

        todo!();
    }

    async fn process_stack(&mut self) -> Result<(), SpfResult> {
    }
}
