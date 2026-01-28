use crate::logging::disposition::{log_disposition, LogDisposition};
use crate::queue::{DeliveryProto, QueueConfig, QueueManager};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::spool::SpoolManager;
use anyhow::Context;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::State;
use axum::Json;
use axum_client_ip::ClientIp;
use chrono::{DateTime, Utc};
use config::{declare_event, load_config, SerdeWrappedValue};
use flate2::write::GzEncoder;
use flate2::Compression;
use kumo_api_types::xfer::XferProtocol;
use kumo_log_types::{RecordType, ResolvedAddress};
use kumo_server_common::authn_authz::AuthInfo;
use kumo_server_common::http_server::{AppError, AppState};
use message::scheduling::Scheduling;
use message::Message;
use reqwest::StatusCode;
use rfc5321::Response;
use serde::{Deserialize, Serialize};
use serde_json::json;
use spool::SpoolId;
use std::io::Write;
use std::time::Duration;
use utoipa::{ToResponse, ToSchema};

declare_event! {
static XFER_IN: Single(
    "xfer_message_received",
    message: Message,
    auth_info: SerdeWrappedValue<AuthInfo>,
) -> ();
}

pub mod cancel;
pub mod lua;
pub mod request;

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub(crate) struct SavedQueueInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    routing_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tenant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    campaign: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schedule: Option<Scheduling>,
    #[serde(default)]
    num_attempts: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due: Option<DateTime<Utc>>,
}

impl SavedQueueInfo {
    pub async fn save_info(msg: &Message) -> anyhow::Result<()> {
        let info = SavedQueueInfo {
            queue: msg.get_meta_string("queue").await?,
            routing_domain: msg.get_meta_string("routing_domain").await?,
            tenant: msg.get_meta_string("tenant").await?,
            campaign: msg.get_meta_string("campaign").await?,
            schedule: msg.get_scheduling().await?,
            num_attempts: msg.get_num_attempts(),
            due: msg.get_due(),
        };

        let info = serde_json::to_value(info)?;

        msg.set_meta("xfer_queue_info", info).await?;
        msg.unset_meta("queue").await?;
        msg.unset_meta("routing_domain").await?;
        msg.unset_meta("tenant").await?;
        msg.unset_meta("campaign").await?;
        msg.set_scheduling(None).await?;
        msg.set_due(None).await?;

        Ok(())
    }

    pub async fn restore_info(msg: &Message) -> anyhow::Result<()> {
        let info = msg.get_meta("xfer_queue_info").await?;
        let info: SavedQueueInfo = serde_json::from_value(info)?;

        if let Some(queue) = info.queue {
            msg.set_meta("queue", queue).await?;
        } else {
            msg.unset_meta("queue").await?;
        }

        if let Some(routing_domain) = info.routing_domain {
            msg.set_meta("routing_domain", routing_domain).await?;
        } else {
            msg.unset_meta("routing_domain").await?;
        }

        if let Some(tenant) = info.tenant {
            msg.set_meta("tenant", tenant).await?;
        } else {
            msg.unset_meta("tenant").await?;
        }

        if let Some(campaign) = info.campaign {
            msg.set_meta("campaign", campaign).await?;
        } else {
            msg.unset_meta("campaign").await?;
        }

        msg.set_num_attempts(info.num_attempts);
        msg.set_scheduling(info.schedule).await?;
        msg.set_due(info.due).await?;

        msg.unset_meta("xfer_queue_info").await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct XferDispatcher {
    pub proto: XferProtocol,
}

pub fn make_xfer_queue(name: &str) -> Option<QueueConfig> {
    let xfer = XferProtocol::from_queue_name(name)?;
    Some(QueueConfig {
        protocol: DeliveryProto::Xfer { xfer },
        retry_interval: Duration::from_secs(10),
        ..QueueConfig::default()
    })
}

impl XferDispatcher {
    pub async fn init(_dispatcher: &mut Dispatcher, proto: &XferProtocol) -> anyhow::Result<Self> {
        Ok(Self {
            proto: proto.clone(),
        })
    }
}

#[async_trait]
impl QueueDispatcher for XferDispatcher {
    async fn close_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn attempt_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        Ok(())
    }

    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }

    async fn deliver_message(
        &mut self,
        mut msgs: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            msgs.len() == 1,
            "XferDispatcher only supports a batch size of 1"
        );
        let msg = msgs.pop().expect("just verified that there is one");

        let nodeid = kumo_server_common::nodeid::NodeId::get_uuid();

        // Capture some originating info that might be useful
        // for the target node
        let additional_meta = json!({
            "xfer_prior_id": msg.id(),
            "xfer_prior_node": nodeid,
        });

        let serialized = msg.serialize_for_xfer(additional_meta).await?;

        // Compress with gzip
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&serialized)?;
        let compressed_data = encoder.finish()?;

        let mut url = self.proto.target.clone();
        url.set_path("/api/xfer/inject/v1");
        let path_config = dispatcher.path_config.borrow();

        let response = reqwest::Client::builder()
            .timeout(path_config.client_timeouts.data_dot_timeout)
            .build()?
            .post(url.clone())
            .header("Content-Encoding", "gzip")
            .header("Content-Type", "application/octet-stream")
            .body(compressed_data)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body_bytes = response.bytes().await.with_context(|| {
                format!(
                    "request status {url}: {}: {}, and failed to read response body",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )
            })?;
            anyhow::bail!(
                "request status {url}: {}: {}. Response body: {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or(""),
                String::from_utf8_lossy(&body_bytes)
            );
        }

        let data = response.bytes().await.context("read response body")?;
        let response: XferResponseV1 = serde_json::from_slice(&data).with_context(|| {
            format!(
                "parsing response as json: {}",
                String::from_utf8_lossy(&data)
            )
        })?;

        if let Some(msg) = dispatcher.msgs.pop() {
            log_disposition(LogDisposition {
                kind: RecordType::XferOut,
                msg: msg.clone(),
                site: &dispatcher.name,
                peer_address: None,
                response: Response {
                    code: 250,
                    enhanced_code: None,
                    content: format!("new id is {}", response.id),
                    command: None,
                },
                egress_pool: None,
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: Some("xfer"),
                tls_info: None,
                source_address: None,
                provider: None,
                session_id: None,
                recipient_list: None,
            })
            .await;

            SpoolManager::remove_from_spool(*msg.id()).await?;
        }
        dispatcher.metrics.inc_delivered();

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, ToResponse, ToSchema)]
pub struct XferResponseV1 {
    pub id: SpoolId,
}

/// Performs a message transfer.
/// This is considered to be an internal API and should not be
/// targeted by external consumers.
#[utoipa::path(
    post,
    tag="xfer",
    path="/api/xfer/inject/v1",
    request_body=String,
    responses(
      (status = 200, description = "Message transferred successfully", body=XferResponseV1)
    ),
)]
pub async fn inject_xfer_v1(
    auth: AuthInfo,
    ClientIp(peer_address): ClientIp,
    State(app_state): State<AppState>,
    body: Bytes,
) -> Result<Json<XferResponseV1>, AppError> {
    if kumo_server_memory::get_headroom() == 0 {
        // Using too much memory
        return Err(AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "load shedding",
        ));
    }
    if kumo_server_common::disk_space::is_over_limit() {
        return Err(AppError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "disk is too full",
        ));
    }

    let msg = Message::deserialize_from_xfer(&body)?;

    // Sanity check for the most basic kind of loop
    let nodeid = kumo_server_common::nodeid::NodeId::get_uuid();
    let prior_node = msg
        .get_meta("xfer_prior_node")
        .await
        .context("failed to get xfer_prior_node")?
        .to_string();

    if nodeid.to_string() == prior_node {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("cannot xfer to myself. xfer_prior_node={nodeid} which is my node id"),
        ));
    }

    msg.set_meta("xfer_from", peer_address.to_string()).await?;
    msg.set_meta("xfer_via", app_state.local_addr().to_string())
        .await?;
    msg.set_meta("xfer_auth", auth.summarize_for_http_auth())
        .await?;

    // set up the next due time using the source due+scheduling info
    SavedQueueInfo::restore_info(&msg).await?;

    {
        let mut config = load_config().await?;
        config
            .async_call_callback(&XFER_IN, (msg.clone(), SerdeWrappedValue(auth.clone())))
            .await?;
        config.put();
    }

    msg.save(None).await?;
    log_disposition(LogDisposition {
        kind: RecordType::XferIn,
        msg: msg.clone(),
        site: "",
        peer_address: Some(&ResolvedAddress {
            name: "".to_string(),
            addr: peer_address.into(),
        }),
        response: Response {
            code: 250,
            enhanced_code: None,
            command: None,
            content: "".to_string(),
        },
        egress_source: None,
        egress_pool: None,
        relay_disposition: None,
        delivery_protocol: Some("xfer"),
        tls_info: None,
        source_address: None,
        provider: None,
        session_id: None,
        recipient_list: None,
    })
    .await;

    let queue_name = msg.get_queue_name().await?;
    let deferred_spool = false;

    let id = *msg.id();
    QueueManager::insert_or_unwind(&queue_name, msg.clone(), deferred_spool, None).await?;

    Ok(Json(XferResponseV1 { id }))
}
