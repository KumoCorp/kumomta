use clap::Parser;
use dns_resolver::MailExchanger;
use kumo_api_types::{BounceV1ListEntry, SuspendReadyQueueV1ListEntry, SuspendV1ListEntry};
use lexicmp::natural_lexical_cmp;
use message::message::QueueNameComponents;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use tabout::{Alignment, Column};

/// Prints a summary of the state of the queues, for a human to read.
///
/// Note that this output format is subject to change and is not suitable
/// for a machine to parse. It is expressly unstable and you must not
/// depend upon it in automation.
///
/// The data behind this output is pulled from the metrics.json endpoint,
/// which is machine readable.
///
/// The output is presented in two sections:
///
/// 1. The ready queues
///
/// 2. The scheduled queues
///
/// The ready queue data is presented in columns that are mostly self
/// explanatory, but the numeric counts are labelled with single character
/// labels:
///
/// D - the total number of delivered messages
///
/// T - the total number of transiently failed messages
///
/// C - the number of open connections
///
/// Q - the number of ready messages in the queue
///
/// Note that the ready queue counter values reset whenever the ready
/// queue is reaped, which occurs within a few minutes of the ready queue
/// being idle, so those numbers are only useful to get a sense of
/// recent/current activity. Accurate accounting must be performed using
/// the delivery logs and not via this utility.
///
/// The scheduled queue data is presented in two columns; the queue
/// name and the number of messages in that queue.
#[derive(Debug, Parser)]
pub struct QueueSummaryCommand {
    /// Limit results to LIMIT results
    #[arg(long)]
    limit: Option<usize>,

    /// Instead of ordering by name, order by volume, descending
    #[arg(long)]
    by_volume: bool,

    /// Filter queues to those associated with a DNS domain
    #[arg(long)]
    domain: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub struct IndividualCounter {
    pub help: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub value: f64,
}

#[derive(Deserialize, Serialize)]
pub struct CounterGroup {
    pub help: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub value: ServiceMap,
}

#[derive(Deserialize, Serialize)]
pub struct ServiceMap {
    pub service: HashMap<String, f64>,
}

#[derive(Deserialize, Serialize)]
pub struct QueueCounterGroup {
    pub help: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub value: QueueMap,
}

#[derive(Deserialize, Serialize)]
pub struct QueueMap {
    pub queue: HashMap<String, f64>,
}

#[derive(Deserialize, Serialize)]
pub struct Metrics {
    pub connection_count: Option<CounterGroup>,
    pub ready_count: Option<CounterGroup>,
    pub scheduled_count: Option<QueueCounterGroup>,
    pub total_connection_count: Option<CounterGroup>,
    pub total_messages_delivered: Option<CounterGroup>,
    pub total_messages_transfail: Option<CounterGroup>,
    pub total_messages_received: Option<CounterGroup>,
    pub message_count: Option<IndividualCounter>,
    pub message_data_resident_count: Option<IndividualCounter>,
    pub message_meta_resident_count: Option<IndividualCounter>,
    pub memory_usage: Option<IndividualCounter>,
    pub memory_limit: Option<IndividualCounter>,
}

#[derive(Default, Debug)]
pub struct ReadyQueueMetrics {
    pub name: String,
    pub delivered: usize,
    pub transfail: usize,
    pub connection_count: usize,
    pub queue_size: usize,
}

impl ReadyQueueMetrics {
    fn with_name(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    pub fn site_name(&self) -> &str {
        let source_len = self
            .source()
            .map(|s| s.len() + 2 /* for the "->" */)
            .unwrap_or(0);
        let proto_len = self
            .protocol()
            .map(|p| p.len() + 1 /* for the "@" */)
            .unwrap_or(0);
        let len = self.name.len() - (source_len + proto_len);
        &self.name[source_len..source_len + len]
    }

    pub fn source(&self) -> Option<&str> {
        let pos = self.name.find("->")?;
        Some(&self.name[0..pos])
    }

    pub fn protocol(&self) -> Option<&str> {
        if self.name.ends_with("@smtp_client") {
            return Some("smtp_client");
        }
        if let Some(pos) = self.name.rfind("@lua:") {
            return Some(&self.name[pos + 1..]);
        }
        if let Some(pos) = self.name.rfind("@maildir:") {
            return Some(&self.name[pos + 1..]);
        }
        None
    }

    pub fn volume(&self) -> usize {
        self.delivered + self.transfail + self.connection_count + self.queue_size
    }

    pub fn compare_volume(&self, other: &Self) -> Ordering {
        self.volume().cmp(&other.volume()).reverse()
    }
}

#[derive(Default, Debug)]
pub struct ScheduledQueueMetrics {
    pub name: String,
    pub queue_size: usize,
}

impl ScheduledQueueMetrics {
    fn with_name(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }

    pub fn volume(&self) -> usize {
        self.queue_size
    }

    pub fn compare_volume(&self, other: &Self) -> Ordering {
        self.volume().cmp(&other.volume()).reverse()
    }
}

pub struct ProcessedMetrics {
    pub ready: Vec<ReadyQueueMetrics>,
    pub scheduled: Vec<ScheduledQueueMetrics>,
    pub raw: Metrics,
}

pub async fn obtain_metrics(endpoint: &Url, by_volume: bool) -> anyhow::Result<ProcessedMetrics> {
    let result: Metrics = crate::request_with_json_response(
        reqwest::Method::GET,
        endpoint.join("/metrics.json")?,
        &(),
    )
    .await?;

    let mut ready_metrics = HashMap::new();
    if let Some(conn_count) = &result.connection_count {
        for (service, &count) in conn_count.value.service.iter() {
            if let Some((_protocol, queue_name)) = service.split_once(':') {
                let entry = ready_metrics
                    .entry(queue_name)
                    .or_insert_with(|| ReadyQueueMetrics::with_name(queue_name));
                entry.connection_count += count as usize;
            }
        }
    }
    if let Some(delivered_count) = &result.total_messages_delivered {
        for (service, &count) in delivered_count.value.service.iter() {
            if let Some((_protocol, queue_name)) = service.split_once(':') {
                let entry = ready_metrics
                    .entry(queue_name)
                    .or_insert_with(|| ReadyQueueMetrics::with_name(queue_name));
                entry.delivered += count as usize;
            }
        }
    }
    if let Some(transfail_count) = &result.total_messages_transfail {
        for (service, &count) in transfail_count.value.service.iter() {
            if let Some((_protocol, queue_name)) = service.split_once(':') {
                let entry = ready_metrics
                    .entry(queue_name)
                    .or_insert_with(|| ReadyQueueMetrics::with_name(queue_name));
                entry.transfail += count as usize;
            }
        }
    }
    if let Some(ready_count) = &result.ready_count {
        for (service, &count) in ready_count.value.service.iter() {
            if let Some((_protocol, queue_name)) = service.split_once(':') {
                let entry = ready_metrics
                    .entry(queue_name)
                    .or_insert_with(|| ReadyQueueMetrics::with_name(queue_name));
                entry.queue_size += count as usize;
            }
        }
    }

    let mut ready_metrics: Vec<ReadyQueueMetrics> =
        ready_metrics.into_iter().map(|(_k, v)| v).collect();

    if by_volume {
        ready_metrics.sort_by(|a, b| match a.compare_volume(b) {
            Ordering::Equal => natural_lexical_cmp(&a.name, &b.name),
            ordering => ordering,
        });
    } else {
        ready_metrics.sort_by(|a, b| natural_lexical_cmp(&a.name, &b.name));
    }

    let mut scheduled_metrics = HashMap::new();
    if let Some(item) = &result.scheduled_count {
        for (domain, &count) in item.value.queue.iter() {
            let entry = scheduled_metrics
                .entry(domain)
                .or_insert_with(|| ScheduledQueueMetrics::with_name(domain));
            entry.queue_size += count as usize;
        }
    }

    let mut scheduled_metrics: Vec<ScheduledQueueMetrics> =
        scheduled_metrics.into_iter().map(|(_k, v)| v).collect();

    if by_volume {
        scheduled_metrics.sort_by(|a, b| match a.compare_volume(b) {
            Ordering::Equal => natural_lexical_cmp(&a.name, &b.name),
            ordering => ordering,
        });
    } else {
        scheduled_metrics.sort_by(|a, b| natural_lexical_cmp(&a.name, &b.name));
    }

    Ok(ProcessedMetrics {
        ready: ready_metrics,
        scheduled: scheduled_metrics,
        raw: result,
    })
}

impl QueueSummaryCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let suspended_domains: Vec<SuspendV1ListEntry> = crate::request_with_json_response(
            reqwest::Method::GET,
            endpoint.join("/api/admin/suspend/v1")?,
            &(),
        )
        .await?;

        let bounced_domains: Vec<BounceV1ListEntry> = crate::request_with_json_response(
            reqwest::Method::GET,
            endpoint.join("/api/admin/bounce/v1")?,
            &(),
        )
        .await?;

        let suspended_sites: Vec<SuspendReadyQueueV1ListEntry> = crate::request_with_json_response(
            reqwest::Method::GET,
            endpoint.join("/api/admin/suspend-ready-q/v1")?,
            &(),
        )
        .await?;

        let mut metrics = obtain_metrics(endpoint, self.by_volume).await?;

        if let Some(domain) = &self.domain {
            let mx = MailExchanger::resolve(domain).await?;

            // Include all ready queues for the same site
            metrics.ready.retain(|m| m.site_name() == mx.site_name);

            // Resolve the sites of all the scheduled queue domains
            let mut futures = vec![];
            for m in &metrics.scheduled {
                futures.push(MailExchanger::resolve(&m.name));
            }

            let mut domain_to_site = HashMap::new();
            for res in futures::future::join_all(futures).await {
                if let Ok(mx) = res {
                    domain_to_site.insert(mx.domain_name.to_string(), mx.site_name.to_string());
                }
            }

            // Include all the scheduled queues that either directly match
            // the requested domain name, or which have the same site name
            // as the requested domain name
            metrics.scheduled.retain(|m| {
                m.name == *domain
                    || domain_to_site
                        .get(&m.name)
                        .map(|s| *s == mx.site_name)
                        .unwrap_or(false)
            });
        }

        if let Some(limit) = self.limit {
            metrics.ready.truncate(limit);
            metrics.scheduled.truncate(limit);
        }

        let ready_columns = [
            Column {
                name: "SITE".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "SOURCE".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "PROTO".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "D".to_string(),
                alignment: Alignment::Right,
            },
            Column {
                name: "T".to_string(),
                alignment: Alignment::Right,
            },
            Column {
                name: "C".to_string(),
                alignment: Alignment::Right,
            },
            Column {
                name: "Q".to_string(),
                alignment: Alignment::Right,
            },
            Column {
                name: "".to_string(),
                alignment: Alignment::Left,
            },
        ];

        let mut ready_rows = vec![];
        for m in &metrics.ready {
            let paused = suspended_sites.iter().any(|s| s.name == m.name);
            let status = if paused { "🛑" } else { "" };

            ready_rows.push(vec![
                m.site_name().to_string(),
                m.source().unwrap_or("").to_string(),
                m.protocol().unwrap_or("").to_string(),
                m.delivered.to_string(),
                m.transfail.to_string(),
                m.connection_count.to_string(),
                m.queue_size.to_string(),
                status.to_string(),
            ]);
        }

        tabout::tabulate_output(&ready_columns, &ready_rows, &mut std::io::stdout())?;

        let sched_columns = [
            Column {
                name: "SCHEDULED QUEUE".to_string(),
                alignment: Alignment::Left,
            },
            Column {
                name: "COUNT".to_string(),
                alignment: Alignment::Right,
            },
            Column {
                name: "".to_string(),
                alignment: Alignment::Left,
            },
        ];

        let mut sched_rows = vec![];
        for m in &metrics.scheduled {
            let components = QueueNameComponents::parse(&m.name);
            let paused = suspended_domains
                .iter()
                .any(|s| domain_matches(&components, &s.campaign, &s.tenant, &s.domain));
            let bounced = bounced_domains
                .iter()
                .any(|s| domain_matches(&components, &s.campaign, &s.tenant, &s.domain));

            let status = if bounced {
                "🗑️"
            } else if paused {
                "🛑"
            } else {
                ""
            };

            sched_rows.push(vec![
                m.name.to_string(),
                m.queue_size.to_string(),
                status.to_string(),
            ]);
        }

        println!();

        tabout::tabulate_output(&sched_columns, &sched_rows, &mut std::io::stdout())?;

        Ok(())
    }
}

fn domain_matches(
    components: &QueueNameComponents,
    campaign: &Option<String>,
    tenant: &Option<String>,
    domain: &Option<String>,
) -> bool {
    if !match_criteria(campaign.as_deref(), components.campaign.as_deref()) {
        return false;
    }
    if !match_criteria(tenant.as_deref(), components.tenant.as_deref()) {
        return false;
    }
    if !match_criteria(domain.as_deref(), Some(components.domain)) {
        return false;
    }
    true
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
