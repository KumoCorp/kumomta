use anyhow::Context;
use sqlite::{Connection, ConnectionThreadSafe};
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[derive(Clone)]
pub struct Database {
    db: Arc<ConnectionThreadSafe>,
}

impl Database {
    /// Carry out the blocking operation on the database object
    pub async fn perform<T: Send + 'static>(
        &self,
        mut func: impl FnMut(&ConnectionThreadSafe) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let db = self.db.clone();
        spawn_blocking(move || (func)(&db)).await?
    }

    pub fn open(path: &str) -> anyhow::Result<Self> {
        let mut db = Connection::open_thread_safe(path)
            .with_context(|| format!("failed to open TSA database {path}"))?;

        db.set_busy_timeout(60_000)?;

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

CREATE TABLE IF NOT EXISTS sched_q_bounces (
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
        db.execute("PRAGMA synchronous = OFF")?;

        // This one is risky and doesn't make a significant
        // impact on overall performance
        // db.execute("PRAGMA journal_mode = MEMORY")?;

        Ok(Self { db: Arc::new(db) })
    }
}
