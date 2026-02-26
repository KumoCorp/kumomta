use crate::record::Record;
use crate::spec::MacroSpec;
use dns_resolver::{DnsError, Resolver};
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Name;
use instant_xml::{FromXml, ToXml};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

pub mod record;
mod spec;
use record::Qualifier;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Eq, FromXml, PartialEq, ToXml, Serialize, Deserialize)]
#[xml(scalar, rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
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

impl From<String> for SpfDisposition {
    fn from(value: String) -> Self {
        match value.to_lowercase().as_str() {
            "none" => Self::None,
            "neutral" => Self::Neutral,
            "pass" => Self::Pass,
            "fail" => Self::Fail,
            "softfail" => Self::SoftFail,
            "temperror" => Self::TempError,
            "permerror" => Self::PermError,
            _ => Self::None,
        }
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

pub struct CheckHostParams {
    /// Domain that provides the sought-after authorization information.
    ///
    /// Initially, the domain portion of the "MAIL FROM" or "HELO" identity.
    pub domain: String,

    /// The "MAIL FROM" email address if available.
    pub sender: Option<String>,

    /// IP address of the SMTP client that is emitting the mail (v4 or v6).
    pub client_ip: IpAddr,

    /// Explicitly the domain name passed to HELO/EHLO,
    /// regardless of the `domain` value.
    pub ehlo_domain: Option<String>,

    /// The host name of this host, the one doing the check
    pub relaying_host_name: Option<String>,
}

impl CheckHostParams {
    pub async fn check(self, resolver: &dyn Resolver) -> SpfResult {
        let Self {
            domain,
            sender,
            client_ip,
            ehlo_domain,
            relaying_host_name,
        } = self;

        let sender = match sender {
            Some(sender) => sender,
            None => format!("postmaster@{domain}"),
        };

        match SpfContext::new(&sender, &domain, client_ip) {
            Ok(cx) => {
                cx.with_ehlo_domain(ehlo_domain.as_deref())
                    .with_relaying_host_name(relaying_host_name.as_deref())
                    .check(resolver, true)
                    .await
            }
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
    pub(crate) ehlo_domain: Option<&'a str>,
    pub(crate) relaying_host_name: &'a str,
    lookups_remaining: Arc<AtomicUsize>,
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
            ehlo_domain: None,
            relaying_host_name: "localhost",
            lookups_remaining: Arc::new(AtomicUsize::new(10)),
        })
    }

    pub fn with_ehlo_domain(&self, ehlo_domain: Option<&'a str>) -> Self {
        Self {
            ehlo_domain,
            lookups_remaining: self.lookups_remaining.clone(),
            ..*self
        }
    }

    pub fn with_relaying_host_name(&self, relaying_host_name: Option<&'a str>) -> Self {
        Self {
            relaying_host_name: relaying_host_name.unwrap_or(self.relaying_host_name),
            lookups_remaining: self.lookups_remaining.clone(),
            ..*self
        }
    }

    pub(crate) fn with_domain(&self, domain: &'a str) -> Self {
        Self {
            domain,
            lookups_remaining: self.lookups_remaining.clone(),
            ..*self
        }
    }

    pub(crate) fn check_lookup_limit(&self) -> Result<(), SpfResult> {
        let remain = self.lookups_remaining.load(Ordering::Relaxed);
        if remain > 0 {
            self.lookups_remaining.store(remain - 1, Ordering::Relaxed);
            return Ok(());
        }

        Err(SpfResult {
            disposition: SpfDisposition::PermError,
            context: "DNS lookup limits exceeded".to_string(),
        })
    }

    pub async fn check(&self, resolver: &dyn Resolver, initial: bool) -> SpfResult {
        if !initial {
            if let Err(err) = self.check_lookup_limit() {
                return err;
            }
        }

        let name = match Name::from_str_relaxed(self.domain) {
            Ok(mut name) => {
                name.set_fqdn(true);
                name
            }
            Err(_) => {
                // Per <https://www.rfc-editor.org/rfc/rfc7208#section-4.3>, invalid
                // domain names yield a "none" result during initial processing.
                let context = format!("invalid domain name: {}", self.domain);
                return if initial {
                    SpfResult {
                        disposition: SpfDisposition::None,
                        context,
                    }
                } else {
                    SpfResult {
                        disposition: SpfDisposition::TempError,
                        context,
                    }
                };
            }
        };

        let initial_txt = match resolver.resolve(name, RecordType::TXT).await {
            Ok(answer) => {
                if answer.records.is_empty() || answer.nxdomain {
                    return SpfResult {
                        disposition: SpfDisposition::None,
                        context: if answer.records.is_empty() {
                            format!("no SPF records found for {}", &self.domain)
                        } else {
                            format!("domain {} not found", &self.domain)
                        },
                    };
                } else {
                    answer.as_txt()
                }
            }
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
        for txt in initial_txt {
            // a little bit of a layering violation: we need to know
            // whether we had an SPF record candidate or not to be
            // able to return an appropriate disposition if they have
            // TXT records but no SPF records.
            if txt.starts_with("v=spf1 ") {
                match Record::parse(&txt) {
                    Ok(record) => return record.evaluate(self, resolver).await,
                    Err(err) => {
                        return SpfResult {
                            disposition: SpfDisposition::PermError,
                            context: format!("failed to parse spf record: {err}"),
                        };
                    }
                }
            }
        }
        SpfResult {
            disposition: SpfDisposition::None,
            context: format!("no SPF records found for {}", &self.domain),
        }
    }

    pub(crate) async fn domain(
        &self,
        spec: Option<&MacroSpec>,
        resolver: &dyn Resolver,
    ) -> Result<String, SpfResult> {
        let Some(spec) = spec else {
            return Ok(self.domain.to_owned());
        };

        spec.expand(self, resolver).await.map_err(|err| SpfResult {
            disposition: SpfDisposition::TempError,
            context: format!("error evaluating domain spec: {err}"),
        })
    }

    pub(crate) async fn validated_domain(
        &self,
        spec: Option<&MacroSpec>,
        resolver: &dyn Resolver,
    ) -> Result<Option<String>, SpfResult> {
        // https://datatracker.ietf.org/doc/html/rfc7208#section-4.6.4
        self.check_lookup_limit()?;

        let domain = self.domain(spec, resolver).await?;

        let domain = match Name::from_str_relaxed(&domain) {
            Ok(domain) => domain,
            Err(err) => {
                return Err(SpfResult {
                    disposition: SpfDisposition::PermError,
                    context: format!("error parsing domain name: {err}"),
                })
            }
        };

        let ptrs = match resolver.resolve_ptr(self.client_ip).await {
            Ok(ptrs) => ptrs,
            Err(err) => {
                return Err(SpfResult {
                    disposition: SpfDisposition::TempError,
                    context: format!("error looking up PTR for {}: {err}", self.client_ip),
                })
            }
        };

        for (idx, ptr) in ptrs.iter().filter(|ptr| domain.zone_of(ptr)).enumerate() {
            if idx >= 10 {
                // https://datatracker.ietf.org/doc/html/rfc7208#section-4.6.4
                return Err(SpfResult {
                    disposition: SpfDisposition::PermError,
                    context: format!("too many PTR records for {}", self.client_ip),
                });
            }
            match resolver.resolve_ip(&fully_qualify(&ptr.to_string())).await {
                Ok(ips) => {
                    if ips.iter().any(|&ip| ip == self.client_ip) {
                        let mut ptr = ptr.clone();
                        // Remove trailing dot
                        ptr.set_fqdn(false);
                        return Ok(Some(ptr.to_string()));
                    }
                }
                Err(err) => {
                    return Err(SpfResult {
                        disposition: SpfDisposition::TempError,
                        context: format!("error looking up IP for {ptr}: {err}"),
                    })
                }
            }
        }

        Ok(None)
    }
}

pub(crate) fn fully_qualify(domain_name: &str) -> String {
    match dns_resolver::fully_qualify(domain_name) {
        Ok(name) => name.to_string(),
        Err(_) => domain_name.to_string(),
    }
}
