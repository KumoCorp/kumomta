use anyhow::{anyhow, Context};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::DateTime;
use config::CallbackSignature;
use kumo_api_types::shaping::{Action, EgressPathConfigValue, Regex, Rule, Shaping, Trigger};
use kumo_api_types::tsa::{ReadyQSuspension, SchedQSuspension, SuspensionEntry, Suspensions};
use kumo_log_types::*;
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::{AppError, RouterAndDocs};
use kumo_server_runtime::rt_spawn;
use message::message::QueueNameComponents;
use rfc5321::ForwardPath;
use serde::Serialize;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use sqlite::{Connection, ConnectionThreadSafe};
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{LazyLock, Mutex};
use tokio::sync::broadcast::{channel, Sender};
use toml_edit::{value, Value as TomlValue};
use utoipa::OpenApi;

pub static DB_PATH: LazyLock<Mutex<String>> =
    LazyLock::new(|| Mutex::new("/var/spool/kumomta/tsa.db".to_string()));
static HISTORY: LazyLock<ConnectionThreadSafe> = LazyLock::new(|| open_history_db().unwrap());
static SUSPENSION_TX: LazyLock<SuspensionSubscriberMgr> =
    LazyLock::new(|| SuspensionSubscriberMgr::new());

fn open_history_db() -> anyhow::Result<ConnectionThreadSafe> {
    let path = DB_PATH.lock().unwrap().clone();
    let db = Connection::open_thread_safe(&path)
        .with_context(|| format!("opening TSA database {path}"))?;

    let query = r#"
CREATE TABLE IF NOT EXISTS event_history (
    rule_hash text,
    record_hash text,
    ts int,
    PRIMARY KEY (rule_hash, record_hash)
);

CREATE TABLE IF NOT EXISTS config (
    rule_hash text,
    site_name text,
    reason text,
    domain text,
    mx_rollup bool,
    source text,
    name text,
    value text,
    expires DATETIME,
    PRIMARY KEY (rule_hash, site_name)
);

CREATE TABLE IF NOT EXISTS ready_q_suspensions (
    rule_hash text,
    site_name text,
    reason text,
    source text,
    expires DATETIME,
    PRIMARY KEY (rule_hash, site_name)
);

CREATE TABLE IF NOT EXISTS sched_q_suspensions (
    rule_hash text,
    campaign text,
    tenant text,
    domain text,
    reason text,
    expires DATETIME,
    PRIMARY KEY (rule_hash, campaign, tenant, domain)
);

    "#;

    db.execute(query)?;

    Ok(db)
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
            .route("/subscribe_suspension_v1", get(subscribe_suspension_v1)),
        docs: ApiDoc::openapi(),
    }
}

fn create_config(
    rule_hash: &str,
    rule: &Rule,
    record: &JsonLogRecord,
    config: &EgressPathConfigValue,
    domain: &str,
    source: &str,
) -> anyhow::Result<()> {
    let mut upsert = HISTORY.prepare(
        "INSERT INTO config
                 (rule_hash, site_name, domain, mx_rollup, source, name, value, reason, expires)
                 VALUES
                 ($hash, $site, $domain, $mx_rollup, $source, $name, $value, $reason, $expires)
                 ON CONFLICT (rule_hash, site_name)
                 DO UPDATE SET expires=$expires",
    )?;

    let expires = (record.timestamp + chrono::Duration::from_std(rule.duration)?).to_rfc3339();

    upsert.bind(("$hash", rule_hash))?;
    upsert.bind(("$site", record.site.as_str()))?;
    upsert.bind(("$domain", domain))?;
    upsert.bind(("$mx_rollup", if rule.was_rollup { 1 } else { 0 }))?;
    upsert.bind(("$source", source))?;
    upsert.bind(("$name", config.name.as_str()))?;
    let value = serde_json::to_string(&config.value)?;
    upsert.bind(("$value", value.as_str()))?;

    let reason = format!("automation rule: {}", regex_list_to_string(&rule.regex));
    upsert.bind(("$reason", reason.as_str()))?;
    upsert.bind(("$expires", expires.as_str()))?;

    upsert.next()?;

    Ok(())
}

fn regex_list_to_string(list: &[Regex]) -> String {
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

fn create_tenant_suspension(
    rule_hash: &str,
    rule: &Rule,
    record: &JsonLogRecord,
    use_campaign: bool,
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

    let campaign = if use_campaign {
        components.campaign
    } else {
        None
    };

    let mut upsert = HISTORY
        .prepare(
            "INSERT INTO sched_q_suspensions
                 (rule_hash, campaign, tenant, domain, reason, expires)
                 VALUES
                 ($hash, $campaign, $tenant, $domain, $reason, $expires)
                 ON CONFLICT (rule_hash, campaign, tenant, domain)
                 DO UPDATE SET expires=$expires",
        )
        .context("prepare sched_q_suspensions upsert")?;

    let expires = record.timestamp + chrono::Duration::from_std(rule.duration)?;
    let expires_str = expires.to_rfc3339();

    upsert.bind(("$hash", rule_hash))?;
    upsert.bind(("$campaign", campaign))?;
    upsert.bind(("$tenant", tenant))?;
    upsert.bind(("$domain", components.domain))?;

    let mut reason = format!(
        "automation rule: {} tenant={tenant} domain={}",
        regex_list_to_string(&rule.regex),
        components.domain
    );
    if let Some(campaign) = &campaign {
        reason.push_str(&format!(" campaign={campaign}"));
    }
    upsert.bind(("$reason", reason.as_str()))?;
    upsert.bind(("$expires", expires_str.as_str()))?;

    upsert
        .next()
        .context("execute sched_q_suspensions upsert")?;

    SuspensionSubscriberMgr::submit(SuspensionEntry::SchedQ(SchedQSuspension {
        rule_hash: rule_hash.to_string(),
        domain: components.domain.to_string(),
        tenant: tenant.to_string(),
        campaign: campaign.map(|s| s.to_string()),
        reason,
        expires,
    }));

    Ok(())
}

fn create_ready_q_suspension(
    rule_hash: &str,
    rule: &Rule,
    record: &JsonLogRecord,
    source: &str,
) -> anyhow::Result<()> {
    let mut upsert = HISTORY.prepare(
        "INSERT INTO ready_q_suspensions
                 (rule_hash, site_name, source, reason, expires)
                 VALUES
                 ($hash, $site, $source, $reason, $expires)
                 ON CONFLICT (rule_hash, site_name)
                 DO UPDATE SET expires=$expires",
    )?;

    let expires = record.timestamp + chrono::Duration::from_std(rule.duration)?;
    let expires_str = expires.to_rfc3339();

    upsert.bind(("$hash", rule_hash))?;
    upsert.bind(("$site", record.site.as_str()))?;
    upsert.bind(("$source", source))?;

    let reason = format!("automation rule: {}", regex_list_to_string(&rule.regex));
    upsert.bind(("$reason", reason.as_str()))?;
    upsert.bind(("$expires", expires_str.as_str()))?;

    upsert.next()?;

    SuspensionSubscriberMgr::submit(SuspensionEntry::ReadyQ(ReadyQSuspension {
        rule_hash: rule_hash.to_string(),
        site_name: record.site.to_string(),
        reason,
        source: source.to_string(),
        expires,
    }));

    Ok(())
}

fn insert_record(rule_hash: &str, record: &JsonLogRecord, record_hash: &str) -> anyhow::Result<()> {
    let unix: i64 = record.timestamp.format("%s").to_string().parse()?;
    let mut insert = HISTORY
        .prepare("INSERT INTO event_history (rule_hash, record_hash, ts) values (?, ?, ?)")?;
    insert.bind((1, rule_hash))?;
    insert.bind((2, record_hash))?;
    insert.bind((3, unix))?;
    insert.next()?;
    Ok(())
}

fn prune_old_records(rule: &Rule, rule_hash: &str) -> anyhow::Result<()> {
    match rule.trigger {
        Trigger::Immediate => Ok(()),
        Trigger::Threshold(spec) => {
            let mut query = HISTORY.prepare(
                "delete from event_history where rule_hash = ? and ts < unixepoch() - ?",
            )?;
            query.bind((1, rule_hash))?;
            // Keep up to 2x the period
            query.bind((2, 2 * spec.period as i64))?;
            query.next()?;
            Ok(())
        }
    }
}

fn count_matching_records(rule: &Rule, rule_hash: &str) -> anyhow::Result<u64> {
    match rule.trigger {
        Trigger::Immediate => Ok(0),
        Trigger::Threshold(spec) => {
            let mut query = HISTORY.prepare(
                "SELECT COUNT(ts) from event_history where rule_hash = ? and ts >= unixepoch() - ?",
            )?;
            query.bind((1, rule_hash))?;
            query.bind((2, spec.period as i64))?;
            query.next()?;

            let count: i64 = query.read(0)?;
            Ok(count as u64)
        }
    }
}

async fn publish_log_v1_impl(record: JsonLogRecord) -> anyhow::Result<()> {
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

    let mut config = config::load_config().await?;
    let sig = CallbackSignature::<(), Shaping>::new("tsa_load_shaping_data");
    let shaping: Shaping = config
        .async_call_callback_non_default(&sig, ())
        .await
        .context("in tsa_load_shaping_data event")?;

    let matches = shaping.match_rules(&record).await?;
    let record_hash = sha256hex(&record)?;

    for m in &matches {
        let m_hash = match_hash(m);

        let rule_hash = format!("{store_key}-{m_hash}");

        let triggered = match m.trigger {
            Trigger::Immediate => true,
            Trigger::Threshold(spec) => {
                insert_record(&rule_hash, &record, &record_hash)?;
                prune_old_records(m, &rule_hash)?;

                let count = count_matching_records(m, &rule_hash)?;

                count >= spec.limit
            }
        };

        tracing::trace!("match={m:?} triggered={triggered} for {record:?}");

        // To enact the action, we need to generate (or update) a row
        // in the db with its effects and its expiry
        if triggered {
            for action in &m.action {
                tracing::info!("{action:?} for {record:?}");
                match action {
                    Action::Suspend => {
                        create_ready_q_suspension(&rule_hash, m, &record, &source)?;
                    }
                    Action::SuspendTenant => {
                        create_tenant_suspension(&rule_hash, m, &record, false)?;
                    }
                    Action::SuspendCampaign => {
                        create_tenant_suspension(&rule_hash, m, &record, true)?;
                    }
                    Action::SetConfig(config) => {
                        create_config(&rule_hash, m, &record, config, &domain, &source)?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Serialize T as json, then sha256 hash it, returning the hash as a hex string
fn sha256hex<T: Serialize>(t: &T) -> anyhow::Result<String> {
    let json = serde_json::to_string(t)?;
    let mut h = Sha256::new();
    h.update(&json);
    Ok(hex::encode(h.finalize()))
}

/// A helper for computing a hash of a rust struct via the
/// derived Hash trait
struct Sha256Hasher {
    h: Option<Sha256>,
}

impl Sha256Hasher {
    fn new() -> Self {
        Self {
            h: Some(Sha256::new()),
        }
    }

    fn get(mut self) -> String {
        let result = self.h.take().unwrap().finalize();
        hex::encode(&result)
    }
}

impl std::hash::Hasher for Sha256Hasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.h.as_mut().map(|h| h.update(bytes));
    }
}

fn match_hash(m: &Rule) -> String {
    let mut hasher = Sha256Hasher::new();
    m.hash(&mut hasher);
    hasher.get()
}

async fn publish_log_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(record): Json<JsonLogRecord>,
) -> Result<(), AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Bounce to the thread pool where we can run async lua
    rt_spawn("process record".to_string(), move || {
        Ok(async move {
            tx.send(publish_log_v1_impl(record).await.map_err(|err| {
                tracing::error!("while processing /publish_log_v1: {err:#}");
                let app_err: AppError = err.into();
                app_err
            }))
        })
    })
    .await?;
    rx.await?
}

fn json_to_toml_value(item_value: &JsonValue) -> anyhow::Result<TomlValue> {
    use toml_edit::Formatted;
    Ok(match item_value {
        JsonValue::Bool(b) => TomlValue::Boolean(Formatted::new(*b)),
        JsonValue::String(s) => TomlValue::String(Formatted::new(s.to_string())),
        JsonValue::Array(a) => {
            let mut res = toml_edit::Array::new();
            for item in a {
                res.push(json_to_toml_value(&item)?);
            }
            TomlValue::Array(res)
        }
        JsonValue::Object(o) => {
            let mut tbl = toml_edit::InlineTable::new();
            for (k, v) in o.iter() {
                tbl.insert(k, json_to_toml_value(v)?);
            }
            TomlValue::InlineTable(tbl)
        }
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                TomlValue::Integer(Formatted::new(i))
            } else if let Some(f) = n.as_f64() {
                TomlValue::Float(Formatted::new(f))
            } else {
                anyhow::bail!("impossible number value {n:?}");
            }
        }
        JsonValue::Null => anyhow::bail!("impossible value {item_value:?}"),
    })
}

async fn do_get_config() -> anyhow::Result<String> {
    use toml_edit::Item;
    let mut doc = toml_edit::DocumentMut::new();

    let mut stmt = HISTORY.prepare(
        "SELECT * from config where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, domain, source, name",
    )?;
    let mut num_entries = 0;
    while let Ok(sqlite::State::Row) = stmt.next() {
        num_entries += 1;
        let reason: String = stmt.read("reason")?;
        let domain: String = stmt.read("domain")?;
        let mx_rollup: i64 = stmt.read("mx_rollup")?;
        let source: String = stmt.read("source")?;
        let name: String = stmt.read("name")?;
        let config_value: String = stmt.read("value")?;
        let expires: String = stmt.read("expires")?;

        let config_value = serde_json::from_str(&config_value)?;
        let config_value = json_to_toml_value(&config_value)?;

        let domain_entry = doc
            .entry(&domain)
            .or_insert_with(|| {
                let mut tbl = toml_edit::Table::new();
                tbl["mx_rollup"] = value(mx_rollup != 0);
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
            .entry(&source)
            .or_insert_with(|| {
                let tbl = toml_edit::Table::new();
                Item::Table(tbl)
            })
            .as_table_mut()
            .unwrap();

        let item = Item::Value(config_value);
        source_entry.insert(&name, item);

        if let Some(mut key) = source_entry.key_mut(&name) {
            key.leaf_decor_mut()
                .set_prefix(format!("# reason: {reason}\n# expires: {expires}\n"));
        }
    }

    Ok(format!(
        "# Generated by tsa-daemon\n# Number of entries: {num_entries}\n\n{}",
        doc.to_string()
    ))
}

async fn get_config_v1(_: TrustedIpRequired) -> Result<String, AppError> {
    let result = do_get_config().await?;
    Ok(result)
}

async fn do_get_suspension() -> anyhow::Result<Json<Suspensions>> {
    let mut suspensions = Suspensions::default();

    let mut stmt = HISTORY.prepare(
        "SELECT * from ready_q_suspensions where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, source",
    )?;

    let mut by_rule_hash = HashMap::new();

    fn add_readyq_susp(
        by_rule_hash: &mut HashMap<String, ReadyQSuspension>,
        item: ReadyQSuspension,
    ) {
        let entry = by_rule_hash
            .entry(item.rule_hash.to_string())
            .or_insert_with(|| item.clone());

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
            &mut by_rule_hash,
            ReadyQSuspension {
                rule_hash,
                site_name,
                reason,
                source,
                expires,
            },
        );
    }

    suspensions.ready_q = by_rule_hash.into_iter().map(|(_, v)| v).collect();

    let mut stmt = HISTORY.prepare(
        "SELECT * from sched_q_suspensions where
                                   unixepoch(expires) - unixepoch() > 0
                                   order by expires, tenant, domain, campaign",
    )?;

    let mut by_rule_hash = HashMap::new();

    fn add_schedq_susp(
        by_rule_hash: &mut HashMap<String, SchedQSuspension>,
        item: SchedQSuspension,
    ) {
        let entry = by_rule_hash
            .entry(item.rule_hash.to_string())
            .or_insert_with(|| item.clone());

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
            &mut by_rule_hash,
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

    suspensions.sched_q = by_rule_hash.into_iter().map(|(_, v)| v).collect();

    Ok(Json(suspensions))
}

async fn get_suspension_v1(_: TrustedIpRequired) -> Result<Json<Suspensions>, AppError> {
    let result = do_get_suspension().await?;
    Ok(result)
}

struct SuspensionSubscriberMgr {
    tx: Sender<SuspensionEntry>,
}

impl SuspensionSubscriberMgr {
    pub fn new() -> Self {
        let (tx, _rx) = channel(16);
        Self { tx }
    }

    pub fn submit(entry: SuspensionEntry) {
        let mgr = &SUSPENSION_TX;
        if mgr.tx.receiver_count() > 0 {
            mgr.tx.send(entry).ok();
        }
    }
}

async fn process_suspension_subscription_inner(mut socket: WebSocket) -> anyhow::Result<()> {
    let mut rx = SUSPENSION_TX.tx.subscribe();

    // send the current set of suspensions first
    {
        let suspensions = do_get_suspension().await?;
        for record in &suspensions.ready_q {
            let json = serde_json::to_string(&SuspensionEntry::ReadyQ(record.clone()))?;
            socket.send(Message::Text(json)).await?;
        }
    }

    // then wait for more to show up
    loop {
        let event = rx.recv().await?;
        let json = serde_json::to_string(&event)?;
        socket.send(Message::Text(json)).await?;
    }
}

async fn process_suspension_subscription(socket: WebSocket) {
    if let Err(err) = process_suspension_subscription_inner(socket).await {
        tracing::error!("error in websocket: {err:#}");
    }
}

pub async fn subscribe_suspension_v1(
    _: TrustedIpRequired,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| process_suspension_subscription(socket))
}
