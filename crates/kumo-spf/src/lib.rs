use crate::record::Record;
use crate::spec::MacroSpec;
use dns_resolver::{DnsError, Resolver};
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Name;
use instant_xml::{FromXml, ToXml};
use serde::{Deserialize, Serialize, Serializer};
use std::fmt;
use std::net::IpAddr;
use std::time::SystemTime;

pub mod record;
mod spec;
use record::Qualifier;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Eq, FromXml, PartialEq, ToXml)]
#[xml(scalar, rename_all = "lowercase")]
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
            Ok(cx) => cx.check(resolver, true).await,
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

    pub async fn check(&self, resolver: &dyn Resolver, initial: bool) -> SpfResult {
        let name = match Name::from_utf8(&self.domain) {
            Ok(name) => name,
            Err(_) => {
                // Per <https://www.rfc-editor.org/rfc/rfc7208#section-4.3>, invalid
                // domain names yield a "none" result during initial processing.
                let context = format!("invalid domain name: {}", self.domain);
                return match initial {
                    true => SpfResult {
                        disposition: SpfDisposition::None,
                        context,
                    },
                    false => SpfResult {
                        disposition: SpfDisposition::TempError,
                        context,
                    },
                };
            }
        };

        let initial_txt = match resolver.resolve(name, RecordType::TXT).await {
            Ok(answer) => match answer.records.is_empty() || answer.nxdomain {
                true => {
                    return SpfResult {
                        disposition: SpfDisposition::None,
                        context: match answer.records.is_empty() {
                            true => format!("no SPF records found for {}", &self.domain),
                            false => format!("domain {} not found", &self.domain),
                        },
                    }
                }
                false => answer.as_txt(),
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

        // TXT records can contain all sorts of stuff, let's walk through
        // the set that we retrieved and take the first one that parses
        let mut failures = vec![];

        for txt in initial_txt {
            match Record::parse(&txt) {
                Ok(record) => return record.evaluate(self, resolver).await,
                Err(err) => {
                    failures.push(format!("'{txt}': {err}"));
                }
            }
        }

        // If we get here, none of the SPF records were any good
        SpfResult {
            disposition: SpfDisposition::PermError,
            context: format!("failed to parse spf record: {}", failures.join(", ")),
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
