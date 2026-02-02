use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::insert_context::{InsertContext, InsertReason};
use crate::queue::maintainer::queue_meta_maintainer;
use crate::queue::queue::QueueHandle;
use crate::queue::{opt_timeout_at, IncrementAttempts, Queue};
use crate::smtp_server::RejectError;
use crate::spool::SpoolManager;
use ::config::{declare_event, load_config};
use config::SerdeWrappedValue;
use dashmap::DashMap;
use message::Message;
use mod_time::TimeDelta;
use prometheus::{Histogram, IntGauge};
use rfc5321::Response;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::instrument;

pub static MANAGER: LazyLock<QueueManager> = LazyLock::new(|| QueueManager::new());
static RESOLVE_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "queue_resolve_latency",
        "latency of QueueManager::resolve operations",
    )
    .unwrap()
});
static INSERT_LATENCY: LazyLock<Histogram> = LazyLock::new(|| {
    prometheus::register_histogram!(
        "queue_insert_latency",
        "latency of QueueManager::insert operations",
    )
    .unwrap()
});
pub static SCHEDULED_QUEUE_COUNT: LazyLock<IntGauge> = LazyLock::new(|| {
    prometheus::register_int_gauge!(
        "scheduled_queue_count",
        "how many scheduled queues are tracked by the QueueManager"
    )
    .unwrap()
});
declare_event! {
pub static REQUEUE_MESSAGE_SIG: Multiple(
    "requeue_message",
    message: Message,
    response: String,
    insert_context: SerdeWrappedValue<InsertContext>,
    increment_attempts: bool,
    delay: Option<TimeDelta>,
) -> ();
}

pub struct QueueManager {
    pub named: DashMap<String, QueueSlot>,
}

pub enum QueueSlot {
    Handle(QueueHandle),
    Resolving(Arc<Semaphore>),
    // Negative caching
    Failed { error: String, expires: Instant },
}

#[derive(Clone)]
enum SlotLease {
    Handle(QueueHandle),
    Resolving(Arc<Semaphore>),
}

impl QueueManager {
    pub fn new() -> Self {
        let main_runtime = kumo_server_runtime::get_main_runtime();
        main_runtime.spawn(queue_meta_maintainer());
        main_runtime.spawn(Queue::queue_config_maintainer());
        Self {
            named: DashMap::new(),
        }
    }

    /// Insert message into a queue named `name`.
    #[instrument(skip(msg))]
    pub async fn insert(name: &str, msg: Message, context: InsertContext) -> anyhow::Result<()> {
        tracing::trace!("QueueManager::insert {context:?}");
        let timer = RESOLVE_LATENCY.start_timer();
        let entry = Self::resolve(name).await?;
        timer.stop_and_record();

        let _timer = INSERT_LATENCY.start_timer();
        entry.insert(msg, context, None).await
    }

    #[instrument(skip(msg))]
    async fn insert_within_deadline(
        name: &str,
        msg: Message,
        context: InsertContext,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        tracing::trace!("QueueManager::insert {context:?}");

        let timer = RESOLVE_LATENCY.start_timer();
        let entry = opt_timeout_at(deadline, Self::resolve(name)).await?;
        timer.stop_and_record();

        let _timer = INSERT_LATENCY.start_timer();
        entry.insert(msg, context, deadline).await
    }

    /// Insert message into a queue named `name`, unwinding it in the case
    /// of error. Unwinding here means that:
    ///
    ///  * The message is removed from the spool
    ///  * An internal Bounce is generated in the disposition logs
    ///
    /// This is a wrapper around QueueManager::insert that will remove the
    /// message from the spool and generate a Bounce entry.
    ///
    /// It is intended to be called at the point of ingress, during reception,
    /// and not as a general purpose loader (eg: most definitely NOT during
    /// spool enumeration, where it would have the consequence of deleting
    /// the spool on startup if there was a config issue!).
    #[instrument(skip(msg))]
    pub async fn insert_or_unwind(
        name: &str,
        msg: Message,
        spool_was_deferred: bool,
        deadline: Option<Instant>,
    ) -> anyhow::Result<()> {
        match Box::pin(Self::insert_within_deadline(
            name,
            msg.clone(),
            InsertReason::Received.into(),
            deadline,
        ))
        .await
        {
            Ok(()) => Ok(()),
            Err(err) => {
                // Well, this sucks. The likely cause is an error in the
                // lua behind either get_queue_config or get_egress_path_config.
                // Since we spooled the message, we need to unwind that before
                // we report the failure back to the user.
                // We cannot just accept the message and continue because
                // we failed to resolve the configuration for it: the message
                // won't go anywhere and we thus cannot accept responsibility
                // from the injector

                // Note that we try to remove from the spool even if we know
                // that spool_was_deferred, because we don't know if someone
                // called msg:save() in some lua code, or if some other logic
                // may be have decided to spool it anyway.
                if let Err(err) = SpoolManager::remove_from_spool(*msg.id()).await {
                    // Note that, at the time of writing this comment,
                    // SpoolManager::remove_from_spool never returns an error.
                    // But let's add some logic here to surface one if it
                    // starts to do so in the future.
                    if !spool_was_deferred {
                        tracing::error!("remove_from_spool({}) failed: {err:#}", msg.id());
                    }
                }

                // Since the caller just logged a Reception, we should now log
                // a Bounce so that the logs reflect that we aren't going
                // to send this message and we don't leave someone scratching
                // their head about it.
                log_disposition(LogDisposition {
                    kind: RecordType::Bounce,
                    msg: msg.clone(),
                    site: "",
                    peer_address: None,
                    response: Response {
                        code: 500,
                        enhanced_code: None,
                        command: None,
                        content: format!(
                        "KumoMTA internal: QueueManager::insert failed during reception: {err:#}"
                    ),
                    },
                    egress_source: None,
                    egress_pool: None,
                    relay_disposition: None,
                    delivery_protocol: None,
                    tls_info: None,
                    source_address: None,
                    provider: None,
                    session_id: None,
                    recipient_list: None,
                })
                .await;

                Err(err)
            }
        }
    }

    /// Re-insert message into the queue subsystem, likely the scheduled queue,
    /// after first calling out to the requeue_message event handler, which
    /// gives the user the opportunity to rebind or do other things to the
    /// message before we put it back into the queues.
    #[instrument(skip(msg))]
    pub async fn requeue_message(
        msg: Message,
        mut increment_attempts: IncrementAttempts,
        mut delay: Option<chrono::Duration>,
        response: Response,
        context: InsertContext,
    ) -> anyhow::Result<()> {
        if !msg.is_meta_loaded() {
            msg.load_meta().await?;
        }
        let mut queue_name = msg.get_queue_name().await?;

        match load_config().await {
            Ok(mut config) => {
                let result: anyhow::Result<()> = config
                    .async_call_callback(
                        &REQUEUE_MESSAGE_SIG,
                        (
                            msg.clone(),
                            response.to_single_line(),
                            SerdeWrappedValue(context.clone()),
                            increment_attempts == IncrementAttempts::Yes,
                            delay.map(Into::into),
                        ),
                    )
                    .await;

                match result {
                    Ok(()) => {
                        config.put();
                        let queue_name_after = msg.get_queue_name().await?;
                        if queue_name != queue_name_after {
                            // We want to avoid the normal due-time adjustment
                            // that would kick in when incrementing attempts
                            // in Queue::requeue_message, but we still want the
                            // number to be incremented.
                            msg.increment_num_attempts();
                            increment_attempts = IncrementAttempts::No;

                            // Avoid adding jitter as part of the queue change
                            delay = Some(chrono::Duration::zero());
                            // and ensure that the message is due now
                            msg.set_due(None).await?;

                            // and use the new queue name
                            queue_name = queue_name_after;
                        }
                    }
                    Err(err) => {
                        // If they did a kumo.reject() in the handler, translate that
                        // into a Bounce. We do this even if they used a 4xx code; it
                        // only makes sense to map it to a Bounce rather than a
                        // TransientFailure because we already just had a TransientFailure.
                        if let Some(rej) = RejectError::from_anyhow(&err) {
                            log_disposition(LogDisposition {
                                kind: RecordType::Bounce,
                                msg: msg.clone(),
                                // There is no site because this was a policy bounce
                                // triggered in an event handler
                                site: "",
                                peer_address: None,
                                response: Response {
                                    code: rej.code,
                                    enhanced_code: None,
                                    content: rej.message,
                                    command: None,
                                },
                                egress_pool: None,
                                egress_source: None,
                                relay_disposition: None,
                                delivery_protocol: None,
                                tls_info: None,
                                source_address: None,
                                provider: None,
                                session_id: None,
                                recipient_list: None,
                            })
                            .await;
                            SpoolManager::remove_from_spool(*msg.id()).await.ok();
                            return Ok(());
                        }

                        tracing::error!(
                            "Error while calling requeue_message event: {err:#}. \
                                 will reuse current queue"
                        );
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    "ReadyQueue::requeue_message: error getting \
                         lua config in order to call requeue_message event: \
                         {err:#}, will reuse current queue"
                );
            }
        }

        let queue = QueueManager::resolve(&queue_name).await?;
        queue
            .requeue_message_internal(msg, increment_attempts, delay, context)
            .await
    }

    fn resolve_lease(name: &str) -> anyhow::Result<SlotLease> {
        match MANAGER.named.get_mut(name) {
            Some(mut item) => {
                match item.value() {
                    QueueSlot::Handle(handle) => return Ok(SlotLease::Handle(handle.clone())),
                    QueueSlot::Resolving(sema) => return Ok(SlotLease::Resolving(sema.clone())),
                    QueueSlot::Failed { error, expires } => {
                        if *expires > Instant::now() {
                            anyhow::bail!("{error}");
                        }
                        // Negative cache expired; can setup the slot for resolve
                        let sema = Arc::new(Semaphore::new(1));
                        *item.value_mut() = QueueSlot::Resolving(sema.clone());
                        return Ok(SlotLease::Resolving(sema));
                    }
                }
            }
            None => {
                let entry = MANAGER.named.entry(name.to_string()).or_insert_with(|| {
                    SCHEDULED_QUEUE_COUNT.inc();
                    QueueSlot::Resolving(Arc::new(Semaphore::new(1)))
                });
                match entry.value() {
                    QueueSlot::Handle(handle) => return Ok(SlotLease::Handle(handle.clone())),
                    QueueSlot::Resolving(sema) => return Ok(SlotLease::Resolving(sema.clone())),
                    QueueSlot::Failed { error, .. } => {
                        // We don't bother looking at expiry here: our first try
                        // found nothing in the map, so if we see an entry now on
                        // our second try then it must be new enough to be current.
                        anyhow::bail!("{error}");
                    }
                }
            }
        }
    }

    /// Get the handle from the slot.
    /// Intended to be called by `resolve` only, as an implementation detail.
    ///
    /// Propagate a negatively cached error without considering its expiry:
    /// the assumption is that we are being called as part of an overlapping
    /// sequence of calls that implicitly must be within whatever negative
    /// caching period we use: if there was a failure in the overlap, we
    /// want to propagate that same failure.
    fn get_slot(name: &str) -> anyhow::Result<Option<QueueHandle>> {
        match MANAGER.named.get(name) {
            Some(item) => match item.value() {
                QueueSlot::Handle(h) => Ok(Some(h.clone())),
                QueueSlot::Resolving(_) => Ok(None),
                QueueSlot::Failed { error, .. } => anyhow::bail!("{error}"),
            },
            None => Ok(None),
        }
    }

    /// Resolve a scheduled queue name to a handle,
    /// returning a pre-existing handle if it is already known.
    #[instrument]
    pub async fn resolve(name: &str) -> anyhow::Result<QueueHandle> {
        loop {
            match Self::resolve_lease(name)? {
                SlotLease::Handle(e) => return Ok(e),
                SlotLease::Resolving(sema) => {
                    match sema.acquire().await {
                        Ok(_permit) => {
                            // If we acquire the permit, we are responsible now for
                            // driving the state of the SlotLease towards either
                            // a resolution or a failure, as we have the only permit.
                            //
                            // We don't explicitly drop the permit here; in the
                            // already-resolved case it will drop naturally when we return,
                            // allowing other callers to proceed into their version of this
                            // branch of code just like we're doing now.
                            // This should be the fast path in the recently-created case.
                            //
                            // In the need-to-resolve case, the permit is also implicitly
                            // dropped, but only after dropping the associated semaphore,
                            // which has the effect of racing all waiters in another
                            // iteration of this resolve loop.

                            match Self::get_slot(name)? {
                                Some(handle) => {
                                    // Someone else fully resolved the entry.
                                    return Ok(handle);
                                }
                                None => {
                                    // The current state is Resolving and we're responsible
                                    // to drive it forwards.

                                    // Try to create the queue
                                    let result = Queue::new(name.to_string()).await;

                                    // Now update the state in the map.
                                    // Both arms will replace the entry with either a success
                                    // or failure entry, which will implicitly drop any
                                    // Resolving entry and its associated Semaphore, which
                                    // will in turn cause all pending sema.acquire operations
                                    // to "fail" and wakeup so that they can attempt to re-acquire.
                                    return match result {
                                        Ok(entry) => {
                                            // Success! move from Resolving -> Handle
                                            if MANAGER
                                                .named
                                                .insert(
                                                    name.to_string(),
                                                    QueueSlot::Handle(entry.clone()),
                                                )
                                                .is_none()
                                            {
                                                SCHEDULED_QUEUE_COUNT.inc();
                                            }
                                            Ok(entry)
                                        }
                                        Err(err) => {
                                            // Failed!
                                            if MANAGER
                                                .named
                                                .insert(
                                                    name.to_string(),
                                                    QueueSlot::Failed {
                                                        error: format!("{err:#}"),
                                                        expires: Instant::now()
                                                            + Duration::from_secs(60),
                                                    },
                                                )
                                                .is_none()
                                            {
                                                SCHEDULED_QUEUE_COUNT.inc();
                                            }
                                            Err(err)
                                        }
                                    };
                                }
                            }
                        }
                        Err(_) => {
                            // Semaphore was closed; perhaps it was cancelled or
                            // otherwise failed. Let's retry the resolve.
                            continue;
                        }
                    }
                }
            }
        }
    }

    pub fn get_opt(name: &str) -> Option<QueueHandle> {
        match MANAGER.named.get(name)?.value() {
            QueueSlot::Handle(h) => Some(h.clone()),
            QueueSlot::Resolving(_) | QueueSlot::Failed { .. } => None,
        }
    }

    pub fn all_queue_names() -> Vec<String> {
        let mut names = vec![];
        for item in MANAGER.named.iter() {
            names.push(item.key().to_string());
        }
        names
    }
}
