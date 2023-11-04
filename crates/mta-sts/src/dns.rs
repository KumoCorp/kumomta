use dns_resolver::resolver::Resolver;
use futures::future::BoxFuture;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::TokioAsyncResolver;
use std::collections::BTreeMap;

// <https://datatracker.ietf.org/doc/html/rfc8461>

#[derive(Debug)]
pub struct MtaStsDnsRecord {
    pub id: String,
    pub fields: BTreeMap<String, String>,
}

/// A trait for entities that perform DNS resolution.
pub trait Lookup: Sync + Send {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<String>>>;
}

impl Lookup for TokioAsyncResolver {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<String>>> {
        Box::pin(async move {
            self.txt_lookup(name)
                .await?
                .into_iter()
                .map(|txt| {
                    Ok(txt
                        .iter()
                        .map(|data| String::from_utf8_lossy(data))
                        .collect())
                })
                .collect()
        })
    }
}

impl Lookup for Resolver {
    fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<String>>> {
        Box::pin(async move {
            let answer = self.resolve(name, RecordType::TXT).await?;
            Ok(answer.as_txt())
        })
    }
}

pub async fn resolve_dns_record(
    policy_domain: &str,
    resolver: &dyn Lookup,
) -> anyhow::Result<MtaStsDnsRecord> {
    let dns_name = format!("_mta-sts.{policy_domain}");
    let res = resolver.lookup_txt(&dns_name).await?;
    let txt = res.join("");

    let mut fields = BTreeMap::new();

    for pair in txt.split(';') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid element in STS text record: {pair}. Full record: {txt}")
        })?;

        let key = key.trim();
        let value = value.trim();

        fields.insert(key.to_string(), value.to_string());
    }

    if fields.get("v").map(|s| s.as_str()) != Some("STSv1") {
        anyhow::bail!("TXT record is not an STSv1 record {txt}");
    }

    let id = fields
        .get("id")
        .ok_or_else(|| anyhow::anyhow!("STSv1 TXT record is missing id parameter. {txt}"))?
        .to_string();

    Ok(MtaStsDnsRecord { id, fields })
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    pub struct TestResolver {
        dns: BTreeMap<&'static str, &'static str>,
    }

    impl TestResolver {
        pub fn new<I: IntoIterator<Item = (&'static str, &'static str)>>(iter: I) -> Self {
            Self {
                dns: BTreeMap::from_iter(iter),
            }
        }
    }

    impl Lookup for TestResolver {
        fn lookup_txt<'a>(&'a self, name: &'a str) -> BoxFuture<'a, anyhow::Result<Vec<String>>> {
            Box::pin(async move {
                match self.dns.get(name) {
                    Some(result) => Ok(vec![result.to_string()]),
                    None => anyhow::bail!("NXDOMAIN {name}"),
                }
            })
        }
    }

    #[tokio::test]
    async fn test_parse_dns_record() {
        let resolver = TestResolver::new([("_mta-sts.gmail.com", "v=STSv1; id=20190429T010101;")]);

        let result = resolve_dns_record("gmail.com", &resolver).await.unwrap();

        k9::snapshot!(
            result,
            r#"
MtaStsDnsRecord {
    id: "20190429T010101",
    fields: {
        "id": "20190429T010101",
        "v": "STSv1",
    },
}
"#
        );
    }
}
