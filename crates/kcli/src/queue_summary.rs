use clap::Parser;
use dns_resolver::MailExchanger;
use futures::StreamExt;
use kumo_api_types::{BounceV1ListEntry, SuspendReadyQueueV1ListEntry, SuspendV1ListEntry};
use kumo_prometheus::parser::Metric;
use lexicmp::natural_lexical_cmp;
use message::message::QueueNameComponents;
use num_format::{Locale, ToFormattedString};
use reqwest::Url;
use std::cmp::Ordering;
use std::collections::HashMap;
use tabout::{Alignment, Column};

/// Prints a summary of the state of the queues, for a human to read.
///
/// Note that this output format is subject to change and is not suitable
/// for a machine to parse. It is expressly unstable and you must not
/// depend upon it in automation.
///
/// The data behind this output is pulled from the metrics endpoint,
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

pub async fn get_metrics<T, F: FnMut(&Metric) -> Option<T>>(
    endpoint: &Url,
    mut filter_map: F,
) -> anyhow::Result<Vec<T>> {
    let mut parser = kumo_prometheus::parser::Parser::new();
    let mut stream = crate::request_with_streaming_text_response(
        reqwest::Method::GET,
        endpoint.join("/metrics")?,
        &(),
    )
    .await?;

    let mut result = vec![];
    while let Some(item) = stream.next().await {
        let bytes = item?;
        parser.push_bytes(bytes, false, |m| {
            if let Some(r) = (filter_map)(&m) {
                result.push(r);
            }
        })?;
    }

    Ok(result)
}

pub struct QueueMetricsParams {
    pub by_volume: bool,
    pub limit: usize,
}

pub struct QueueMetrics {
    pub ready: Vec<ReadyQueueMetrics>,
    pub scheduled: Vec<ScheduledQueueMetrics>,
}

impl QueueMetrics {
    pub async fn obtain(endpoint: &Url, params: QueueMetricsParams) -> anyhow::Result<Self> {
        let mut ready = HashMap::new();
        let mut scheduled: HashMap<String, ScheduledQueueMetrics> = HashMap::new();
        let _: Vec<()> = get_metrics(endpoint, |m| {
            let name = m.name().as_str();
            match name {
                "connection_count"
                | "total_messages_delivered"
                | "total_messages_transfail"
                | "ready_count" => {
                    if let Some(service) = m.labels().get("service") {
                        if let Some((_protocol, queue_name)) = service.split_once(":") {
                            let value = m.value() as usize;

                            let entry = ready
                                .entry(queue_name.to_string())
                                .or_insert_with(|| ReadyQueueMetrics::with_name(queue_name));
                            match name {
                                "connection_count" => {
                                    entry.connection_count += value;
                                }
                                "total_messages_delivered" => {
                                    entry.delivered += value;
                                }
                                "total_messages_transfail" => {
                                    entry.transfail += value;
                                }
                                "ready_count" => {
                                    entry.queue_size += value;
                                }
                                _ => {}
                            }
                        }
                    }
                }
                "scheduled_count" => {
                    if let Some(queue) = m.labels().get("queue") {
                        let queue_size = m.value() as usize;

                        if params.by_volume && scheduled.len() == params.limit {
                            scheduled.retain(|_k, entry| entry.queue_size > queue_size);
                        }

                        if scheduled.len() <= params.limit {
                            let entry = scheduled
                                .entry(queue.to_string())
                                .or_insert_with(|| ScheduledQueueMetrics::with_name(queue));
                            entry.queue_size += queue_size;
                        }
                    }
                }
                _ => {}
            }
            None
        })
        .await?;

        let mut ready_metrics: Vec<ReadyQueueMetrics> =
            ready.into_iter().map(|(_k, v)| v).collect();

        if params.by_volume {
            ready_metrics.sort_by(|a, b| match a.compare_volume(b) {
                Ordering::Equal => natural_lexical_cmp(&a.name, &b.name),
                ordering => ordering,
            });
        } else {
            ready_metrics.sort_by(|a, b| natural_lexical_cmp(&a.name, &b.name));
        }

        let mut scheduled_metrics: Vec<ScheduledQueueMetrics> =
            scheduled.into_iter().map(|(_k, v)| v).collect();

        if params.by_volume {
            scheduled_metrics.sort_by(|a, b| match a.compare_volume(b) {
                Ordering::Equal => natural_lexical_cmp(&a.name, &b.name),
                ordering => ordering,
            });
        } else {
            scheduled_metrics.sort_by(|a, b| natural_lexical_cmp(&a.name, &b.name));
        }

        Ok(Self {
            ready: ready_metrics,
            scheduled: scheduled_metrics,
        })
    }
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

        let mut metrics = QueueMetrics::obtain(
            endpoint,
            QueueMetricsParams {
                by_volume: self.by_volume,
                limit: self.limit.unwrap_or(usize::MAX),
            },
        )
        .await?;

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
                m.delivered.to_formatted_string(&Locale::en),
                m.transfail.to_formatted_string(&Locale::en),
                m.connection_count.to_formatted_string(&Locale::en),
                m.queue_size.to_formatted_string(&Locale::en),
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
                m.queue_size.to_formatted_string(&Locale::en),
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
