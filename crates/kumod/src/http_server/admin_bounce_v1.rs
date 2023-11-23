use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::queue::QueueManager;
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use kumo_api_types::{BounceV1CancelRequest, BounceV1ListEntry, BounceV1Request, BounceV1Response};
use kumo_server_common::http_server::auth::TrustedIpRequired;
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn_non_blocking;
use message::message::QueueNameComponents;
use message::Message;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use uuid::Uuid;

lazy_static::lazy_static! {
    static ref ENTRIES: Mutex<Vec<AdminBounceEntry>> = Mutex::new(vec![]);
}

#[derive(Clone, Debug)]
pub struct AdminBounceEntry {
    pub id: Uuid,
    pub campaign: Option<String>,
    pub tenant: Option<String>,
    pub domain: Option<String>,
    pub routing_domain: Option<String>,
    pub reason: String,
    pub suppress_logging: bool,
    pub expires: Instant,
    pub bounced: Arc<Mutex<HashMap<String, usize>>>,
}

fn match_criteria(current_thing: Option<&str>, wanted_thing: Option<&str>) -> bool {
    match (current_thing, wanted_thing) {
        (Some(a), Some(b)) => a == b,
        (None, Some(_)) => {
            // Needs to match a specific thing and there is none
            false
        }
        (_, None) => {
            // No specific campaign required
            true
        }
    }
}

impl AdminBounceEntry {
    pub fn get_all() -> Vec<Self> {
        let mut entries = ENTRIES.lock().unwrap();
        let now = Instant::now();
        entries.retain(|ent| ent.expires > now);
        entries.clone()
    }

    pub fn remove_by_id(id: &Uuid) -> bool {
        let mut entries = ENTRIES.lock().unwrap();
        let len_before = entries.len();
        entries.retain(|e| e.id != *id);
        len_before != entries.len()
    }

    pub fn add(entry: Self) {
        let mut entries = ENTRIES.lock().unwrap();
        let now = Instant::now();
        // Age out expired entries, and replace any entries with the
        // same criteria; this allows updating the reason with a newer
        // version of the bounce info.
        entries.retain(|ent| {
            ent.expires > now
                && !(ent.campaign == entry.campaign
                    && ent.tenant == entry.tenant
                    && ent.domain == entry.domain
                    && ent.routing_domain == entry.routing_domain)
        });

        entries.push(entry);
    }

    pub fn matches(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
    ) -> bool {
        if !match_criteria(campaign, self.campaign.as_deref()) {
            return false;
        }
        if !match_criteria(tenant, self.tenant.as_deref()) {
            return false;
        }
        if !match_criteria(domain, self.domain.as_deref()) {
            return false;
        }
        if !match_criteria(routing_domain, self.routing_domain.as_deref()) {
            return false;
        }
        true
    }

    pub fn get_matching(
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
    ) -> Vec<Self> {
        let mut entries = Self::get_all();
        entries.retain(|ent| ent.matches(campaign, tenant, domain, routing_domain));
        entries
    }

    pub fn get_for_queue_name(queue_name: &str) -> Option<Self> {
        let components = QueueNameComponents::parse(queue_name);
        let mut entries = Self::get_matching(
            components.campaign,
            components.tenant,
            Some(components.domain),
            components.routing_domain,
        );
        entries.pop()
    }

    pub async fn list_matching_queues(&self) -> Vec<String> {
        let mut names = QueueManager::all_queue_names().await;
        names.retain(|queue_name| {
            let components = QueueNameComponents::parse(queue_name);
            self.matches(
                components.campaign,
                components.tenant,
                Some(components.domain),
                components.routing_domain,
            )
        });
        names
    }

    pub async fn log(&self, msg: Message, queue_name: Option<&str>) {
        let local_name;
        let queue_name = match queue_name {
            Some(n) => n,
            None => {
                local_name = msg.get_queue_name().unwrap_or_else(|_| "?".to_string());
                &local_name
            }
        };

        if !self.suppress_logging {
            log_disposition(LogDisposition {
                kind: RecordType::AdminBounce,
                msg,
                site: "localhost",
                peer_address: None,
                response: rfc5321::Response {
                    code: 551,
                    enhanced_code: Some(rfc5321::EnhancedStatusCode {
                        class: 5,
                        subject: 7,
                        detail: 1,
                    }),
                    content: format!("Administrator bounced with reason: {}", self.reason),
                    command: None,
                },
                egress_source: None,
                egress_pool: None,
                relay_disposition: None,
                delivery_protocol: None,
                tls_info: None,
            })
            .await;
        }

        let mut bounced = self.bounced.lock().unwrap();
        if let Some(entry) = bounced.get_mut(queue_name) {
            *entry += 1;
        } else {
            bounced.insert(queue_name.to_string(), 1);
        }
    }
}

/// Allows the system operator to administratively bounce messages that match
/// certain criteria, or if no criteria are provided, ALL messages.
#[utoipa::path(
    post,
    tag="bounce",
    path="/api/admin/bounce/v1",
    responses(
        (status = 200, description = "Bounce added successfully", body=BounceV1Response)
    ),
)]
pub async fn bounce_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<BounceV1Request>,
) -> Result<Json<BounceV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminBounceEntry {
        id: Uuid::new_v4(),
        campaign: request.campaign,
        tenant: request.tenant,
        domain: request.domain,
        routing_domain: request.routing_domain,
        reason: request.reason,
        suppress_logging: request.suppress_logging,
        expires: Instant::now() + duration,
        bounced: Arc::new(Mutex::new(HashMap::new())),
    };

    AdminBounceEntry::add(entry.clone());

    let queue_names = entry.list_matching_queues().await;
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Move into a lua-capable thread so that logging related
    // lua events can be triggered by log_disposition.
    rt_spawn_non_blocking("process_bounce_v1".to_string(), move || {
        Ok(async move {
            for name in &queue_names {
                if let Some(q) = QueueManager::get_opt(name).await {
                    q.lock().await.bounce_all(&entry).await;
                }
            }
            tx.send(entry)
        })
    })?;

    let entry = rx.await?;

    let bounced = entry.bounced.lock().unwrap().clone();
    let total_bounced = bounced.values().sum();

    Ok(Json(BounceV1Response {
        id: entry.id,
        bounced,
        total_bounced,
    }))
}

/// Allows the system operator to list all currently active administrative bounces that have been
/// configured.
#[utoipa::path(
    get,
    tag="bounce",
    path="/api/admin/bounce/v1",
    responses(
        (status = 200, description = "Returned information about current admin bounces", body=[BounceV1ListEntry])
    ),
)]
pub async fn bounce_v1_list(
    _: TrustedIpRequired,
) -> Result<Json<Vec<BounceV1ListEntry>>, AppError> {
    let now = Instant::now();
    Ok(Json(
        AdminBounceEntry::get_all()
            .into_iter()
            .filter_map(|entry| {
                let bounced = entry.bounced.lock().unwrap().clone();
                let total_bounced = bounced.values().sum();
                entry
                    .expires
                    .checked_duration_since(now)
                    .map(|duration| BounceV1ListEntry {
                        id: entry.id,
                        campaign: entry.campaign,
                        tenant: entry.tenant,
                        domain: entry.domain,
                        routing_domain: entry.routing_domain,
                        reason: entry.reason,
                        bounced,
                        total_bounced,
                        duration,
                    })
            })
            .collect(),
    ))
}

/// Allows the system operator to delete an administrative bounce entry by its id.
#[utoipa::path(
    delete,
    tag="bounce",
    path="/api/admin/bounce/v1",
    responses(
        (status = 200, description = "Removed the requested bounce id"),
        (status = 404, description = "The requested bounce id is no longer, or never was, valid"),
    ),
)]
pub async fn bounce_v1_delete(
    _: TrustedIpRequired,
    Json(request): Json<BounceV1CancelRequest>,
) -> Response {
    let removed = AdminBounceEntry::remove_by_id(&request.id);
    if removed {
        (StatusCode::OK, format!("removed {}", request.id))
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("bounce entry {} not found", request.id),
        )
    }
    .into_response()
}
