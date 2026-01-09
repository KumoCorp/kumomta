use crate::http_server::queue_name_multi_index::{
    CachedEntry, Criteria, GetCriteria, QueueNameMultiIndexMap,
};
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::QueueManager;
use arc_swap::ArcSwap;
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use config::get_or_create_sub_module;
use kumo_api_types::{BounceV1CancelRequest, BounceV1ListEntry, BounceV1Request, BounceV1Response};
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::rt_spawn;
use message::message::QueueNameComponents;
use message::Message;
use mlua::{Lua, LuaSerdeExt};
use parking_lot::FairMutex as Mutex;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use uuid::Uuid;

static ENTRIES: LazyLock<Mutex<QueueNameMultiIndexMap<AdminBounceEntry>>> =
    LazyLock::new(|| Mutex::new(QueueNameMultiIndexMap::new()));

#[derive(Clone, Debug)]
pub struct AdminBounceEntry {
    pub id: Uuid,
    pub criteria: Criteria,
    pub reason: String,
    pub suppress_logging: bool,
    pub expires: Instant,
    pub bounced: Arc<Mutex<HashMap<String, usize>>>,
}

impl GetCriteria for AdminBounceEntry {
    fn get_id(&self) -> &Uuid {
        &self.id
    }

    fn get_criteria(&self) -> &Criteria {
        &self.criteria
    }

    fn get_expires(&self) -> Instant {
        self.expires
    }
}

impl AdminBounceEntry {
    pub fn get_all() -> Vec<Self> {
        let mut entries = ENTRIES.lock();
        entries.prune_expired();
        entries.get_all()
    }

    pub fn get_all_v1() -> Vec<BounceV1ListEntry> {
        let now = Instant::now();
        Self::get_all()
            .into_iter()
            .filter_map(|entry| {
                let bounced = entry.bounced.lock().clone();
                let total_bounced = bounced.values().sum();
                entry
                    .expires
                    .checked_duration_since(now)
                    .map(|duration| BounceV1ListEntry {
                        id: entry.id,
                        campaign: entry.criteria.campaign,
                        tenant: entry.criteria.tenant,
                        domain: entry.criteria.domain,
                        routing_domain: entry.criteria.routing_domain,
                        reason: entry.reason,
                        bounced,
                        total_bounced,
                        duration,
                    })
            })
            .collect()
    }

    pub fn remove_by_id(id: &Uuid) -> bool {
        let mut entries = ENTRIES.lock();
        entries.remove_by_id(id).is_some()
    }

    pub fn add(entry: Self) {
        let mut entries = ENTRIES.lock();
        // Age out expired entries, and replace any entries with the
        // same criteria; this allows updating the reason with a newer
        // version of the bounce info.
        entries.maybe_prune();
        entries.insert(entry);
    }

    pub fn get_for_queue_name(queue_name: &str) -> Option<Self> {
        let components = QueueNameComponents::parse(queue_name);
        let mut entries = ENTRIES.lock();
        entries.maybe_prune();
        entries.get_matching(
            components.campaign,
            components.tenant,
            Some(components.domain),
            components.routing_domain,
            Some(queue_name),
        )
    }

    pub fn cached_get_for_queue_name(
        queue_name: &str,
        cache: &ArcSwap<Option<CachedEntry<Self>>>,
    ) -> Option<Self> {
        let components = QueueNameComponents::parse(queue_name);
        let mut entries = ENTRIES.lock();
        entries.maybe_prune();
        entries.cached_get_matching(
            components.campaign,
            components.tenant,
            Some(components.domain),
            components.routing_domain,
            Some(queue_name),
            cache,
        )
    }

    pub async fn list_matching_queues(&self) -> Vec<String> {
        let mut names = QueueManager::all_queue_names();
        names.retain(|queue_name| {
            let components = QueueNameComponents::parse(queue_name);
            self.criteria.matches(
                components.campaign,
                components.tenant,
                Some(components.domain),
                components.routing_domain,
                Some(queue_name),
            )
        });
        names
    }

    pub async fn log(&self, msg: Message, queue_name: Option<&str>) {
        let local_name;
        let queue_name = match queue_name {
            Some(n) => n,
            None => {
                local_name = msg
                    .get_queue_name()
                    .await
                    .unwrap_or_else(|_| "?".to_string());
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
                provider: None,
                tls_info: None,
                source_address: None,
                session_id: None,
                recipient_list: None,
            })
            .await;
        }

        let mut bounced = self.bounced.lock();
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
    // Note: Json<> must be last in the param list
    Json(request): Json<BounceV1Request>,
) -> Result<Json<BounceV1Response>, AppError> {
    let duration = request.duration();

    let id = Uuid::new_v4();
    let entry = AdminBounceEntry {
        id,
        criteria: Criteria {
            campaign: request.campaign,
            tenant: request.tenant,
            domain: request.domain,
            routing_domain: request.routing_domain,
            queue_names: request.queue_names.into_iter().collect(),
        },
        reason: request.reason,
        suppress_logging: request.suppress_logging,
        expires: Instant::now() + duration,
        bounced: Arc::new(Mutex::new(HashMap::new())),
    };

    AdminBounceEntry::add(entry.clone());

    let queue_names = entry.list_matching_queues().await;

    // Move into a lua-capable thread so that logging related
    // lua events can be triggered by log_disposition.
    rt_spawn("process_bounce_v1".to_string(), async move {
        for name in &queue_names {
            if let Some(q) = QueueManager::get_opt(name) {
                q.bounce_all(&entry).await;
            }
        }
    })?;

    Ok(Json(BounceV1Response {
        id,
        bounced: Default::default(),
        total_bounced: 0,
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
pub async fn bounce_v1_list() -> Result<Json<Vec<BounceV1ListEntry>>, AppError> {
    Ok(Json(AdminBounceEntry::get_all_v1()))
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
pub async fn bounce_v1_delete(Json(request): Json<BounceV1CancelRequest>) -> Response {
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

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "api.admin.bounce")?;

    module.set(
        "list",
        lua.create_function(move |lua, ()| {
            let result = AdminBounceEntry::get_all_v1();
            lua.to_value(&result)
        })?,
    )?;

    module.set(
        "bounce",
        lua.create_function(move |lua, request: mlua::Value| {
            let request: BounceV1Request = lua.from_value(request)?;

            let duration = request.duration();
            let id = Uuid::new_v4();
            let entry = AdminBounceEntry {
                id,
                criteria: Criteria {
                    campaign: request.campaign,
                    tenant: request.tenant,
                    domain: request.domain,
                    routing_domain: request.routing_domain,
                    queue_names: request.queue_names.into_iter().collect(),
                },
                reason: request.reason,
                expires: Instant::now() + duration,
                suppress_logging: false,
                bounced: Arc::new(Mutex::new(HashMap::new())),
            };

            AdminBounceEntry::add(entry);
            lua.to_value(&id)
        })?,
    )?;

    module.set(
        "delete",
        lua.create_function(move |lua, id: mlua::Value| {
            let id: Uuid = lua.from_value(id)?;
            let removed = AdminBounceEntry::remove_by_id(&id);
            Ok(removed)
        })?,
    )?;

    Ok(())
}
