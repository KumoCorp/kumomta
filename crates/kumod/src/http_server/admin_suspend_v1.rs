use crate::http_server::queue_name_multi_index::{Criteria, GetCriteria, QueueNameMultiIndexMap};
use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use config::get_or_create_sub_module;
use kumo_api_types::{
    SuspendV1CancelRequest, SuspendV1ListEntry, SuspendV1Request, SuspendV1Response,
};
use kumo_server_common::http_server::AppError;
use message::message::QueueNameComponents;
use mlua::{Lua, LuaSerdeExt, Value};
use parking_lot::FairMutex as Mutex;
use std::sync::LazyLock;
use std::time::Instant;
use uuid::Uuid;

static ENTRIES: LazyLock<Mutex<QueueNameMultiIndexMap<AdminSuspendEntry>>> =
    LazyLock::new(|| Mutex::new(QueueNameMultiIndexMap::new()));

#[derive(Clone, Debug)]
pub struct AdminSuspendEntry {
    pub id: Uuid,
    pub criteria: Criteria,
    pub reason: String,
    pub expires: Instant,
}

impl GetCriteria for AdminSuspendEntry {
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

impl AdminSuspendEntry {
    pub fn get_duration(&self) -> chrono::Duration {
        let duration = self.expires.saturating_duration_since(Instant::now());
        chrono::Duration::from_std(duration).unwrap_or(kumo_chrono_helper::SECOND)
    }
}

impl AdminSuspendEntry {
    pub fn get_all() -> Vec<Self> {
        let mut entries = ENTRIES.lock();
        entries.prune_expired();
        entries.get_all()
    }

    pub fn get_all_v1() -> Vec<SuspendV1ListEntry> {
        let now = Instant::now();
        Self::get_all()
            .into_iter()
            .filter_map(|entry| {
                entry
                    .expires
                    .checked_duration_since(now)
                    .map(|duration| SuspendV1ListEntry {
                        id: entry.id,
                        campaign: entry.criteria.campaign,
                        tenant: entry.criteria.tenant,
                        domain: entry.criteria.domain,
                        reason: entry.reason,
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
        // version of the suspend info.
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
            None,
            Some(queue_name),
        )
    }
}

/// Define a suspension for a scheduled queue
#[utoipa::path(
    post,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1Response),
    ),
)]
pub async fn suspend(
    // Note: Json<> must be last in the param list
    Json(request): Json<SuspendV1Request>,
) -> Result<Json<SuspendV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminSuspendEntry {
        id: Uuid::new_v4(),
        criteria: Criteria {
            campaign: request.campaign,
            tenant: request.tenant,
            domain: request.domain,
            routing_domain: None,
            queue_names: request.queue_names.into_iter().collect(),
        },
        reason: request.reason,
        expires: Instant::now() + duration,
    };

    AdminSuspendEntry::add(entry.clone());

    Ok(Json(SuspendV1Response { id: entry.id }))
}

/// List the active scheduled-queue suspensions
#[utoipa::path(
    get,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1ListEntry),
    ),
)]
pub async fn list() -> Result<Json<Vec<SuspendV1ListEntry>>, AppError> {
    Ok(Json(AdminSuspendEntry::get_all_v1()))
}

/// Remove a scheduled-queue suspension
#[utoipa::path(
    delete,
    tag="suspend",
    path="/api/admin/suspend/v1",
    responses(
        (status = 200, description = "Removed the suspension"),
        (status = 404, description = "Suspension either expired or was never valid"),
    ),
)]
pub async fn delete(Json(request): Json<SuspendV1CancelRequest>) -> Response {
    let removed = AdminSuspendEntry::remove_by_id(&request.id);
    if removed {
        (StatusCode::OK, format!("removed {}", request.id))
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("suspend entry {} not found", request.id),
        )
    }
    .into_response()
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "api.admin.suspend")?;

    module.set(
        "list",
        lua.create_function(move |lua, ()| {
            let result = AdminSuspendEntry::get_all_v1();
            lua.to_value(&result)
        })?,
    )?;

    module.set(
        "suspend",
        lua.create_function(move |lua, request: Value| {
            let request: SuspendV1Request = lua.from_value(request)?;

            let duration = request.duration();
            let id = Uuid::new_v4();
            let entry = AdminSuspendEntry {
                id,
                criteria: Criteria {
                    campaign: request.campaign,
                    tenant: request.tenant,
                    domain: request.domain,
                    routing_domain: None, // FIXME: add to API surface
                    queue_names: request.queue_names.into_iter().collect(),
                },
                reason: request.reason,
                expires: Instant::now() + duration,
            };

            AdminSuspendEntry::add(entry);
            lua.to_value(&id)
        })?,
    )?;

    module.set(
        "delete",
        lua.create_function(move |lua, id: Value| {
            let id: Uuid = lua.from_value(id)?;
            let removed = AdminSuspendEntry::remove_by_id(&id);
            Ok(removed)
        })?,
    )?;

    Ok(())
}
