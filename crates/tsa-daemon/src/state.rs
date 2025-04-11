use crate::http_server::{
    import_configs_from_sqlite, regex_list_to_string, toml_to_toml_edit_value, PreferRollup,
    Sha256Hasher, DB_PATH,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use kumo_api_types::shaping::{
    Action, EgressPathConfigValue, EgressPathConfigValueUnchecked, Rule,
};
use kumo_log_types::JsonLogRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

pub static TSA_STATE: OnceLock<TsaState> = OnceLock::new();

/// Represents a specific rule definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleHash(#[serde(with = "serde_bytes")] [u8; 32]);

impl RuleHash {
    pub fn from_rule(rule: &Rule) -> Self {
        let mut hasher = Sha256Hasher::new();
        rule.hash(&mut hasher);
        Self(hasher.get_binary())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SiteKey(String);

impl SiteKey {
    pub fn from_record(record: &JsonLogRecord) -> Self {
        Self(record.site.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionHash(#[serde(with = "serde_bytes")] [u8; 32], SiteKey);

impl ActionHash {
    pub fn from_rule_and_record(rule: &Rule, action: &Action, record: &JsonLogRecord) -> Self {
        let mut hasher = Sha256Hasher::new();
        rule.hash(&mut hasher);
        action.hash(&mut hasher);
        Self(hasher.get_binary(), SiteKey::from_record(record))
    }

    pub fn from_legacy_hash_and_site(hash: &str, site: &str) -> Self {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(hash, &mut bytes).ok();
        Self(bytes, SiteKey(site.to_string()))
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

#[derive(Default)]
pub struct TsaState {
    event_history: DashMap<MatchingScope, EventData>,
    config_overrides: DashMap<ActionHash, ConfigurationOverride>,
}

#[derive(Serialize, Deserialize)]
struct SerializableState {
    #[serde(default)]
    event_history: HashMap<MatchingScope, EventData>,
    #[serde(default)]
    config_overrides: HashMap<ActionHash, ConfigurationOverride>,
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
            tracing::warn!("Skip insertion of {scope:?} {over:?} because it has already expired");
            return;
        }
        tracing::warn!("inserting {scope:?} {over:?}");
        self.config_overrides.insert(scope, over);
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
        }
    }

    async fn prune(&self) {
        let now = Utc::now();
        let now_ts = to_unix_ts(&now);
        self.prune_events(now_ts).await;
        self.prune_config_overrides(&now).await;
    }

    async fn prune_config_overrides(&self, now: &DateTime<Utc>) {
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
        tracing::debug!(
            "visited {visited} and pruned {num_pruned} \
            config_overrides entries in {:?}",
            start.elapsed()
        );
    }

    async fn prune_events(&self, now_ts: UnixTimeStamp) {
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
                    state.prune().await;

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

    if import_holder.config_overrides.is_empty() {
        // Import configs from the sqlite database
        if let Err(err) = import_configs_from_sqlite(import_holder.clone()).await {
            tracing::warn!(
                "Failed to import legacy config entries from sqlite: {err:#}. Proceeding without them");
        } else {
            tracing::info!(
                "Imported {} config entries from sqlite",
                import_holder.config_overrides.len()
            );
        }
    }

    TSA_STATE
        .set(Arc::into_inner(import_holder).expect("only we hold a ref"))
        .ok();
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

    let message = format!(
        "stored {} of data to {path}. (Extract took {extract:?}, write took {write:?})",
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
            state.prune().await;
        }

        if last_save.elapsed() > std::time::Duration::from_secs(300) {
            if let Err(err) = save_state(true).await {
                tracing::error!("{err:#} saving state file");
            }
            last_save = Instant::now();
        }
    }
}
