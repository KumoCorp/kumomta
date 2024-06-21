use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{ToResponse, ToSchema};

/// Describes which messages should be rebound.
/// The criteria apply to the scheduled queue associated
/// with a given message.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct RebindV1Request {
    /// The campaign name to match. If omitted, any campaign will match.
    #[serde(default)]
    pub campaign: Option<String>,

    /// The tenant to match. If omitted, any tenant will match.
    #[serde(default)]
    pub tenant: Option<String>,

    /// The domain name to match. If omitted, any domain will match.
    #[serde(default)]
    #[schema(example = "example.com")]
    pub domain: Option<String>,

    /// The routing_domain name to match. If omitted, any routing_domain will match.
    #[serde(default)]
    pub routing_domain: Option<String>,

    /// Reason to log in the delivery log. Each matching message will log
    /// with an AdminRebind record unless you suppress logging.
    #[schema(example = "Cleaning up a bad send")]
    pub reason: String,

    /// If true, do not generate AdminRebind delivery logs for matching
    /// messages.
    #[serde(default)]
    pub suppress_logging: bool,

    /// The data, a json object with string keys AND values to pass to the
    /// rebind operation
    // Currently limited to String values due to inability to implicitly
    // convert serde_json::Value -> mlua::Value in the mlua bindings
    pub data: HashMap<String, String>,

    /// If true, a `rebind` event will be triggered and passed each
    /// message and the supplied data.
    /// If false, no event will be triggered and each field in data
    /// will be applied to the msg metadata, overwriting any previous
    /// value for that key.
    #[serde(default)]
    pub trigger_rebind_event: bool,

    /// If true, make all matched messages immediately eligible for
    /// delivery.  When false, (the default), only messages whose
    /// queue has changed will be made immediately eligible.
    #[serde(default)]
    pub always_flush: bool,
}

#[derive(Serialize, Deserialize, Debug, ToSchema, ToResponse)]
pub struct RebindV1Response {}
