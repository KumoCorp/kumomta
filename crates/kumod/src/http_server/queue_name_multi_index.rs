use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

/// QueueNameMultiIndexMap is a data structure that keeps track
/// of a type T that is uniquely identified by a Uuid, but for
/// which we want to maintain secondary indices for fields
/// derived from QueueNameComponents.
///
/// The intended purpose is to track the set of active bounce
/// or suspension rules defined by the administrator where a
/// given rule can match based on some number of criteria which
/// are defined by the `Criteria` struct in this module.
///
/// Additionally, Criteria is considered to be a unique index,
/// so defining a rule with the same criteria
/// will replace an existing rule with that criteria.
pub struct QueueNameMultiIndexMap<T: GetCriteria> {
    by_id: HashMap<Uuid, T>,
    by_criteria: HashMap<Criteria, Uuid>,
    // The `BounceCampaign` and `SuspendCampaign` types use this index
    by_domain_campaign_tenant: HashMap<DCT, UuidHashSet>,
    // The `BounceTenant` and `SuspendTenant` types use this index
    by_domain_tenant: HashMap<DT, UuidHashSet>,
    // The `Bounce` type uses this index
    by_domain: HashMap<String, UuidHashSet>,
    // catch all for other stuff
    other: UuidHashSet,
    match_all: Option<Uuid>,
    lookup_count: usize,
    generation_count: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CachedEntry<T> {
    pub entry: Option<T>,
    generation_count: usize,
}

/// Domain, Campaign, Tenant key into by_domain_campaign_tenant
#[derive(Eq, PartialEq, Hash)]
struct DCT {
    pub domain: String,
    pub campaign: String,
    pub tenant: String,
}

/// Domain, and Tenant key into by_domain_tenant
#[derive(Eq, PartialEq, Hash)]
struct DT {
    pub domain: String,
    pub tenant: String,
}

enum KeyType {
    FullCriteria,
    DCT(DCT),
    DT(DT),
    D(String),
    Other,
}

/// At the time of writing HashSet has no stable entry() API,
/// so we use a map with () as the value type.
type UuidHashSet = HashMap<Uuid, ()>;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Criteria {
    pub campaign: Option<String>,
    pub tenant: Option<String>,
    pub domain: Option<String>,
    pub routing_domain: Option<String>,
}

impl Criteria {
    /// Returns true if this Criteria matches anything/everything
    pub fn is_match_all(&self) -> bool {
        self.campaign.is_none()
            && self.tenant.is_none()
            && self.domain.is_none()
            && self.routing_domain.is_none()
    }

    /// Returns true if the supplied fields match this critiera
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

    #[cfg(test)]
    fn new_match_all() -> Self {
        Self {
            campaign: None,
            tenant: None,
            domain: None,
            routing_domain: None,
        }
    }

    /// Classify the criteria to the most appropriate key/index type
    fn key(&self) -> KeyType {
        match (
            &self.domain,
            &self.campaign,
            &self.tenant,
            &self.routing_domain,
        ) {
            (Some(_), Some(_), Some(_), Some(_)) => KeyType::FullCriteria,
            (Some(d), Some(c), Some(t), _) => KeyType::DCT(DCT {
                domain: d.to_string(),
                campaign: c.to_string(),
                tenant: t.to_string(),
            }),
            (Some(d), None, Some(t), _) => KeyType::DT(DT {
                domain: d.to_string(),
                tenant: t.to_string(),
            }),
            (Some(d), _, _, _) => KeyType::D(d.to_string()),
            _ => KeyType::Other,
        }
    }
}

/// Elements stored in QueueNameMultiIndexMap must impl this trait,
/// which describes the id, criteria and expiration of the element.
pub trait GetCriteria: Clone {
    fn get_id(&self) -> &Uuid;
    fn get_criteria(&self) -> &Criteria;
    fn get_expires(&self) -> Instant;
}

impl<T: GetCriteria> QueueNameMultiIndexMap<T> {
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_criteria: HashMap::new(),
            by_domain_campaign_tenant: HashMap::new(),
            by_domain_tenant: HashMap::new(),
            by_domain: HashMap::new(),
            other: HashMap::new(),
            match_all: None,
            lookup_count: 0,
            generation_count: 0,
        }
    }

    /// Returns a copy of all of the elements in their current state
    pub fn get_all(&self) -> Vec<T> {
        self.by_id.values().cloned().collect()
    }

    fn remove_existing_entry_with_same_critiera(&mut self, entry: &T) {
        let criteria = entry.get_criteria();
        if let Some(existing_id) = self.by_criteria.get(criteria).cloned() {
            self.remove_by_id(&existing_id);
        }
    }

    /// Remove any entry with the same criteria, then insert `entry`
    pub fn insert(&mut self, entry: T) {
        let id = entry.get_id();

        self.remove_existing_entry_with_same_critiera(&entry);

        let criteria = entry.get_criteria();
        match criteria.key() {
            KeyType::FullCriteria => {
                // Implicitly handled via by_criteria which is populated below
            }
            KeyType::DCT(dct) => {
                self.by_domain_campaign_tenant
                    .entry(dct)
                    .or_default()
                    .insert(*id, ());
            }
            KeyType::DT(dt) => {
                self.by_domain_tenant.entry(dt).or_default().insert(*id, ());
            }
            KeyType::D(d) => {
                self.by_domain.entry(d).or_default().insert(*id, ());
            }
            KeyType::Other => {
                if criteria.is_match_all() {
                    self.match_all.replace(*id);
                } else {
                    self.other.insert(*id, ());
                }
            }
        }

        self.by_criteria.insert(criteria.clone(), *id);
        self.by_id.insert(*id, entry);
        self.generation_count += 1;
    }

    /// Remove the entry with the specified id
    pub fn remove_by_id(&mut self, id: &Uuid) -> Option<T> {
        let entry = self.by_id.remove(id)?;
        self.generation_count += 1;

        let criteria = entry.get_criteria();
        self.by_criteria.remove(criteria);

        match criteria.key() {
            KeyType::FullCriteria => {
                // Implicitly handled via by_criteria which is updated above
            }
            KeyType::DCT(dct) => {
                self.by_domain_campaign_tenant
                    .entry(dct)
                    .or_default()
                    .remove(id);
            }
            KeyType::DT(dt) => {
                self.by_domain_tenant.entry(dt).or_default().remove(id);
            }
            KeyType::D(d) => {
                self.by_domain.entry(d).or_default().remove(id);
            }
            KeyType::Other => {
                if criteria.is_match_all() {
                    self.match_all.take();
                } else {
                    self.other.remove(id);
                }
            }
        }

        Some(entry)
    }

    pub fn maybe_prune(&mut self) {
        self.lookup_count += 1;
        if self.lookup_count > 10_000 {
            self.lookup_count = 0;
            self.prune_expired();
        }
    }

    /// Remove any expired entries
    pub fn prune_expired(&mut self) {
        let mut expired_ids = vec![];

        let now = Instant::now();
        for (id, entry) in self.by_id.iter() {
            if entry.get_expires() <= now {
                expired_ids.push(*id);
            }
        }

        for id in expired_ids {
            self.remove_by_id(&id);
        }
    }

    pub fn cached_get_matching(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
        cache: &ArcSwap<Option<CachedEntry<T>>>,
    ) -> Option<T> {
        if let Some(hit) = cache.load().as_ref() {
            if hit.generation_count == self.generation_count {
                match &hit.entry {
                    Some(entry) => {
                        if entry.get_expires() > Instant::now() {
                            return Some(entry.clone());
                        }
                    }
                    None => {
                        return None;
                    }
                }
            }
        }

        let entry = self.get_matching(campaign, tenant, domain, routing_domain);

        cache.store(Arc::new(Some(CachedEntry {
            entry: entry.clone(),
            generation_count: self.generation_count,
        })));

        entry
    }

    /// Get any entry that matches the specified criteria.
    /// If multiple could match, it is unspecified which one will
    /// be used for the match.
    pub fn get_matching(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
        routing_domain: Option<&str>,
    ) -> Option<T> {
        if self.by_id.is_empty() {
            // Fast path: if there are no rules, nothing could match!
            return None;
        }

        // If we have a match all, then we can cheaply use that one
        if let Some(id) = &self.match_all {
            if let Some(entry) = self.by_id.get(id) {
                return Some(entry.clone());
            }
        }

        // Otherwise, we logically need to iterate every item and
        // check if it matches.  Since that is O(n), it will be
        // pretty terrible for any largeish number of defined entries.
        // We need to reduce the search space!
        let now = Instant::now();

        let criteria = Criteria {
            campaign: campaign.map(|s| s.to_string()),
            tenant: tenant.map(|s| s.to_string()),
            domain: domain.map(|s| s.to_string()),
            routing_domain: routing_domain.map(|s| s.to_string()),
        };
        if let Some(id) = self.by_criteria.get(&criteria) {
            // Exactly matching criteria!
            if let Some(entry) = self.by_id.get(id) {
                if entry.get_expires() > now {
                    return Some(entry.clone());
                }
            }
        }

        if let (Some(d), Some(c), Some(t)) = (domain, campaign, tenant) {
            if let Some(ids) = self.by_domain_campaign_tenant.get(&DCT {
                domain: d.to_string(),
                campaign: c.to_string(),
                tenant: t.to_string(),
            }) {
                tracing::trace!("{} DCT: candidates for c={campaign:?} tenant={tenant:?} domain={domain:?} routing_domain={routing_domain:?}", ids.len());
                for id in ids.keys() {
                    if let Some(entry) = self.by_id.get(id) {
                        if entry
                            .get_criteria()
                            .matches(campaign, tenant, domain, routing_domain)
                        {
                            if entry.get_expires() > now {
                                return Some(entry.clone());
                            }
                        }
                    }
                }
            }
        }

        if let (Some(d), Some(t)) = (domain, tenant) {
            if let Some(ids) = self.by_domain_tenant.get(&DT {
                domain: d.to_string(),
                tenant: t.to_string(),
            }) {
                tracing::trace!("{} DT: candidates for c={campaign:?} tenant={tenant:?} domain={domain:?} routing_domain={routing_domain:?}", ids.len());
                for id in ids.keys() {
                    if let Some(entry) = self.by_id.get(id) {
                        if entry
                            .get_criteria()
                            .matches(campaign, tenant, domain, routing_domain)
                        {
                            if entry.get_expires() > now {
                                return Some(entry.clone());
                            }
                        }
                    }
                }
            }
        }

        if let Some(d) = domain {
            if let Some(ids) = self.by_domain.get(d) {
                tracing::trace!("{} DOMAIN: candidates for c={campaign:?} tenant={tenant:?} domain={domain:?} routing_domain={routing_domain:?}", ids.len());
                for id in ids.keys() {
                    if let Some(entry) = self.by_id.get(id) {
                        if entry
                            .get_criteria()
                            .matches(campaign, tenant, domain, routing_domain)
                        {
                            if entry.get_expires() > now {
                                return Some(entry.clone());
                            }
                        }
                    }
                }
            }
        }

        tracing::trace!("{} OTHER: candidates for c={campaign:?} tenant={tenant:?} domain={domain:?} routing_domain={routing_domain:?}", self.other.len());

        let now = Instant::now();

        for id in self.other.keys() {
            if let Some(entry) = self.by_id.get(id) {
                if entry
                    .get_criteria()
                    .matches(campaign, tenant, domain, routing_domain)
                {
                    if entry.get_expires() > now {
                        return Some(entry.clone());
                    }
                }
            }
        }
        None
    }
}

fn match_criteria(current_thing: Option<&str>, wanted_thing: Option<&str>) -> bool {
    match (current_thing, wanted_thing) {
        (Some(a), Some(b)) => a == b,
        (None, Some(_)) => {
            // Needs to match a specific thing and there is none
            false
        }
        (_, None) => {
            // No specific thing required
            true
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;

    #[derive(Clone, Debug, PartialEq)]
    struct Entry {
        id: Uuid,
        criteria: Criteria,
        expires: Instant,
        reason: String,
    }

    impl GetCriteria for Entry {
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

    #[test]
    fn match_any() {
        let mut map = QueueNameMultiIndexMap::new();

        map.insert(Entry {
            id: Uuid::new_v4(),
            criteria: Criteria::new_match_all(),
            expires: Instant::now() + Duration::from_secs(60),
            reason: "any".to_string(),
        });

        assert_eq!(
            map.get_matching(None, None, Some("woot"), None)
                .unwrap()
                .reason,
            "any"
        );

        map.insert(Entry {
            id: Uuid::new_v4(),
            criteria: Criteria::new_match_all(),
            expires: Instant::now() + Duration::from_secs(60),
            reason: "a different any".to_string(),
        });

        assert_eq!(
            map.get_matching(None, None, None, None).unwrap().reason,
            "a different any"
        );

        assert_eq!(map.by_id.len(), 1);
        assert_eq!(map.by_criteria.len(), 1);
        assert!(map.match_all.is_some());
    }

    #[test]
    fn match_domain() {
        let mut map = QueueNameMultiIndexMap::new();

        map.insert(Entry {
            id: Uuid::new_v4(),
            criteria: Criteria {
                domain: Some("domain".to_string()),
                campaign: None,
                tenant: None,
                routing_domain: None,
            },
            expires: Instant::now() + Duration::from_secs(60),
            reason: "only domain".to_string(),
        });

        assert_eq!(map.get_matching(Some("campaign"), None, None, None), None);
        assert_eq!(map.get_matching(None, Some("tenant"), None, None), None);
        assert_eq!(
            map.get_matching(None, None, Some("otherdomain"), None),
            None
        );
        assert_eq!(map.get_matching(None, None, None, Some("routing")), None);
        assert_eq!(
            map.get_matching(None, None, Some("domain"), None)
                .unwrap()
                .reason,
            "only domain"
        );
        assert_eq!(
            map.get_matching(
                Some("camp"),
                Some("tenant"),
                Some("domain"),
                Some("routing")
            )
            .unwrap()
            .reason,
            "only domain"
        );
    }

    #[test]
    fn match_domain_and_tenant() {
        let mut map = QueueNameMultiIndexMap::new();

        map.insert(Entry {
            id: Uuid::new_v4(),
            criteria: Criteria {
                domain: Some("domain".to_string()),
                campaign: None,
                tenant: Some("tenant".to_string()),
                routing_domain: None,
            },
            expires: Instant::now() + Duration::from_secs(60),
            reason: "tenant and domain".to_string(),
        });

        assert_eq!(map.get_matching(Some("campaign"), None, None, None), None);
        assert_eq!(map.get_matching(None, Some("tenant"), None, None), None);
        assert_eq!(map.get_matching(None, None, Some("domain"), None), None);
        assert_eq!(map.get_matching(None, None, None, Some("routing")), None);
        assert_eq!(
            map.get_matching(
                Some("camp"),
                Some("tenant"),
                Some("domain"),
                Some("routing")
            )
            .unwrap()
            .reason,
            "tenant and domain"
        );

        // Now add another entry that can match the same domain
        map.insert(Entry {
            id: Uuid::new_v4(),
            criteria: Criteria {
                domain: Some("domain".to_string()),
                campaign: None,
                tenant: None,
                routing_domain: None,
            },
            expires: Instant::now() + Duration::from_secs(60),
            reason: "only domain".to_string(),
        });

        assert_eq!(map.by_id.len(), 2);
        assert_eq!(map.by_criteria.len(), 2);

        assert!(
            map.get_matching(
                Some("camp"),
                Some("tenant"),
                Some("domain"),
                Some("routing")
            )
            .is_some(),
            "must match either entry"
        );

        assert!(
            map.get_matching(None, None, Some("domain"), None,)
                .is_some(),
            "must match either entry"
        );
    }

    #[test]
    fn match_combinations() {
        let mut map = QueueNameMultiIndexMap::new();

        let mut queries = vec![];

        for d in [Some("domain"), None] {
            for c in [Some("campaign"), None] {
                for t in [Some("tenant"), None] {
                    for rd in [Some("rd"), None] {
                        let reason = format!("{d:?} {c:?} {t:?} {rd:?}");
                        if reason == "None None None None" {
                            // Don't add a match-all entry
                            continue;
                        }
                        map.insert(Entry {
                            id: Uuid::new_v4(),
                            criteria: Criteria {
                                domain: d.map(|s| s.to_string()),
                                campaign: c.map(|s| s.to_string()),
                                tenant: t.map(|s| s.to_string()),
                                routing_domain: rd.map(|s| s.to_string()),
                            },
                            expires: Instant::now() + Duration::from_secs(60),
                            reason: reason.clone(),
                        });

                        queries.push((d, c, t, rd, reason));
                    }
                }
            }
        }

        eprintln!("{queries:#?}");

        for (d, c, t, rd, reason) in queries {
            let e = map.get_matching(c, t, d, rd);
            assert_eq!(e.unwrap().reason, reason);
        }
    }
}
