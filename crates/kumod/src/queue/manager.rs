use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::insert_context::{InsertContext, InsertReason};
use crate::queue::maintainer::queue_meta_maintainer;
use crate::queue::queue::QueueHandle;
use crate::queue::resolver::{
    wrap_cached, Acquire, AdmitToken, ResolverMap, NEGATIVE_CACHE, SCHEDULED_QUEUE_COUNT,
};
use crate::queue::{opt_timeout_at, IncrementAttempts, Queue};
use crate::smtp_server::RejectError;
use crate::spool::SpoolManager;
use ::config::{declare_event, load_config};
use config::SerdeWrappedValue;
use kumo_prometheus::declare_metric;
use kumo_server_lifecycle::{is_shutting_down, Activity, ShuttingDownError};
use message::Message;
use mod_time::TimeDelta;
use rfc5321::Response;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use tracing::instrument;

pub static MANAGER: LazyLock<QueueManager> = LazyLock::new(|| QueueManager::new());

declare_metric! {
/// latency of QueueManager::resolve operations
static RESOLVE_LATENCY: Histogram("queue_resolve_latency");
}

declare_metric! {
/// latency of QueueManager::insert operations
static INSERT_LATENCY: Histogram("queue_insert_latency");
}

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
    resolver: ResolverMap<Queue>,
}

/// The outcome of finalizing a construction result.
enum FinalizeDecision<T> {
    /// Install the constructed handle.
    Publish(T),
    /// Remove the slot without caching, and return the failure to the caller.
    Remove(anyhow::Error),
    /// Cache the failure as a negative result, and return it to the caller.
    CacheFailure(anyhow::Error),
}

impl QueueManager {
    pub fn new() -> Self {
        let main_runtime = kumo_server_runtime::get_main_runtime();
        main_runtime.spawn(queue_meta_maintainer());
        main_runtime.spawn(Queue::queue_config_maintainer());
        let resolver = ResolverMap::new(SCHEDULED_QUEUE_COUNT.clone(), |name| {
            Activity::get(format!("Resolving queue {name}"))
                .map(AdmitToken::new)
                .map_err(anyhow::Error::from)
        });
        Self { resolver }
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

    /// Resolve a scheduled queue name to a handle,
    /// returning a pre-existing handle if it is already known.
    #[instrument]
    pub async fn resolve(name: &str) -> anyhow::Result<QueueHandle> {
        loop {
            match MANAGER.resolver.acquire(name) {
                Acquire::Ready(handle) => return Ok(handle),
                Acquire::Failed(error) => return Err(wrap_cached(error)),
                Acquire::Refused(err) => return Err(err),
                Acquire::Wait(attempt) => {
                    // Another caller is already constructing this name. Wait
                    // for it to finish, then re-acquire to read the outcome.
                    attempt.wait().await;
                    continue;
                }
                Acquire::Owner(owner) => {
                    // This caller is admitted to build the queue. Do so
                    // without holding any map lock.
                    let result = Queue::new(name.to_string()).await;
                    return Self::finalize(owner, result);
                }
            }
        }
    }

    /// Record the result of an owned construction: publish a handle, drop a
    /// shutdown-caused failure, or cache any other failure as a negative
    /// result for a limited time.
    fn finalize(
        owner: crate::queue::resolver::Owner<Queue>,
        result: anyhow::Result<QueueHandle>,
    ) -> anyhow::Result<QueueHandle> {
        match Self::finalize_decision(result, is_shutting_down) {
            FinalizeDecision::Publish(handle) => owner.publish(handle),
            FinalizeDecision::Remove(err) => {
                owner.remove();
                Err(err)
            }
            FinalizeDecision::CacheFailure(err) => {
                let err = Arc::new(err);
                owner.fail(err.clone(), Instant::now() + NEGATIVE_CACHE);
                Err(wrap_cached(err))
            }
        }
    }

    /// Decide how to finalize a construction result: publish the handle, drop
    /// the slot for a shutdown-caused failure, or cache any other failure as a
    /// negative result.
    fn finalize_decision<T>(
        result: anyhow::Result<T>,
        shutting_down: impl Fn() -> bool,
    ) -> FinalizeDecision<T> {
        match result {
            Ok(handle) => FinalizeDecision::Publish(handle),
            Err(err) => {
                if shutting_down() || ShuttingDownError::is_shutting_down(&err) {
                    FinalizeDecision::Remove(err)
                } else {
                    FinalizeDecision::CacheFailure(err)
                }
            }
        }
    }

    pub fn get_opt(name: &str) -> Option<QueueHandle> {
        MANAGER.resolver.get_handle(name)
    }

    pub fn all_queue_names() -> Vec<String> {
        MANAGER.resolver.names()
    }

    /// Reap the handle named `name`, but only if the entry still holds exactly
    /// `handle` and `pred` holds. Returns whether it was removed.
    pub fn remove_handle_if(name: &str, handle: &QueueHandle, pred: impl FnOnce() -> bool) -> bool {
        MANAGER.resolver.remove_handle_if(name, handle, pred)
    }

    /// Drop every negatively cached failure.
    pub fn purge_failed() {
        MANAGER.resolver.purge_failed();
    }

    /// Whether any handle or in-progress resolution remains. Cached failures do
    /// not count as outstanding work.
    pub fn work_remains() -> bool {
        MANAGER.resolver.work_remains()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn finalize_decision_publishes_ok_result() {
        let decision: FinalizeDecision<u32> = QueueManager::finalize_decision(Ok(7), || false);
        assert!(matches!(decision, FinalizeDecision::Publish(7)));
    }

    #[test]
    fn finalize_decision_removes_failure_during_shutdown() {
        // Simulates is_shutting_down() flipping true while construction is
        // already in flight: we drop the failure instead of caching it.
        let err = anyhow::anyhow!("dns lookup failed");
        let decision: FinalizeDecision<u32> = QueueManager::finalize_decision(Err(err), || true);
        assert!(matches!(decision, FinalizeDecision::Remove(_)));
    }

    #[test]
    fn finalize_decision_removes_shutting_down_error_before_flag() {
        // A ShuttingDownError is authoritative on its own: it is never cached,
        // regardless of what is_shutting_down() reports at finalize time.
        let err = anyhow::Error::new(ShuttingDownError::new("Resolving queue q"));
        let decision: FinalizeDecision<u32> = QueueManager::finalize_decision(Err(err), || false);
        assert!(matches!(decision, FinalizeDecision::Remove(_)));
    }

    #[test]
    fn finalize_decision_caches_other_failures() {
        let err = anyhow::anyhow!("dns lookup failed");
        let decision: FinalizeDecision<u32> = QueueManager::finalize_decision(Err(err), || false);
        assert!(matches!(decision, FinalizeDecision::CacheFailure(_)));
    }
}
