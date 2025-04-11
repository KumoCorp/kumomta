use crate::database::Database;
use crate::publish::submit_record;
use crate::shaping_config::get_shaping;
use crate::state::{
    ActionHash, ConfigurationOverride, MatchingScope, SchedQBounceEntry, SchedQBounceKey, TsaState,
    TSA_STATE,
};
use anyhow::{anyhow, Context};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use kumo_api_types::shaping::{
    Action, EgressPathConfigValueUnchecked, Regex, Rule, Shaping, Trigger,
};
use kumo_api_types::tsa::{
    ReadyQSuspension, SchedQBounce, SchedQSuspension, SubscriptionItem, SuspensionEntry,
    Suspensions,
};
use kumo_log_types::*;
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::{AppError, RouterAndDocs};
use message::message::QueueNameComponents;
use parking_lot::Mutex;
use rfc5321::ForwardPath;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use sqlite::ConnectionThreadSafe;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tokio::sync::broadcast::{channel, Sender};
use utoipa::OpenApi;

pub static DB_PATH: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("/var/spool/kumomta/tsa.db".to_string()));
static HISTORY: LazyLock<Database> = LazyLock::new(|| open_history_db().unwrap());
static SUSPENSION_TX: LazyLock<SubscriberMgr> = LazyLock::new(SubscriberMgr::new);

pub fn open_history_db() -> anyhow::Result<Database> {
    let path = DB_PATH.lock().clone();
    Database::open(&path)
}

#[derive(OpenApi)]
#[openapi(info(title = "tsa-daemon",), paths(), components())]
struct ApiDoc;

pub fn make_router() -> RouterAndDocs {
    RouterAndDocs {
        router: Router::new()
            .route("/publish_log_v1", post(publish_log_v1))
            .route("/get_config_v1/shaping.toml", get(get_config_v1))
            .route("/get_suspension_v1/suspended.json", get(get_suspension_v1))
            .route("/subscribe_suspension_v1", get(subscribe_suspension_v1))
            .route("/get_bounce_v1/bounced.json", get(get_bounce_v1))
            .route("/subscribe_event_v1", get(subscribe_event_v1)),
        docs: ApiDoc::openapi(),
    }
}

#[derive(PartialEq, Clone, Copy)]
pub enum PreferRollup {
    Yes,
    No,
}

pub fn regex_list_to_string(list: &[Regex]) -> String {
    if list.len() == 1 {
        list[0].to_string()
    } else {
        let mut result = "(".to_string();
        for (n, r) in list.iter().enumerate() {
            if n > 0 {
                result.push(',');
            }
            result.push_str(&r.to_string());
        }
        result.push(')');
        result
    }
}

#[derive(PartialEq, Clone, Copy)]
enum UseCampaign {
    Yes,
    No,
}

#[derive(PartialEq, Clone, Copy)]
enum UseTenant {
    Yes,
    No,
}

async fn create_bounce(
    action_hash: &ActionHash,
    rule: &Rule,
    record: &JsonLogRecord,
    use_tenant: UseTenant,
    use_campaign: UseCampaign,
    events: &mut Vec<SubscriptionItem>,
) -> anyhow::Result<()> {
    let components = QueueNameComponents::parse(&record.queue);

    let tenant = match components.tenant {
        Some(tenant) => Some(tenant),
        None if use_tenant == UseTenant::Yes => {
            tracing::error!(
                "Cannot create tenant based bounce for {rule:?} \
                because the incoming record queue {} has no tenant component",
                record.queue
            );
            return Ok(());
        }
        None => None,
    };

    let campaign = if use_campaign == UseCampaign::Yes {
        components.campaign
    } else {
        None
    };
    let mut reason = format!(
        "automation rule: {} domain={}",
        regex_list_to_string(&rule.regex),
        components.domain
    );
    if let Some(tenant) = &tenant {
        reason.push_str(&format!(" tenant={tenant}"));
    }
    if let Some(campaign) = &campaign {
        reason.push_str(&format!(" campaign={campaign}"));
    }
    let expires = record.timestamp + chrono::Duration::from_std(rule.duration)?;

    TSA_STATE
        .get()
        .expect("tsa_state missing")
        .insert_schedq_bounce(
            SchedQBounceKey {
                action_hash: action_hash.clone(),
                domain: components.domain.to_string(),
                campaign: campaign.as_ref().map(|c| c.to_string()),
                tenant: tenant.as_ref().map(|c| c.to_string()),
            },
            SchedQBounceEntry {
                reason: reason.clone(),
                expires,
            },
        );

    events.push(SubscriptionItem::SchedQBounce(SchedQBounce {
        rule_hash: action_hash.to_string(),
        domain: components.domain.to_string(),
        tenant: tenant.map(|s| s.to_string()),
        campaign: campaign.map(|s| s.to_string()),
        reason,
        expires,
    }));

    Ok(())
}

async fn create_tenant_suspension(
    db: &Arc<Database>,
    rule_hash: &str,
    rule: &Rule,
    record: &JsonLogRecord,
    use_campaign: UseCampaign,
    events: &mut Vec<SubscriptionItem>,
) -> anyhow::Result<()> {
    let components = QueueNameComponents::parse(&record.queue);
    let Some(tenant) = components.tenant else {
        tracing::error!(
            "Cannot create tenant based suspension for {rule:?} \
             because the incoming record queue {} has no tenant component",
            record.queue
        );
        return Ok(());
    };

    let campaign = if use_campaign == UseCampaign::Yes {
        components.campaign
    } else {
        None
    };
    let expires = record.timestamp + chrono::Duration::from_std(rule.duration)?;
    let mut reason = format!(
        "automation rule: {} tenant={tenant} domain={}",
        regex_list_to_string(&rule.regex),
        components.domain
    );
    if let Some(campaign) = &campaign {
        reason.push_str(&format!(" campaign={campaign}"));
    }

    {
        let reason = reason.to_string();
        let rule_hash = rule_hash.to_string();
        let campaign = campaign.as_ref().map(|c| c.to_string());
        let tenant = tenant.to_string();
        let domain = components.domain.to_string();

        db.perform("create_tenant_suspension", move |db| {
            let mut upsert = db
                .prepare(
                    "INSERT INTO sched_q_suspensions
                 (rule_hash, campaign, tenant, domain, reason, expires)
                 VALUES
                 ($hash, $campaign, $tenant, $domain, $reason, $expires)
                 ON CONFLICT (rule_hash, campaign, tenant, domain)
                 DO UPDATE SET expires=$expires",
                )
                .context("prepare sched_q_suspensions upsert")?;

            let expires_str = expires.to_rfc3339();

            upsert.bind(("$hash", rule_hash.as_str()))?;
            upsert.bind(("$campaign", campaign.as_deref()))?;
            upsert.bind(("$tenant", tenant.as_str()))?;
            upsert.bind(("$domain", domain.as_str()))?;

            upsert.bind(("$reason", reason.as_str()))?;
            upsert.bind(("$expires", expires_str.as_str()))?;

            upsert
                .next()
                .context("execute sched_q_suspensions upsert")?;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
    }

    events.push(SubscriptionItem::SchedQSuspension(SchedQSuspension {
        rule_hash: rule_hash.to_string(),
        domain: components.domain.to_string(),
        tenant: tenant.to_string(),
        campaign: campaign.map(|s| s.to_string()),
        reason,
        expires,
    }));

    Ok(())
}

async fn create_ready_q_suspension(
    db: &Arc<Database>,
    rule_hash: &str,
    rule: &Rule,
    record: &JsonLogRecord,
    source: &str,
    events: &mut Vec<SubscriptionItem>,
) -> anyhow::Result<()> {
    let expires = record.timestamp + chrono::Duration::from_std(rule.duration)?;
    let reason = format!("automation rule: {}", regex_list_to_string(&rule.regex));

    {
        let reason = reason.to_string();
        let source = source.to_string();
        let site = record.site.to_string();
        let rule_hash = rule_hash.to_string();

        db.perform("create_ready_q_suspension", move |db| {
            let mut upsert = db.prepare(
                "INSERT INTO ready_q_suspensions
                 (rule_hash, site_name, source, reason, expires)
                 VALUES
                 ($hash, $site, $source, $reason, $expires)
                 ON CONFLICT (rule_hash, site_name)
                 DO UPDATE SET expires=$expires",
            )?;

            let expires_str = expires.to_rfc3339();

            upsert.bind(("$hash", rule_hash.as_str()))?;
            upsert.bind(("$site", site.as_str()))?;
            upsert.bind(("$source", source.as_str()))?;

            upsert.bind(("$reason", reason.as_str()))?;
            upsert.bind(("$expires", expires_str.as_str()))?;

            upsert.next()?;
            Ok::<_, anyhow::Error>(())
        })
        .await?;
    }

    events.push(SubscriptionItem::ReadyQSuspension(ReadyQSuspension {
        rule_hash: rule_hash.to_string(),
        site_name: record.site.to_string(),
        reason,
        source: source.to_string(),
        expires,
    }));

    Ok(())
}

pub async fn publish_log_batch(
    db: &Arc<Database>,
    records: &mut Vec<JsonLogRecord>,
) -> anyhow::Result<()> {
    let shaping = get_shaping();

    let mut events = vec![];

    tracing::trace!("publish_log_batch with {} records", records.len());

    db.perform("publish_log_batch begin", |db| {
        db.execute("BEGIN")?;
        Ok(())
    })
    .await?;

    let now = Utc::now();

    for record in records.drain(..) {
        if let Err(err) = publish_log_v1_impl(&now, db, &shaping, record, &mut events).await {
            tracing::error!("error processing record: {err:#}");
        }
    }

    db.perform("publish_log_batch COMMIT", |db| {
        db.execute("COMMIT")?;
        Ok(())
    })
    .await?;

    for event in events {
        SubscriberMgr::submit(event);
    }

    Ok(())
}

async fn publish_log_v1_impl(
    now: &DateTime<Utc>,
    db: &Arc<Database>,
    shaping: &Shaping,
    record: JsonLogRecord,
    events: &mut Vec<SubscriptionItem>,
) -> anyhow::Result<()> {
    tracing::trace!("got record: {record:?}");
    // Extract the domain from the recipient.
    let recipient = ForwardPath::try_from(record.recipient.as_str())
        .map_err(|err| anyhow!("parsing record.recipient: {err}"))?;

    let recipient = match recipient {
        ForwardPath::Postmaster => {
            // It doesn't make sense to apply automation on the
            // local postmaster address, so we ignore this.
            return Ok(());
        }
        ForwardPath::Path(path) => path.mailbox,
    };
    let domain = recipient.domain.to_string();

    // Track events/outcomes by site.
    let source = record.egress_source.as_deref().unwrap_or("unspecified");
    let store_key = record.site.to_string();

    let matches = shaping.match_rules(&record).await?;

    for m in &matches {
        let expires = record.timestamp + chrono::Duration::from_std(m.duration)?;
        if expires <= *now {
            // Record was perhaps delayed and is already expired, no sense recording it now
            continue;
        }

        let matching_scope = MatchingScope::from_rule_and_record(m, &record);

        let triggered = match m.trigger {
            Trigger::Immediate => true,
            Trigger::Threshold(spec) => {
                let count = TSA_STATE
                    .get()
                    .expect("state not initialized")
                    .record_event(&matching_scope, m, &record);

                count >= spec.limit
            }
        };

        tracing::trace!("match={m:?} triggered={triggered} for {record:?}");

        // To enact the action, we need to generate (or update) a row
        // in the db with its effects and its expiry
        if triggered {
            for action in &m.action {
                // Since there can be multiple actions within a match,
                // ensure that the rule_hash that we use to record
                // the effects of an action varies by the current
                // action that we are iterating
                let a_hash = action_hash(m, action);
                let rule_hash = format!("{store_key}-{a_hash}");
                let action_hash = ActionHash::from_rule_and_record(m, action, &record);

                tracing::debug!("{action:?} for {record:?}");
                match action {
                    Action::Suspend => {
                        create_ready_q_suspension(db, &rule_hash, m, &record, source, events)
                            .await?;
                    }
                    Action::SuspendTenant => {
                        create_tenant_suspension(
                            db,
                            &rule_hash,
                            m,
                            &record,
                            UseCampaign::No,
                            events,
                        )
                        .await?;
                    }
                    Action::SuspendCampaign => {
                        create_tenant_suspension(
                            db,
                            &rule_hash,
                            m,
                            &record,
                            UseCampaign::Yes,
                            events,
                        )
                        .await?;
                    }
                    Action::SetConfig(config) => {
                        TSA_STATE
                            .get()
                            .expect("tsa_state missing")
                            .create_config_override(
                                &action_hash,
                                m,
                                &record,
                                config,
                                &domain,
                                source,
                                PreferRollup::Yes,
                            );
                    }
                    Action::SetDomainConfig(config) => {
                        TSA_STATE
                            .get()
                            .expect("tsa_state missing")
                            .create_config_override(
                                &action_hash,
                                m,
                                &record,
                                config,
                                &domain,
                                source,
                                PreferRollup::No,
                            );
                    }
                    Action::Bounce => {
                        create_bounce(
                            &action_hash,
                            m,
                            &record,
                            UseTenant::No,
                            UseCampaign::No,
                            events,
                        )
                        .await?;
                    }
                    Action::BounceTenant => {
                        create_bounce(
                            &action_hash,
                            m,
                            &record,
                            UseTenant::Yes,
                            UseCampaign::No,
                            events,
                        )
                        .await?;
                    }
                    Action::BounceCampaign => {
                        create_bounce(
                            &action_hash,
                            m,
                            &record,
                            UseTenant::Yes,
                            UseCampaign::Yes,
                            events,
                        )
                        .await?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// A helper for computing a hash of a rust struct via the
/// derived Hash trait
pub struct Sha256Hasher {
    h: Option<Sha256>,
}

impl Sha256Hasher {
    pub fn new() -> Self {
        Self {
            h: Some(Sha256::new()),
        }
    }

    pub fn get(mut self) -> String {
        let result = self.h.take().unwrap().finalize();
        hex::encode(result)
    }

    pub fn get_binary(mut self) -> [u8; 32] {
        self.h.take().unwrap().finalize().into()
    }
}

impl std::hash::Hasher for Sha256Hasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        if let Some(h) = self.h.as_mut() {
            h.update(bytes)
        }
    }
}

fn action_hash(m: &Rule, action: &Action) -> String {
    let mut hasher = Sha256Hasher::new();
    m.hash(&mut hasher);
    action.hash(&mut hasher);
    hasher.get()
}

async fn publish_log_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(record): Json<JsonLogRecord>,
) -> Result<(), AppError> {
    submit_record(record).await.map_err(|err| {
        tracing::error!("while processing /publish_log_v1: {err:#}");
        let app_err: AppError = err.into();
        app_err
    })
}

fn json_to_toml_value(item_value: &JsonValue) -> anyhow::Result<toml::Value> {
    Ok(match item_value {
        JsonValue::Bool(b) => toml::Value::Boolean(*b),
        JsonValue::String(s) => toml::Value::String(s.to_string()),
        JsonValue::Array(a) => {
            let mut res = toml::value::Array::new();
            for item in a {
                res.push(json_to_toml_value(item)?);
            }
            toml::Value::Array(res)
        }
        JsonValue::Object(o) => {
            let mut tbl = toml::Table::new();
            for (k, v) in o.iter() {
                tbl.insert(k.to_string(), json_to_toml_value(v)?);
            }
            toml::Value::Table(tbl)
        }
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                anyhow::bail!("impossible number value {n:?}");
            }
        }
        JsonValue::Null => anyhow::bail!("impossible value {item_value:?}"),
    })
}

pub fn toml_to_toml_edit_value(v: toml::Value) -> toml_edit::Value {
    use toml_edit::Formatted;
    match v {
        toml::Value::String(s) => toml_edit::Value::String(Formatted::new(s)),
        toml::Value::Integer(s) => toml_edit::Value::Integer(Formatted::new(s)),
        toml::Value::Float(s) => toml_edit::Value::Float(Formatted::new(s)),
        toml::Value::Boolean(s) => toml_edit::Value::Boolean(Formatted::new(s)),
        toml::Value::Datetime(s) => toml_edit::Value::Datetime(Formatted::new(s)),
        toml::Value::Array(s) => {
            let mut array = toml_edit::Array::new();
            for item in s.into_iter().map(toml_to_toml_edit_value) {
                array.push(item);
            }
            toml_edit::Value::Array(array)
        }
        toml::Value::Table(t) => {
            let mut tbl = toml_edit::InlineTable::new();
            for (k, v) in t {
                tbl.insert(k, toml_to_toml_edit_value(v));
            }
            toml_edit::Value::InlineTable(tbl)
        }
    }
}

pub async fn import_bounces_from_sqlite(state: Arc<TsaState>) -> anyhow::Result<()> {
    HISTORY
        .perform("import bounces", move |db| {
            let mut stmt = db.prepare("SELECT * from sched_q_bounces")?;

            while let Ok(sqlite::State::Row) = stmt.next() {
                let rule_hash: String = stmt.read("rule_hash")?;
                let tenant: Option<String> = stmt.read("tenant")?;
                let domain: String = stmt.read("domain")?;
                let campaign: Option<String> = stmt.read("campaign")?;
                let reason: String = stmt.read("reason")?;
                let expires: String = stmt.read("expires")?;

                let expires = DateTime::parse_from_rfc3339(&expires)?.to_utc();

                let action_hash = ActionHash::from_legacy_action_hash_string(&rule_hash);

                state.insert_schedq_bounce(
                    SchedQBounceKey {
                        action_hash,
                        domain,
                        tenant,
                        campaign,
                    },
                    SchedQBounceEntry { reason, expires },
                );
            }

            Ok(())
        })
        .await
}

pub async fn import_configs_from_sqlite(state: Arc<TsaState>) -> anyhow::Result<()> {
    HISTORY
        .perform("import config", move |db| {
            let mut stmt = db.prepare(
                "SELECT * from config where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, domain, source, name",
            )?;
            while let Ok(sqlite::State::Row) = stmt.next() {
                let rule_hash: String = stmt.read("rule_hash")?;
                let site_name: String = stmt.read("site_name")?;
                let reason: String = stmt.read("reason")?;
                let domain: String = stmt.read("domain")?;
                let mx_rollup: i64 = stmt.read("mx_rollup")?;
                let source: String = stmt.read("source")?;
                let name: String = stmt.read("name")?;
                let config_value: String = stmt.read("value")?;
                let expires: String = stmt.read("expires")?;

                let config_value = serde_json::from_str(&config_value)?;
                let config_value = json_to_toml_value(&config_value)?;

                let matching_scope = ActionHash::from_legacy_hash_and_site(&rule_hash, &site_name);
                state.insert_config_override(
                    matching_scope,
                    ConfigurationOverride {
                        domain,
                        reason,
                        mx_rollup: mx_rollup != 0,
                        source,
                        option: EgressPathConfigValueUnchecked {
                            name,
                            value: config_value.into(),
                        },
                        expires: expires.parse()?,
                    },
                );
            }
            Ok(())
        })
        .await
}

async fn get_config_v1(_: TrustedIpRequired) -> Result<String, AppError> {
    let result = TSA_STATE
        .get()
        .expect("tsa_state missing")
        .export_config_override_toml();
    Ok(result)
}

fn do_get_suspension(db: &ConnectionThreadSafe) -> anyhow::Result<Json<Suspensions>> {
    let mut suspensions = Suspensions::default();

    let mut stmt = db.prepare(
        "SELECT * from ready_q_suspensions where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, source",
    )?;

    let mut dedup = HashMap::new();

    #[derive(Eq, PartialEq, Hash)]
    struct ReadyKey {
        rule_hash: String,
        site_name: String,
    }

    fn add_readyq_susp(dedup: &mut HashMap<ReadyKey, ReadyQSuspension>, item: ReadyQSuspension) {
        let key = ReadyKey {
            rule_hash: item.rule_hash.clone(),
            site_name: item.site_name.clone(),
        };

        let entry = dedup.entry(key).or_insert_with(|| item.clone());

        if item.expires > entry.expires {
            entry.expires = item.expires;
        }
    }

    while let Ok(sqlite::State::Row) = stmt.next() {
        let rule_hash: String = stmt.read("rule_hash")?;
        let site_name: String = stmt.read("site_name")?;
        let reason: String = stmt.read("reason")?;
        let source: String = stmt.read("source")?;
        let expires: String = stmt.read("expires")?;

        let expires = DateTime::parse_from_rfc3339(&expires)?.to_utc();

        add_readyq_susp(
            &mut dedup,
            ReadyQSuspension {
                rule_hash,
                site_name,
                reason,
                source,
                expires,
            },
        );
    }

    suspensions.ready_q = dedup.drain().map(|(_, v)| v).collect();

    let mut stmt = db.prepare(
        "SELECT * from sched_q_suspensions where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, tenant, domain, campaign",
    )?;

    let mut dedup = HashMap::new();

    #[derive(Eq, PartialEq, Hash)]
    struct SusKey {
        rule_hash: String,
        campaign: Option<String>,
        tenant: String,
        domain: String,
    }

    fn add_schedq_susp(dedup: &mut HashMap<SusKey, SchedQSuspension>, item: SchedQSuspension) {
        let key = SusKey {
            rule_hash: item.rule_hash.clone(),
            campaign: item.campaign.clone(),
            tenant: item.tenant.clone(),
            domain: item.domain.clone(),
        };
        let entry = dedup.entry(key).or_insert_with(|| item.clone());

        if item.expires > entry.expires {
            entry.expires = item.expires;
        }
    }

    while let Ok(sqlite::State::Row) = stmt.next() {
        let rule_hash: String = stmt.read("rule_hash")?;
        let tenant: String = stmt.read("tenant")?;
        let domain: String = stmt.read("domain")?;
        let campaign: Option<String> = stmt.read("campaign")?;
        let reason: String = stmt.read("reason")?;
        let expires: String = stmt.read("expires")?;

        let expires = DateTime::parse_from_rfc3339(&expires)?.to_utc();

        add_schedq_susp(
            &mut dedup,
            SchedQSuspension {
                rule_hash,
                domain,
                tenant,
                campaign,
                reason,
                expires,
            },
        );
    }

    suspensions.sched_q = dedup.drain().map(|(_, v)| v).collect();

    Ok(Json(suspensions))
}

async fn get_suspension_v1(_: TrustedIpRequired) -> Result<Json<Suspensions>, AppError> {
    let result = HISTORY
        .perform("get_suspension_v1", do_get_suspension)
        .await?;
    Ok(result)
}

struct SubscriberMgr {
    tx: Sender<SubscriptionItem>,
}

impl SubscriberMgr {
    pub fn new() -> Self {
        let (tx, _rx) = channel(128 * 1024);
        Self { tx }
    }

    pub fn submit(entry: SubscriptionItem) {
        let mgr = &SUSPENSION_TX;
        if mgr.tx.receiver_count() > 0 {
            mgr.tx.send(entry).ok();
        }
    }
}

/// This is a legacy endpoint that can only report on the old SuspensionEntry
/// enum variants
async fn process_suspension_subscription_inner(mut socket: WebSocket) -> anyhow::Result<()> {
    let mut rx = SUSPENSION_TX.tx.subscribe();

    // send the current set of suspensions first
    {
        let suspensions = HISTORY
            .perform("ws get_suspension", do_get_suspension)
            .await?
            .0;
        for record in suspensions.ready_q {
            let json = serde_json::to_string(&SuspensionEntry::ReadyQ(record))?;
            socket.send(Message::Text(json)).await?;
        }
        for record in suspensions.sched_q {
            let json = serde_json::to_string(&SuspensionEntry::SchedQ(record))?;
            socket.send(Message::Text(json)).await?;
        }
    }

    // then wait for more to show up
    loop {
        let event = rx.recv().await?;
        let event = match event {
            SubscriptionItem::ReadyQSuspension(s) => SuspensionEntry::ReadyQ(s),
            SubscriptionItem::SchedQSuspension(s) => SuspensionEntry::SchedQ(s),
            _ => continue,
        };
        let json = serde_json::to_string(&event)?;
        socket.send(Message::Text(json)).await?;
    }
}

/// This is a legacy endpoint that can only report on the old SuspensionEntry
/// enum variants
async fn process_suspension_subscription(socket: WebSocket) {
    if let Err(err) = process_suspension_subscription_inner(socket).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

/// This is a legacy endpoint that can only report on the old SuspensionEntry
/// enum variants
pub async fn subscribe_suspension_v1(
    _: TrustedIpRequired,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(process_suspension_subscription)
}

async fn get_bounce_v1(_: TrustedIpRequired) -> Result<Json<Vec<SchedQBounce>>, AppError> {
    let result = TSA_STATE
        .get()
        .expect("tsa_state missing")
        .export_schedq_bounces();
    Ok(Json(result))
}

async fn process_event_subscription_inner(mut socket: WebSocket) -> anyhow::Result<()> {
    let mut rx = SUSPENSION_TX.tx.subscribe();

    {
        let start = Instant::now();
        let num_ready_q_sus;
        let num_sched_q_sus;
        let num_bounces;

        // send the current set of suspensions first
        {
            let suspensions = HISTORY
                .perform("ws get_suspension", do_get_suspension)
                .await?
                .0;
            num_ready_q_sus = suspensions.ready_q.len();
            num_sched_q_sus = suspensions.sched_q.len();
            tracing::debug!(
                "new sub, has {num_ready_q_sus} readyq suspensions,\
                {num_sched_q_sus} schedq suspensions",
            );
            for record in suspensions.ready_q {
                let json = serde_json::to_string(&SubscriptionItem::ReadyQSuspension(record))?;
                socket.send(Message::Text(json)).await?;
            }
            for record in suspensions.sched_q {
                let json = serde_json::to_string(&SubscriptionItem::SchedQSuspension(record))?;
                socket.send(Message::Text(json)).await?;
            }
        }
        // and then bounces
        {
            let bounces = TSA_STATE
                .get()
                .expect("tsa_state missing")
                .export_schedq_bounces();
            num_bounces = bounces.len();
            tracing::debug!("new sub, has {num_bounces} bounces");
            for record in bounces {
                let json = serde_json::to_string(&SubscriptionItem::SchedQBounce(record))?;
                socket.send(Message::Text(json)).await?;
            }
        }

        tracing::info!(
            "new sub, took {:?} to produce initial data and send to client. \
            ({num_ready_q_sus} readyq suspensions, \
             {num_sched_q_sus} schedq suspensions, \
             {num_bounces} bounces). \
            waiting for data to pass on",
            start.elapsed()
        );
    }

    // then wait for more to show up
    loop {
        let event = rx.recv().await?;
        let json = serde_json::to_string(&event)?;
        socket.send(Message::Text(json)).await?;
    }
}

async fn process_event_subscription(socket: WebSocket) {
    if let Err(err) = process_event_subscription_inner(socket).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

pub async fn subscribe_event_v1(_: TrustedIpRequired, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(process_event_subscription)
}
