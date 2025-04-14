use crate::http_server::{
    import_bounces_from_sqlite, import_configs_from_sqlite, import_suspensions_from_sqlite,
    open_history_db, regex_list_to_string, toml_to_toml_edit_value, PreferRollup, Sha256Hasher,
    DB_PATH,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use kumo_api_types::shaping::{
    Action, EgressPathConfigValue, EgressPathConfigValueUnchecked, Rule,
};
use kumo_api_types::tsa::{ReadyQSuspension, SchedQBounce, SchedQSuspension};
use kumo_log_types::JsonLogRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

pub static TSA_STATE: OnceLock<TsaState> = OnceLock::new();

/// Represents a specific rule definition.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleHash(#[serde(with = "serde_bytes")] [u8; 32]);

impl RuleHash {
    pub fn from_rule(rule: &Rule) -> Self {
        let mut hasher = Sha256Hasher::new();
        rule.hash(&mut hasher);
        Self(hasher.get_binary())
    }
}

impl std::fmt::Display for RuleHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        hex::encode(&self.0).fmt(fmt)
    }
}

impl std::fmt::Debug for RuleHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_tuple("RuleHash")
            .field(&hex::encode(&self.0))
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
        write!(fmt, "{}-{}", self.1 .0, hex::encode(&self.0))
    }
}

impl std::fmt::Debug for ActionHash {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        fmt.debug_tuple("ActionHash")
            .field(&hex::encode(&self.0))
            .field(&self.1)
            .finish()
    }
}

impl ActionHash {
    pub fn from_rule_and_record(rule: &Rule, action: &Action, record: &JsonLogRecord) -> Self {
        let mut hasher = Sha256Hasher::new();
        rule.hash(&mut hasher);
        action.hash(&mut hasher);
        Self(hasher.get_binary(), SiteKey::from_record(record))
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
        hex::encode(&self.0)
    }

    pub fn site_name(&self) -> &str {
        &self.1 .0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MatchingScope(RuleHash, SiteKey);

impl MatchingScope {
    pub fn from_rule_and_record(rule: &Rule, record: &JsonLogRecord) -> Self {
        Self(RuleHash::from_rule(rule), SiteKey::from_record(record))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigurationOverride {
    pub domain: String,
    pub mx_rollup: bool,
    pub source: String,
    pub reason: String,
    /// Explicitly store unchecked to accommodate version skew
    /// where we might not know about a value yet
    pub option: EgressPathConfigValueUnchecked,
    pub expires: DateTime<Utc>,
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
    event_history: DashMap<MatchingScope, EventData>,
    config_overrides: DashMap<ActionHash, ConfigurationOverride>,
    schedq_bounces: DashMap<SchedQBounceKey, SchedQBounceEntry>,
    readyq_suspensions: DashMap<ActionHash, ReadyQSuspensionEntry>,
    schedq_suspensions: DashMap<SchedQSuspensionKey, SchedQSuspensionEntry>,
}

#[derive(Serialize, Deserialize)]
struct SerializableState {
    #[serde(default)]
    event_history: HashMap<MatchingScope, EventData>,
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
    pub fn record_event(&self, scope: &MatchingScope, rule: &Rule, record: &JsonLogRecord) -> u64 {
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
        rule: &Rule,
        record: &JsonLogRecord,
        config: &EgressPathConfigValue,
        domain: &str,
        source: &str,
        prefer_rollup: PreferRollup,
    ) {
        let reason = format!("automation rule: {}", regex_list_to_string(&rule.regex));
        self.insert_config_override(
            scope.clone(),
            ConfigurationOverride {
                domain: domain.to_string(),
                reason,
                mx_rollup: match prefer_rollup {
                    PreferRollup::Yes => rule.was_rollup,
                    PreferRollup::No => false,
                },
                source: source.to_string(),
                option: config.clone().into(),
                expires: record.timestamp + rule.duration,
            },
        );
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
                expires: value.expires.clone(),
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
                expires: value.expires.clone(),
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
                expires: value.expires.clone(),
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
            (
                over.expires,
                over.domain.clone(),
                over.source.clone(),
                over.option.name.clone(),
            )
        });
        let num_entries = entries.len();

        for over in entries {
            let domain_entry = doc
                .entry(&over.domain)
                .or_insert_with(|| {
                    let mut tbl = toml_edit::Table::new();
                    tbl["mx_rollup"] = value(over.mx_rollup);
                    Item::Table(tbl)
                })
                .as_table_mut()
                .unwrap();
            let sources = domain_entry
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
                key.leaf_decor_mut().set_prefix(format!(
                    "# reason: {}\n# expires: {}\n",
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

        let keys_to_prune: Vec<MatchingScope> = self
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
