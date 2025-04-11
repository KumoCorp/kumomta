use anyhow::Context;
use sqlite::{Connection, ConnectionThreadSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::spawn_blocking;

const BUSY_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct Database {
    db: Arc<ConnectionThreadSafe>,
}

impl Database {
    /// Carry out the blocking operation on the database object
    pub async fn perform<T: Send + 'static>(
        &self,
        reason: impl Into<String>,
        mut func: impl FnMut(&ConnectionThreadSafe) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let db = self.db.clone();
        let start = Instant::now();
        let result = spawn_blocking(move || (func)(&db)).await?.map_err(|err| {
            if let Some(s) = err.root_cause().downcast_ref::<sqlite::Error>() {
                if let Some(code) = s.code {
                    if code == sqlite::ffi::SQLITE_BUSY as isize {
                        return err.context(format!(
                            "failed to acquire database within {BUSY_TIMEOUT:?}"
                        ));
                    }
                }
            }

            err
        });
        let took = start.elapsed();
        if took > Duration::from_secs(1) {
            let is_ok = result.is_ok();
            tracing::warn!(
                "Database::perform {} took {took:?}. is_ok={is_ok}",
                reason.into()
            );
        }
        result
    }

    pub fn open(path: &str) -> anyhow::Result<Self> {
        let mut db = Connection::open_thread_safe(path)
            .with_context(|| format!("failed to open TSA database {path}"))?;

        db.set_busy_timeout(
            BUSY_TIMEOUT
                .as_millis()
                .try_into()
                .expect("timeout to be in range"),
        )?;

        let query = r#"
DROP TABLE IF EXISTS event_history;

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
