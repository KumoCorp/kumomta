use anyhow::{anyhow, Context};
use axum::routing::post;
use axum::{Json, Router};
use dns_resolver::MailExchanger;
use kumo_api_types::shaping::Shaping;
use kumo_log_types::*;
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn;
use rfc5321::ForwardPath;

pub fn make_router() -> Router {
    Router::new().route("/publish_log_v1", post(publish_log_v1))
}

async fn publish_log_v1_impl(record: JsonLogRecord) -> Result<(), AppError> {
    tracing::info!("got record: {record:?}");

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

    // From there we'll compute the site_name for ourselves, even though
    // the record includes its own idea of the site_name. The rationale for
    // this is that we prefer our understanding of domain->site_name so that
    // we are more likely to have a consistent mapping in case we are handed
    // stale data and the MX records changed, and also to isolate us from
    // other weird stuff in the future; for example, if we change the format
    // of the computed site_name in the future and there is a rolling deploy
    // of the changed code, it is safer for us to re-derive it for ourselves
    // so that we don't end up in a situation where we can't match any rollup
    // rules.
    let mx = MailExchanger::resolve(&domain).await?;

    // Track events/outcomes by site.
    // At the time of writing, `record.site` looks like `source->site_name`
    // which may technically be a bug (it should probably just be `site_name`),
    // so we explicitly include the source in our key to future proof against
    // fixing that bug later on.
    let source = record.egress_source.as_deref().unwrap_or("unspecified");
    let store_key = format!("{source}->{}", mx.site_name);

    let mut config = config::load_config().await?;
    let shaping: Shaping = config
        .async_call_callback_non_default("tsa_load_shaping_data", ())
        .await
        .context("in tsa_load_shaping_data event")?;

    let matches = shaping.match_rules(&record, &domain, &mx.site_name);

    for m in &matches {
        tracing::info!("Matched: {m:?}");
    }

    Ok(())
}

async fn publish_log_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(record): Json<JsonLogRecord>,
) -> Result<(), AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Bounce to the thread pool where we can run async lua
    rt_spawn("process record".to_string(), move || {
        Ok(async move { tx.send(publish_log_v1_impl(record).await) })
    })
    .await?;
    rx.await?
}
