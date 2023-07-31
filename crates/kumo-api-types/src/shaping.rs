use crate::egress_path::EgressPathConfig;
use anyhow::Context;
use config::any_err;
use dns_resolver::MailExchanger;
use mlua::prelude::LuaUserData;
use mlua::{LuaSerdeExt, UserDataMethods};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

#[derive(Deserialize, Serialize, Debug, Hash, Clone)]
pub enum Action {
    Suspend,
}

#[derive(Deserialize, Serialize, Debug, Hash, Clone)]
pub struct Rule {
    #[serde(default)]
    pub mx_rollup: bool,

    pub regex: Regex,

    pub action: Action,

    #[serde(with = "humantime_serde")]
    pub duration: Duration,
}

#[derive(Debug)]
struct ShapingInner {
    by_site: HashMap<String, PartialEntry>,
    by_domain: HashMap<String, PartialEntry>,
    warnings: Vec<String>,
}

impl ShapingInner {
    pub fn get_egress_path_config(
        &self,
        domain: &str,
        egress_source: &str,
        site_name: &str,
    ) -> PartialEntry {
        let mut params = PartialEntry::default();

        // Apply basic/default configuration
        if let Some(default) = self.by_domain.get("default") {
            params.merge_from(default.clone());
        }

        // Then site config
        if let Some(by_site) = self.by_site.get(site_name) {
            params.merge_from(by_site.clone());
        }

        // Then domain config
        if let Some(by_domain) = self.by_domain.get(domain) {
            params.merge_from(by_domain.clone());
        }

        // Then source config for the site
        if let Some(by_site) = self.by_site.get(site_name) {
            if let Some(source) = by_site.sources.get(egress_source) {
                toml_table_merge_from(&mut params.params, &source);
            }
        }

        // Then source config for the domain
        if let Some(by_domain) = self.by_domain.get(domain) {
            if let Some(source) = by_domain.sources.get(egress_source) {
                toml_table_merge_from(&mut params.params, &source);
            }
        }

        params
    }
}

#[derive(Debug, Clone)]
pub struct Shaping {
    inner: Arc<ShapingInner>,
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

        for (site, partial) in &by_site {
            partial
                .clone()
                .finish()
                .with_context(|| format!("site: {site}"))?;
        }

        for (domain, partial) in &by_domain {
            partial
                .clone()
                .finish()
                .with_context(|| format!("domain: {domain}"))?;
        }

        Ok(Self {
            inner: Arc::new(ShapingInner {
                by_site,
                by_domain,
                warnings,
            }),
        })
    }

    fn get_egress_path_config(
        &self,
        domain: &str,
        egress_source: &str,
        site_name: &str,
    ) -> PartialEntry {
        self.inner
            .get_egress_path_config(domain, egress_source, site_name)
    }

    pub fn get_warnings(&self) -> &[String] {
        &self.inner.warnings
    }
}

impl LuaUserData for Shaping {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_method(
            "get_egress_path_config",
            move |lua, this, (domain, egress_source, site_name): (String, String, String)| {
                let params = this.get_egress_path_config(&domain, &egress_source, &site_name);
                lua.to_value(&params.params)
            },
        );
        methods.add_method("get_warnings", move |_lua, this, ()| {
            let warnings: Vec<String> = this.get_warnings().iter().map(|s| s.to_string()).collect();
            Ok(warnings)
        });
    }
}

#[derive(Default, Debug)]
pub struct MergedEntry {
    pub params: EgressPathConfig,
    pub sources: HashMap<String, EgressPathConfig>,
    pub automation: Vec<Rule>,
}

#[derive(Deserialize, Debug, Clone, Default)]
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

fn toml_table_merge_from(tbl: &mut toml::Table, source: &toml::Table) {
    for (k, v) in source {
        tbl.insert(k.clone(), v.clone());
    }
}

impl PartialEntry {
    fn merge_from(&mut self, mut other: Self) {
        if other.replace_base {
            self.params = other.params;
            self.automation = other.automation;
            self.sources = other.sources;
        } else {
            toml_table_merge_from(&mut self.params, &other.params);

            for (source, tbl) in other.sources {
                match self.sources.get_mut(&source) {
                    Some(existing) => {
                        toml_table_merge_from(existing, &tbl);
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

pub fn register(lua: &mlua::Lua) -> anyhow::Result<()> {
    let shaping_mod = config::get_or_create_sub_module(lua, "shaping")?;

    shaping_mod.set(
        "load",
        lua.create_async_function(move |_lua, paths: Vec<String>| async move {
            let paths: Vec<PathBuf> = paths.into_iter().map(|p| p.into()).collect();
            let shaping = Shaping::merge_files(&paths).await.map_err(any_err)?;
            Ok(shaping)
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_defaults() {
        let shaping = Shaping::merge_files(&["../../assets/policy-extras/shaping.toml".into()])
            .await
            .unwrap();

        let default = shaping
            .get_egress_path_config("invalid.domain", "invalid.source", "invalid.site")
            .finish()
            .unwrap();
        k9::snapshot!(
            default,
            "
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 10,
        enable_tls: Opportunistic,
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            ehlo_timeout: 300s,
            mail_from_timeout: 300s,
            rcpt_to_timeout: 300s,
            data_timeout: 30s,
            data_dot_timeout: 60s,
            rset_timeout: 5s,
            idle_timeout: 60s,
            starttls_timeout: 5s,
            auth_timeout: 60s,
        },
        max_ready: 1024,
        consecutive_connection_failures_before_delay: 100,
        smtp_port: 25,
        smtp_auth_plain_username: None,
        smtp_auth_plain_password: None,
        allow_smtp_auth_plain_without_tls: false,
        max_message_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 1,
                max_burst: None,
            },
        ),
        max_connection_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 60,
                max_burst: None,
            },
        ),
        max_deliveries_per_connection: 100,
        prohibited_hosts: CidrSet(
            CidrMap {
                root: Some(
                    InnerNode(
                        InnerNode {
                            key: Any,
                            children: Children {
                                left: Leaf(
                                    Leaf {
                                        key: V4(
                                            127.0.0.0/8,
                                        ),
                                        value: (),
                                    },
                                ),
                                right: Leaf(
                                    Leaf {
                                        key: V6(
                                            ::1/128,
                                        ),
                                        value: (),
                                    },
                                ),
                            },
                        },
                    ),
                ),
            },
        ),
        skip_hosts: CidrSet(
            CidrMap {
                root: None,
            },
        ),
        ehlo_domain: None,
    },
    sources: {},
    automation: [],
}
"
        );

        let example_com = shaping
            .get_egress_path_config("example.com", "invalid.source", "invalid.site")
            .finish()
            .unwrap();
        k9::snapshot!(
            example_com,
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 3,
        enable_tls: Opportunistic,
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            ehlo_timeout: 300s,
            mail_from_timeout: 300s,
            rcpt_to_timeout: 300s,
            data_timeout: 30s,
            data_dot_timeout: 60s,
            rset_timeout: 5s,
            idle_timeout: 60s,
            starttls_timeout: 5s,
            auth_timeout: 60s,
        },
        max_ready: 1024,
        consecutive_connection_failures_before_delay: 100,
        smtp_port: 25,
        smtp_auth_plain_username: None,
        smtp_auth_plain_password: None,
        allow_smtp_auth_plain_without_tls: false,
        max_message_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 1,
                max_burst: None,
            },
        ),
        max_connection_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 60,
                max_burst: None,
            },
        ),
        max_deliveries_per_connection: 100,
        prohibited_hosts: CidrSet(
            CidrMap {
                root: Some(
                    InnerNode(
                        InnerNode {
                            key: Any,
                            children: Children {
                                left: Leaf(
                                    Leaf {
                                        key: V4(
                                            127.0.0.0/8,
                                        ),
                                        value: (),
                                    },
                                ),
                                right: Leaf(
                                    Leaf {
                                        key: V6(
                                            ::1/128,
                                        ),
                                        value: (),
                                    },
                                ),
                            },
                        },
                    ),
                ),
            },
        ),
        skip_hosts: CidrSet(
            CidrMap {
                root: None,
            },
        ),
        ehlo_domain: None,
    },
    sources: {
        "my source name": EgressPathConfig {
            connection_limit: 5,
            enable_tls: Opportunistic,
            client_timeouts: SmtpClientTimeouts {
                connect_timeout: 60s,
                ehlo_timeout: 300s,
                mail_from_timeout: 300s,
                rcpt_to_timeout: 300s,
                data_timeout: 300s,
                data_dot_timeout: 300s,
                rset_timeout: 5s,
                idle_timeout: 5s,
                starttls_timeout: 5s,
                auth_timeout: 60s,
            },
            max_ready: 1024,
            consecutive_connection_failures_before_delay: 100,
            smtp_port: 25,
            smtp_auth_plain_username: None,
            smtp_auth_plain_password: None,
            allow_smtp_auth_plain_without_tls: false,
            max_message_rate: None,
            max_connection_rate: None,
            max_deliveries_per_connection: 1024,
            prohibited_hosts: CidrSet(
                CidrMap {
                    root: Some(
                        InnerNode(
                            InnerNode {
                                key: Any,
                                children: Children {
                                    left: Leaf(
                                        Leaf {
                                            key: V4(
                                                127.0.0.0/8,
                                            ),
                                            value: (),
                                        },
                                    ),
                                    right: Leaf(
                                        Leaf {
                                            key: V6(
                                                ::1/128,
                                            ),
                                            value: (),
                                        },
                                    ),
                                },
                            },
                        ),
                    ),
                },
            ),
            skip_hosts: CidrSet(
                CidrMap {
                    root: None,
                },
            ),
            ehlo_domain: None,
        },
    },
    automation: [],
}
"#
        );

        // The site name here will need to be updated if yahoo changes
        // their MX records
        let yahoo_com = shaping
            .get_egress_path_config(
                "yahoo.com",
                "invalid.source",
                "(mta5|mta6|mta7).am0.yahoodns.net",
            )
            .finish()
            .unwrap();
        k9::snapshot!(
            yahoo_com,
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 10,
        enable_tls: Opportunistic,
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            ehlo_timeout: 300s,
            mail_from_timeout: 300s,
            rcpt_to_timeout: 300s,
            data_timeout: 30s,
            data_dot_timeout: 60s,
            rset_timeout: 5s,
            idle_timeout: 60s,
            starttls_timeout: 5s,
            auth_timeout: 60s,
        },
        max_ready: 1024,
        consecutive_connection_failures_before_delay: 100,
        smtp_port: 25,
        smtp_auth_plain_username: None,
        smtp_auth_plain_password: None,
        allow_smtp_auth_plain_without_tls: false,
        max_message_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 1,
                max_burst: None,
            },
        ),
        max_connection_rate: Some(
            ThrottleSpec {
                limit: 100,
                period: 60,
                max_burst: None,
            },
        ),
        max_deliveries_per_connection: 20,
        prohibited_hosts: CidrSet(
            CidrMap {
                root: Some(
                    InnerNode(
                        InnerNode {
                            key: Any,
                            children: Children {
                                left: Leaf(
                                    Leaf {
                                        key: V4(
                                            127.0.0.0/8,
                                        ),
                                        value: (),
                                    },
                                ),
                                right: Leaf(
                                    Leaf {
                                        key: V6(
                                            ::1/128,
                                        ),
                                        value: (),
                                    },
                                ),
                            },
                        },
                    ),
                ),
            },
        ),
        skip_hosts: CidrSet(
            CidrMap {
                root: None,
            },
        ),
        ehlo_domain: None,
    },
    sources: {},
    automation: [
        Rule {
            mx_rollup: false,
            regex: Regex(
                \[TS04\],
            ),
            action: Suspend,
            duration: 7200s,
        },
    ],
}
"#
        );
    }
}
