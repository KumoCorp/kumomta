use axum::extract::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use config::get_or_create_sub_module;
use kumo_api_types::{
    SuspendReadyQueueV1ListEntry, SuspendReadyQueueV1Request, SuspendV1CancelRequest,
    SuspendV1Response,
};
use kumo_server_common::http_server::AppError;
use mlua::{Lua, LuaSerdeExt, Value};
use parking_lot::FairMutex as Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::Instant;
use uuid::Uuid;

static ENTRIES: LazyLock<Mutex<Suspensions>> = LazyLock::new(|| Mutex::new(Suspensions::default()));

static GENERATION: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Suspensions {
    entries: HashMap<String, AdminSuspendReadyQEntry>,
    generation: usize,
    counter: usize,
}

impl Suspensions {
    fn do_expire(&mut self) {
        if !self.entries.is_empty() {
            let mut changed = false;
            let now = Instant::now();

            self.entries.retain(|_, entry| {
                if entry.expires > now {
                    true
                } else {
                    changed = true;
                    false
                }
            });

            if changed {
                self.inc_generation();
            }
        }
    }

    fn inc_generation(&mut self) {
        self.generation += 1;
        GENERATION.store(self.generation, Ordering::Relaxed);
    }

    fn remove_by_id(&mut self, id: &Uuid) -> bool {
        let mut changed = false;
        self.entries.retain(|_, entry| {
            if entry.id == *id {
                changed = true;
                false
            } else {
                true
            }
        });
        if changed {
            self.inc_generation();
        }
        changed
    }

    fn maybe_expire(&mut self) {
        self.counter += 1;
        if self.counter > 100_000 {
            self.counter = 0;
            self.do_expire();
        }
    }

    /// Replace any entries with the
    /// same criteria; this allows updating the reason with a newer
    /// version of the suspend info.
    fn insert(&mut self, entry: AdminSuspendReadyQEntry) {
        self.entries.insert(entry.name.clone(), entry);
        self.inc_generation();
    }
}

#[derive(Clone, Debug)]
pub struct AdminSuspendReadyQEntryRef {
    generation: usize,
    entry: AdminSuspendReadyQEntry,
}

impl AdminSuspendReadyQEntryRef {
    pub fn has_expired(&self) -> bool {
        self.generation != GENERATION.load(Ordering::Relaxed) || self.expires <= Instant::now()
    }
}

impl std::ops::Deref for AdminSuspendReadyQEntryRef {
    type Target = AdminSuspendReadyQEntry;
    fn deref(&self) -> &AdminSuspendReadyQEntry {
        &self.entry
    }
}

#[derive(Clone, Debug)]
pub struct AdminSuspendReadyQEntry {
    pub id: Uuid,
    pub name: String,
    pub reason: String,
    pub expires: Instant,
}

impl AdminSuspendReadyQEntry {
    pub fn get_duration(&self) -> std::time::Duration {
        self.expires.saturating_duration_since(Instant::now())
    }
    pub fn get_duration_chrono(&self) -> chrono::Duration {
        chrono::Duration::from_std(self.get_duration()).unwrap_or(kumo_chrono_helper::MINUTE)
    }
}

impl AdminSuspendReadyQEntry {
    fn get_all() -> Vec<Self> {
        let mut entries = ENTRIES.lock();
        entries.do_expire();
        entries.entries.values().cloned().collect()
    }

    pub fn get_all_v1() -> Vec<SuspendReadyQueueV1ListEntry> {
        let now = Instant::now();
        AdminSuspendReadyQEntry::get_all()
            .into_iter()
            .filter_map(|entry| {
                entry.expires.checked_duration_since(now).map(|duration| {
                    SuspendReadyQueueV1ListEntry {
                        id: entry.id,
                        name: entry.name,
                        reason: entry.reason,
                        duration,
                        expires: chrono::Utc::now() + duration,
                    }
                })
            })
            .collect()
    }

    pub fn remove_by_id(id: &Uuid) -> bool {
        let mut entries = ENTRIES.lock();
        entries.remove_by_id(id)
    }

    pub fn add(entry: Self) {
        let mut entries = ENTRIES.lock();
        entries.insert(entry);
    }

    pub fn get_for_queue_name(name: &str) -> Option<AdminSuspendReadyQEntryRef> {
        let mut entries = ENTRIES.lock();
        entries.maybe_expire();
        if let Some(entry) = entries.entries.get(name) {
            let now = Instant::now();
            if entry.expires > now {
                return Some(AdminSuspendReadyQEntryRef {
                    entry: entry.clone(),
                    generation: entries.generation,
                });
            }
        }
        None
    }
}

/// Define a suspension for a ready queue
#[utoipa::path(
    post,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendV1Response),
    ),
)]
pub async fn suspend(
    // Note: Json<> must be last in the param list
    Json(request): Json<SuspendReadyQueueV1Request>,
) -> Result<Json<SuspendV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminSuspendReadyQEntry {
        id: Uuid::new_v4(),
        name: request.name,
        reason: request.reason,
        expires: Instant::now() + duration,
    };

    AdminSuspendReadyQEntry::add(entry.clone());

    Ok(Json(SuspendV1Response { id: entry.id }))
}

/// List the active ready-queue suspensions
#[utoipa::path(
    get,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Suspended", body=SuspendReadyQueueV1ListEntry),
    ),
)]
pub async fn list() -> Result<Json<Vec<SuspendReadyQueueV1ListEntry>>, AppError> {
    Ok(Json(AdminSuspendReadyQEntry::get_all_v1()))
}

/// Remove a ready-queue suspension
#[utoipa::path(
    delete,
    tag="suspend",
    path="/api/admin/suspend-ready-q/v1",
    responses(
        (status = 200, description = "Removed the suspension"),
        (status = 404, description = "Suspension either expired or was never valid"),
    ),
)]
pub async fn delete(Json(request): Json<SuspendV1CancelRequest>) -> Response {
    let removed = AdminSuspendReadyQEntry::remove_by_id(&request.id);
    if removed {
        (StatusCode::OK, format!("removed {}", request.id))
    } else {
        (
            StatusCode::NOT_FOUND,
            format!("suspend-ready-q entry {} not found", request.id),
        )
    }
    .into_response()
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "api.admin.suspend_ready_q")?;

    module.set(
        "list",
        lua.create_function(move |lua, ()| {
            let result = AdminSuspendReadyQEntry::get_all_v1();
            lua.to_value(&result)
        })?,
    )?;

    module.set(
        "suspend",
        lua.create_function(move |lua, request: Value| {
            let request: SuspendReadyQueueV1Request = lua.from_value(request)?;

            let duration = request.duration();
            let id = Uuid::new_v4();
            let entry = AdminSuspendReadyQEntry {
                id,
                name: request.name,
                reason: request.reason,
                expires: Instant::now() + duration,
            };

            AdminSuspendReadyQEntry::add(entry);
            lua.to_value(&id)
        })?,
    )?;

    module.set(
        "delete",
        lua.create_function(move |lua, id: Value| {
            let id: Uuid = lua.from_value(id)?;
            let removed = AdminSuspendReadyQEntry::remove_by_id(&id);
            Ok(removed)
        })?,
    )?;

    Ok(())
}
