use std::net::IpAddr;

pub mod dns;
pub mod error;
use error::SpfError;
pub mod eval;
use eval::EvalContext;
pub mod record;
use record::{MacroName, Qualifier, Record};

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

impl CheckHostParams {
    pub async fn run(&self, resolver: &dyn dns::Lookup) -> SpfResult {
        let initial_txt = match resolver.lookup_txt(&self.domain).await {
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
            Ok(r) => r,
            Err(context) => {
                return SpfResult {
                    disposition: SpfDisposition::PermError,
                    context: format!("failed to parse spf record: {context}"),
                };
            }
        };

        let mut cx = EvalContext::new();
        cx.set_ip(self.client_ip);
        if let Err(err) = cx.set_sender(&self.sender) {
            return SpfResult {
                disposition: SpfDisposition::TempError,
                context: format!(
                    "input sender parameter '{}' is malformed: {err}",
                    self.sender
                ),
            };
        }
        cx.set_var(MacroName::Domain, &self.domain);

        record.evaluate(&cx, resolver).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::Lookup;
    use crate::error::SpfError;
    use futures::future::BoxFuture;
    use hickory_proto::rr::rdata::TXT;
    use hickory_proto::rr::{LowerName, RData, RecordData, RecordSet, RecordType, RrKey};
    use hickory_proto::serialize::txt::Parser;
    use hickory_resolver::Name;
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;
    use std::str::FromStr;

    /// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
    #[tokio::test]
    async fn all() {
        let resolver = TestResolver::default()
            .with_zone(EXAMPLE_COM)
            .with_spf("example.com", "v=spf1 +all".to_string());

        let result = CheckHostParams {
            client_ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            domain: "example.com".to_string(),
            sender: "sender@example.com".to_string(),
        }
        .run(&resolver)
        .await;

        k9::assert_equal!(
            &result,
            &SpfResult {
                disposition: SpfDisposition::Pass,
                context: "matched 'all' directive".to_owned(),
            },
            "{result:?}"
        );
    }

    /// https://www.rfc-editor.org/rfc/rfc7208#appendix-A
    const EXAMPLE_COM: &str = r#"; A domain with two mail servers, two hosts, and two servers
; at the domain name
$ORIGIN example.com.
@       600 MX  10 mail-a
            MX  20 mail-b
            A   192.0.2.10
            A   192.0.2.11
amy         A   192.0.2.65
bob         A   192.0.2.66
mail-a      A   192.0.2.129
mail-b      A   192.0.2.130
www         CNAME example.com."#;

    #[derive(Default)]
    struct TestResolver {
        records: BTreeMap<Name, BTreeMap<RrKey, RecordSet>>,
    }

    impl TestResolver {
        fn with_zone(mut self, zone: &str) -> Self {
            let (name, records) = Parser::new(zone, None, None).parse().unwrap();
            self.records.insert(name, records);
            self
        }

        fn with_spf(mut self, domain: &str, policy: String) -> Self {
            let fqdn = format!("{}.", domain);
            let authority = Name::from_str(&fqdn).unwrap();
            let key = RrKey {
                name: LowerName::from_str(&fqdn).unwrap(),
                record_type: RecordType::TXT,
            };

            let mut records = RecordSet::new(&authority, RecordType::TXT, 0);
            records.add_rdata(RData::TXT(TXT::new(vec![policy])));
            self.records
                .entry(authority)
                .or_insert_with(BTreeMap::new)
                .insert(key, records);

            self
        }
    }

    impl Lookup for TestResolver {
        fn lookup_txt<'a>(&'a self, full: &'a str) -> BoxFuture<'a, Result<Vec<String>, SpfError>> {
            Box::pin(async move {
                let (mut name, mut authority) = ("", full);
                loop {
                    let authority_name = Name::from_str(authority).unwrap();
                    let Some(records) = self.records.get(&authority_name) else {
                        match authority.split_once('.') {
                            Some(new) => {
                                name = new.0;
                                authority = new.1;
                                continue;
                            }
                            None => {
                                println!("authority not found: {full}");
                                return Err(SpfError::DnsRecordNotFound(full.to_string()));
                            }
                        }
                    };

                    let fqdn = format!("{full}.");
                    let key = RrKey {
                        name: LowerName::from_str(&fqdn).unwrap(),
                        record_type: RecordType::TXT,
                    };

                    let Some(records) = records.get(&key) else {
                        println!("key not found: {key:?}");
                        return Err(SpfError::DnsRecordNotFound(name.to_string()));
                    };

                    let mut values = vec![];
                    for record in records.records_without_rrsigs() {
                        let txt = TXT::try_borrow(record.data().unwrap()).unwrap();
                        for slice in txt.iter() {
                            values.push(String::from_utf8(slice.to_vec()).unwrap());
                        }
                    }

                    return Ok(values);
                }
            })
        }
    }
}
