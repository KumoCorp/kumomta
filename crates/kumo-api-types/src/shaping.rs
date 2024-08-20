use crate::egress_path::EgressPathConfig;
#[cfg(feature = "lua")]
use anyhow::Context;
#[cfg(feature = "lua")]
use config::any_err;
#[cfg(feature = "lua")]
use config::serialize_options;
#[cfg(feature = "lua")]
use dns_resolver::{fully_qualify, MailExchanger};
#[cfg(feature = "lua")]
use kumo_log_types::JsonLogRecord;
#[cfg(feature = "lua")]
use mlua::prelude::LuaUserData;
#[cfg(feature = "lua")]
use mlua::{LuaSerdeExt, UserDataMethods};
use ordermap::OrderMap;
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::formats::PreferOne;
use serde_with::{serde_as, OneOrMany};
#[cfg(feature = "lua")]
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
#[cfg(feature = "lua")]
use std::sync::Arc;
use std::time::Duration;
use throttle::ThrottleSpec;

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

/// toml::Value is not Hash because it may contain floating
/// point numbers, which are problematic from a Ord and Eq
/// perspective. We're okay with skirting around that for
/// our purposes here, so we implement our own hashable
/// wrapper around the toml value.
#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(from = "toml::Value", into = "toml::Value")]
pub struct HashableTomlValue {
    value: toml::Value,
}

impl From<toml::Value> for HashableTomlValue {
    fn from(value: toml::Value) -> Self {
        Self { value }
    }
}

impl From<HashableTomlValue> for toml::Value {
    fn from(value: HashableTomlValue) -> toml::Value {
        value.value
    }
}

impl std::ops::Deref for HashableTomlValue {
    type Target = toml::Value;
    fn deref(&self) -> &toml::Value {
        &self.value
    }
}

fn hash_toml<H>(value: &toml::Value, h: &mut H)
where
    H: Hasher,
{
    match value {
        toml::Value::Boolean(v) => v.hash(h),
        toml::Value::Datetime(v) => {
            if let Some(d) = &v.date {
                d.year.hash(h);
                d.month.hash(h);
                d.day.hash(h);
            }
            if let Some(t) = &v.time {
                t.hour.hash(h);
                t.minute.hash(h);
                t.second.hash(h);
                t.nanosecond.hash(h);
            }
            if let Some(toml::value::Offset::Custom { minutes }) = &v.offset {
                minutes.hash(h);
            }
        }
        toml::Value::String(v) => v.hash(h),
        toml::Value::Integer(v) => v.hash(h),
        toml::Value::Float(v) => v.to_ne_bytes().hash(h),
        toml::Value::Array(a) => {
            for v in a.iter() {
                hash_toml(v, h);
            }
        }
        toml::Value::Table(m) => {
            for (k, v) in m.iter() {
                k.hash(h);
                hash_toml(v, h);
            }
        }
    }
}

impl Hash for HashableTomlValue {
    fn hash<H>(&self, h: &mut H)
    where
        H: Hasher,
    {
        hash_toml(&self.value, h);
    }
}

/// Represents an individual EgressPathConfig field name and value.
/// It only allows deserializing from valid EgressPathConfig field + values.
#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
#[serde(
    try_from = "EgressPathConfigValueUnchecked",
    into = "EgressPathConfigValueUnchecked"
)]
pub struct EgressPathConfigValue {
    pub name: String,
    pub value: HashableTomlValue,
}

/// This is the type that we actually use to deserialize EgressPathConfigValue items.
/// It doesn't care about validity; it is used solely to tell serde what shape of
/// data to expect.
/// The validation is performed by the TryFrom impl that is used to convert to the
/// checked form below.
#[derive(Deserialize, Serialize, Debug, Clone)]
struct EgressPathConfigValueUnchecked {
    pub name: String,
    pub value: toml::Value,
}

impl TryFrom<EgressPathConfigValueUnchecked> for EgressPathConfigValue {
    type Error = anyhow::Error;
    fn try_from(config: EgressPathConfigValueUnchecked) -> anyhow::Result<EgressPathConfigValue> {
        let mut map = toml::map::Map::new();
        map.insert(config.name.clone(), config.value.clone());
        let table = toml::Value::Table(map);

        // Attempt to deserialize as EgressPathConfig.
        // If it fails, then the field name/value are invalid
        EgressPathConfig::deserialize(table)?;

        // If we reach this point, we can pass along the name/value
        Ok(EgressPathConfigValue {
            name: config.name,
            value: HashableTomlValue {
                value: config.value,
            },
        })
    }
}

impl From<EgressPathConfigValue> for EgressPathConfigValueUnchecked {
    fn from(config: EgressPathConfigValue) -> EgressPathConfigValueUnchecked {
        EgressPathConfigValueUnchecked {
            name: config.name,
            value: config.value.value,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash)]
pub enum Action {
    Suspend,
    SetConfig(EgressPathConfigValue),
    SuspendTenant,
    SuspendCampaign,
}

#[derive(Deserialize, Serialize, Debug, Clone, Hash, Default)]
pub enum Trigger {
    /// Trigger on the first match, immediately
    #[default]
    Immediate,
    /// Trigger when a certain number of matches occur
    /// over a certain time period.
    Threshold(ThrottleSpec),
}

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Hash, Clone)]
pub struct Rule {
    #[serde(deserialize_with = "string_or_array")]
    pub regex: Vec<Regex>,

    #[serde_as(deserialize_as = "OneOrMany<_, PreferOne>")]
    pub action: Vec<Action>,

    #[serde(default)]
    pub trigger: Trigger,

    #[serde(with = "duration_serde")]
    pub duration: Duration,

    #[serde(skip)]
    pub was_rollup: bool,
}

impl Rule {
    pub fn matches(&self, response: &str) -> bool {
        self.regex
            .iter()
            .any(|r| r.is_match(response).unwrap_or(false))
    }

    pub fn clone_and_set_rollup(&self) -> Self {
        let mut result = self.clone();
        result.was_rollup = true;
        result
    }
}

#[cfg(feature = "lua")]
#[derive(Debug)]
struct ShapingInner {
    by_site: OrderMap<String, PartialEntry>,
    by_domain: OrderMap<String, PartialEntry>,
    warnings: Vec<String>,
}

#[cfg(feature = "lua")]
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

    pub fn match_rules(&self, record: &JsonLogRecord) -> anyhow::Result<Vec<Rule>> {
        use rfc5321::ForwardPath;
        // Extract the domain from the recipient.
        let recipient = ForwardPath::try_from(record.recipient.as_str())
            .map_err(|err| anyhow::anyhow!("parsing record.recipient: {err}"))?;

        let recipient = match recipient {
            ForwardPath::Postmaster => {
                // It doesn't make sense to apply automation on the
                // local postmaster address, so we ignore this.
                return Ok(vec![]);
            }
            ForwardPath::Path(path) => path.mailbox,
        };
        let domain = recipient.domain.to_string();

        // Track events/outcomes by site.
        let source = record.egress_source.as_deref().unwrap_or("unspecified");
        // record.site is poorly named; it is really an identifier for the
        // egress path. For matching purposes, we want just the site_name
        // in the form produced by our MX resolution process.
        // In an earlier incarnation of this logic, we would resolve the
        // site_name for ourselves based on other data in the record,
        // but that could lead to over-resolution of some names and
        // yield surprising results.
        // What we do here is extract the egress path decoration from
        // record.site to arrive at something that looks like the
        // mx site_name.
        // NOTE: this is coupled with the logic in
        // ReadyQueueManager::compute_queue_name
        let site_name = record
            .site
            .trim_start_matches(&format!("{source}->"))
            .trim_end_matches("@smtp_client")
            .to_string();

        Ok(self.match_rules_impl(record, &domain, &site_name))
    }

    pub fn match_rules_impl(
        &self,
        record: &JsonLogRecord,
        domain: &str,
        site_name: &str,
    ) -> Vec<Rule> {
        let mut result = vec![];
        let response = record.response.to_single_line();
        tracing::trace!("Consider rules for {response}");

        if let Some(default) = self.by_domain.get("default") {
            for rule in &default.automation {
                tracing::trace!("Consider \"default\" rule {rule:?} for {response}");
                if rule.matches(&response) {
                    // For automation under `default`, we always
                    // assume that mx_rollup should be true.
                    // If you somehow have a domain where that isn't
                    // true, you should avoid using `default` for
                    // automation.  Honestly, it's best to avoid
                    // using `default` for automation.
                    result.push(rule.clone_and_set_rollup());
                }
            }
        }

        // Then site config
        if let Some(by_site) = self.by_site.get(site_name) {
            for rule in &by_site.automation {
                tracing::trace!("Consider \"{site_name}\" rule {rule:?} for {response}");
                if rule.matches(&response) {
                    result.push(rule.clone_and_set_rollup());
                }
            }
        }

        // Then domain config
        if let Some(by_domain) = self.by_domain.get(domain) {
            for rule in &by_domain.automation {
                tracing::trace!("Consider \"{domain}\" rule {rule:?} for {response}");
                if rule.matches(&response) {
                    result.push(rule.clone());
                }
            }
        }

        result
    }
}

#[cfg(feature = "lua")]
#[derive(Debug, Clone, mlua::FromLua)]
pub struct Shaping {
    inner: Arc<ShapingInner>,
}

#[cfg(feature = "lua")]
impl Shaping {
    async fn load_from_file(path: &str) -> anyhow::Result<OrderMap<String, PartialEntry>> {
        let data: String = if path.starts_with("http://") || path.starts_with("https://") {
            // To facilitate startup ordering races, and listing multiple subscription
            // host replicas and allowing one or more of them to be temporarily down,
            // we allow the http request to fail.
            // We'll log the error message but consider it to be an empty map

            async fn http_get(url: &str) -> anyhow::Result<String> {
                reqwest::get(url)
                    .await
                    .with_context(|| format!("making HTTP request to {url}"))?
                    .text()
                    .await
                    .with_context(|| format!("reading text from {url}"))
            }

            match http_get(path).await {
                Ok(s) => s,
                Err(err) => {
                    tracing::error!("{err:#}. Ignoring this shaping source for now");
                    return Ok(OrderMap::new());
                }
            }
        } else {
            std::fs::read_to_string(path)
                .with_context(|| format!("loading data from file {path}"))?
        };

        if path.ends_with(".toml") {
            toml::from_str(&data).with_context(|| format!("parsing toml from file {path}"))
        } else {
            serde_json::from_str(&data).with_context(|| format!("parsing json from file {path}"))
        }
    }

    pub async fn merge_files(files: &[String]) -> anyhow::Result<Self> {
        use futures_util::stream::FuturesUnordered;
        use futures_util::StreamExt;

        let mut loaded = vec![];
        for p in files {
            loaded.push(Self::load_from_file(p).await?);
        }

        let mut by_site: OrderMap<String, PartialEntry> = OrderMap::new();
        let mut by_domain: OrderMap<String, PartialEntry> = OrderMap::new();
        let mut warnings = vec![];

        // Pre-resolve domains. We don't interleave the resolution with
        // the work below, because we want to ensure that the ordering
        // is preserved
        let limiter = Arc::new(tokio::sync::Semaphore::new(128));
        let mut mx = std::collections::HashMap::new();
        let mut lookups = FuturesUnordered::new();
        for item in &loaded {
            for (domain, partial) in item {
                let mx_rollup = if domain == "default" {
                    false
                } else {
                    partial.mx_rollup
                };

                if mx_rollup {
                    let domain = domain.to_string();
                    let limiter = limiter.clone();
                    lookups.push(tokio::spawn(async move {
                        match limiter.acquire().await {
                            Ok(permit) => {
                                let mx_result = MailExchanger::resolve(&domain).await;
                                drop(permit);
                                (domain, mx_result)
                            }
                            Err(err) => (domain, Err(err).context("failed to acquire permit")),
                        }
                    }));
                }
            }
        }

        while let Some(Ok((domain, result))) = lookups.next().await {
            mx.insert(domain, result);
        }

        for item in loaded {
            for (domain, mut partial) in item {
                partial.domain_name.replace(domain.clone());

                let mx_rollup = if domain == "default" {
                    false
                } else {
                    if let Ok(name) = fully_qualify(&domain) {
                        if name.num_labels() == 1 {
                            warnings.push(format!(
                                "Entry for domain '{domain}' consists of a \
                                 single DNS label. Domain names in TOML sections \
                                 need to be quoted like '[\"{domain}.com\"]` otherwise \
                                 the '.' will create a nested table rather than being \
                                 added to the domain name."
                            ));
                        }
                    }

                    partial.mx_rollup
                };

                if mx_rollup {
                    let mx = match mx.get(&domain) {
                        Some(Ok(mx)) => mx,
                        Some(Err(err)) => {
                            warnings.push(format!(
                                "error resolving MX for {domain}: {err:#}. \
                                 Ignoring the shaping config for that domain."
                            ));
                            continue;
                        }
                        None => {
                            warnings.push(format!(
                                "We didn't try to resolve the MX for {domain} for some reason!?. \
                                 Ignoring the shaping config for that domain."
                            ));
                            continue;
                        }
                    };

                    if mx.site_name.is_empty() {
                        warnings.push(format!(
                            "domain {domain} has a NULL MX and cannot be used with mx_rollup=true. \
                             Ignoring the shaping config for that domain."));
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

    pub fn match_rules(&self, record: &JsonLogRecord) -> anyhow::Result<Vec<Rule>> {
        self.inner.match_rules(record)
    }

    pub fn get_referenced_sources(&self) -> BTreeMap<String, Vec<String>> {
        let mut result = BTreeMap::new();

        for (site_name, site) in &self.inner.by_site {
            for source_name in site.sources.keys() {
                result
                    .entry(source_name.to_string())
                    .or_insert(vec![])
                    .push(format!("site:{site_name}"));
            }
        }
        for (domain_name, domain) in &self.inner.by_domain {
            for source_name in domain.sources.keys() {
                result
                    .entry(source_name.to_string())
                    .or_insert(vec![])
                    .push(format!("domain:{domain_name}"));
            }
        }

        result
    }
}

#[cfg(feature = "lua")]
impl LuaUserData for Shaping {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_method(
            "get_egress_path_config",
            move |lua, this, (domain, egress_source, site_name): (String, String, String)| {
                let params = this.get_egress_path_config(&domain, &egress_source, &site_name);
                lua.to_value_with(&params.params, serialize_options())
            },
        );
        methods.add_method("get_warnings", move |_lua, this, ()| {
            let warnings: Vec<String> = this.get_warnings().iter().map(|s| s.to_string()).collect();
            Ok(warnings)
        });

        methods.add_method("get_referenced_sources", move |_lua, this, ()| {
            Ok(this.get_referenced_sources())
        });

        methods.add_method("match_rules", move |lua, this, record: mlua::Value| {
            let record: JsonLogRecord = lua.from_value(record)?;
            let rules = this.match_rules(&record).map_err(any_err)?;
            let mut result = vec![];
            for rule in rules {
                result.push(lua.to_value(&rule)?);
            }
            Ok(result)
        });
    }
}

#[derive(Default, Debug)]
pub struct MergedEntry {
    pub params: EgressPathConfig,
    pub sources: OrderMap<String, EgressPathConfig>,
    pub automation: Vec<Rule>,
}

#[cfg(feature = "lua")]
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
    pub sources: OrderMap<String, toml::Table>,
}

#[cfg(feature = "lua")]
fn toml_table_merge_from(tbl: &mut toml::Table, source: &toml::Table) {
    for (k, v) in source {
        tbl.insert(k.clone(), v.clone());
    }
}

#[cfg(feature = "lua")]
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
        let mut sources = OrderMap::new();

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

fn string_or_array<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: Deserialize<'de> + TryFrom<String>,
    <T as TryFrom<String>>::Error: std::fmt::Debug,
    D: Deserializer<'de>,
{
    // This is a Visitor that forwards string types to T's `TryFrom<String>` impl and
    // forwards map types to T's `Deserialize` impl. The `PhantomData` is to
    // keep the compiler from complaining about T being an unused generic type
    // parameter. We need T in order to know the Value type for the Visitor
    // impl.
    struct StringOrArray<T>(PhantomData<fn() -> T>);

    impl<'de, T> Visitor<'de> for StringOrArray<T>
    where
        T: Deserialize<'de> + TryFrom<String>,
        <T as TryFrom<String>>::Error: std::fmt::Debug,
    {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("string or array")
        }

        fn visit_str<E>(self, value: &str) -> Result<Vec<T>, E>
        where
            E: serde::de::Error,
        {
            Ok(vec![
                T::try_from(value.to_string()).map_err(|e| E::custom(format!("{e:?}")))?
            ])
        }

        fn visit_seq<S>(self, seq: S) -> Result<Vec<T>, S::Error>
        where
            S: SeqAccess<'de>,
        {
            Deserialize::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))
        }
    }

    deserializer.deserialize_any(StringOrArray(PhantomData))
}

#[cfg(feature = "lua")]
fn default_true() -> bool {
    true
}

#[cfg(feature = "lua")]
pub fn register(lua: &mlua::Lua) -> anyhow::Result<()> {
    let shaping_mod = config::get_or_create_sub_module(lua, "shaping")?;

    shaping_mod.set(
        "load",
        lua.create_async_function(move |_lua, paths: Vec<String>| async move {
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
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 10,
        enable_tls: Opportunistic,
        enable_mta_sts: true,
        enable_dane: false,
        tls_prefer_openssl: false,
        openssl_cipher_list: None,
        openssl_cipher_suites: None,
        openssl_options: None,
        rustls_cipher_suites: [],
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            banner_timeout: 60s,
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
        suspended: false,
        aggressive_connection_opening: false,
    },
    sources: {},
    automation: [
        Rule {
            regex: [
                Regex(
                    /Messages from \d+\.\d+\.\d+\.\d+ temporarily deferred/,
                ),
                Regex(
                    /All messages from \d+\.\d+\.\d+\.\d+ will be permanently deferred/,
                ),
                Regex(
                    /has been temporarily rate limited due to IP reputation/,
                ),
                Regex(
                    /Unfortunately, messages from \d+\.\d+\.\d+\.\d+ weren.t sent/,
                ),
                Regex(
                    /Server busy\. Please try again later from/,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "max_message_rate",
                        value: HashableTomlValue {
                            value: String(
                                "1/minute",
                            ),
                        },
                    },
                ),
                SetConfig(
                    EgressPathConfigValue {
                        name: "connection_limit",
                        value: HashableTomlValue {
                            value: Integer(
                                1,
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 5400s,
            was_rollup: false,
        },
        Rule {
            regex: [
                Regex(
                    KumoMTA internal: failed to connect to any candidate hosts: All failures are related to OpportunisticInsecure STARTTLS. Consider setting enable_tls=Disabled for this site,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "enable_tls",
                        value: HashableTomlValue {
                            value: String(
                                "Disabled",
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 2592000s,
            was_rollup: false,
        },
    ],
}
"#
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
        enable_mta_sts: true,
        enable_dane: false,
        tls_prefer_openssl: false,
        openssl_cipher_list: None,
        openssl_cipher_suites: None,
        openssl_options: None,
        rustls_cipher_suites: [],
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            banner_timeout: 60s,
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
        suspended: false,
        aggressive_connection_opening: false,
    },
    sources: {
        "my source name": EgressPathConfig {
            connection_limit: 5,
            enable_tls: Opportunistic,
            enable_mta_sts: true,
            enable_dane: false,
            tls_prefer_openssl: false,
            openssl_cipher_list: None,
            openssl_cipher_suites: None,
            openssl_options: None,
            rustls_cipher_suites: [],
            client_timeouts: SmtpClientTimeouts {
                connect_timeout: 60s,
                banner_timeout: 60s,
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
            suspended: false,
            aggressive_connection_opening: false,
        },
    },
    automation: [
        Rule {
            regex: [
                Regex(
                    /Messages from \d+\.\d+\.\d+\.\d+ temporarily deferred/,
                ),
                Regex(
                    /All messages from \d+\.\d+\.\d+\.\d+ will be permanently deferred/,
                ),
                Regex(
                    /has been temporarily rate limited due to IP reputation/,
                ),
                Regex(
                    /Unfortunately, messages from \d+\.\d+\.\d+\.\d+ weren.t sent/,
                ),
                Regex(
                    /Server busy\. Please try again later from/,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "max_message_rate",
                        value: HashableTomlValue {
                            value: String(
                                "1/minute",
                            ),
                        },
                    },
                ),
                SetConfig(
                    EgressPathConfigValue {
                        name: "connection_limit",
                        value: HashableTomlValue {
                            value: Integer(
                                1,
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 5400s,
            was_rollup: false,
        },
        Rule {
            regex: [
                Regex(
                    KumoMTA internal: failed to connect to any candidate hosts: All failures are related to OpportunisticInsecure STARTTLS. Consider setting enable_tls=Disabled for this site,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "enable_tls",
                        value: HashableTomlValue {
                            value: String(
                                "Disabled",
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 2592000s,
            was_rollup: false,
        },
    ],
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
        enable_mta_sts: true,
        enable_dane: false,
        tls_prefer_openssl: false,
        openssl_cipher_list: None,
        openssl_cipher_suites: None,
        openssl_options: None,
        rustls_cipher_suites: [],
        client_timeouts: SmtpClientTimeouts {
            connect_timeout: 60s,
            banner_timeout: 60s,
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
        suspended: false,
        aggressive_connection_opening: false,
    },
    sources: {},
    automation: [
        Rule {
            regex: [
                Regex(
                    /Messages from \d+\.\d+\.\d+\.\d+ temporarily deferred/,
                ),
                Regex(
                    /All messages from \d+\.\d+\.\d+\.\d+ will be permanently deferred/,
                ),
                Regex(
                    /has been temporarily rate limited due to IP reputation/,
                ),
                Regex(
                    /Unfortunately, messages from \d+\.\d+\.\d+\.\d+ weren.t sent/,
                ),
                Regex(
                    /Server busy\. Please try again later from/,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "max_message_rate",
                        value: HashableTomlValue {
                            value: String(
                                "1/minute",
                            ),
                        },
                    },
                ),
                SetConfig(
                    EgressPathConfigValue {
                        name: "connection_limit",
                        value: HashableTomlValue {
                            value: Integer(
                                1,
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 5400s,
            was_rollup: false,
        },
        Rule {
            regex: [
                Regex(
                    KumoMTA internal: failed to connect to any candidate hosts: All failures are related to OpportunisticInsecure STARTTLS. Consider setting enable_tls=Disabled for this site,
                ),
            ],
            action: [
                SetConfig(
                    EgressPathConfigValue {
                        name: "enable_tls",
                        value: HashableTomlValue {
                            value: String(
                                "Disabled",
                            ),
                        },
                    },
                ),
            ],
            trigger: Immediate,
            duration: 2592000s,
            was_rollup: false,
        },
        Rule {
            regex: [
                Regex(
                    \[TS04\],
                ),
            ],
            action: [
                Suspend,
            ],
            trigger: Immediate,
            duration: 7200s,
            was_rollup: false,
        },
    ],
}
"#
        );
    }
}
