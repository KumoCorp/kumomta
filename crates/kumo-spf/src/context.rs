use crate::dns::{DnsError, Lookup};
use crate::record::Record;
use crate::spec::MacroSpec;
use crate::{SpfDisposition, SpfResult};
use std::net::IpAddr;
use std::time::SystemTime;

pub struct SpfContext<'a> {
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
    pub fn new(sender: &'a str, domain: &'a str, client_ip: IpAddr) -> Result<Self, SpfResult> {
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

    pub async fn check(&self, resolver: &dyn Lookup) -> SpfResult {
        let initial_txt = match resolver.lookup_txt(self.domain).await {
            Ok(parts) => parts.join(""),
            Err(err) => {
                return SpfResult {
                    disposition: match err {
                        DnsError::NotFound(_) => SpfDisposition::None,
                        DnsError::LookupFailed(_) => SpfDisposition::TempError,
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
