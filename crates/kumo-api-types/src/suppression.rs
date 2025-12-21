//! Suppression List API types
//!
//! This module defines the API types for managing email suppression lists.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToResponse, ToSchema};

/// The type of email that should be suppressed for this recipient.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuppressionType {
    /// Suppress transactional emails (order confirmations, password resets, etc.)
    Transactional,
    /// Suppress non-transactional/marketing emails
    NonTransactional,
}

impl std::fmt::Display for SuppressionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transactional => write!(f, "transactional"),
            Self::NonTransactional => write!(f, "non_transactional"),
        }
    }
}

impl std::str::FromStr for SuppressionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "transactional" => Ok(Self::Transactional),
            "non_transactional" | "non-transactional" | "nontransactional" => {
                Ok(Self::NonTransactional)
            }
            _ => Err(format!("invalid suppression type: {s}")),
        }
    }
}

/// The source/reason why this email address was added to the suppression list.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuppressionSource {
    /// Manually added by an administrator or API call
    Manual,
    /// Added due to a hard bounce
    Bounce,
    /// Added due to a spam complaint (FBL)
    Complaint,
    /// Added due to a list-unsubscribe request
    ListUnsubscribe,
    /// Added due to a link unsubscribe action
    LinkUnsubscribe,
}

impl std::fmt::Display for SuppressionSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Manual => write!(f, "manual"),
            Self::Bounce => write!(f, "bounce"),
            Self::Complaint => write!(f, "complaint"),
            Self::ListUnsubscribe => write!(f, "list_unsubscribe"),
            Self::LinkUnsubscribe => write!(f, "link_unsubscribe"),
        }
    }
}

impl std::str::FromStr for SuppressionSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "manual" => Ok(Self::Manual),
            "bounce" => Ok(Self::Bounce),
            "complaint" | "spam_complaint" => Ok(Self::Complaint),
            "list_unsubscribe" | "list-unsubscribe" => Ok(Self::ListUnsubscribe),
            "link_unsubscribe" | "link-unsubscribe" => Ok(Self::LinkUnsubscribe),
            _ => Err(format!("invalid suppression source: {s}")),
        }
    }
}

/// A suppression list entry representing an email address that should not
/// receive certain types of emails.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionEntry {
    /// The email address that is suppressed
    #[schema(example = "user@example.com")]
    pub recipient: String,

    /// The type of email suppression
    #[serde(rename = "type")]
    pub suppression_type: SuppressionType,

    /// How the entry was added to the suppression list
    pub source: SuppressionSource,

    /// Optional description or reason for the suppression
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// When this entry was created
    pub created: DateTime<Utc>,

    /// When this entry was last updated
    pub updated: DateTime<Utc>,

    /// Optional tenant/subaccount identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subaccount_id: Option<String>,
}

/// Request to create or update a suppression entry.
///
/// When creating a new entry, if the email already exists with the same type
/// and subaccount, the entry will be updated instead.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionCreateRequest {
    /// The email address to suppress
    #[schema(example = "user@example.com")]
    pub recipient: String,

    /// The type of email suppression
    #[serde(rename = "type")]
    pub suppression_type: SuppressionType,

    /// How the entry was added (defaults to Manual for API calls)
    #[serde(default)]
    pub source: Option<SuppressionSource>,

    /// Optional description or reason for the suppression
    #[serde(default)]
    #[schema(example = "User requested to stop receiving marketing emails")]
    pub description: Option<String>,

    /// Optional tenant/subaccount identifier
    #[serde(default)]
    pub subaccount_id: Option<String>,
}

/// Request to create or update multiple suppression entries in bulk.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionBulkCreateRequest {
    /// List of suppression entries to create or update
    pub recipients: Vec<SuppressionCreateRequest>,
}

/// Response from creating suppression entries.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionCreateResponse {
    /// Number of entries successfully created
    pub created: usize,
    /// Number of entries that were updated (already existed)
    pub updated: usize,
    /// Any errors that occurred during processing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<SuppressionError>,
}

/// An error that occurred while processing a suppression entry.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SuppressionError {
    /// The recipient that caused the error
    pub recipient: String,
    /// The error message
    pub message: String,
}

/// Request to delete a suppression entry.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionDeleteRequest {
    /// The email address to remove from suppression
    #[schema(example = "user@example.com")]
    pub recipient: String,

    /// The type of suppression to remove. If not specified, removes all types.
    #[serde(default, rename = "type")]
    pub suppression_type: Option<SuppressionType>,

    /// Optional tenant/subaccount identifier
    #[serde(default)]
    pub subaccount_id: Option<String>,
}

/// Request to delete multiple suppression entries in bulk.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionBulkDeleteRequest {
    /// List of recipients to remove from suppression
    pub recipients: Vec<SuppressionDeleteRequest>,
}

/// Response from deleting suppression entries.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionDeleteResponse {
    /// Number of entries successfully deleted
    pub deleted: usize,
    /// Any errors that occurred during processing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<SuppressionError>,
}

/// Query parameters for listing or searching suppression entries.
#[derive(Serialize, Deserialize, Debug, Clone, Default, IntoParams, ToSchema)]
pub struct SuppressionListRequest {
    /// Filter by email address (partial match supported)
    #[serde(default)]
    pub recipient: Option<String>,

    /// Filter by suppression type
    #[serde(default, rename = "type")]
    pub suppression_type: Option<SuppressionType>,

    /// Filter by source
    #[serde(default)]
    pub source: Option<SuppressionSource>,

    /// Filter by subaccount
    #[serde(default)]
    pub subaccount_id: Option<String>,

    /// Filter by entries created after this timestamp
    #[serde(default)]
    pub from: Option<DateTime<Utc>>,

    /// Filter by entries created before this timestamp
    #[serde(default)]
    pub to: Option<DateTime<Utc>>,

    /// Maximum number of results to return (default: 1000, max: 10000)
    #[serde(default)]
    pub limit: Option<usize>,

    /// Cursor for pagination (returned from previous request)
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Response from listing suppression entries.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionListResponse {
    /// The suppression entries matching the query
    pub results: Vec<SuppressionEntry>,
    /// Total count of matching entries (may be approximate for large sets)
    pub total_count: usize,
    /// Cursor for fetching the next page, if more results exist
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Request to check if recipients are suppressed.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct SuppressionCheckRequest {
    /// List of email addresses to check
    pub recipients: Vec<String>,

    /// The type of email to check suppression for
    #[serde(default, rename = "type")]
    pub suppression_type: Option<SuppressionType>,

    /// Optional tenant/subaccount identifier
    #[serde(default)]
    pub subaccount_id: Option<String>,
}

/// Response from checking suppression status.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionCheckResponse {
    /// Map of recipient to their suppression status (true = suppressed)
    pub results: std::collections::HashMap<String, bool>,
}

/// Summary statistics for the suppression list.
#[derive(Serialize, Deserialize, Debug, Clone, ToSchema, ToResponse)]
pub struct SuppressionSummaryResponse {
    /// Total number of suppression entries
    pub total: usize,
    /// Count by suppression type
    pub by_type: std::collections::HashMap<String, usize>,
    /// Count by source
    pub by_source: std::collections::HashMap<String, usize>,
}
