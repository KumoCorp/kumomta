use crate::http_server::auth::HttpEndpointResource;
use async_trait::async_trait;
use axum::http::Uri;
use cidr_map::CidrSet;
use config::{
    any_err, get_or_create_sub_module, load_config, serialize_options, SerdeWrappedValue,
};
use data_loader::KeySource;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, UserDataFields, UserDataMethods, UserDataRef};
use mod_memoize::Memoized;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

static ACL_MAP: LazyLock<AccessControlListMap> =
    LazyLock::new(|| AccessControlListMap::compiled_default());
static FALL_BACK_TO_ACL_MAP: AtomicBool = AtomicBool::new(true);

lruttl::declare_cache! {
static ACL_CACHE: LruCacheWithTtl<String, AccessControlListCacheEntry>::new("acl_definition", 128);
}
static ACL_CACHE_TTL_MS: AtomicUsize = AtomicUsize::new(300);

lruttl::declare_cache! {
static CHECK_CACHE: LruCacheWithTtl<CheckKey, AuditRecord>::new("acl_check", 128);
}
static CHECK_CACHE_TTL_MS: AtomicUsize = AtomicUsize::new(300);

config::declare_event! {
static GET_ACL_DEF_SIG: Multiple(
    "get_acl_definition",
    resource: &'static str
) -> Option<UserDataRef<AccessControlListWrap>>;
}

// TODO: AAA log for audit/debugging purposes

// TODO:
//  * Formalize object mapping. For example, in kumod we should map
//    scheduled queues to some kind of object hierarchy. The default should
//    be something like:
//       scheduled_queues (as a container for all scheduled queues)
//          tenant_<tenant>_queues (as a container for all of a tenant's queues)
//             tenant_<tenant>_<campaign>_queues (contains each of the tenant's campaign queues
//                <queue_name> - a queue inside a tenant-campaign
//             <queue_name> - a queue inside a tenant but not a campaign
//          campaign_<campaign>_queues (contains queues that have campaign but not tenant set)
//             <queue_name> - a queue inside a campaign but not a tenant
//          <queue_name> - a queue that has no tenant nor campaign
//
//    with those resources mapped out, we can then define ACLs with hierarchy.
//    eg: `mailops` group has `queue:flush`, `queue:inspect`, `queue:summarize`
//    privs set to allow on `scheduled_queues` at the top, granting those privs
//    to all queues
//
//    `customer_xyz` group has `queue:summarize` and `queue:relay` set on
//    `tenant_xyz_queues`, allowing those privs to just their tenant.
//
//    Need to sit down and think about what specific privileges these are
//    for the different sorts of objects, because those will need to be encoded
//    in the rust logic in a number of cases.  eg: in the smtp server, we'll
//    need to check any privs that we define that control assignment prior to
//    enqueuing the message.
//
//  * Some customers piggy-back on our tenant and/or campaign identifies to further
//    sub-divide the queue space, so we need to allow them a way to influence
//    that object mapping.  eg: one encodes `pool_tenant` in the campaign identifier.
//    Is that neutral wrt. access control?
//
//  * Sanity check that we can apply this model to the things that matter:
//     * mailops doing queue flushing, bouncing, inspecting
//     * mailops summarizing queue stats
//     * smtp client being able to inject mail at all
//     * smtp client being able to relay to a specific destination. OR: tenant
//       being allowed to relay to a specific destination.
//     * smtp client being able to assign a specific tenant
//     * http client being able to use injection API
//     * smtp/http client being able to set a specific meta item
//     * smtp/http client OR tenant having access to a dkim/arc key for signing purposes
//     * machine being allowed to do message transfer
//     * socks proxy client being allowed to use a specific source address
//     * socks proxy client being allowed to connect to a specific destination address

#[derive(Debug, PartialEq, Eq, Clone, Hash, Serialize, Deserialize)]
pub struct Identity {
    pub identity: String,
    pub context: IdentityContext,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, Serialize, Deserialize)]
pub enum IdentityContext {
    SmtpAuthPlainAuthentication,
    SmtpAuthPlainAuthorization,
    HttpBasicAuth,
    BearerToken,
    ProxyAuthRfc1929,
    LocalSystem,
    GenericAuth,
}

#[derive(Default, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthInfo {
    /// The peer machine, if known
    #[serde(default)]
    pub peer_address: Option<IpAddr>,
    /// Authenticated identities
    #[serde(default)]
    pub identities: Vec<Identity>,
    /// Any groups to which we might belong
    #[serde(default)]
    pub groups: Vec<String>,
}

impl std::fmt::Display for AuthInfo {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}", self.public_descriptor())
    }
}

impl AuthInfo {
    /// Authentication that represents the local system taking action.
    /// It should be considered to be the most privileged identity
    pub fn new_local_system() -> Self {
        Self {
            peer_address: None,
            identities: vec![Identity {
                identity: "kumomta.internal".to_string(),
                context: IdentityContext::LocalSystem,
            }],
            groups: vec![],
        }
    }

    /// Summarize the info in a form that should be reasonable to report
    /// back to a connected peer. This form doesn't include any groups
    /// that may have been added by the server and/or via policy
    pub fn public_descriptor(&self) -> String {
        let mut result = String::new();

        if self.identities.is_empty() {
            result.push_str("<Unauthenticated>");
        } else {
            result.push_str("id=[");
            for (idx, id) in self.identities.iter().enumerate() {
                if idx > 0 {
                    result.push_str(", ");
                }
                result.push_str(&id.identity);
            }
            result.push(']');
        }
        if let Some(peer) = &self.peer_address {
            result.push_str(&format!("@ ip={peer}"));
        }
        result
    }

    /// Merges groups and identities from other, ignoring its peer_address
    pub fn merge_from(&mut self, mut other: Self) {
        self.identities.append(&mut other.identities);
        self.groups.append(&mut other.groups);
    }

    /// Add an identity to the list
    pub fn add_identity(&mut self, identity: Identity) {
        self.identities.push(identity);
    }

    pub fn add_group(&mut self, group_name: impl Into<String>) {
        self.groups.push(group_name.into());
    }

    /// Set the peer address, ignoring and replacing any prior value
    pub fn set_peer_address(&mut self, peer_address: Option<IpAddr>) {
        self.peer_address = peer_address;
    }

    /// Test to see if this AuthInfo matches a specific identity from an ACL entry
    pub fn matches_identity(&self, identity: &ACLIdentity) -> bool {
        match identity {
            ACLIdentity::Any => true,
            ACLIdentity::Unauthenticated => self.identities.is_empty(),
            ACLIdentity::Authenticated => !self.identities.is_empty(),
            ACLIdentity::Individual(candidate_ident) => {
                for ident in &self.identities {
                    if ident.identity == *candidate_ident {
                        return true;
                    }
                }
                false
            }
            ACLIdentity::Group(candidate_group) => self.groups.iter().any(|g| g == candidate_group),
            ACLIdentity::Machine(candidate_ip) => self
                .peer_address
                .as_ref()
                .map(|ip| ip == candidate_ip)
                .unwrap_or(false),
            ACLIdentity::MachineSet(cidr_set) => self
                .peer_address
                .as_ref()
                .map(|ip| cidr_set.contains(*ip))
                .unwrap_or(false),
        }
    }

    pub fn matches_criteria(&self, criteria: &ACLRuleTerm) -> bool {
        match criteria {
            ACLRuleTerm::Not(term) => !self.matches_criteria(term),
            ACLRuleTerm::AnyOf(terms) => terms.iter().any(|term| self.matches_criteria(term)),
            ACLRuleTerm::AllOf(terms) => terms.iter().all(|term| self.matches_criteria(term)),
            ACLRuleTerm::Identity(ident) => self.matches_identity(ident),
        }
    }
}

#[derive(Clone, Debug)]
enum AccessControlListCacheEntry {
    None,
    Hit(Arc<AccessControlList>),
    Err(String),
}

#[derive(Hash, PartialEq, Eq, Debug, Clone)]
struct CheckKey {
    pub target_resource: String,
    pub privilege: String,
    pub auth_info: AuthInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditRecord {
    /// The resource being accessed
    pub target_resource: String,
    /// The privilege requested
    pub privilege: String,
    /// Was access granted or denied?
    pub access: Access,
    /// The resource whose ACL provided the decision.
    /// If None, no rules matched and the decision was to deny by default.
    pub matching_resource: Option<String>,
    /// A structured representation of the matching ACL rule
    pub rule: Option<AccessControlRule>,
    /// A copy of the authentication information used to make the decision
    pub auth_info: AuthInfo,
    /// A list of the resource ids that were considered prior to reaching
    /// this final disposition; these will be on the inheritance path
    /// between the target_resource and the matching_resource (or the root,
    /// if there was no matching_resource).
    pub considered_resources: Vec<String>,
}

impl AuditRecord {
    pub fn new(
        disposition: &ACLQueryDisposition,
        target_resource: &str,
        privilege: &str,
        info: &AuthInfo,
        considered_resources: Vec<String>,
    ) -> Self {
        let (access, matching_resource, rule) = match disposition {
            ACLQueryDisposition::Allow { rule, resource } => (
                Access::Allow,
                Some(resource.to_string()),
                Some(rule.clone()),
            ),
            ACLQueryDisposition::Deny { rule, resource } => {
                (Access::Deny, Some(resource.to_string()), Some(rule.clone()))
            }
            ACLQueryDisposition::DenyByDefault => (Access::Deny, None, None),
        };

        Self {
            target_resource: target_resource.to_string(),
            privilege: privilege.to_string(),
            access,
            matching_resource,
            rule,
            auth_info: info.clone(),
            considered_resources,
        }
    }

    pub fn log(&self) {
        // FIXME: formalize AAA audit log destination
        tracing::info!("Audit: {self:?}");
    }

    pub fn disposition(&self) -> ACLQueryDisposition {
        match (&self.matching_resource, &self.rule, self.access) {
            (None, None, Access::Deny) => ACLQueryDisposition::DenyByDefault,
            (Some(resource), Some(rule), Access::Allow) => ACLQueryDisposition::Allow {
                rule: rule.clone(),
                resource: resource.clone(),
            },
            (Some(resource), Some(rule), Access::Deny) => ACLQueryDisposition::Deny {
                rule: rule.clone(),
                resource: resource.clone(),
            },
            _ => ACLQueryDisposition::DenyByDefault,
        }
    }
}

#[derive(Clone)]
struct AccessControlListWrap {
    wrapped_acl: Arc<AccessControlList>,
}
impl LuaUserData for AccessControlListWrap {}

/// An access control list for a specific resource
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessControlList {
    /// An ordered set of rules; the first matching rule
    /// defines the allowed access level.  If we run
    /// out of rules, then the caller should proceed to evaluate the
    /// ACL for the parent resource, if any.
    pub rules: Vec<AccessControlRule>,
}

impl AccessControlList {
    pub async fn query_resource_access(
        resource: &mut impl Resource,
        info: &AuthInfo,
        privilege: &str,
    ) -> anyhow::Result<ACLQueryDisposition> {
        let key = CheckKey {
            auth_info: info.clone(),
            privilege: privilege.to_string(),
            target_resource: resource.resource_id().to_string(),
        };

        let lookup = CHECK_CACHE
            .get_or_try_insert(&key, |_| get_check_cache_ttl(), async move {
                let mut considered_resources = vec![];

                while let Some(resource_id) = resource.next_resource_id().await {
                    tracing::trace!("ACL: consider {resource_id} priv={privilege} {info:?}");
                    if let Some(acl) = Self::load_for_resource(&resource_id).await? {
                        let result = acl.query_access(resource_id.clone(), info, privilege);
                        tracing::trace!(
                            "ACL: check {resource_id} priv={privilege} {info:?} -> {result:?}"
                        );
                        if result != ACLQueryDisposition::DenyByDefault {
                            let record = AuditRecord::new(
                                &result,
                                resource.resource_id(),
                                privilege,
                                info,
                                considered_resources,
                            );

                            return Ok::<_, anyhow::Error>(record);
                        }
                    }

                    considered_resources.push(resource_id);
                }

                Ok(AuditRecord::new(
                    &ACLQueryDisposition::DenyByDefault,
                    resource.resource_id(),
                    privilege,
                    info,
                    considered_resources,
                ))
            })
            .await
            .map_err(|err| anyhow::anyhow!("{err:#}"))?;

        lookup.item.log();
        Ok(lookup.item.disposition())
    }

    fn query_access(
        &self,
        resource: String,
        info: &AuthInfo,
        privilege: &str,
    ) -> ACLQueryDisposition {
        for rule in &self.rules {
            if !info.matches_criteria(&rule.criteria) {
                continue;
            }
            if rule.privilege != privilege {
                continue;
            }
            return match &rule.access {
                Access::Allow => ACLQueryDisposition::Allow {
                    rule: rule.clone(),
                    resource,
                },
                Access::Deny => ACLQueryDisposition::Deny {
                    rule: rule.clone(),
                    resource,
                },
            };
        }

        ACLQueryDisposition::DenyByDefault
    }

    async fn load_for_resource_impl(resource: &str) -> anyhow::Result<Option<Arc<Self>>> {
        let mut config = load_config().await?;
        let result = config.call_callback(&GET_ACL_DEF_SIG, resource).await?;
        config.put();

        if let Some(Some(result)) = result.result {
            return Ok(Some(result.wrapped_acl.clone()));
        }

        if !result.handler_was_defined || FALL_BACK_TO_ACL_MAP.load(Ordering::Relaxed) {
            Ok(ACL_MAP.get(resource))
        } else {
            Ok(None)
        }
    }

    pub async fn load_for_resource(resource: &str) -> anyhow::Result<Option<Arc<Self>>> {
        let resource = resource.to_string();
        match ACL_CACHE
            .get_or_try_insert(&resource, |_| get_acl_cache_ttl(), {
                let resource = resource.clone();
                async move {
                    match Self::load_for_resource_impl(&resource).await {
                        Err(err) => Ok::<_, anyhow::Error>(AccessControlListCacheEntry::Err(
                            format!("{err:#}"),
                        )),
                        Ok(Some(result)) => Ok(AccessControlListCacheEntry::Hit(result)),
                        Ok(None) => Ok(AccessControlListCacheEntry::None),
                    }
                }
            })
            .await
        {
            Err(err) => Err(anyhow::anyhow!("{err:#}")),
            Ok(lookup) => match lookup.item {
                AccessControlListCacheEntry::Err(err) => Err(anyhow::anyhow!("{err}")),
                AccessControlListCacheEntry::None => Ok(None),
                AccessControlListCacheEntry::Hit(res) => Ok(Some(res)),
            },
        }
    }
}

pub fn get_acl_cache_ttl() -> Duration {
    Duration::from_millis(ACL_CACHE_TTL_MS.load(Ordering::Relaxed) as u64)
}
pub fn get_check_cache_ttl() -> Duration {
    Duration::from_millis(CHECK_CACHE_TTL_MS.load(Ordering::Relaxed) as u64)
}

#[derive(PartialEq, Debug)]
pub enum ACLQueryDisposition {
    /// An explicit allow rule matched
    Allow {
        resource: String,
        rule: AccessControlRule,
    },
    /// An explicit Deny rule matched
    Deny {
        resource: String,
        rule: AccessControlRule,
    },
    /// Exhausted the list of rules for this resource,
    /// so the behavior is to deny as a default.
    /// If there is a parent resource, the caller should
    /// proceed to query that one.
    DenyByDefault,
}

impl LuaUserData for ACLQueryDisposition {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("allow", move |_lua, this| {
            Ok(matches!(this, Self::Allow { .. }))
        });
        fields.add_field_method_get("rule", move |lua, this| match this {
            Self::Allow { rule, .. } | Self::Deny { rule, .. } => {
                lua.to_value_with(rule, serialize_options())
            }
            Self::DenyByDefault => Ok(mlua::Value::Nil),
        });
        fields.add_field_method_get("resource", move |_lua, this| match this {
            Self::Allow { resource, .. } | Self::Deny { resource, .. } => {
                Ok(Some(resource.to_string()))
            }
            Self::DenyByDefault => Ok(None),
        });
    }
}

#[derive(Debug, PartialEq, Clone, Deserialize, Serialize)]
pub enum ACLIdentity {
    /// An individual identity
    Individual(String),
    /// A Group
    Group(String),
    /// A machine
    Machine(IpAddr),
    /// A set of machines
    MachineSet(CidrSet),
    /// wildcard to match any explicitly authenticated individual
    Authenticated,
    /// wildcard to match any unauthenticated session
    Unauthenticated,
    /// wildcard to match any thing
    Any,
}

impl std::fmt::Display for ACLIdentity {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Authenticated => write!(fmt, "Authenticated"),
            Self::Unauthenticated => write!(fmt, "Unauthenticated"),
            Self::Any => write!(fmt, "Any"),
            Self::Individual(user) => write!(fmt, "{user}"),
            Self::Group(group) => write!(fmt, "group={group}"),
            Self::Machine(ip) => write!(fmt, "ip={ip}"),
            Self::MachineSet(cidr) => write!(fmt, "ip={cidr:?}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, Serialize, Deserialize)]
pub enum Access {
    Allow,
    Deny,
}

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum ACLRuleTerm {
    AllOf(Vec<ACLRuleTerm>),
    AnyOf(Vec<ACLRuleTerm>),
    Not(Box<ACLRuleTerm>),
    Identity(ACLIdentity),
}

impl std::fmt::Display for ACLRuleTerm {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::AllOf(terms) => {
                write!(fmt, "AllOf [")?;
                for (idx, t) in terms.iter().enumerate() {
                    if idx > 0 {
                        write!(fmt, ", ")?;
                    }
                    t.fmt(fmt)?;
                }
                write!(fmt, "]")
            }
            Self::AnyOf(terms) => {
                write!(fmt, "AnyOf [")?;
                for (idx, t) in terms.iter().enumerate() {
                    if idx > 0 {
                        write!(fmt, ", ")?;
                    }
                    t.fmt(fmt)?;
                }
                write!(fmt, "]")
            }
            Self::Not(term) => {
                write!(fmt, "!{term}")
            }
            Self::Identity(ident) => {
                write!(fmt, "{ident}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccessControlRule {
    pub criteria: ACLRuleTerm,
    pub privilege: String,
    pub access: Access,
}

impl std::fmt::Display for AccessControlRule {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            fmt,
            "{} {} {}",
            self.criteria,
            match self.access {
                Access::Allow => "allows",
                Access::Deny => "denies",
            },
            self.privilege
        )
    }
}

/// This trait represents a Resource against which we'd like
/// to perform an access control check.
/// Each call to next_resource_id() will produce the identifier
/// of the resource which is being checked, following its
/// child -> parent relationship.
///
/// So a hypothetical filesystem path resource of `/foo/bar/baz`
/// would return on each successive call:
///
/// `Some("/foo/bar/baz")`
/// `Some("/foo/bar")`
/// `Some("/foo")`
/// `Some("/")`
/// `None`
///
/// so that the ACL checker knows how to resolve the resources
/// up to their containing root.
///
/// This trait is essentially an asynchronous iterator.
#[async_trait]
pub trait Resource {
    /// Returns the resource to which access is desired
    fn resource_id(&self) -> &str;
    /// Returns the next resource in the inheritance hierarchy;
    /// starts with the targeted resource_id and walks through
    /// its parents.
    async fn next_resource_id(&mut self) -> Option<String>;
}

#[derive(Clone)]
struct AccessControlListMapWrap {
    wrapped_map: Arc<AccessControlListMap>,
}

impl LuaUserData for AccessControlListMapWrap {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", move |_lua, this, resource: String| {
            Ok(this
                .wrapped_map
                .get(&resource)
                .map(|acl| AccessControlListWrap { wrapped_acl: acl }))
        });

        Memoized::impl_memoize(methods);
    }
}

#[derive(Default)]
pub struct AccessControlListMap {
    map: HashMap<String, Arc<AccessControlList>>,
}

impl AccessControlListMap {
    pub fn get(&self, resource: &str) -> Option<Arc<AccessControlList>> {
        self.map.get(resource).cloned()
    }

    pub fn compiled_default() -> Self {
        let acl_file: ACLFile = toml::from_str(include_str!("../../../assets/acls/default.toml"))
            .expect("default ACL is invalid!");
        acl_file.build_map()
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
struct ACLFile {
    acl: HashMap<String, Vec<ACLFileEntry>>,
}

impl ACLFile {
    fn build_map(self) -> AccessControlListMap {
        let mut map = HashMap::new();

        for (resource_id, entries) in self.acl {
            let mut rules = vec![];
            for entry in entries {
                let access = if entry.allow {
                    Access::Allow
                } else {
                    Access::Deny
                };
                for privilege in entry.privileges {
                    match &entry.condition {
                        ACLFileCondition::Identity(identity) => {
                            rules.push(AccessControlRule {
                                criteria: ACLRuleTerm::Identity(identity.clone()),
                                privilege,
                                access,
                            });
                        }
                        ACLFileCondition::Criteria(term) => {
                            rules.push(AccessControlRule {
                                criteria: term.clone(),
                                privilege,
                                access,
                            });
                        }
                    }
                }
            }
            map.insert(resource_id, Arc::new(AccessControlList { rules }));
        }

        AccessControlListMap { map }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
struct ACLFileEntry {
    allow: bool,
    privileges: Vec<String>,
    #[serde(flatten)]
    condition: ACLFileCondition,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
enum ACLFileCondition {
    #[serde(rename = "identity")]
    Identity(ACLIdentity),
    #[serde(rename = "criteria")]
    Criteria(ACLRuleTerm),
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let aaa_mod = get_or_create_sub_module(lua, "aaa")?;

    aaa_mod.set(
        "set_acl_cache_ttl",
        lua.create_function(move |lua, duration: mlua::Value| {
            let duration: duration_serde::Wrap<Duration> = lua.from_value(duration)?;
            ACL_CACHE_TTL_MS.store(
                duration.into_inner().as_millis().try_into().map_err(|_| {
                    mlua::Error::external("set_acl_cache_ttl: duration is too large")
                })?,
                Ordering::Relaxed,
            );
            Ok(())
        })?,
    )?;
    aaa_mod.set(
        "set_check_cache_ttl",
        lua.create_function(move |lua, duration: mlua::Value| {
            let duration: duration_serde::Wrap<Duration> = lua.from_value(duration)?;
            CHECK_CACHE_TTL_MS.store(
                duration.into_inner().as_millis().try_into().map_err(|_| {
                    mlua::Error::external("set_check_cache_ttl: duration is too large")
                })?,
                Ordering::Relaxed,
            );
            Ok(())
        })?,
    )?;
    aaa_mod.set(
        "make_access_control_list",
        lua.create_function(move |lua, rules: mlua::Value| {
            let rules: Vec<AccessControlRule> = lua.from_value(rules)?;
            let list = AccessControlList { rules };
            Ok(AccessControlListWrap {
                wrapped_acl: Arc::new(list),
            })
        })?,
    )?;

    aaa_mod.set(
        "set_fall_back_to_acl_map",
        lua.create_function(move |_lua, fall_back: bool| {
            FALL_BACK_TO_ACL_MAP.store(fall_back, Ordering::Relaxed);
            Ok(())
        })?,
    )?;

    aaa_mod.set(
        "load_acl_map",
        lua.create_async_function(
            move |_lua, acl_file: SerdeWrappedValue<KeySource>| async move {
                let data = acl_file.get().await.map_err(any_err)?;
                let acl_file: ACLFile = toml::from_slice(&data).map_err(|err| {
                    mlua::Error::external(format!("failed to parse acl map data: {err}"))
                })?;
                let map = AccessControlListMapWrap {
                    wrapped_map: Arc::new(acl_file.build_map()),
                };
                Ok(map)
            },
        )?,
    )?;

    #[derive(Clone)]
    enum ResourceWrap {
        HttpEndpoint(HttpEndpointResource),
    }
    impl LuaUserData for ResourceWrap {}
    #[async_trait]
    impl Resource for ResourceWrap {
        fn resource_id(&self) -> &str {
            match self {
                Self::HttpEndpoint(r) => r.resource_id(),
            }
        }
        async fn next_resource_id(&mut self) -> Option<String> {
            match self {
                Self::HttpEndpoint(r) => r.next_resource_id().await,
            }
        }
    }

    aaa_mod.set(
        "make_http_url_resource",
        lua.create_function(move |_lua, (local_addr, url): (String, String)| {
            let local_addr: std::net::SocketAddr = local_addr.parse().map_err(any_err)?;
            let url: Uri = url.parse().map_err(any_err)?;
            let resource = HttpEndpointResource::new(local_addr, &url).map_err(any_err)?;
            Ok(ResourceWrap::HttpEndpoint(resource))
        })?,
    )?;

    aaa_mod.set(
        "query_resource_access",
        lua.create_async_function(
            move |_lua,
                  (resource, info, privilege): (
                UserDataRef<ResourceWrap>,
                SerdeWrappedValue<AuthInfo>,
                String,
            )| async move {
                let mut resource = resource.clone();
                AccessControlList::query_resource_access(&mut resource, &info, &privilege)
                    .await
                    .map_err(any_err)
            },
        )?,
    )?;

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn verify_compiled_in_acl_compiles() {
        LazyLock::force(&ACL_MAP);
    }

    #[test]
    fn acl_file_syntax_critiera() {
        let _: ACLFile = toml::from_str(
            r#"
[[acl."http_listener"]]
allow = true
privileges = ["GET"]
[[acl."http_listener".criteria.AllOf]]
Identity.Authenticated = {}
[[acl."http_listener".criteria.AllOf]]
Identity.Machine = "10.0.0.1"

# Alternative criteria syntax
[[acl."http_listener/*/api/inject"]]
allow = true
privileges = ["POST"]
criteria.AllOf = [{Identity={Individual = "hunter"}}, {Identity={Machine = "10.0.0.1"}}]
        "#,
        )
        .unwrap();
    }

    #[test]
    fn acl_file_syntax() {
        let acl_file: ACLFile = toml::from_str(
            r#"
[[acl."http_listener"]]
allow = true
privileges = ["GET", "PUT"]
identity.Individual = "user"

[[acl."http_listener"]]
allow = false
privileges = ["POST"]
identity.Group = "group"

[[acl."http_listener"]]
allow = true
privileges = ["GET"]
identity.Machine = "10.0.0.1"

[[acl."http_listener"]]
allow = true
privileges = ["PUT"]
identity.MachineSet = ["10.0.0.1", "1.2.3.4"]

[[acl."http_listener"]]
allow = true
privileges = ["GET"]
identity.Unauthenticated = {}
        "#,
        )
        .unwrap();

        let ordered = acl_file
            .build_map()
            .map
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        k9::snapshot!(
            ordered,
            r#"
{
    "http_listener": AccessControlList {
        rules: [
            AccessControlRule {
                criteria: Identity(
                    Individual(
                        "user",
                    ),
                ),
                privilege: "GET",
                access: Allow,
            },
            AccessControlRule {
                criteria: Identity(
                    Individual(
                        "user",
                    ),
                ),
                privilege: "PUT",
                access: Allow,
            },
            AccessControlRule {
                criteria: Identity(
                    Group(
                        "group",
                    ),
                ),
                privilege: "POST",
                access: Deny,
            },
            AccessControlRule {
                criteria: Identity(
                    Machine(
                        10.0.0.1,
                    ),
                ),
                privilege: "GET",
                access: Allow,
            },
            AccessControlRule {
                criteria: Identity(
                    MachineSet(
                        {
                            "1.2.3.4",
                            "10.0.0.1",
                        },
                    ),
                ),
                privilege: "PUT",
                access: Allow,
            },
            AccessControlRule {
                criteria: Identity(
                    Unauthenticated,
                ),
                privilege: "GET",
                access: Allow,
            },
        ],
    },
}
"#
        );
    }
}
