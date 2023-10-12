//! The purpose of this module is to keep an overall accounting
//! of the volume of messages that were received and delivered
//! by this instance

use anyhow::Context;
use chrono::prelude::*;
use core::sync::atomic::AtomicUsize;
use kumo_server_lifecycle::ShutdownSubcription;
use once_cell::sync::Lazy;
use sqlite::{Connection, ConnectionWithFullMutex};
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use tokio::task::JoinHandle;

pub static ACCT: Lazy<Accounting> = Lazy::new(|| Accounting::default());
static FLUSHER: Lazy<JoinHandle<()>> = Lazy::new(|| tokio::task::spawn(flusher()));
pub static DB_PATH: Lazy<Mutex<String>> =
    Lazy::new(|| Mutex::new("/var/spool/kumomta/accounting.db".to_string()));

#[derive(Default)]
pub struct Accounting {
    received: AtomicUsize,
    delivered: AtomicUsize,
}

impl Accounting {
    /// Increment the received counter by the specified amount
    fn inc_received(&self, amount: usize) {
        self.received.fetch_add(amount, Ordering::SeqCst);
        // and ensure that the flusher gets started
        Lazy::force(&FLUSHER);
    }

    /// Increment the delivered counter by the specified amount
    fn inc_delivered(&self, amount: usize) {
        self.delivered.fetch_add(amount, Ordering::SeqCst);
        // and ensure that the flusher gets started
        Lazy::force(&FLUSHER);
    }

    /// Grab the current counters, zeroing the state out.
    fn grab(&self) -> (usize, usize) {
        let mut received;
        loop {
            received = self.received.load(Ordering::SeqCst);
            if self
                .received
                .compare_exchange(received, 0, Ordering::SeqCst, Ordering::SeqCst)
                == Ok(received)
            {
                break;
            }
        }
        let mut delivered;
        loop {
            delivered = self.delivered.load(Ordering::SeqCst);
            if self
                .delivered
                .compare_exchange(delivered, 0, Ordering::SeqCst, Ordering::SeqCst)
                == Ok(delivered)
            {
                break;
            }
        }
        (received, delivered)
    }

    pub fn flush(&self) -> anyhow::Result<()> {
        tracing::trace!("flushing");
        let db = open_accounting_db().context("open_accounting_db")?;

        let now = Utc::now().date_naive();
        let now = now.format("%Y-%m-01 00:00:00").to_string();

        let mut insert = db
            .prepare(
                "INSERT INTO accounting
                    (event_time, received, delivered)
                    values ($now, $received, $delivered)
                    on conflict (event_time)
                    do update set received=received+$received, delivered=delivered+$delivered
                ",
            )
            .context("prepare")?;

        let (received, delivered) = self.grab();

        if received + delivered == 0 {
            // Nothing to do
            return Ok(());
        }

        insert.bind(("$now", now.as_str())).context("bind $now")?;
        insert
            .bind(("$received", received as i64))
            .context("bind $received")?;
        insert
            .bind(("$delivered", delivered as i64))
            .context("bind $delivered")?;

        let res = insert.next();

        if res.is_err() {
            self.inc_received(received);
            self.inc_delivered(delivered);

            tracing::error!(
                "Failed to record {received} receptions + \
                {delivered} deliveries to accounting log, will retry later"
            );
        }

        res?;

        Ok(())
    }
}

/// Only record protocols that correspond to ingress/egress.
/// At this time, that means everything except LogRecords produced
/// by logging hooks
fn is_accounted_protocol(protocol: &str) -> bool {
    protocol != "LogRecord"
}

/// Called by the logging layer to account for a reception
pub fn account_reception(protocol: &str) {
    if !is_accounted_protocol(protocol) {
        return;
    }
    ACCT.inc_received(1);
}

/// Called by the logging layer to account for a delivery
pub fn account_delivery(protocol: &str) {
    if !is_accounted_protocol(protocol) {
        return;
    }
    ACCT.inc_delivered(1);
}

fn open_accounting_db() -> anyhow::Result<ConnectionWithFullMutex> {
    let path = DB_PATH.lock().unwrap().clone();
    tracing::trace!("using path {path:?} for accounting db");
    let db = Connection::open_with_full_mutex(&path)
        .with_context(|| format!("opening accounting database {path}"))?;

    let query = r#"
CREATE TABLE IF NOT EXISTS accounting (
    event_time DATETIME NOT NULL PRIMARY KEY,
    received int NOT NULL,
    delivered int NOT NULL
);
    "#;

    db.execute(query)?;

    tracing::trace!("completed setup for {path:?}");

    Ok(db)
}

async fn flusher() {
    tracing::trace!("flusher started");
    let mut shutdown = ShutdownSubcription::get();
    loop {
        tokio::select! {
            _ = shutdown.shutting_down() => {
                tracing::trace!("flusher shutting down");
                break;
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(5 * 60)) => {}
        };

        let result = tokio::task::spawn_blocking(|| ACCT.flush()).await;
        if let Err(err) = result {
            tracing::error!("Error flushing accounting logs: {err:#}");
        }
    }
}
