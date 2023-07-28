use crate::egress_path::EgressPathConfig;
use anyhow::Context;
use dns_resolver::MailExchanger;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(try_from = "String", into = "String")]
pub struct Regex(fancy_regex::Regex);

impl TryFrom<String> for Regex {
    type Error = fancy_regex::Error;

    fn try_from(s: String) -> fancy_regex::Result<Self> {
        Ok(Self(fancy_regex::Regex::new(&s)?))
    }
}

impl From<Regex> for String {
    fn from(r: Regex) -> String {
        r.0.as_str().to_string()
    }
}

impl std::ops::Deref for Regex {
    type Target = fancy_regex::Regex;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::hash::Hash for Regex {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.0.as_str().hash(hasher)
    }
}

#[derive(Deserialize, Serialize, Debug, Hash)]
pub enum Action {
    Suspend,
}

#[derive(Deserialize, Serialize, Debug, Hash)]
pub struct Rule {
    pub domain: String,

    #[serde(default)]
    pub mx_rollup: bool,

    pub regex: Regex,

    pub action: Action,

    #[serde(with = "humantime_serde")]
    pub duration: Duration,
}

#[derive(Debug)]
pub struct Shaping {
    pub by_site: HashMap<String, MergedEntry>,
    pub by_domain: HashMap<String, MergedEntry>,
    pub warnings: Vec<String>,
}

impl Shaping {
    fn load_from_file(path: &Path) -> anyhow::Result<HashMap<String, PartialEntry>> {
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("loading data from file {}", path.display()))?;

        if path.extension().and_then(|s| s.to_str()) == Some("toml") {
            toml::from_str(&data)
                .with_context(|| format!("parsing toml from file {}", path.display()))
        } else {
            serde_json::from_str(&data)
                .with_context(|| format!("parsing json from file {}", path.display()))
        }
    }

    pub async fn merge_files(files: &[PathBuf]) -> anyhow::Result<Self> {
        let mut loaded = vec![];
        for p in files {
            loaded.push(Self::load_from_file(p)?);
        }

        let mut site_to_domains: HashMap<String, HashSet<String>> = HashMap::new();
        let mut by_site: HashMap<String, PartialEntry> = HashMap::new();
        let mut by_domain: HashMap<String, PartialEntry> = HashMap::new();
        let mut warnings = vec![];

        for item in loaded {
            for (domain, mut partial) in item {
                partial.domain_name.replace(domain.clone());

                let mx_rollup = if domain == "default" {
                    false
                } else {
                    partial.mx_rollup
                };

                if mx_rollup {
                    let mx = match MailExchanger::resolve(&domain).await {
                        Ok(mx) => mx,
                        Err(err) => {
                            warnings.push(format!("error resolving MX for {domain}: {err:#}. Ignoring the shaping config for that domain."));
                            continue;
                        }
                    };

                    if mx.site_name.is_empty() {
                        warnings.push(format!("domain {domain} has a NULL MX and cannot be used with mx_rollup=true. Ignoring the shaping config for that domain."));
                        continue;
                    }

                    match by_site.get_mut(&mx.site_name) {
                        Some(existing) => {
                            existing.merge_from(partial);
                        }
                        None => {
                            by_site.insert(mx.site_name.clone(), partial);
                        }
                    }

                    site_to_domains
                        .entry(mx.site_name.clone())
                        .or_default()
                        .insert(domain);
                } else {
                    match by_domain.get_mut(&domain) {
                        Some(existing) => {
                            existing.merge_from(partial);
                        }
                        None => {
                            by_domain.insert(domain, partial);
                        }
                    }
                }
            }
        }

        let mut conflicted = vec![];
        for (site, domains) in site_to_domains {
            if domains.len() > 1 {
                let domains = domains.into_iter().collect::<Vec<_>>().join(", ");
                warnings.push(format!(
                    "Multiple domains rollup to the same site: {site} -> {domains}"
                ));
                conflicted.push(domains);
            }
        }

        if !conflicted.is_empty() {
            anyhow::bail!(
                "Multiple conflicting rollup domains: {}",
                conflicted.join(" ")
            );
        }

        let mut merged_site = HashMap::new();
        for (site, partial) in by_site {
            merged_site.insert(
                site.clone(),
                partial.finish().with_context(|| format!("site: {site}"))?,
            );
        }

        let mut merged_domain = HashMap::new();
        for (domain, partial) in by_domain {
            merged_domain.insert(
                domain.clone(),
                partial
                    .finish()
                    .with_context(|| format!("domain: {domain}"))?,
            );
        }

        Ok(Self {
            by_site: merged_site,
            by_domain: merged_domain,
            warnings,
        })
    }
}

#[derive(Default, Debug)]
pub struct MergedEntry {
    pub params: EgressPathConfig,
    pub sources: HashMap<String, EgressPathConfig>,
    pub automation: Vec<Rule>,
}

#[derive(Deserialize, Debug)]
struct PartialEntry {
    #[serde(skip)]
    pub domain_name: Option<String>,

    #[serde(flatten)]
    pub params: toml::Table,

    #[serde(default = "default_true")]
    pub mx_rollup: bool,

    #[serde(default)]
    pub replace_base: bool,

    #[serde(default)]
    pub automation: Vec<Rule>,

    #[serde(default)]
    pub sources: HashMap<String, toml::Table>,
}

impl PartialEntry {
    fn merge_from(&mut self, mut other: Self) {
        if other.replace_base {
            self.params = other.params;
            self.automation = other.automation;
            self.sources = other.sources;
        } else {
            for (k, v) in other.params {
                self.params.insert(k, v);
            }

            for (source, tbl) in other.sources {
                match self.sources.get_mut(&source) {
                    Some(existing) => {
                        for (k, v) in tbl {
                            existing.insert(k, v);
                        }
                    }
                    None => {
                        self.sources.insert(source, tbl);
                    }
                }
            }

            self.automation.append(&mut other.automation);
        }
    }

    fn finish(self) -> anyhow::Result<MergedEntry> {
        let domain = self.domain_name.unwrap_or_default();

        let params = EgressPathConfig::deserialize(self.params.clone()).with_context(|| {
            format!(
                "interpreting domain '{domain}' params {:#?} as EgressPathConfig",
                self.params
            )
        })?;
        let mut sources = HashMap::new();

        for (source, params) in self.sources {
            sources.insert(
                source.clone(),
                EgressPathConfig::deserialize(params.clone()).with_context(|| {
                    format!("interpreting domain '{domain}' source '{source}' {params:#} as EgressPathConfig")
                })?,
            );
        }

        Ok(MergedEntry {
            params,
            sources,
            automation: self.automation,
        })
    }
}

fn default_true() -> bool {
    true
}
