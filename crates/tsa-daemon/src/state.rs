use crate::http_server::{
    import_bounces_from_sqlite, import_configs_from_sqlite, import_suspensions_from_sqlite,
    open_history_db, regex_list_to_string, toml_to_toml_edit_value, PreferRollup, Sha256Hasher,
    DB_PATH,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use kumo_api_types::shaping::{
    Action, EgressPathConfigValue, EgressPathConfigValueUnchecked, MatchScope, MatchedRule, Rule,
};
use kumo_api_types::tsa::{ReadyQSuspension, SchedQBounce, SchedQSuspension};
use kumo_log_types::JsonLogRecord;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::hash::Hash;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

pub static TSA_STATE: OnceLock<TsaState> = OnceLock::new();

/// Represents a specific rule definition.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleHash(#[serde(with = "serde_bytes")] [u8; 32]);

impl RuleHash {
    /// Builds a `RuleHash` from the matched rule and its scope.
    pub fn from_matched(matched: &MatchedRule) -> Self {
        let mut hasher = Sha256Hasher::new();
        matched.hash_into(&mut hasher);
        Self(hasher.get_binary())
    }
}

impl std::fmt::Display for RuleHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        hex::encode(self.0).fmt(fmt)
    }
}

impl std::fmt::Debug for RuleHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_tuple("RuleHash")
            .field(&hex::encode(self.0))
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SiteKey(String);

impl SiteKey {
    pub fn from_record(record: &JsonLogRecord) -> Self {
        Self(record.site.to_string())
    }
}

impl std::fmt::Display for SiteKey {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(fmt)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionHash(#[serde(with = "serde_bytes")] [u8; 32], SiteKey);

impl std::fmt::Display for ActionHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}-{}", self.1 .0, hex::encode(self.0))
    }
}

impl std::fmt::Debug for ActionHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_tuple("ActionHash")
            .field(&hex::encode(self.0))
            .field(&self.1)
            .finish()
    }
}

impl ActionHash {
    pub fn from_matched_and_record(
        matched: &MatchedRule,
        action: &Action,
        record: &JsonLogRecord,
    ) -> Self {
        let mut hasher = Sha256Hasher::new();
        matched.hash_into(&mut hasher);
        action.hash(&mut hasher);
        Self(hasher.get_binary(), SiteKey::from_record(record))
    }

    /// Returns the key for a provider-scoped override, derived from the
    /// provider name and source rather than the triggering record's site.
    pub fn from_matched_action_and_provider(
        matched: &MatchedRule,
        action: &Action,
        source: &str,
        provider: &str,
    ) -> Self {
        let mut hasher = Sha256Hasher::new();
        matched.hash_into(&mut hasher);
        action.hash(&mut hasher);
        Self(
            hasher.get_binary(),
            SiteKey(format!("{source}->provider:{provider}")),
        )
    }

    pub fn from_legacy_hash_and_site(hash: &str, site: &str) -> Self {
        let mut bytes = [0u8; 32];
        if let Err(err) = hex::decode_to_slice(hash, &mut bytes) {
            panic!("invalid action hash ahash={hash} {err:#}");
        }
        Self(bytes, SiteKey(site.to_string()))
    }

    pub fn from_legacy_action_hash_string(full_string: &str) -> Self {
        let Some((site, ahash)) = full_string.rsplit_once('-') else {
            panic!("invalid action hash {full_string}");
        };
        Self::from_legacy_hash_and_site(ahash, site)
    }

    pub fn hash_portion(&self) -> String {
        hex::encode(self.0)
    }

    pub fn site_name(&self) -> &str {
        &self.1 .0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventKey(RuleHash, SiteKey);

impl EventKey {
    pub fn from_matched_and_record(matched: &MatchedRule, record: &JsonLogRecord) -> Self {
        Self(
            RuleHash::from_matched(matched),
            SiteKey::from_record(record),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct EventData {
    /// Used to determine how to prune
    duration: i64,
    series: Vec<UnixTimeStamp>,
}

type UnixTimeStamp = i64;
fn to_unix_ts(dt: &DateTime<Utc>) -> UnixTimeStamp {
    dt.signed_duration_since(DateTime::<Utc>::UNIX_EPOCH)
        .num_seconds()
}

impl EventData {
    fn insert_and_count(&mut self, record: &JsonLogRecord) -> usize {
        let ts = to_unix_ts(&record.timestamp);
        let idx = match self.series.binary_search(&ts) {
            Ok(idx) | Err(idx) => idx,
        };

        self.series.insert(idx, ts);
        let now = Utc::now();
        let now_ts = to_unix_ts(&now);
        let report_thresh = now_ts - self.duration;
        let oldest_permitted = report_thresh - 300;

        self.series.retain(|&ts| ts > oldest_permitted);
        self.series
            .iter()
            .filter(|&&ts| ts >= report_thresh)
            .count()
    }
}

/// Which shaping scope a configuration override is written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideScope {
    /// A domain, or the site_name it rolls up to when `mx_rollup` is set.
    Domain { domain: String, mx_rollup: bool },
    /// Every site_name matched by the named provider. `triggering_domain` is
    /// the recipient domain whose failure produced the override, retained for
    /// provenance and included as a comment in the generated TOML.
    Provider {
        provider: String,
        triggering_domain: String,
    },
}

#[derive(Debug, Clone)]
pub struct ConfigurationOverride {
    pub scope: OverrideScope,
    pub source: String,
    pub reason: String,
    /// Explicitly store unchecked to accommodate version skew
    /// where we might not know about a value yet
    pub option: EgressPathConfigValueUnchecked,
    pub expires: DateTime<Utc>,
}

/// The on-disk form of a [`ConfigurationOverride`]. It is flat and predates
/// [`OverrideScope`]. Keeping serialization pinned to it lets older state
/// files, written before `provider_name` existed, still load.
#[derive(Serialize, Deserialize, Clone)]
struct FlatConfigurationOverride {
    domain: String,
    mx_rollup: bool,
    source: String,
    reason: String,
    option: EgressPathConfigValueUnchecked,
    expires: DateTime<Utc>,
    #[serde(default)]
    provider_name: Option<String>,
}

impl From<FlatConfigurationOverride> for ConfigurationOverride {
    fn from(flat: FlatConfigurationOverride) -> Self {
        let scope = match flat.provider_name {
            Some(provider) => OverrideScope::Provider {
                provider,
                triggering_domain: flat.domain,
            },
            None => OverrideScope::Domain {
                domain: flat.domain,
                mx_rollup: flat.mx_rollup,
            },
        };
        Self {
            scope,
            source: flat.source,
            reason: flat.reason,
            option: flat.option,
            expires: flat.expires,
        }
    }
}

impl From<ConfigurationOverride> for FlatConfigurationOverride {
    fn from(over: ConfigurationOverride) -> Self {
        let (domain, mx_rollup, provider_name) = match over.scope {
            OverrideScope::Domain { domain, mx_rollup } => (domain, mx_rollup, None),
            OverrideScope::Provider {
                provider,
                triggering_domain,
            } => (triggering_domain, false, Some(provider)),
        };
        Self {
            domain,
            mx_rollup,
            source: over.source,
            reason: over.reason,
            option: over.option,
            expires: over.expires,
            provider_name,
        }
    }
}

impl Serialize for ConfigurationOverride {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        FlatConfigurationOverride::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ConfigurationOverride {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        FlatConfigurationOverride::deserialize(deserializer).map(Into::into)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SchedQBounceKey {
    pub action_hash: ActionHash,
    pub domain: String,
    pub tenant: Option<String>,
    pub campaign: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedQBounceEntry {
    pub reason: String,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyQSuspensionEntry {
    pub reason: String,
    pub source: String,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct SchedQSuspensionKey {
    pub action_hash: ActionHash,
    pub domain: String,
    pub tenant: String,
    pub campaign: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedQSuspensionEntry {
    pub reason: String,
    pub expires: DateTime<Utc>,
}

#[derive(Default)]
pub struct TsaState {
    event_history: DashMap<EventKey, EventData>,
    config_overrides: DashMap<ActionHash, ConfigurationOverride>,
    schedq_bounces: DashMap<SchedQBounceKey, SchedQBounceEntry>,
    readyq_suspensions: DashMap<ActionHash, ReadyQSuspensionEntry>,
    schedq_suspensions: DashMap<SchedQSuspensionKey, SchedQSuspensionEntry>,
}

#[derive(Serialize, Deserialize)]
struct SerializableState {
    #[serde(default)]
    event_history: HashMap<EventKey, EventData>,
    #[serde(default)]
    config_overrides: HashMap<ActionHash, ConfigurationOverride>,
    #[serde(default)]
    schedq_bounces: HashMap<SchedQBounceKey, SchedQBounceEntry>,
    #[serde(default)]
    readyq_suspensions: HashMap<ActionHash, ReadyQSuspensionEntry>,
    #[serde(default)]
    schedq_suspensions: HashMap<SchedQSuspensionKey, SchedQSuspensionEntry>,
}

impl TsaState {
    /// Record the current event and return the total number
    /// of records in the time period defined by the rule
    pub fn record_event(&self, scope: &EventKey, rule: &Rule, record: &JsonLogRecord) -> u64 {
        let mut series = self
            .event_history
            .entry(scope.clone())
            .or_insert_with(|| EventData {
                duration: rule.duration.as_secs() as i64,
                series: vec![],
            });

        series.insert_and_count(record) as u64
    }

    pub fn create_config_override(
        &self,
        scope: &ActionHash,
        matched: &MatchedRule,
        record: &JsonLogRecord,
        config: &EgressPathConfigValue,
        domain: &str,
        source: &str,
        prefer_rollup: PreferRollup,
    ) {
        let reason = format!(
            "automation rule: {}",
            regex_list_to_string(&matched.rule.regex)
        );
        self.insert_config_override(
            scope.clone(),
            ConfigurationOverride {
                scope: OverrideScope::Domain {
                    domain: domain.to_string(),
                    mx_rollup: match prefer_rollup {
                        // SetConfig honors the scope the rule matched in.
                        // mx_rollup=true applies the override at the triggering
                        // domain's site_name (its MX cluster). A rollup
                        // (default/site) or provider match uses it, while a
                        // rule from an mx_rollup=false domain block stays
                        // pinned to the domain.
                        PreferRollup::Yes => match &matched.scope {
                            MatchScope::Rollup | MatchScope::Provider(_) => true,
                            MatchScope::Domain => false,
                        },
                        // SetDomainConfig always pins to the triggering domain.
                        PreferRollup::No => false,
                    },
                },
                reason,
                source: source.to_string(),
                option: config.clone().into(),
                expires: record.timestamp + matched.rule.duration,
            },
        );
    }

    pub fn create_provider_config_override(
        &self,
        scope: &ActionHash,
        rule: &Rule,
        record: &JsonLogRecord,
        config: &EgressPathConfigValue,
        provider_name: &str,
        domain: &str,
        source: &str,
    ) {
        let reason = format!("automation rule: {}", regex_list_to_string(&rule.regex));
        let over = ConfigurationOverride {
            scope: OverrideScope::Provider {
                provider: provider_name.to_string(),
                triggering_domain: domain.to_string(),
            },
            reason,
            source: source.to_string(),
            option: config.clone().into(),
            expires: record.timestamp + rule.duration,
        };
        self.insert_provider_config_override(scope.clone(), over);
    }

    /// Inserts a provider-scoped override, keyed by provider and source,
    /// replacing any existing row for that key only if it has a later
    /// expiry.
    fn insert_provider_config_override(&self, scope: ActionHash, over: ConfigurationOverride) {
        if Utc::now() >= over.expires {
            // Skip already expired entry
            return;
        }
        if let Some(existing) = self.config_overrides.get(&scope) {
            if existing.expires > over.expires {
                return;
            }
        }
        tracing::debug!("new config override {scope:?} = {over:?}");
        self.config_overrides.insert(scope, over);
    }

    pub fn insert_config_override(&self, scope: ActionHash, over: ConfigurationOverride) {
        if Utc::now() >= over.expires {
            // Skip already expired entry
            return;
        }

        tracing::debug!("new config override {scope:?} = {over:?}");
        self.config_overrides.insert(scope, over);
    }

    pub fn insert_schedq_bounce(&self, key: SchedQBounceKey, bounce: SchedQBounceEntry) {
        if Utc::now() >= bounce.expires {
            // Skip already expired entry
            return;
        }

        tracing::debug!("new schedq bounce {key:?} = {bounce:?}");
        self.schedq_bounces.insert(key, bounce);
    }

    pub fn insert_readyq_suspension(&self, key: ActionHash, entry: ReadyQSuspensionEntry) {
        if Utc::now() >= entry.expires {
            // Skip already expired entry
            return;
        }

        tracing::debug!("new readyq suspension {key:?} = {entry:?}");
        self.readyq_suspensions.insert(key, entry);
    }

    pub fn insert_schedq_suspension(&self, key: SchedQSuspensionKey, entry: SchedQSuspensionEntry) {
        if Utc::now() >= entry.expires {
            // Skip already expired entry
            return;
        }

        tracing::debug!("new sched suspension {key:?} = {entry:?}");
        self.schedq_suspensions.insert(key, entry);
    }

    pub fn export_schedq_suspensions(&self) -> Vec<SchedQSuspension> {
        let mut entries = vec![];
        let now = Utc::now();
        for entry in self.schedq_suspensions.iter() {
            let value = entry.value();
            if now >= value.expires {
                continue;
            }
            let key = entry.key();
            entries.push(SchedQSuspension {
                rule_hash: key.action_hash.to_string(),
                domain: key.domain.clone(),
                campaign: key.campaign.clone(),
                tenant: key.tenant.clone(),
                reason: value.reason.clone(),
                expires: value.expires,
            });
        }

        entries.sort_by_key(|over| {
            (
                over.expires,
                over.tenant.clone(),
                over.domain.clone(),
                over.campaign.clone(),
            )
        });

        entries
    }

    pub fn export_readyq_suspensions(&self) -> Vec<ReadyQSuspension> {
        let mut entries = vec![];
        let now = Utc::now();
        for entry in self.readyq_suspensions.iter() {
            let value = entry.value();
            if now >= value.expires {
                continue;
            }
            let key = entry.key();
            entries.push(ReadyQSuspension {
                rule_hash: key.hash_portion(),
                site_name: key.site_name().to_string(),
                source: value.source.clone(),
                reason: value.reason.clone(),
                expires: value.expires,
            });
        }

        entries.sort_by_key(|over| (over.expires, over.source.clone()));

        entries
    }

    pub fn export_schedq_bounces(&self) -> Vec<SchedQBounce> {
        let mut entries = vec![];
        let now = Utc::now();
        for entry in self.schedq_bounces.iter() {
            let value = entry.value();
            if now >= value.expires {
                continue;
            }
            let key = entry.key();
            entries.push(SchedQBounce {
                rule_hash: key.action_hash.to_string(),
                domain: key.domain.clone(),
                tenant: key.tenant.clone(),
                campaign: key.campaign.clone(),
                reason: value.reason.clone(),
                expires: value.expires,
            });
        }

        entries.sort_by_key(|over| {
            (
                over.expires,
                over.tenant.clone(),
                over.domain.clone(),
                over.campaign.clone(),
            )
        });

        entries
    }

    pub fn export_config_override_toml(&self) -> String {
        use toml_edit::{value, Item};
        let mut doc = toml_edit::DocumentMut::new();
        let now = Utc::now();

        let mut entries = vec![];
        for entry in self.config_overrides.iter() {
            let over = entry.value();
            if now >= over.expires {
                continue;
            }
            entries.push(over.clone());
        }

        entries.sort_by_key(|over| {
            // Sort by expiry, then by provider name or domain, grouping
            // entries that share a scope.
            let (provider, domain) = match &over.scope {
                OverrideScope::Provider { provider, .. } => (Some(provider.clone()), String::new()),
                OverrideScope::Domain { domain, .. } => (None, domain.clone()),
            };
            (
                over.expires,
                provider,
                domain,
                over.source.clone(),
                over.option.name.clone(),
            )
        });
        let num_entries = entries.len();

        // A domain's overrides all share one mx_rollup flag in the exported
        // table. When a domain has both a rolled-up and a pinned override, we
        // resolve the conflict to rolled-up: that keeps a site- or
        // provider-scoped override covering its whole MX cluster rather than
        // shrinking to the one triggering domain. Such a conflict means the
        // operator's rules for that domain are internally inconsistent, and it
        // is up to them to make the domain consistently roll up or not.
        let mut rollup_domains = BTreeSet::new();
        for over in &entries {
            if let OverrideScope::Domain {
                domain,
                mx_rollup: true,
            } = &over.scope
            {
                rollup_domains.insert(domain.clone());
            }
        }

        for over in entries {
            let scope_entry = match &over.scope {
                OverrideScope::Provider { provider, .. } => {
                    let providers = doc
                        .entry("provider")
                        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .unwrap();
                    providers
                        .entry(provider)
                        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
                        .as_table_mut()
                        .unwrap()
                }
                OverrideScope::Domain { domain, .. } => doc
                    .entry(domain)
                    .or_insert_with(|| {
                        let mut tbl = toml_edit::Table::new();
                        // Use the rollup-wins flag computed over all of this
                        // domain's overrides (rollup_domains above), rather
                        // than deciding it from whichever override for the
                        // domain is inserted into the table first.
                        tbl["mx_rollup"] = value(rollup_domains.contains(domain));
                        Item::Table(tbl)
                    })
                    .as_table_mut()
                    .unwrap(),
            };
            let sources = scope_entry
                .entry("sources")
                .or_insert_with(|| {
                    let tbl = toml_edit::Table::new();
                    Item::Table(tbl)
                })
                .as_table_mut()
                .unwrap();
            let source_entry = sources
                .entry(&over.source)
                .or_insert_with(|| {
                    let tbl = toml_edit::Table::new();
                    Item::Table(tbl)
                })
                .as_table_mut()
                .unwrap();

            let item = toml_to_toml_edit_value(over.option.value.clone());
            source_entry.insert(&over.option.name, Item::Value(item));

            if let Some(mut key) = source_entry.key_mut(&over.option.name) {
                // This is the only place a provider override's triggering
                // domain appears in the output.
                let provenance = match &over.scope {
                    OverrideScope::Provider {
                        triggering_domain, ..
                    } => format!("# triggering domain: {triggering_domain}\n"),
                    OverrideScope::Domain { .. } => String::new(),
                };
                key.leaf_decor_mut().set_prefix(format!(
                    "# reason: {}\n{provenance}# expires: {}\n",
                    over.reason,
                    over.expires.to_rfc3339()
                ));
            }
        }

        format!(
            "# Generated by tsa-daemon\n# Number of entries: {num_entries}\n\n\
            {}\n\n\
            # Generated by tsa-daemon\n# Number of entries: {num_entries}\n",
            doc
        )
    }

    /// Return a serializable version of the state
    fn serializable(&self) -> SerializableState {
        SerializableState {
            event_history: self
                .event_history
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
            config_overrides: self
                .config_overrides
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
            schedq_bounces: self
                .schedq_bounces
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
            readyq_suspensions: self
                .readyq_suspensions
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
            schedq_suspensions: self
                .schedq_suspensions
                .iter()
                .map(|entry| (entry.key().clone(), entry.value().clone()))
                .collect(),
        }
    }

    async fn prune(&self, verbose: bool) {
        let now = Utc::now();
        let now_ts = to_unix_ts(&now);
        self.prune_events(now_ts, verbose).await;
        self.prune_config_overrides(&now, verbose).await;
        self.prune_readyq_suspensions(&now, verbose).await;
        self.prune_schedq_suspensions(&now, verbose).await;
        self.prune_schedq_bounces(&now, verbose).await;
    }

    async fn prune_schedq_bounces(&self, now: &DateTime<Utc>, verbose: bool) {
        let mut visited = 0;
        let start = Instant::now();

        let is_prunable = |entry: &SchedQBounceEntry| *now >= entry.expires;

        let keys_to_prune: Vec<SchedQBounceKey> = self
            .schedq_bounces
            .iter()
            .filter_map(|entry| {
                visited += 1;
                let over = entry.value();
                if is_prunable(over) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let mut num_pruned = 0;
        for key in keys_to_prune {
            let pruned = self
                .schedq_bounces
                .remove_if(&key, |_key, entry| is_prunable(entry))
                .is_some();
            if pruned {
                num_pruned += 1;
            }
        }
        if verbose && num_pruned > 0 {
            tracing::info!("Pruned {num_pruned} schedq_bounces");
        }
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            schedq_bounces in {:?}",
            start.elapsed()
        );
    }

    async fn prune_schedq_suspensions(&self, now: &DateTime<Utc>, verbose: bool) {
        let mut visited = 0;
        let start = Instant::now();

        let is_prunable = |entry: &SchedQSuspensionEntry| *now >= entry.expires;

        let keys_to_prune: Vec<SchedQSuspensionKey> = self
            .schedq_suspensions
            .iter()
            .filter_map(|entry| {
                visited += 1;
                let over = entry.value();
                if is_prunable(over) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let mut num_pruned = 0;
        for key in keys_to_prune {
            let pruned = self
                .schedq_suspensions
                .remove_if(&key, |_key, entry| is_prunable(entry))
                .is_some();
            if pruned {
                num_pruned += 1;
            }
        }
        if verbose && num_pruned > 0 {
            tracing::info!("Pruned {num_pruned} schedq_suspensions");
        }
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            scheq_suspensions in {:?}",
            start.elapsed()
        );
    }

    async fn prune_readyq_suspensions(&self, now: &DateTime<Utc>, verbose: bool) {
        let mut visited = 0;
        let start = Instant::now();

        let is_prunable = |entry: &ReadyQSuspensionEntry| *now >= entry.expires;

        let keys_to_prune: Vec<ActionHash> = self
            .readyq_suspensions
            .iter()
            .filter_map(|entry| {
                visited += 1;
                let over = entry.value();
                if is_prunable(over) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let mut num_pruned = 0;
        for key in keys_to_prune {
            let pruned = self
                .readyq_suspensions
                .remove_if(&key, |_key, entry| is_prunable(entry))
                .is_some();
            if pruned {
                num_pruned += 1;
            }
        }
        if verbose && num_pruned > 0 {
            tracing::info!("Pruned {num_pruned} readyq_suspensions");
        }
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            readyq_suspensions in {:?}",
            start.elapsed()
        );
    }

    async fn prune_config_overrides(&self, now: &DateTime<Utc>, verbose: bool) {
        let mut visited = 0;
        let start = Instant::now();

        let is_prunable = |over: &ConfigurationOverride| *now >= over.expires;

        let keys_to_prune: Vec<ActionHash> = self
            .config_overrides
            .iter()
            .filter_map(|entry| {
                visited += 1;
                let over = entry.value();
                if is_prunable(over) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let mut num_pruned = 0;
        for key in keys_to_prune {
            let pruned = self
                .config_overrides
                .remove_if(&key, |_key, over| is_prunable(over))
                .is_some();
            if pruned {
                num_pruned += 1;
            }
        }
        if verbose && num_pruned > 0 {
            tracing::info!("Pruned {num_pruned} config_overrides");
        }
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            config_overrides entries in {:?}",
            start.elapsed()
        );
    }

    async fn prune_events(&self, now_ts: UnixTimeStamp, verbose: bool) {
        let mut visited = 0;
        let start = Instant::now();

        let is_prunable = |event_data: &EventData| {
            event_data
                .series
                .last()
                .map(|&last_ts| {
                    let oldest_permitted = now_ts - event_data.duration - 300;
                    last_ts < oldest_permitted
                })
                .unwrap_or(true)
        };

        let keys_to_prune: Vec<EventKey> = self
            .event_history
            .iter()
            .filter_map(|entry| {
                visited += 1;
                let event_data = entry.value();
                if is_prunable(event_data) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let mut num_pruned = 0;
        for key in keys_to_prune {
            let pruned = self
                .event_history
                .remove_if(&key, |_key, event_data| is_prunable(event_data))
                .is_some();
            if pruned {
                num_pruned += 1;
            }
        }
        if verbose && num_pruned > 0 {
            tracing::info!("Pruned {num_pruned} event_history entries");
        }
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            event_history entries in {:?}",
            start.elapsed()
        );
    }
}

fn state_path() -> String {
    let path = DB_PATH.lock().clone();
    format!("{path}.state")
}

pub async fn load_state() -> anyhow::Result<()> {
    let path = state_path();
    let state = match tokio::fs::read(&path).await {
        Ok(data) => {
            let state = TsaState::default();
            match rmp_serde::from_slice::<SerializableState>(&data) {
                Ok(loaded) => {
                    for (key, value) in loaded.event_history.into_iter() {
                        state.event_history.insert(key, value);
                    }
                    for (key, value) in loaded.config_overrides.into_iter() {
                        state.config_overrides.insert(key, value);
                    }
                    for (key, value) in loaded.schedq_bounces.into_iter() {
                        state.schedq_bounces.insert(key, value);
                    }
                    for (key, value) in loaded.readyq_suspensions.into_iter() {
                        state.readyq_suspensions.insert(key, value);
                    }
                    for (key, value) in loaded.schedq_suspensions.into_iter() {
                        state.schedq_suspensions.insert(key, value);
                    }
                    state.prune(true).await;

                    tracing::info!(
                        "Loaded {} of state data from {path}",
                        humansize::format_size(data.len(), humansize::DECIMAL)
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        "Failed to deserialize {path}: {err:#}, proceeding with fresh state"
                    );
                }
            }
            state
        }
        Err(err) => {
            tracing::warn!(
                "Failed to load state from {path}, proceeding with fresh state. Error was: {err:#}"
            );
            TsaState::default()
        }
    };

    let import_holder = Arc::new(state);

    let need_import = import_holder.config_overrides.is_empty()
        || import_holder.schedq_bounces.is_empty()
        || import_holder.schedq_suspensions.is_empty()
        || import_holder.readyq_suspensions.is_empty();

    if need_import {
        if let Ok(database) = open_history_db() {
            let mut num_config_overrides = 0;
            let mut num_schedq_bounces = 0;
            let mut num_schedq_suspensions = 0;
            let mut num_readyq_suspensions = 0;

            if import_holder.config_overrides.is_empty() {
                // Import configs from the sqlite database
                if let Err(err) = import_configs_from_sqlite(&database, import_holder.clone()).await
                {
                    tracing::warn!(
                        "Failed to import legacy config entries from sqlite: {err:#}. Proceeding without them");
                } else {
                    num_config_overrides += import_holder.config_overrides.len();
                }
            }

            if import_holder.schedq_bounces.is_empty() {
                if let Err(err) = import_bounces_from_sqlite(&database, import_holder.clone()).await
                {
                    tracing::warn!(
                        "Failed to import legacy bounce entries from sqlite: {err:#}. Proceeding without them");
                } else {
                    num_schedq_bounces += import_holder.schedq_bounces.len();
                }
            }

            if import_holder.schedq_suspensions.is_empty()
                && import_holder.readyq_suspensions.is_empty()
            {
                if let Err(err) =
                    import_suspensions_from_sqlite(&database, import_holder.clone()).await
                {
                    tracing::warn!(
                        "Failed to import legacy suspension entries from sqlite: {err:#}. Proceeding without them");
                } else {
                    num_readyq_suspensions += import_holder.readyq_suspensions.len();
                    num_schedq_suspensions += import_holder.schedq_suspensions.len();
                }
            }

            let did_import = num_config_overrides
                + num_schedq_bounces
                + num_schedq_suspensions
                + num_readyq_suspensions
                > 0;

            if did_import {
                tracing::info!(
                    "Imported {num_config_overrides} config overrides, \
                    {num_schedq_bounces} schedq bounces, \
                    {num_schedq_suspensions} schedq suspensions, \
                    {num_readyq_suspensions} readyq suspensions \
                    from sqlite"
                );
            }
        }
    }

    let state = Arc::into_inner(import_holder).expect("only we hold a ref");

    let num_config_overrides = state.config_overrides.len();
    let num_schedq_bounces = state.schedq_bounces.len();
    let num_schedq_suspensions = state.schedq_suspensions.len();
    let num_readyq_suspensions = state.readyq_suspensions.len();
    let num_events = state.event_history.len();

    tracing::info!(
        "State has {num_config_overrides} config overrides, \
        {num_schedq_bounces} schedq bounces, {num_schedq_suspensions} schedq suspensions, \
        {num_readyq_suspensions} readyq suspensions, {num_events} events."
    );

    TSA_STATE.set(state).ok();
    Ok(())
}

pub async fn save_state(background: bool) -> anyhow::Result<()> {
    let start = Instant::now();
    let state = TSA_STATE
        .get()
        .expect("state not initialized")
        .serializable();
    let extract = start.elapsed();

    let data = rmp_serde::to_vec_named(&state).context("failed to serialize state")?;
    let path = state_path();

    let start = Instant::now();
    tokio::fs::write(&path, &data)
        .await
        .with_context(|| format!("failed to write to {path}"))?;
    let write = start.elapsed();

    let num_config_overrides = state.config_overrides.len();
    let num_schedq_bounces = state.schedq_bounces.len();
    let num_schedq_suspensions = state.schedq_suspensions.len();
    let num_readyq_suspensions = state.readyq_suspensions.len();
    let num_events = state.event_history.len();

    let message = format!(
        "stored {} of data to {path}. State has {num_config_overrides} config overrides, \
        {num_schedq_bounces} schedq bounces, {num_schedq_suspensions} schedq suspensions, \
        {num_readyq_suspensions} readyq suspensions, {num_events} events. \
        (Extract took {extract:?}, write took {write:?})",
        humansize::format_size(data.len(), humansize::DECIMAL)
    );

    if background {
        tracing::debug!("{message}");
    } else {
        tracing::info!("{message}");
    }

    Ok(())
}

pub async fn state_pruner() -> anyhow::Result<()> {
    let mut last_save = Instant::now();

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Some(state) = TSA_STATE.get() {
            state.prune(false).await;
        }

        if last_save.elapsed() > std::time::Duration::from_secs(300) {
            if let Err(err) = save_state(true).await {
                tracing::error!("{err:#} saving state file");
            }
            last_save = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kumo_api_types::shaping::{HashableTomlValue, MatchScope, Regex, Trigger};
    use kumo_log_types::RecordType;
    use rfc5321::Response;
    use uuid::Uuid;

    fn override_with_scope(scope: OverrideScope) -> ConfigurationOverride {
        ConfigurationOverride {
            scope,
            source: "my-source".to_string(),
            reason: "automation rule: TS04".to_string(),
            option: EgressPathConfigValueUnchecked {
                name: "connection_limit".to_string(),
                value: toml::Value::Integer(3),
            },
            expires: Utc::now() + chrono::Duration::hours(1),
        }
    }

    /// Replaces the non-deterministic `# expires:` lines in an exported
    /// overlay with a relative offset (rounded up to a 10s boundary), making
    /// the overlay stable to snapshot while keeping the ordering between
    /// entries visible.
    fn normalize_overlay(exported: String) -> String {
        let now = Utc::now();
        exported
            .lines()
            .map(|line| match line.strip_prefix("# expires: ") {
                Some(ts) => {
                    // Render the expiry relative to now, rounded up to a 10s
                    // boundary to absorb test execution time. That keeps the
                    // snapshot stable while still revealing the ordering
                    // between entries.
                    let expires = DateTime::parse_from_rfc3339(ts)
                        .expect("expires is rfc3339")
                        .with_timezone(&Utc);
                    let secs = (expires - now).num_seconds().max(0);
                    let rounded = (secs + 9) / 10 * 10;
                    let rel = if rounded % 60 == 0 {
                        format!("{} minutes", rounded / 60)
                    } else {
                        format!("{rounded}s")
                    };
                    format!("# expires: <in {rel}>")
                }
                None => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Renders the overlay for one override, normalized for snapshotting.
    fn export_normalized(over: ConfigurationOverride) -> String {
        let state = TsaState::default();
        state.insert_config_override(
            ActionHash::from_legacy_hash_and_site(&"0".repeat(64), "site"),
            over,
        );
        normalize_overlay(state.export_config_override_toml())
    }

    #[test]
    fn provider_override_renders_as_provider_block() {
        k9::snapshot!(
            export_normalized(override_with_scope(OverrideScope::Provider {
                provider: "yahoo".to_string(),
                triggering_domain: "one.example.com".to_string(),
            })),
            "
# Generated by tsa-daemon
# Number of entries: 1

[provider]

[provider.yahoo]

[provider.yahoo.sources]

[provider.yahoo.sources.my-source]
# reason: automation rule: TS04
# triggering domain: one.example.com
# expires: <in 60 minutes>
connection_limit = 3


# Generated by tsa-daemon
# Number of entries: 1
"
        );
    }

    #[test]
    fn domain_override_renders_as_domain_block() {
        k9::snapshot!(
            export_normalized(override_with_scope(OverrideScope::Domain {
                domain: "example.com".to_string(),
                mx_rollup: false,
            })),
            r#"
# Generated by tsa-daemon
# Number of entries: 1

["example.com"]
mx_rollup = false

["example.com".sources]

["example.com".sources.my-source]
# reason: automation rule: TS04
# expires: <in 60 minutes>
connection_limit = 3


# Generated by tsa-daemon
# Number of entries: 1
"#
        );
    }

    fn make_rule_with_action(action: Action) -> Rule {
        Rule {
            regex: vec![Regex::try_from("x".to_string()).unwrap()],
            action: vec![action],
            trigger: Trigger::Immediate,
            duration: std::time::Duration::from_secs(3600),
            match_internal: false,
        }
    }

    fn make_rule() -> Rule {
        make_rule_with_action(Action::SetProviderConfig(EgressPathConfigValue {
            name: "connection_limit".to_string(),
            value: HashableTomlValue::from(toml::Value::Integer(3)),
        }))
    }

    fn make_matched(rule: Rule, scope: MatchScope) -> MatchedRule {
        MatchedRule { rule, scope }
    }

    fn make_record_at(timestamp: DateTime<Utc>, recipient: &str) -> JsonLogRecord {
        JsonLogRecord {
            kind: RecordType::TransientFailure,
            id: String::new(),
            sender: String::new(),
            recipient: vec![recipient.to_string()],
            queue: String::new(),
            site: "yahoo.example".to_string(),
            size: 0,
            response: Response {
                code: 400,
                command: None,
                enhanced_code: None,
                content: "x".to_string(),
            },
            peer_address: None,
            timestamp,
            created: timestamp,
            num_attempts: 1,
            bounce_classification: Default::default(),
            egress_pool: None,
            egress_source: None,
            source_address: None,
            feedback_report: None,
            meta: Default::default(),
            headers: Default::default(),
            delivery_protocol: None,
            reception_protocol: None,
            nodeid: Uuid::default(),
            tls_cipher: None,
            tls_protocol_version: None,
            tls_peer_subject_name: None,
            provider_name: None,
            session_id: None,
        }
    }

    #[test]
    fn provider_override_keeps_latest_expiry_on_refresh() {
        let state = TsaState::default();
        let rule = make_rule();
        let matched = make_matched(rule, MatchScope::Provider("yahoo".to_string()));
        let action = &matched.rule.action[0];
        let config = match action {
            Action::SetProviderConfig(config) => config,
            _ => panic!("expected SetProviderConfig"),
        };
        let scope =
            ActionHash::from_matched_action_and_provider(&matched, action, "my-source", "yahoo");

        let base = Utc::now();
        // A record timestamped 10 minutes later arrives first...
        let later = make_record_at(base + chrono::Duration::minutes(10), "two.example.com");
        state.create_provider_config_override(
            &scope,
            &matched.rule,
            &later,
            config,
            "yahoo",
            "two.example.com",
            "my-source",
        );
        let first_expires = state.config_overrides.get(&scope).unwrap().expires;

        // ...then a delayed sibling record with an earlier timestamp arrives.
        // It must not shorten the stored override.
        let earlier = make_record_at(base, "one.example.com");
        state.create_provider_config_override(
            &scope,
            &matched.rule,
            &earlier,
            config,
            "yahoo",
            "one.example.com",
            "my-source",
        );
        let second_expires = state.config_overrides.get(&scope).unwrap().expires;

        k9::assert_equal!(
            second_expires,
            first_expires,
            "earlier-timestamped record must not shorten the override"
        );
    }

    #[test]
    fn identical_rules_in_different_scopes_get_distinct_keys() {
        let base = Utc::now();
        let record = make_record_at(base, "one.example.com");

        // Same rule reached through a default/site rollup block and through a
        // domain block: the scope must keep their keys apart.
        let rollup = make_matched(make_rule(), MatchScope::Rollup);
        let domain = make_matched(make_rule(), MatchScope::Domain);

        let rollup_key = EventKey::from_matched_and_record(&rollup, &record);
        let domain_key = EventKey::from_matched_and_record(&domain, &record);
        assert_ne!(
            rollup_key, domain_key,
            "rollup and domain matches of the same rule must not share an EventKey"
        );

        // Same rule reached through two different provider blocks.
        let prov_a = make_matched(make_rule(), MatchScope::Provider("yahoo".to_string()));
        let prov_b = make_matched(make_rule(), MatchScope::Provider("office365".to_string()));
        let prov_a_hash =
            ActionHash::from_matched_and_record(&prov_a, &prov_a.rule.action[0], &record);
        let prov_b_hash =
            ActionHash::from_matched_and_record(&prov_b, &prov_b.rule.action[0], &record);
        assert_ne!(
            prov_a_hash, prov_b_hash,
            "provider matches of the same rule must not share an ActionHash"
        );
    }

    #[test]
    fn provider_set_config_override_rolls_up() {
        // A provider-matched SetConfig rolls the override up to the triggering
        // domain's site_name: the rendered table contains mx_rollup=true.
        let state = TsaState::default();
        let matched = make_matched(make_rule(), MatchScope::Provider("yahoo".to_string()));
        let record = make_record_at(Utc::now(), "one.example.com");
        let config = EgressPathConfigValue {
            name: "connection_limit".to_string(),
            value: HashableTomlValue::from(toml::Value::Integer(3)),
        };
        let scope = ActionHash::from_matched_and_record(&matched, &matched.rule.action[0], &record);
        state.create_config_override(
            &scope,
            &matched,
            &record,
            &config,
            "one.example.com",
            "my-source",
            PreferRollup::Yes,
        );

        k9::snapshot!(
            normalize_overlay(state.export_config_override_toml()),
            r#"
# Generated by tsa-daemon
# Number of entries: 1

["one.example.com"]
mx_rollup = true

["one.example.com".sources]

["one.example.com".sources.my-source]
# reason: automation rule: x
# expires: <in 60 minutes>
connection_limit = 3


# Generated by tsa-daemon
# Number of entries: 1
"#
        );
    }

    /// Insert one override for `one.example.com` under the shared `my-source`,
    /// with the given scope, rollup preference, option name and expiry.
    fn insert_scoped_override(
        state: &TsaState,
        scope: MatchScope,
        prefer_rollup: PreferRollup,
        option: &str,
        expiry: DateTime<Utc>,
    ) {
        let matched = make_matched(make_rule(), scope);
        // Back-date the record to make the override it produce expire at
        // `expiry`.
        let record = make_record_at(
            expiry - chrono::Duration::from_std(matched.rule.duration).unwrap(),
            "one.example.com",
        );
        let action_hash =
            ActionHash::from_matched_and_record(&matched, &matched.rule.action[0], &record);
        let config = EgressPathConfigValue {
            name: option.to_string(),
            value: HashableTomlValue::from(toml::Value::Integer(1)),
        };
        state.create_config_override(
            &action_hash,
            &matched,
            &record,
            &config,
            "one.example.com",
            "my-source",
            prefer_rollup,
        );
    }

    /// Give one domain both a pinned (SetDomainConfig) and a rolled-up (provider
    /// SetConfig) override with the given expiries, and return the rendered
    /// overlay (normalized for snapshotting).
    fn render_mixed_scope(pinned_expiry: DateTime<Utc>, rollup_expiry: DateTime<Utc>) -> String {
        let state = TsaState::default();
        insert_scoped_override(
            &state,
            MatchScope::Domain,
            PreferRollup::No,
            "max_message_rate",
            pinned_expiry,
        );
        insert_scoped_override(
            &state,
            MatchScope::Provider("yahoo".to_string()),
            PreferRollup::Yes,
            "connection_limit",
            rollup_expiry,
        );
        normalize_overlay(state.export_config_override_toml())
    }

    #[test]
    fn mixed_scope_override_table_rolls_up() {
        // A domain with both a pinned (SetDomainConfig) and a rolled-up
        // (provider SetConfig) override shares one mx_rollup flag, which comes
        // out true whichever override's expiry sorts first in the export. The
        // two renderings differ only in the order of the two options.
        let early = Utc::now() + chrono::Duration::minutes(10);
        let late = Utc::now() + chrono::Duration::minutes(20);

        // Pinned override expires first and sorts ahead of the rolled-up one.
        k9::snapshot!(
            render_mixed_scope(early, late),
            r#"
# Generated by tsa-daemon
# Number of entries: 2

["one.example.com"]
mx_rollup = true

["one.example.com".sources]

["one.example.com".sources.my-source]
# reason: automation rule: x
# expires: <in 10 minutes>
max_message_rate = 1
# reason: automation rule: x
# expires: <in 20 minutes>
connection_limit = 1


# Generated by tsa-daemon
# Number of entries: 2
"#
        );

        // Rolled-up override expires first.
        k9::snapshot!(
            render_mixed_scope(late, early),
            r#"
# Generated by tsa-daemon
# Number of entries: 2

["one.example.com"]
mx_rollup = true

["one.example.com".sources]

["one.example.com".sources.my-source]
# reason: automation rule: x
# expires: <in 10 minutes>
connection_limit = 1
# reason: automation rule: x
# expires: <in 20 minutes>
max_message_rate = 1


# Generated by tsa-daemon
# Number of entries: 2
"#
        );
    }
}
