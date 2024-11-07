use crate::record::Record;
use crate::spec::MacroSpec;
use dns_resolver::{DnsError, Resolver};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::net::IpAddr;
use std::time::SystemTime;

pub mod record;
mod spec;
use record::Qualifier;
#[cfg(test)]
mod tests;

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

impl SpfDisposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Neutral => "neutral",
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::SoftFail => "softfail",
            Self::TempError => "temperror",
            Self::PermError => "permerror",
        }
    }
}

impl Serialize for SpfDisposition {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl From<Qualifier> for SpfDisposition {
    fn from(qualifier: Qualifier) -> Self {
        match qualifier {
            Qualifier::Pass => Self::Pass,
            Qualifier::Fail => Self::Fail,
            Qualifier::SoftFail => Self::SoftFail,
            Qualifier::Neutral => Self::Neutral,
        }
    }
}

impl fmt::Display for SpfDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpfResult {
    pub disposition: SpfDisposition,
    pub context: String,
}

impl SpfResult {
    fn fail(context: String) -> Self {
        Self {
            disposition: SpfDisposition::Fail,
            context,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CheckHostParams {
    /// Domain that provides the sought-after authorization information.
    ///
    /// Initially, the domain portion of the "MAIL FROM" or "HELO" identity.
    pub domain: String,

    /// The "MAIL FROM" email address if available.
    pub sender: Option<String>,

    /// IP address of the SMTP client that is emitting the mail (v4 or v6).
    pub client_ip: IpAddr,
}

impl CheckHostParams {
    pub async fn check(self, resolver: &dyn Resolver) -> SpfResult {
        let Self {
            domain,
            sender,
            client_ip,
        } = self;

        let sender = match sender {
            Some(sender) => sender,
            None => format!("postmaster@{domain}"),
        };

        match SpfContext::new(&sender, &domain, client_ip) {
            Ok(cx) => cx.check(resolver).await,
            Err(result) => result,
        }
    }
}

struct SpfContext<'a> {
    pub(crate) sender: &'a str,
    pub(crate) local_part: &'a str,
    pub(crate) sender_domain: &'a str,
    pub(crate) domain: &'a str,
    pub(crate) client_ip: IpAddr,
    pub(crate) now: SystemTime,
}

impl<'a> SpfContext<'a> {
    /// Create a new evaluation context.
    ///
    /// - `sender` is the "MAIL FROM" or "HELO" identity
    /// - `domain` is the domain that provides the sought-after authorization information;
    ///   initially, the domain portion of the "MAIL FROM" or "HELO" identity
    /// - `client_ip` is the IP address of the SMTP client that is emitting the mail
    fn new(sender: &'a str, domain: &'a str, client_ip: IpAddr) -> Result<Self, SpfResult> {
        let Some((local_part, sender_domain)) = sender.split_once('@') else {
            return Err(SpfResult {
                disposition: SpfDisposition::PermError,
                context:
                    "input sender parameter '{sender}' is missing @ sign to delimit local part and domain".to_owned(),
            });
        };

        Ok(Self {
            sender,
            local_part,
            sender_domain,
            domain,
            client_ip,
            now: SystemTime::now(),
        })
    }

    pub(crate) fn with_domain(&self, domain: &'a str) -> Self {
        Self { domain, ..*self }
    }

    pub async fn check(&self, resolver: &dyn Resolver) -> SpfResult {
        let initial_txt = match resolver.resolve_txt(self.domain).await {
            Ok(parts) => match parts.records.is_empty() {
                true => {
                    return SpfResult {
                        disposition: SpfDisposition::None,
                        context: "no spf records found".to_owned(),
                    }
                }
                false => parts.as_txt().join(""),
            },
            Err(err) => {
                return SpfResult {
                    disposition: match err {
                        DnsError::InvalidName(_) => SpfDisposition::PermError,
                        DnsError::ResolveFailed(_) => SpfDisposition::TempError,
                    },
                    context: format!("{err}"),
                };
            }
        };

        match Record::parse(&initial_txt) {
            Ok(record) => record.evaluate(self, resolver).await,
            Err(err) => {
                return SpfResult {
                    disposition: SpfDisposition::PermError,
                    context: format!("failed to parse spf record: {err}"),
                }
            }
        }
    }

    pub(crate) fn domain(&self, spec: Option<&MacroSpec>) -> Result<String, SpfResult> {
        let Some(spec) = spec else {
            return Ok(self.domain.to_owned());
        };

        spec.expand(self).map_err(|err| SpfResult {
            disposition: SpfDisposition::TempError,
            context: format!("error evaluating domain spec: {err}"),
        })
    }
}
