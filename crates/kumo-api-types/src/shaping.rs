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
use sha2::{Digest, Sha256};
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
    by_provider: OrderMap<String, ProviderEntry>,
    warnings: Vec<String>,
    hash: String,
}

#[cfg(feature = "lua")]
impl ShapingInner {
    pub async fn get_egress_path_config(
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

        // Provider rules come next
        for prov in self.by_provider.values() {
            if prov.domain_matches(domain).await {
                toml_table_merge_from(&mut params.params, &prov.params);
                prov.apply_provider_params_to(egress_source, &mut params.params);
            }
        }

        // Then Provider source rules
        for prov in self.by_provider.values() {
            if prov.sources.is_empty() {
                continue;
            }
            if prov.domain_matches(domain).await {
                if let Some(source) = prov.sources.get(egress_source) {
                    toml_table_merge_from(&mut params.params, &source);
                    prov.apply_provider_params_to(egress_source, &mut params.params);
                }
            }
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

    pub async fn match_rules(&self, record: &JsonLogRecord) -> anyhow::Result<Vec<Rule>> {
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
    async fn load_from_file(path: &str) -> anyhow::Result<ShapingFile> {
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
                    return Ok(ShapingFile::default());
                }
            }
        } else {
            std::fs::read_to_string(path)
                .with_context(|| format!("loading data from file {path}"))?
        };

        if path.ends_with(".toml") {
            toml::from_str(&data).with_context(|| format!("parsing toml from file {path}"))
        } else if path.ends_with(".json") {
            serde_json::from_str(&data).with_context(|| format!("parsing json from file {path}"))
        } else {
            // Try parsing both ways and see which wins
            let mut errors = vec![];
            match toml::from_str(&data) {
                Ok(s) => return Ok(s),
                Err(err) => errors.push(format!("as toml: {err:#}")),
            }
            match serde_json::from_str(&data) {
                Ok(s) => return Ok(s),
                Err(err) => errors.push(format!("as json: {err:#}")),
            }

            anyhow::bail!("parsing {path}: {}", errors.join(", "));
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
        let mut by_provider: OrderMap<String, ProviderEntry> = OrderMap::new();
        let mut warnings = vec![];

        // Pre-resolve domains. We don't interleave the resolution with
        // the work below, because we want to ensure that the ordering
        // is preserved
        let limiter = Arc::new(tokio::sync::Semaphore::new(128));
        let mut mx = std::collections::HashMap::new();
        let mut lookups = FuturesUnordered::new();
        for item in &loaded {
            for (domain, partial) in &item.domains {
                if partial.mx_rollup {
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

        for mut item in loaded {
            if let Some(mut partial) = item.default.take() {
                let domain = "default";
                partial.domain_name.replace(domain.to_string());
                match by_domain.get_mut(domain) {
                    Some(existing) => {
                        existing.merge_from(partial);
                    }
                    None => {
                        by_domain.insert(domain.to_string(), partial);
                    }
                }
            }

            for (domain, mut partial) in item.domains {
                partial.domain_name.replace(domain.clone());

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

                if partial.mx_rollup {
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

            for (provider, mut prov) in item.provider {
                prov.provider_name = provider.to_string();
                match by_provider.get_mut(&provider) {
                    Some(existing) => {
                        existing.merge_from(prov);
                    }
                    None => {
                        by_provider.insert(provider.to_string(), prov);
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

        for (provider, prov) in &by_provider {
            prov.finish_params()
                .with_context(|| format!("provider: {provider}"))?;
        }

        let mut ctx = Sha256::new();
        ctx.update("by_site");
        for (site, entry) in &by_site {
            ctx.update(site);
            entry.hash_into(&mut ctx);
        }
        ctx.update("by_domain");
        for (domain, entry) in &by_domain {
            ctx.update(domain);
            entry.hash_into(&mut ctx);
        }
        ctx.update("by_provider");
        for (provider, prov) in &by_provider {
            ctx.update(provider);
            prov.hash_into(&mut ctx);
        }
        ctx.update("warnings");
        for warn in &warnings {
            ctx.update(warn);
        }
        let hash = ctx.finalize();
        let hash = data_encoding::HEXLOWER.encode(&hash);

        Ok(Self {
            inner: Arc::new(ShapingInner {
                by_site,
                by_domain,
                by_provider,
                warnings,
                hash,
            }),
        })
    }

    async fn get_egress_path_config(
        &self,
        domain: &str,
        egress_source: &str,
        site_name: &str,
    ) -> PartialEntry {
        self.inner
            .get_egress_path_config(domain, egress_source, site_name)
            .await
    }

    pub fn get_warnings(&self) -> &[String] {
        &self.inner.warnings
    }

    pub async fn match_rules(&self, record: &JsonLogRecord) -> anyhow::Result<Vec<Rule>> {
        self.inner.match_rules(record).await
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

    pub fn hash(&self) -> String {
        self.inner.hash.clone()
    }
}

#[cfg(feature = "lua")]
impl LuaUserData for Shaping {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        mod_memoize::Memoized::impl_memoize(methods);
        methods.add_async_method(
            "get_egress_path_config",
            |lua, this, (domain, egress_source, site_name): (String, String, String)| async move {
                let params = this
                    .get_egress_path_config(&domain, &egress_source, &site_name)
                    .await;
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

        methods.add_async_method("match_rules", |lua, this, record: mlua::Value| async move {
            let record: JsonLogRecord = lua.from_value(record)?;
            let rules = this.match_rules(&record).await.map_err(any_err)?;
            let mut result = vec![];
            for rule in rules {
                result.push(lua.to_value(&rule)?);
            }
            Ok(result)
        });

        methods.add_method("hash", move |_, this, ()| Ok(this.hash()));
    }
}

#[derive(Default, Debug)]
pub struct MergedEntry {
    pub params: EgressPathConfig,
    pub sources: OrderMap<String, EgressPathConfig>,
    pub automation: Vec<Rule>,
}

#[cfg(feature = "lua")]
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
struct ShapingFile {
    pub default: Option<PartialEntry>,
    #[serde(flatten, default)]
    pub domains: OrderMap<String, PartialEntry>,
    #[serde(default)]
    pub provider: OrderMap<String, ProviderEntry>,
}

#[cfg(feature = "lua")]
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
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
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ProviderEntry {
    #[serde(skip, default)]
    pub provider_name: String,

    #[serde(default)]
    pub provider_connection_limit: Option<usize>,

    #[serde(default)]
    pub provider_max_message_rate: Option<ThrottleSpec>,

    #[serde(default, rename = "match")]
    pub matches: Vec<ProviderMatch>,

    #[serde(default)]
    pub replace_base: bool,

    #[serde(flatten)]
    pub params: toml::Table,

    #[serde(default)]
    pub automation: Vec<Rule>,

    #[serde(default)]
    pub sources: OrderMap<String, toml::Table>,
}

#[cfg(feature = "lua")]
fn suffix_matches(candidate: &str, suffix: &str) -> bool {
    // Remove trailing dot from candidate, as our resolver tends
    // to leave the canonical dot on the input host name
    let candidate = candidate.strip_suffix(".").unwrap_or(candidate);
    if candidate.len() < suffix.len() {
        return false;
    }
    let candidate = &candidate[candidate.len() - suffix.len()..];
    candidate.eq_ignore_ascii_case(suffix)
}

#[cfg(feature = "lua")]
#[cfg(test)]
#[test]
fn test_suffix_matches() {
    assert!(suffix_matches("a", "a"));
    assert!(suffix_matches("foo.Com", ".com"));
    assert!(!suffix_matches("foo.Cam", ".com"));
    assert!(suffix_matches("foo.com", "foo.com"));
    assert!(!suffix_matches("foo.com", ".foo.com"));
    assert!(!suffix_matches("foo.com", "longer.com"));
}

#[cfg(feature = "lua")]
impl ProviderEntry {
    async fn domain_matches(&self, domain: &str) -> bool {
        // We'd like to avoid doing DNS if we can do a simple suffix match,
        // so we bias to looking at those first
        let mut has_mx_rules = false;

        for rule in &self.matches {
            match rule {
                ProviderMatch::DomainSuffix(suffix) => {
                    if suffix_matches(domain, suffix) {
                        return true;
                    }
                }
                ProviderMatch::MXSuffix(_) => {
                    has_mx_rules = true;
                }
            }
        }

        if !has_mx_rules {
            return false;
        }

        // Now we can consider DNS
        match MailExchanger::resolve(&domain).await {
            Err(err) => {
                tracing::error!(
                    "Error resolving MX for {domain}: {err:#}. \
                    Provider {} match rules will be ignored",
                    self.provider_name
                );
                false
            }
            Ok(mx) => {
                for rule in &self.matches {
                    match rule {
                        ProviderMatch::MXSuffix(suffix) => {
                            // For a given MX suffix rule, all hosts must match
                            // it for it to be valid. This is so that we don't
                            // falsely lump a vanity domain that blends providers
                            // together.
                            let mut matched = true;
                            for host in &mx.hosts {
                                if !suffix_matches(host, suffix) {
                                    matched = false;
                                    break;
                                }
                            }

                            if matched {
                                return true;
                            }
                        }
                        ProviderMatch::DomainSuffix(_) => {}
                    }
                }

                false
            }
        }
    }

    fn merge_from(&mut self, mut other: Self) {
        if other.replace_base {
            self.provider_connection_limit = other.provider_connection_limit;
            self.matches = other.matches;
            self.params = other.params;
            self.sources = other.sources;
            self.automation = other.automation;
        } else {
            if other.provider_connection_limit.is_some() {
                self.provider_connection_limit = other.provider_connection_limit;
            }

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

            self.matches.append(&mut other.matches);
            self.automation.append(&mut other.automation);
        }
    }

    fn apply_provider_params_to(&self, source: &str, target: &mut toml::Table) {
        let mut implied = toml::Table::new();
        if let Some(limit) = &self.provider_connection_limit {
            let mut limits = toml::Table::new();
            limits.insert(
                format!("shaping-provider-{}-{source}-limit", self.provider_name),
                toml::Value::Integer((*limit) as i64),
            );
            implied.insert(
                "additional_connection_limits".to_string(),
                toml::Value::Table(limits),
            );
        }
        if let Some(rate) = &self.provider_max_message_rate {
            match rate.as_string() {
                Ok(rate) => {
                    let mut limits = toml::Table::new();
                    limits.insert(
                        format!("shaping-provider-{}-{source}-rate", self.provider_name),
                        rate.into(),
                    );
                    implied.insert(
                        "additional_message_rate_throttles".to_string(),
                        toml::Value::Table(limits),
                    );
                }
                Err(err) => {
                    tracing::error!("Error representing provider_max_message_rate: {err}");
                }
            }
        }

        toml_table_merge_from(target, &implied);
    }

    fn finish_params(&self) -> anyhow::Result<MergedEntry> {
        let provider_name = &self.provider_name;

        let params = EgressPathConfig::deserialize(self.params.clone()).with_context(|| {
            format!(
                "interpreting provider '{provider_name}' params {:#?} as EgressPathConfig",
                self.params
            )
        })?;
        let mut sources = OrderMap::new();

        for (source, params) in &self.sources {
            sources.insert(
                source.clone(),
                EgressPathConfig::deserialize(params.clone()).with_context(|| {
                    format!("interpreting provider '{provider_name}' source '{source}' {params:#} as EgressPathConfig")
                })?,
            );
        }

        Ok(MergedEntry {
            params,
            sources,
            automation: self.automation.clone(),
        })
    }

    fn hash_into(&self, ctx: &mut Sha256) {
        ctx.update(&self.provider_name);
        ctx.update(serde_json::to_string(self).unwrap_or_else(|_| String::new()));
    }
}

#[cfg(feature = "lua")]
#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum ProviderMatch {
    MXSuffix(String),
    DomainSuffix(String),
}

#[cfg(feature = "lua")]
fn toml_table_merge_from(tbl: &mut toml::Table, source: &toml::Table) {
    // Limit merging to just the throttle related fields, as their purpose
    // is for creating broader scoped limits that cut across normal boundaries
    fn is_mergeable(name: &str) -> bool {
        match name {
            "additional_connection_limits" | "additional_message_rate_throttles" => true,
            _ => false,
        }
    }

    for (k, v) in source {
        match (tbl.get_mut(k), v.as_table()) {
            // Merge Table values together, rather than simply replacing them.
            (Some(toml::Value::Table(existing)), Some(v)) if is_mergeable(k) => {
                for (inner_k, inner_v) in v {
                    existing.insert(inner_k.clone(), inner_v.clone());
                }
            }
            _ => {
                tbl.insert(k.clone(), v.clone());
            }
        }
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

    fn hash_into(&self, ctx: &mut Sha256) {
        self.domain_name.as_ref().map(|name| ctx.update(name));
        ctx.update(serde_json::to_string(self).unwrap_or_else(|_| String::new()));
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
    use std::io::Write;
    use tempfile::NamedTempFile;

    async fn make_shaping_configs(inputs: &[&str]) -> Shaping {
        let mut files = vec![];
        let mut file_names = vec![];

        for (i, content) in inputs.iter().enumerate() {
            let mut shaping_file = NamedTempFile::with_prefix(format!("file{i}")).unwrap();
            shaping_file.write_all(content.as_bytes()).unwrap();
            file_names.push(shaping_file.path().to_str().unwrap().to_string());
            files.push(shaping_file);
        }

        Shaping::merge_files(&file_names).await.unwrap()
    }

    #[tokio::test]
    async fn test_merge_additional() {
        let shaping = make_shaping_configs(&[
            r#"
["example.com"]
mx_rollup = false
additional_connection_limits = {"first"=10}
        "#,
            r#"
["example.com"]
mx_rollup = false
additional_connection_limits = {"second"=32}
additional_message_rate_throttles = {"second"="100/hr"}
        "#,
        ])
        .await;

        let resolved = shaping
            .get_egress_path_config("example.com", "invalid.source", "invalid.site")
            .await
            .finish()
            .unwrap();

        k9::snapshot!(
            resolved.params.additional_connection_limits,
            r#"
{
    "first": 10,
    "second": 32,
}
"#
        );
        k9::snapshot!(
            resolved.params.additional_message_rate_throttles,
            r#"
{
    "second": 100/h,
}
"#
        );
    }

    #[tokio::test]
    async fn test_provider() {
        let shaping = make_shaping_configs(&[r#"
[provider."Office 365"]
match=[{MXSuffix=".olc.protection.outlook.com"},{DomainSuffix=".outlook.com"}]
enable_tls = "Required"
provider_connection_limit = 10
provider_max_message_rate = "120/s"
        "#])
        .await;

        let resolved = shaping
            .get_egress_path_config("outlook.com", "invalid.source", "invalid.site")
            .await
            .finish()
            .unwrap();

        k9::assert_equal!(
            resolved.params.enable_tls,
            crate::egress_path::Tls::Required
        );

        k9::snapshot!(
            resolved.params.additional_connection_limits,
            r#"
{
    "shaping-provider-Office 365-invalid.source-limit": 10,
}
"#
        );
        k9::snapshot!(
            resolved.params.additional_message_rate_throttles,
            r#"
{
    "shaping-provider-Office 365-invalid.source-rate": 120/s,
}
"#
        );
    }

    #[tokio::test]
    async fn test_defaults() {
        let shaping = make_shaping_configs(&[
            r#"
["default"]
connection_limit = 10
max_connection_rate = "100/min"
max_deliveries_per_connection = 100
max_message_rate = "100/s"
idle_timeout = "60s"
data_timeout = "30s"
data_dot_timeout = "60s"
enable_tls = "Opportunistic"
consecutive_connection_failures_before_delay = 100

[["default".automation]]
regex=[
        '/Messages from \d+\.\d+\.\d+\.\d+ temporarily deferred/',
        '/All messages from \d+\.\d+\.\d+\.\d+ will be permanently deferred/',
        '/has been temporarily rate limited due to IP reputation/',
        '/Unfortunately, messages from \d+\.\d+\.\d+\.\d+ weren.t sent/',
        '/Server busy\. Please try again later from/'
]
action = [
        {SetConfig={name="max_message_rate", value="1/minute"}},
        {SetConfig={name="connection_limit", value=1}}
]
duration = "90m"

[["default".automation]]
regex="KumoMTA internal: failed to connect to any candidate hosts: All failures are related to OpportunisticInsecure STARTTLS. Consider setting enable_tls=Disabled for this site"
action = {SetConfig={name="enable_tls", value="Disabled"}}
duration = "30 days"

["gmail.com"]
max_deliveries_per_connection = 50
connection_limit = 5
enable_tls = "Required"
consecutive_connection_failures_before_delay = 5

["yahoo.com"]
max_deliveries_per_connection = 20

[["yahoo.com".automation]]
regex = "\\[TS04\\]"
action = "Suspend"
duration = "2 hours"

["comcast.net"]
connection_limit = 25
max_deliveries_per_connection = 250
enable_tls = "Required"
idle_timeout = "30s"
consecutive_connection_failures_before_delay = 24

["mail.com"]
max_deliveries_per_connection = 100

["orange.fr"]
connection_limit = 3

["smtp.mailgun.com"]
connection_limit = 7000
max_deliveries_per_connection = 3

["example.com"]
mx_rollup = false
max_deliveries_per_connection = 100
connection_limit = 3

["example.com".sources."my source name"]
connection_limit = 5
        "#,
        ])
        .await;

        let default = shaping
            .get_egress_path_config("invalid.domain", "invalid.source", "invalid.site")
            .await
            .finish()
            .unwrap();
        k9::snapshot!(
            default,
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 10,
        additional_connection_limits: {},
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
            100/s,
        ),
        additional_message_rate_throttles: {},
        max_connection_rate: Some(
            100/m,
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
        refresh_interval: 60s,
        refresh_strategy: Ttl,
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
            .await
            .finish()
            .unwrap();
        k9::snapshot!(
            example_com,
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 3,
        additional_connection_limits: {},
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
            100/s,
        ),
        additional_message_rate_throttles: {},
        max_connection_rate: Some(
            100/m,
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
        refresh_interval: 60s,
        refresh_strategy: Ttl,
    },
    sources: {
        "my source name": EgressPathConfig {
            connection_limit: 5,
            additional_connection_limits: {},
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
            additional_message_rate_throttles: {},
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
            refresh_interval: 60s,
            refresh_strategy: Ttl,
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
            .await
            .finish()
            .unwrap();
        k9::snapshot!(
            yahoo_com,
            r#"
MergedEntry {
    params: EgressPathConfig {
        connection_limit: 10,
        additional_connection_limits: {},
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
            100/s,
        ),
        additional_message_rate_throttles: {},
        max_connection_rate: Some(
            100/m,
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
        refresh_interval: 60s,
        refresh_strategy: Ttl,
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

    #[tokio::test]
    async fn test_load_default_shaping_toml() {
        Shaping::merge_files(&["../../assets/policy-extras/shaping.toml".into()])
            .await
            .unwrap();
    }
}
