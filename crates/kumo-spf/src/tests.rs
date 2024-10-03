use crate::dns::{DnsError, Lookup};
use crate::{CheckHostParams, SpfDisposition, SpfResult};
use futures::future::BoxFuture;
use hickory_proto::rr::rdata::{A, AAAA, MX, TXT};
use hickory_proto::rr::{LowerName, RData, RecordData, RecordSet, RecordType, RrKey};
use hickory_proto::serialize::txt::Parser;
use hickory_resolver::Name;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
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

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn ip() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_spf("example.com", "v=spf1 a -all".to_string());

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
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 10])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'a' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_spf("example.com", "v=spf1 a:example.org -all".to_string());

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 10])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Fail,
            context: "matched '-all' directive".to_owned(),
        },
        "{result:?}"
    );
}

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A.1
#[tokio::test]
async fn mx() {
    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_spf("example.com", "v=spf1 mx -all".to_string());

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 129])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_spf("example.com", "v=spf1 mx:example.org -all".to_string());

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 140])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx:example.org' directive".to_owned(),
        },
        "{result:?}"
    );

    let resolver = TestResolver::default()
        .with_zone(EXAMPLE_COM)
        .with_zone(EXAMPLE_ORG)
        .with_spf(
            "example.com",
            "v=spf1 mx/30 mx:example.org/30 -all".to_string(),
        );

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 131])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx/30' directive".to_owned(),
        },
        "{result:?}"
    );

    let result = CheckHostParams {
        client_ip: IpAddr::V4(Ipv4Addr::from([192, 0, 2, 141])),
        domain: "example.com".to_string(),
        sender: "sender@example.com".to_string(),
    }
    .run(&resolver)
    .await;

    k9::assert_equal!(
        &result,
        &SpfResult {
            disposition: SpfDisposition::Pass,
            context: "matched 'mx:example.org/30' directive".to_owned(),
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

/// https://www.rfc-editor.org/rfc/rfc7208#appendix-A
const EXAMPLE_ORG: &str = r#"; A related domain
$ORIGIN example.org.
@       600 MX  10 mail-c
mail-c      A   192.0.2.140"#;

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

    fn get<'a>(
        &'a self,
        full: &str,
        record_type: RecordType,
    ) -> Result<Option<&'a RecordSet>, DnsError> {
        let mut authority = full;
        loop {
            let authority_name = Name::from_str(authority).unwrap();
            let Some(records) = self.records.get(&authority_name) else {
                match authority.split_once('.') {
                    Some(new) => {
                        authority = new.1;
                        continue;
                    }
                    None => {
                        println!("authority not found: {full}");
                        return Err(DnsError::NotFound(full.to_string()));
                    }
                }
            };

            let fqdn = match full.ends_with('.') {
                true => full,
                false => &format!("{}.", full),
            };

            return Ok(records.get(&RrKey {
                name: LowerName::from_str(&fqdn).unwrap(),
                record_type,
            }));
        }
    }
}

impl Lookup for TestResolver {
    fn lookup_ip<'a>(&'a self, full: &'a str) -> BoxFuture<'a, Result<Vec<IpAddr>, DnsError>> {
        Box::pin(async move {
            let mut values = vec![];

            if let Some(records) = self.get(full, RecordType::A)? {
                for record in records.records_without_rrsigs() {
                    let a = A::try_borrow(record.data().unwrap()).unwrap();
                    values.push(IpAddr::V4(a.0));
                }
            };

            if let Some(records) = self.get(full, RecordType::AAAA)? {
                for record in records.records_without_rrsigs() {
                    let a = AAAA::try_borrow(record.data().unwrap()).unwrap();
                    values.push(IpAddr::V6(a.0));
                }
            }

            Ok(values)
        })
    }

    fn lookup_mx<'a>(&'a self, full: &'a str) -> BoxFuture<'a, Result<Vec<Name>, DnsError>> {
        Box::pin(async move {
            let records = match self.get(full, RecordType::MX)? {
                Some(records) => records,
                None => {
                    println!("key not found: {full}");
                    return Err(DnsError::NotFound(full.to_string()));
                }
            };

            let mut values = vec![];
            for record in records.records_without_rrsigs() {
                let mx = MX::try_borrow(record.data().unwrap()).unwrap();
                values.push(mx.exchange().clone());
            }

            Ok(values)
        })
    }

    fn lookup_txt<'a>(&'a self, full: &'a str) -> BoxFuture<'a, Result<Vec<String>, DnsError>> {
        Box::pin(async move {
            let records = match self.get(full, RecordType::TXT)? {
                Some(records) => records,
                None => {
                    println!("key not found: {full}");
                    return Err(DnsError::NotFound(full.to_string()));
                }
            };

            let mut values = vec![];
            for record in records.records_without_rrsigs() {
                let txt = TXT::try_borrow(record.data().unwrap()).unwrap();
                for slice in txt.iter() {
                    values.push(String::from_utf8(slice.to_vec()).unwrap());
                }
            }

            Ok(values)
        })
    }
}
