use futures::future::BoxFuture;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub enum PolicyMode {
    Enforce,
    Testing,
    None,
}

#[derive(Debug)]
pub struct MtaStsPolicy {
    pub mode: PolicyMode,
    pub mx: Vec<String>,
    pub max_age: u64,
    pub fields: BTreeMap<String, Vec<String>>,
}

impl MtaStsPolicy {
    pub fn parse(data: &str) -> anyhow::Result<Self> {
        let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for line in data.lines() {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| anyhow::anyhow!("invalid STS policy {data}"))?;
            let key = key.trim();
            let value = value.trim();

            fields
                .entry(key.to_string())
                .or_default()
                .push(value.to_string());
        }

        let version = fields
            .remove("version")
            .ok_or_else(|| anyhow::anyhow!("STS policy {data} is missing a version"))?;
        if version.len() != 1 || version[0] != "STSv1" {
            anyhow::bail!("STS policy {data} has incompatible STS version");
        }

        let mode = match fields.remove("mode") {
            None => anyhow::bail!("STS policy {data} is missing required mode"),
            Some(mode) if mode.len() == 1 => match mode[0].as_str() {
                "enforce" => PolicyMode::Enforce,
                "testing" => PolicyMode::Testing,
                "none" => PolicyMode::None,
                _ => anyhow::bail!("STS policy {data} has invalid mode"),
            },
            _ => anyhow::bail!("STS policy {data} has invalid mode"),
        };

        let mut mx = match fields.remove("mx") {
            None if mode == PolicyMode::None => vec![],
            None => anyhow::bail!("STS policy {data} is missing required mx"),
            Some(v) => v,
        };

        // Ensure that the mx entries are lowercased to aid
        // the mx_name_matches method
        mx.iter_mut()
            .for_each(|entry| *entry = entry.to_lowercase());

        let max_age: u64 = match fields.remove("max_age") {
            None => anyhow::bail!("STS policy {data} is missing required max_age"),
            Some(v) if v.len() == 1 => {
                let max_age = &v[0];
                max_age.parse().map_err(|err| anyhow::anyhow!("STS policy {data} has max_age {max_age} that is not a valid integer: {err:#}"))?
            }
            _ => anyhow::bail!("STS policy {data} has invalid max_age"),
        };

        Ok(Self {
            fields,
            mode,
            mx,
            max_age,
        })
    }

    /// Returns true if `name` matches any of the allowed mx
    /// host name patterns.
    /// `name` must be lowercase.
    pub fn mx_name_matches(&self, name: &str) -> bool {
        for pattern in &self.mx {
            if name_match(name, pattern) {
                return true;
            }
        }
        false
    }
}

fn name_match(name: &str, pattern: &str) -> bool {
    // kumo uses canonicalized names that include a trailing period.
    // remove that from the name when matching against a pattern.
    let name = name.trim_end_matches('.');

    if pattern.starts_with("*.") {
        let suffix = &pattern[1..];
        if let Some(lhs) = name.strip_suffix(suffix) {
            // Wildcards only match the first component
            return lhs.find('.').is_none();
        }
        false
    } else {
        name == pattern
    }
}

pub trait Get: Sync + Send {
    fn http_get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, anyhow::Result<String>>;
}

pub async fn load_policy_for_domain(
    policy_domain: &str,
    getter: &dyn Get,
) -> anyhow::Result<MtaStsPolicy> {
    let url = format!("https://mta-sts.{policy_domain}/.well-known/mta-sts.txt");
    let policy = getter.http_get(&url).await?;
    MtaStsPolicy::parse(&policy)
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

    pub struct TestGetter {
        policies: BTreeMap<&'static str, &'static str>,
    }

    impl TestGetter {
        pub fn new<I: IntoIterator<Item = (&'static str, &'static str)>>(iter: I) -> Self {
            Self {
                policies: BTreeMap::from_iter(iter),
            }
        }
    }

    impl Get for TestGetter {
        fn http_get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, anyhow::Result<String>> {
            Box::pin(async move {
                match self.policies.get(url) {
                    Some(result) => Ok(result.to_string()),
                    None => anyhow::bail!("404 {url}"),
                }
            })
        }
    }

    const SAMPLE_POLICY: &str =
        "version: STSv1 \nmode: enforce\nmx: mail.example.com\r\nmx:\t*.example.net\nmx: backupmx.example.com\nmax_age: 604800";

    #[tokio::test]
    async fn get_policy() {
        let getter = TestGetter::new([(
            "https://mta-sts.example.com/.well-known/mta-sts.txt",
            SAMPLE_POLICY,
        )]);

        k9::snapshot!(
            load_policy_for_domain("example.com", &getter)
                .await
                .unwrap(),
            r#"
MtaStsPolicy {
    mode: Enforce,
    mx: [
        "mail.example.com",
        "*.example.net",
        "backupmx.example.com",
    ],
    max_age: 604800,
    fields: {},
}
"#
        );
    }

    #[test]
    fn parse_policy() {
        k9::snapshot!(
            MtaStsPolicy::parse(SAMPLE_POLICY).unwrap(),
            r#"
MtaStsPolicy {
    mode: Enforce,
    mx: [
        "mail.example.com",
        "*.example.net",
        "backupmx.example.com",
    ],
    max_age: 604800,
    fields: {},
}
"#
        );
    }

    #[test]
    fn name_matching() {
        assert!(name_match("foo.com", "foo.com"));
        assert!(name_match("foo.com.", "foo.com"));
        assert!(!name_match("bar.com", "foo.com"));
        assert!(name_match("foo.com", "*.com"));
        assert!(name_match("mx.example.com", "*.example.com"));
        assert!(!name_match("not.mx.example.com", "*.example.com"));
        assert!(!name_match("example.com", "*.example.com"));
    }
}
