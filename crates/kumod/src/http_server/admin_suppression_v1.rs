//! Suppression List API handlers
//!
//! This module implements suppression list management endpoints
//! for managing email addresses that should not receive certain types of emails.
//!
//! Supports two storage backends:
//! - In-memory (default): Fast but data is lost on restart
//! - RocksDB: Persistent storage that survives restarts

use axum::extract::{Json, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use config::{any_err, from_lua_value, get_or_create_module, get_or_create_sub_module};
use kumo_api_types::suppression::{
    SuppressionBulkCreateRequest, SuppressionBulkDeleteRequest, SuppressionCheckRequest,
    SuppressionCheckResponse, SuppressionCreateRequest, SuppressionCreateResponse,
    SuppressionDeleteRequest, SuppressionDeleteResponse, SuppressionEntry, SuppressionError,
    SuppressionListRequest, SuppressionListResponse, SuppressionSource, SuppressionSummaryResponse,
    SuppressionType,
};
use kumo_server_common::http_server::AppError;
use mlua::{Lua, LuaSerdeExt, Value};
use parking_lot::RwLock;
use rocksdb::{DBCompressionType, IteratorMode, Options, WriteBatch, DB};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock};

/// Maximum number of results to return in a single list request
const MAX_LIST_LIMIT: usize = 10_000;
/// Default number of results to return
const DEFAULT_LIST_LIMIT: usize = 1_000;

// ============================================================================
// Storage Abstraction
// ============================================================================

/// Key for identifying a suppression entry
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SuppressionKey {
    pub recipient: String,
    pub suppression_type: SuppressionType,
    pub subaccount_id: Option<String>,
}

impl SuppressionKey {
    pub fn new(
        recipient: &str,
        suppression_type: SuppressionType,
        subaccount_id: Option<&str>,
    ) -> Self {
        Self {
            recipient: recipient.to_lowercase(),
            suppression_type,
            subaccount_id: subaccount_id.map(|s| s.to_string()),
        }
    }

    /// Convert key to bytes for RocksDB storage
    fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("SuppressionKey serialization should not fail")
    }

    /// Parse key from bytes
    fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

/// Trait for suppression list storage backends
pub trait SuppressionStore: Send + Sync {
    /// Get an entry by key
    #[allow(dead_code)]
    fn get(&self, key: &SuppressionKey) -> Option<SuppressionEntry>;

    /// Insert or update an entry, returns true if it was an update
    fn upsert(&self, key: SuppressionKey, entry: SuppressionEntry) -> bool;

    /// Remove an entry, returns true if it existed
    fn remove(&self, key: &SuppressionKey) -> bool;

    /// Check if a key exists
    fn contains(&self, key: &SuppressionKey) -> bool;

    /// Get all entries (for listing)
    fn entries(&self) -> Vec<(SuppressionKey, SuppressionEntry)>;

    /// Get entries for a specific recipient
    fn entries_for_recipient(&self, recipient: &str) -> Vec<SuppressionEntry>;

    /// Get the total count
    fn len(&self) -> usize;

    /// Check if empty
    #[allow(dead_code)]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all entries (mainly for testing)
    #[allow(dead_code)]
    fn clear(&self);
}

// ============================================================================
// In-Memory Storage Implementation
// ============================================================================

/// In-memory storage backend (default)
pub struct InMemorySuppressionStore {
    data: RwLock<HashMap<SuppressionKey, SuppressionEntry>>,
}

impl InMemorySuppressionStore {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemorySuppressionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SuppressionStore for InMemorySuppressionStore {
    fn get(&self, key: &SuppressionKey) -> Option<SuppressionEntry> {
        self.data.read().get(key).cloned()
    }

    fn upsert(&self, key: SuppressionKey, entry: SuppressionEntry) -> bool {
        let mut data = self.data.write();
        let is_update = data.contains_key(&key);
        data.insert(key, entry);
        is_update
    }

    fn remove(&self, key: &SuppressionKey) -> bool {
        self.data.write().remove(key).is_some()
    }

    fn contains(&self, key: &SuppressionKey) -> bool {
        self.data.read().contains_key(key)
    }

    fn entries(&self) -> Vec<(SuppressionKey, SuppressionEntry)> {
        self.data
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn entries_for_recipient(&self, recipient: &str) -> Vec<SuppressionEntry> {
        let recipient_lower = recipient.to_lowercase();
        self.data
            .read()
            .iter()
            .filter(|(k, _)| k.recipient == recipient_lower)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn len(&self) -> usize {
        self.data.read().len()
    }

    fn clear(&self) {
        self.data.write().clear();
    }
}

// ============================================================================
// RocksDB Storage Implementation
// ============================================================================

/// Configuration for RocksDB suppression store
#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct RocksDBSuppressionStoreConfig {
    /// Path to the RocksDB database directory
    pub path: PathBuf,

    /// Enable fsync for durability (default: true)
    #[serde(default = "default_flush")]
    pub flush: bool,

    /// Compression type (default: lz4)
    #[serde(default)]
    pub compression: CompressionType,
}

fn default_flush() -> bool {
    true
}

#[derive(Deserialize, Debug, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    Snappy,
    #[default]
    Lz4,
    Zstd,
}

impl From<CompressionType> for DBCompressionType {
    fn from(ct: CompressionType) -> Self {
        match ct {
            CompressionType::None => DBCompressionType::None,
            CompressionType::Snappy => DBCompressionType::Snappy,
            CompressionType::Lz4 => DBCompressionType::Lz4,
            CompressionType::Zstd => DBCompressionType::Zstd,
        }
    }
}

/// RocksDB-backed storage for persistence
pub struct RocksDBSuppressionStore {
    db: Arc<DB>,
}

impl RocksDBSuppressionStore {
    pub fn new(config: RocksDBSuppressionStoreConfig) -> anyhow::Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_use_fsync(config.flush);
        opts.set_compression_type(config.compression.into());
        opts.set_keep_log_file_num(5);

        // Create parent directories if they don't exist
        if let Some(parent) = config.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = DB::open(&opts, &config.path)?;
        tracing::info!(
            "Opened RocksDB suppression store at {}",
            config.path.display()
        );

        Ok(Self { db: Arc::new(db) })
    }
}

impl SuppressionStore for RocksDBSuppressionStore {
    fn get(&self, key: &SuppressionKey) -> Option<SuppressionEntry> {
        let key_bytes = key.to_bytes();
        match self.db.get(&key_bytes) {
            Ok(Some(value)) => serde_json::from_slice(&value).ok(),
            _ => None,
        }
    }

    fn upsert(&self, key: SuppressionKey, entry: SuppressionEntry) -> bool {
        let key_bytes = key.to_bytes();
        let is_update = self.db.get(&key_bytes).ok().flatten().is_some();

        let value_bytes =
            serde_json::to_vec(&entry).expect("SuppressionEntry serialization should not fail");

        if let Err(e) = self.db.put(&key_bytes, &value_bytes) {
            tracing::error!("Failed to write suppression entry to RocksDB: {e}");
        }

        is_update
    }

    fn remove(&self, key: &SuppressionKey) -> bool {
        let key_bytes = key.to_bytes();
        let existed = self.db.get(&key_bytes).ok().flatten().is_some();

        if existed {
            if let Err(e) = self.db.delete(&key_bytes) {
                tracing::error!("Failed to delete suppression entry from RocksDB: {e}");
                return false;
            }
        }

        existed
    }

    fn contains(&self, key: &SuppressionKey) -> bool {
        let key_bytes = key.to_bytes();
        self.db.get(&key_bytes).ok().flatten().is_some()
    }

    fn entries(&self) -> Vec<(SuppressionKey, SuppressionEntry)> {
        let mut results = Vec::new();

        for item in self.db.iterator(IteratorMode::Start) {
            let Ok(kv) = item else { continue };
            let key_bytes: Box<[u8]> = kv.0;
            let value_bytes: Box<[u8]> = kv.1;
            if let (Ok(key), Ok(entry)) = (
                SuppressionKey::from_bytes(&key_bytes),
                serde_json::from_slice::<SuppressionEntry>(&value_bytes),
            ) {
                results.push((key, entry));
            }
        }

        results
    }

    fn entries_for_recipient(&self, recipient: &str) -> Vec<SuppressionEntry> {
        let recipient_lower = recipient.to_lowercase();
        self.entries()
            .into_iter()
            .filter(|(k, _)| k.recipient == recipient_lower)
            .map(|(_, v)| v)
            .collect()
    }

    fn len(&self) -> usize {
        // Note: This is not efficient for large datasets
        // Consider maintaining a separate counter if needed
        self.db
            .iterator(IteratorMode::Start)
            .filter(|r| r.is_ok())
            .count()
    }

    fn clear(&self) {
        // Delete all entries
        let mut keys: Vec<Vec<u8>> = Vec::new();
        for item in self.db.iterator(IteratorMode::Start) {
            if let Ok(kv) = item {
                keys.push(kv.0.to_vec());
            }
        }

        let mut batch = WriteBatch::default();
        for key in keys {
            batch.delete(&key);
        }

        if let Err(e) = self.db.write(batch) {
            tracing::error!("Failed to clear suppression store: {e}");
        }
    }
}

// ============================================================================
// Global Store Management
// ============================================================================

/// The global suppression store instance
static SUPPRESSION_STORE: OnceLock<Arc<dyn SuppressionStore>> = OnceLock::new();

/// Default in-memory store (used if not configured)
static DEFAULT_STORE: LazyLock<Arc<dyn SuppressionStore>> =
    LazyLock::new(|| Arc::new(InMemorySuppressionStore::new()));

/// Get the configured suppression store
fn get_store() -> &'static Arc<dyn SuppressionStore> {
    SUPPRESSION_STORE.get().unwrap_or(&DEFAULT_STORE)
}

/// Configure the suppression store (can only be called once)
pub fn configure_store(store: Arc<dyn SuppressionStore>) -> anyhow::Result<()> {
    SUPPRESSION_STORE
        .set(store)
        .map_err(|_| anyhow::anyhow!("Suppression store has already been configured"))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create or update a single suppression entry
fn upsert_entry(request: SuppressionCreateRequest) -> Result<bool, SuppressionError> {
    // Validate email format
    if !request.recipient.contains('@') {
        return Err(SuppressionError {
            recipient: request.recipient,
            message: "Invalid email address format".to_string(),
        });
    }

    let key = SuppressionKey::new(
        &request.recipient,
        request.suppression_type,
        request.subaccount_id.as_deref(),
    );

    let now = Utc::now();
    let entry = SuppressionEntry {
        recipient: request.recipient.to_lowercase(),
        suppression_type: request.suppression_type,
        source: request.source.unwrap_or(SuppressionSource::Manual),
        description: request.description,
        subaccount_id: request.subaccount_id,
        created: now,
        updated: now,
    };

    let is_update = get_store().upsert(key, entry);
    Ok(is_update)
}

/// Delete a suppression entry
fn delete_entry(request: &SuppressionDeleteRequest) -> usize {
    let recipient_lower = request.recipient.to_lowercase();
    let store = get_store();

    match request.suppression_type {
        Some(stype) => {
            // Delete specific type
            let key =
                SuppressionKey::new(&recipient_lower, stype, request.subaccount_id.as_deref());
            if store.remove(&key) {
                1
            } else {
                0
            }
        }
        None => {
            // Delete all types for this recipient
            let mut deleted = 0;
            for stype in [
                SuppressionType::Transactional,
                SuppressionType::NonTransactional,
            ] {
                let key =
                    SuppressionKey::new(&recipient_lower, stype, request.subaccount_id.as_deref());
                if store.remove(&key) {
                    deleted += 1;
                }
            }
            deleted
        }
    }
}

// ============================================================================
// HTTP Handlers
// ============================================================================

/// Create or update a suppression entry
#[utoipa::path(
    post,
    tag = "suppression",
    path = "/api/admin/suppression/v1",
    request_body = SuppressionCreateRequest,
    responses(
        (status = 200, description = "Suppression entry created/updated successfully", body = SuppressionCreateResponse),
        (status = 400, description = "Invalid request", body = Vec<SuppressionError>),
    ),
)]
pub async fn suppression_create(
    Json(request): Json<SuppressionCreateRequest>,
) -> Result<Json<SuppressionCreateResponse>, AppError> {
    match upsert_entry(request) {
        Ok(is_update) => Ok(Json(SuppressionCreateResponse {
            created: if is_update { 0 } else { 1 },
            updated: if is_update { 1 } else { 0 },
            errors: vec![],
        })),
        Err(e) => Ok(Json(SuppressionCreateResponse {
            created: 0,
            updated: 0,
            errors: vec![e],
        })),
    }
}

/// Create or update multiple suppression entries in bulk
#[utoipa::path(
    post,
    tag = "suppression",
    path = "/api/admin/suppression/v1/bulk",
    request_body = SuppressionBulkCreateRequest,
    responses(
        (status = 200, description = "Bulk operation completed", body = SuppressionCreateResponse)
    ),
)]
pub async fn suppression_bulk_create(
    Json(request): Json<SuppressionBulkCreateRequest>,
) -> Result<Json<SuppressionCreateResponse>, AppError> {
    let mut created = 0;
    let mut updated = 0;
    let mut errors = Vec::new();

    for entry in request.recipients {
        match upsert_entry(entry) {
            Ok(is_update) => {
                if is_update {
                    updated += 1;
                } else {
                    created += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }

    Ok(Json(SuppressionCreateResponse {
        created,
        updated,
        errors,
    }))
}

/// Retrieve a suppression entry by recipient email
#[utoipa::path(
    get,
    tag = "suppression",
    path = "/api/admin/suppression/v1/{recipient}",
    params(
        ("recipient" = String, Path, description = "Email address to look up")
    ),
    responses(
        (status = 200, description = "Suppression entries found", body = Vec<SuppressionEntry>),
        (status = 404, description = "No suppression entries found for this recipient")
    ),
)]
pub async fn suppression_get(Path(recipient): Path<String>) -> Response {
    let entries = get_store().entries_for_recipient(&recipient);

    if entries.is_empty() {
        (StatusCode::OK, Json(Vec::<SuppressionEntry>::new())).into_response()
    } else {
        Json(entries).into_response()
    }
}

/// List suppression entries with optional filtering
#[utoipa::path(
    get,
    tag = "suppression",
    path = "/api/admin/suppression/v1",
    params(SuppressionListRequest),
    responses(
        (status = 200, description = "List of suppression entries", body = SuppressionListResponse)
    ),
)]
pub async fn suppression_list(
    Query(request): Query<SuppressionListRequest>,
) -> Result<Json<SuppressionListResponse>, AppError> {
    let limit = request
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .min(MAX_LIST_LIMIT);

    // Apply filters
    let mut results: Vec<_> = get_store()
        .entries()
        .into_iter()
        .map(|(_, v)| v)
        .filter(|entry| {
            // Filter by recipient (partial match)
            if let Some(ref r) = request.recipient {
                if !entry.recipient.contains(&r.to_lowercase()) {
                    return false;
                }
            }
            // Filter by type
            if let Some(t) = request.suppression_type {
                if entry.suppression_type != t {
                    return false;
                }
            }
            // Filter by source
            if let Some(s) = request.source {
                if entry.source != s {
                    return false;
                }
            }
            // Filter by subaccount
            if request.subaccount_id.is_some() && entry.subaccount_id != request.subaccount_id {
                return false;
            }
            // Filter by date range
            if let Some(from) = request.from {
                if entry.created < from {
                    return false;
                }
            }
            if let Some(to) = request.to {
                if entry.created > to {
                    return false;
                }
            }
            true
        })
        .collect();

    // Sort by created date descending
    results.sort_by(|a, b| b.created.cmp(&a.created));

    let total_count = results.len();
    let has_more = total_count > limit;

    // Apply limit
    results.truncate(limit);

    // Generate cursor for pagination
    let next_cursor = if has_more {
        results.last().map(|e| e.created.to_rfc3339())
    } else {
        None
    };

    Ok(Json(SuppressionListResponse {
        results,
        total_count,
        next_cursor,
    }))
}

/// Delete a suppression entry
#[utoipa::path(
    delete,
    tag = "suppression",
    path = "/api/admin/suppression/v1",
    request_body = SuppressionDeleteRequest,
    responses(
        (status = 200, description = "Entry deleted successfully", body = SuppressionDeleteResponse),
        (status = 404, description = "Entry not found")
    ),
)]
pub async fn suppression_delete(Json(request): Json<SuppressionDeleteRequest>) -> Response {
    let deleted = delete_entry(&request);

    if deleted > 0 {
        Json(SuppressionDeleteResponse {
            deleted,
            errors: vec![],
        })
        .into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(SuppressionDeleteResponse {
                deleted: 0,
                errors: vec![SuppressionError {
                    recipient: request.recipient,
                    message: "Entry not found".to_string(),
                }],
            }),
        )
            .into_response()
    }
}

/// Delete multiple suppression entries in bulk
#[utoipa::path(
    post,
    tag = "suppression",
    path = "/api/admin/suppression/v1/bulk/delete",
    request_body = SuppressionBulkDeleteRequest,
    responses(
        (status = 200, description = "Bulk delete completed", body = SuppressionDeleteResponse)
    ),
)]
pub async fn suppression_bulk_delete(
    Json(request): Json<SuppressionBulkDeleteRequest>,
) -> Result<Json<SuppressionDeleteResponse>, AppError> {
    let mut deleted = 0;
    let mut errors = Vec::new();

    for entry in request.recipients {
        let count = delete_entry(&entry);
        if count > 0 {
            deleted += count;
        } else {
            errors.push(SuppressionError {
                recipient: entry.recipient,
                message: "Entry not found".to_string(),
            });
        }
    }

    Ok(Json(SuppressionDeleteResponse { deleted, errors }))
}

/// Check if recipients are suppressed
#[utoipa::path(
    post,
    tag = "suppression",
    path = "/api/admin/suppression/v1/check",
    request_body = SuppressionCheckRequest,
    responses(
        (status = 200, description = "Suppression check results", body = SuppressionCheckResponse)
    ),
)]
pub async fn suppression_check(
    Json(request): Json<SuppressionCheckRequest>,
) -> Result<Json<SuppressionCheckResponse>, AppError> {
    let store = get_store();
    let mut results = HashMap::new();

    for recipient in request.recipients {
        let recipient_lower = recipient.to_lowercase();

        let is_suppressed = match request.suppression_type {
            Some(stype) => {
                let key =
                    SuppressionKey::new(&recipient_lower, stype, request.subaccount_id.as_deref());
                store.contains(&key)
            }
            None => {
                // Check both types
                let key_trans = SuppressionKey::new(
                    &recipient_lower,
                    SuppressionType::Transactional,
                    request.subaccount_id.as_deref(),
                );
                let key_non_trans = SuppressionKey::new(
                    &recipient_lower,
                    SuppressionType::NonTransactional,
                    request.subaccount_id.as_deref(),
                );
                store.contains(&key_trans) || store.contains(&key_non_trans)
            }
        };

        results.insert(recipient, is_suppressed);
    }

    Ok(Json(SuppressionCheckResponse { results }))
}

/// Get summary statistics for the suppression list
#[utoipa::path(
    get,
    tag = "suppression",
    path = "/api/admin/suppression/v1/summary",
    responses(
        (status = 200, description = "Suppression list summary", body = SuppressionSummaryResponse)
    ),
)]
pub async fn suppression_summary() -> Result<Json<SuppressionSummaryResponse>, AppError> {
    let entries = get_store().entries();
    let total = entries.len();

    let mut by_type: HashMap<String, usize> = HashMap::new();
    let mut by_source: HashMap<String, usize> = HashMap::new();

    for (_, entry) in entries {
        *by_type
            .entry(entry.suppression_type.to_string())
            .or_insert(0) += 1;
        *by_source.entry(entry.source.to_string()).or_insert(0) += 1;
    }

    Ok(Json(SuppressionSummaryResponse {
        total,
        by_type,
        by_source,
    }))
}

// ============================================================================
// Lua API Registration
// ============================================================================

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    // Register the configuration function
    let kumo_mod = get_or_create_module(lua, "kumo")?;

    kumo_mod.set(
        "configure_suppression_store",
        lua.create_function(|lua, params: Value| {
            #[derive(Deserialize)]
            #[serde(tag = "kind", rename_all = "snake_case")]
            enum StoreConfig {
                InMemory,
                #[serde(rename = "rocks_db")]
                RocksDB(RocksDBSuppressionStoreConfig),
            }

            let config: StoreConfig = from_lua_value(&lua, params)?;

            let store: Arc<dyn SuppressionStore> = match config {
                StoreConfig::InMemory => Arc::new(InMemorySuppressionStore::new()),
                StoreConfig::RocksDB(cfg) => {
                    Arc::new(RocksDBSuppressionStore::new(cfg).map_err(any_err)?)
                }
            };

            configure_store(store).map_err(any_err)?;
            Ok(())
        })?,
    )?;

    // Register the suppression API module
    let suppression_mod = get_or_create_sub_module(lua, "api.admin.suppression")?;

    suppression_mod.set(
        "add",
        lua.create_function(|lua, params: Value| {
            let request: SuppressionCreateRequest = lua.from_value(params)?;
            match upsert_entry(request) {
                Ok(is_update) => lua.to_value(&SuppressionCreateResponse {
                    created: if is_update { 0 } else { 1 },
                    updated: if is_update { 1 } else { 0 },
                    errors: vec![],
                }),
                Err(e) => lua.to_value(&SuppressionCreateResponse {
                    created: 0,
                    updated: 0,
                    errors: vec![e],
                }),
            }
        })?,
    )?;

    suppression_mod.set(
        "remove",
        lua.create_function(|lua, params: Value| {
            let request: SuppressionDeleteRequest = lua.from_value(params)?;
            let deleted = delete_entry(&request);
            lua.to_value(&SuppressionDeleteResponse {
                deleted,
                errors: vec![],
            })
        })?,
    )?;

    suppression_mod.set(
        "check",
        lua.create_function(|lua, params: Value| {
            let request: SuppressionCheckRequest = lua.from_value(params)?;
            let store = get_store();
            let mut results = HashMap::new();

            for recipient in request.recipients {
                let recipient_lower = recipient.to_lowercase();
                let is_suppressed = match request.suppression_type {
                    Some(stype) => {
                        let key = SuppressionKey::new(
                            &recipient_lower,
                            stype,
                            request.subaccount_id.as_deref(),
                        );
                        store.contains(&key)
                    }
                    None => {
                        let key_trans = SuppressionKey::new(
                            &recipient_lower,
                            SuppressionType::Transactional,
                            request.subaccount_id.as_deref(),
                        );
                        let key_non_trans = SuppressionKey::new(
                            &recipient_lower,
                            SuppressionType::NonTransactional,
                            request.subaccount_id.as_deref(),
                        );
                        store.contains(&key_trans) || store.contains(&key_non_trans)
                    }
                };
                results.insert(recipient, is_suppressed);
            }

            lua.to_value(&SuppressionCheckResponse { results })
        })?,
    )?;

    suppression_mod.set(
        "get",
        lua.create_function(|lua, recipient: String| {
            let entries = get_store().entries_for_recipient(&recipient);
            lua.to_value(&entries)
        })?,
    )?;

    suppression_mod.set(
        "list",
        lua.create_function(|lua, params: Value| {
            let request: SuppressionListRequest = lua.from_value(params)?;
            let limit = request
                .limit
                .unwrap_or(DEFAULT_LIST_LIMIT)
                .min(MAX_LIST_LIMIT);

            let mut results: Vec<_> = get_store()
                .entries()
                .into_iter()
                .map(|(_, v)| v)
                .filter(|entry| {
                    if let Some(ref r) = request.recipient {
                        if !entry.recipient.contains(&r.to_lowercase()) {
                            return false;
                        }
                    }
                    if let Some(t) = request.suppression_type {
                        if entry.suppression_type != t {
                            return false;
                        }
                    }
                    if let Some(s) = request.source {
                        if entry.source != s {
                            return false;
                        }
                    }
                    true
                })
                .collect();

            results.sort_by(|a, b| b.created.cmp(&a.created));
            results.truncate(limit);

            lua.to_value(&SuppressionListResponse {
                results,
                total_count: get_store().len(),
                next_cursor: None,
            })
        })?,
    )?;

    suppression_mod.set(
        "summary",
        lua.create_function(|lua, ()| {
            let entries = get_store().entries();
            let total = entries.len();

            let mut by_type: HashMap<String, usize> = HashMap::new();
            let mut by_source: HashMap<String, usize> = HashMap::new();

            for (_, entry) in entries {
                *by_type
                    .entry(entry.suppression_type.to_string())
                    .or_insert(0) += 1;
                *by_source.entry(entry.source.to_string()).or_insert(0) += 1;
            }

            lua.to_value(&SuppressionSummaryResponse {
                total,
                by_type,
                by_source,
            })
        })?,
    )?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suppression_key_case_insensitive() {
        let key1 = SuppressionKey::new("Test@Example.com", SuppressionType::Transactional, None);
        let key2 = SuppressionKey::new("test@example.com", SuppressionType::Transactional, None);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_suppression_key_different_types() {
        let key1 = SuppressionKey::new("test@example.com", SuppressionType::Transactional, None);
        let key2 = SuppressionKey::new("test@example.com", SuppressionType::NonTransactional, None);
        assert_ne!(key1, key2, "Different types should create different keys");
    }

    #[test]
    fn test_suppression_key_with_subaccount() {
        let key1 = SuppressionKey::new(
            "test@example.com",
            SuppressionType::Transactional,
            Some("acct1"),
        );
        let key2 = SuppressionKey::new(
            "test@example.com",
            SuppressionType::Transactional,
            Some("acct2"),
        );
        let key3 = SuppressionKey::new("test@example.com", SuppressionType::Transactional, None);
        assert_ne!(
            key1, key2,
            "Different subaccounts should create different keys"
        );
        assert_ne!(key1, key3, "With/without subaccount should be different");
    }

    #[test]
    fn test_suppression_key_serialization() {
        let key = SuppressionKey::new(
            "test@example.com",
            SuppressionType::Transactional,
            Some("acct1"),
        );
        let bytes = key.to_bytes();
        let parsed = SuppressionKey::from_bytes(&bytes).unwrap();
        assert_eq!(key, parsed);
    }

    #[test]
    fn test_suppression_type_display() {
        assert_eq!(SuppressionType::Transactional.to_string(), "transactional");
        assert_eq!(
            SuppressionType::NonTransactional.to_string(),
            "non_transactional"
        );
    }

    #[test]
    fn test_suppression_source_display() {
        assert_eq!(SuppressionSource::Manual.to_string(), "manual");
        assert_eq!(SuppressionSource::Bounce.to_string(), "bounce");
        assert_eq!(SuppressionSource::Complaint.to_string(), "complaint");
        assert_eq!(
            SuppressionSource::ListUnsubscribe.to_string(),
            "list_unsubscribe"
        );
        assert_eq!(
            SuppressionSource::LinkUnsubscribe.to_string(),
            "link_unsubscribe"
        );
    }

    #[test]
    fn test_in_memory_store_crud() {
        let store = InMemorySuppressionStore::new();

        let key = SuppressionKey::new("test@example.com", SuppressionType::Transactional, None);
        let entry = SuppressionEntry {
            recipient: "test@example.com".to_string(),
            suppression_type: SuppressionType::Transactional,
            source: SuppressionSource::Manual,
            description: Some("Test".to_string()),
            subaccount_id: None,
            created: Utc::now(),
            updated: Utc::now(),
        };

        // Insert
        assert!(!store.upsert(key.clone(), entry.clone()));
        assert_eq!(store.len(), 1);

        // Get
        assert!(store.get(&key).is_some());

        // Update
        assert!(store.upsert(key.clone(), entry));
        assert_eq!(store.len(), 1);

        // Remove
        assert!(store.remove(&key));
        assert_eq!(store.len(), 0);
        assert!(!store.remove(&key));
    }

    #[test]
    fn test_in_memory_store_thread_safety() {
        use std::thread;

        let store = Arc::new(InMemorySuppressionStore::new());

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let store = store.clone();
                thread::spawn(move || {
                    let key = SuppressionKey::new(
                        &format!("thread{}@test.com", i),
                        SuppressionType::Transactional,
                        None,
                    );
                    let entry = SuppressionEntry {
                        recipient: format!("thread{}@test.com", i),
                        suppression_type: SuppressionType::Transactional,
                        source: SuppressionSource::Manual,
                        description: None,
                        subaccount_id: None,
                        created: Utc::now(),
                        updated: Utc::now(),
                    };
                    store.upsert(key, entry);
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(store.len(), 10);
    }

    #[test]
    fn test_suppression_email_edge_cases() {
        let emails = vec![
            "simple@example.com",
            "very.common@example.com",
            "disposable.style.email.with+symbol@example.com",
            "other.email-with-hyphen@example.com",
            "x@example.com",
        ];

        for email in emails {
            let key = SuppressionKey::new(email, SuppressionType::Transactional, None);
            assert_eq!(
                key.recipient,
                email.to_lowercase(),
                "Email should be lowercased"
            );
        }
    }

    #[test]
    fn test_rocksdb_store_crud() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = RocksDBSuppressionStoreConfig {
            path: temp_dir.path().join("suppression_test_db"),
            flush: false,
            compression: CompressionType::None,
        };

        let store = RocksDBSuppressionStore::new(config).unwrap();

        let key = SuppressionKey::new("test@example.com", SuppressionType::Transactional, None);
        let entry = SuppressionEntry {
            recipient: "test@example.com".to_string(),
            suppression_type: SuppressionType::Transactional,
            source: SuppressionSource::Manual,
            description: Some("RocksDB Test".to_string()),
            subaccount_id: None,
            created: Utc::now(),
            updated: Utc::now(),
        };

        // Insert
        assert!(!store.upsert(key.clone(), entry.clone()));
        assert_eq!(store.len(), 1);

        // Get
        let retrieved = store.get(&key);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().recipient, "test@example.com");

        // Update (should return true since key exists)
        assert!(store.upsert(key.clone(), entry));
        assert_eq!(store.len(), 1);

        // Contains
        assert!(store.contains(&key));

        // Remove
        assert!(store.remove(&key));
        assert_eq!(store.len(), 0);
        assert!(!store.contains(&key));
        assert!(!store.remove(&key));
    }

    #[test]
    fn test_rocksdb_store_persistence() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("suppression_persist_db");

        let key = SuppressionKey::new(
            "persist@example.com",
            SuppressionType::NonTransactional,
            None,
        );
        let entry = SuppressionEntry {
            recipient: "persist@example.com".to_string(),
            suppression_type: SuppressionType::NonTransactional,
            source: SuppressionSource::Bounce,
            description: Some("Persistence Test".to_string()),
            subaccount_id: None,
            created: Utc::now(),
            updated: Utc::now(),
        };

        // Create store and insert data
        {
            let config = RocksDBSuppressionStoreConfig {
                path: db_path.clone(),
                flush: true,
                compression: CompressionType::None,
            };
            let store = RocksDBSuppressionStore::new(config).unwrap();
            store.upsert(key.clone(), entry.clone());
            assert_eq!(store.len(), 1);
        }
        // Store is dropped here, simulating shutdown

        // Re-open the store and verify data persisted
        {
            let config = RocksDBSuppressionStoreConfig {
                path: db_path,
                flush: true,
                compression: CompressionType::None,
            };
            let store = RocksDBSuppressionStore::new(config).unwrap();
            assert_eq!(store.len(), 1);

            let retrieved = store.get(&key);
            assert!(retrieved.is_some());
            let retrieved = retrieved.unwrap();
            assert_eq!(retrieved.recipient, "persist@example.com");
            assert_eq!(
                retrieved.suppression_type,
                SuppressionType::NonTransactional
            );
            assert_eq!(retrieved.source, SuppressionSource::Bounce);
        }
    }

    #[test]
    fn test_rocksdb_store_entries_for_recipient() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = RocksDBSuppressionStoreConfig {
            path: temp_dir.path().join("suppression_recipient_db"),
            flush: false,
            compression: CompressionType::None,
        };

        let store = RocksDBSuppressionStore::new(config).unwrap();

        // Add two entries for same recipient, different types
        let entry1 = SuppressionEntry {
            recipient: "multi@example.com".to_string(),
            suppression_type: SuppressionType::Transactional,
            source: SuppressionSource::Manual,
            description: None,
            subaccount_id: None,
            created: Utc::now(),
            updated: Utc::now(),
        };
        let entry2 = SuppressionEntry {
            recipient: "multi@example.com".to_string(),
            suppression_type: SuppressionType::NonTransactional,
            source: SuppressionSource::Complaint,
            description: None,
            subaccount_id: None,
            created: Utc::now(),
            updated: Utc::now(),
        };

        let key1 = SuppressionKey::new("multi@example.com", SuppressionType::Transactional, None);
        let key2 =
            SuppressionKey::new("multi@example.com", SuppressionType::NonTransactional, None);

        store.upsert(key1, entry1);
        store.upsert(key2, entry2);

        let entries = store.entries_for_recipient("multi@example.com");
        assert_eq!(entries.len(), 2);

        // Test case insensitivity
        let entries = store.entries_for_recipient("MULTI@EXAMPLE.COM");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_rocksdb_store_clear() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = RocksDBSuppressionStoreConfig {
            path: temp_dir.path().join("suppression_clear_db"),
            flush: false,
            compression: CompressionType::None,
        };

        let store = RocksDBSuppressionStore::new(config).unwrap();

        // Add multiple entries
        for i in 0..5 {
            let key = SuppressionKey::new(
                &format!("clear{}@example.com", i),
                SuppressionType::Transactional,
                None,
            );
            let entry = SuppressionEntry {
                recipient: format!("clear{}@example.com", i),
                suppression_type: SuppressionType::Transactional,
                source: SuppressionSource::Manual,
                description: None,
                subaccount_id: None,
                created: Utc::now(),
                updated: Utc::now(),
            };
            store.upsert(key, entry);
        }

        assert_eq!(store.len(), 5);

        store.clear();

        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
    }
}
