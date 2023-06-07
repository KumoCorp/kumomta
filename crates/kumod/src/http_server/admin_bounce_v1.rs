use crate::http_server::auth::TrustedIpRequired;
use crate::http_server::AppError;
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::queue::QueueManager;
use axum::extract::Json;
use kumo_api_types::{BounceV1Request, BounceV1Response};
use message::message::QueueNameComponents;
use message::Message;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

lazy_static::lazy_static! {
    static ref ENTRIES: Mutex<Vec<AdminBounceEntry>> = Mutex::new(vec![]);
}

#[derive(Clone, Debug)]
pub struct AdminBounceEntry {
    pub campaign: Option<String>,
    pub tenant: Option<String>,
    pub domain: Option<String>,
    pub reason: String,
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
                    && ent.domain == entry.domain)
        });

        entries.push(entry);
    }

    pub fn matches(
        &self,
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
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
        true
    }

    pub fn get_matching(
        campaign: Option<&str>,
        tenant: Option<&str>,
        domain: Option<&str>,
    ) -> Vec<Self> {
        let mut entries = Self::get_all();
        entries.retain(|ent| ent.matches(campaign, tenant, domain));
        entries
    }

    pub fn get_for_queue_name(queue_name: &str) -> Option<Self> {
        let components = QueueNameComponents::parse(queue_name);
        let mut entries = Self::get_matching(
            components.campaign,
            components.tenant,
            Some(components.domain),
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
        })
        .await;

        let mut bounced = self.bounced.lock().unwrap();
        if let Some(entry) = bounced.get_mut(queue_name) {
            *entry += 1;
        } else {
            bounced.insert(queue_name.to_string(), 1);
        }
    }
}

pub async fn bounce_v1(
    _: TrustedIpRequired,
    // Note: Json<> must be last in the param list
    Json(request): Json<BounceV1Request>,
) -> Result<Json<BounceV1Response>, AppError> {
    let duration = request.duration();
    let entry = AdminBounceEntry {
        campaign: request.campaign,
        tenant: request.tenant,
        domain: request.domain,
        reason: request.reason,
        expires: Instant::now() + duration,
        bounced: Arc::new(Mutex::new(HashMap::new())),
    };

    AdminBounceEntry::add(entry.clone());

    let queue_names = entry.list_matching_queues().await;

    for name in &queue_names {
        if let Some(q) = QueueManager::get_opt(name).await {
            q.lock().await.bounce_all(&entry).await;
        }
    }

    let bounced = entry.bounced.lock().unwrap().clone();
    let total_bounced = bounced.values().sum();

    Ok(Json(BounceV1Response {
        bounced,
        total_bounced,
    }))
}
