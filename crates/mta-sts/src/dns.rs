use dns_resolver::Resolver;
use std::collections::BTreeMap;

// <https://datatracker.ietf.org/doc/html/rfc8461>

#[derive(Debug)]
pub struct MtaStsDnsRecord {
    pub id: String,
    pub fields: BTreeMap<String, String>,
}

pub async fn resolve_dns_record(
    policy_domain: &str,
    resolver: &dyn Resolver,
) -> anyhow::Result<MtaStsDnsRecord> {
    let dns_name = format!("_mta-sts.{policy_domain}");
    let res = resolver.resolve_txt(&dns_name).await?.as_txt();
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
    use dns_resolver::TestResolver;

    #[tokio::test]
    async fn test_parse_dns_record() {
        let resolver = TestResolver::default().with_txt(
            "_mta-sts.gmail.com",
            "v=STSv1; id=20190429T010101;".to_owned(),
        );

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
