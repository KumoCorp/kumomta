use crate::queue_summary::get_metrics;
use anyhow::Context;
use clap::Parser;
use dns_resolver::MailExchanger;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use itertools::Itertools;
use lexicmp::natural_lexical_cmp;
use message::message::QueueNameComponents;
use num_format::{Locale, ToFormattedString};
use reqwest::Url;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tabout::{Alignment, Column};

/// Prints a summary of the aggregate state of the queues from the perspective
/// of the provider or destination site.
///
/// Note that this output format is subject to change and is not suitable
/// for a machine to parse. It is expressly unstable and you must not
/// depend upon it in automation.
///
/// The data behind this output is pulled from the metrics endpoint,
/// which is machine readable.
///
/// The default output mode is to show the total volume of traffic
/// grouped by the provider, or, if not applicable provider matching
/// rules were defined on the server, the site name that is derived
/// from the MX records for a domain.
///
/// The data is shown ordered by descending volume, where volume is the sum
/// of the delivered, failed, transiently failed and queued message counts.
///
/// The --by-pool flag will further sub-divide the display by the egress pool.
///
/// The column labels have the following meanings:
///
/// PROVIDER - either the provider (if explicitly set through the config on the server),
///            or the site name for the underlying domain.
///
/// POOL     - (when --by-pool is used) the name of the egress pool
///
/// D        - the total number of delivered messages
///
/// T        - the total number of transiently failed messages
///
/// F        - the total number of failed/bounced messages
///
/// C        - the current number of open connections
///
/// Q        - the total number of ready and scheduled messages in queue
///
/// DOMAINS  - (when --show-domains is used) a list of domains that correspond to
///            rows that do not have an explicitly configured provider.
#[derive(Debug, Parser)]
pub struct ProviderSummaryCommand {
    /// Include a POOL column in the output, and break down the volume on
    /// a per-pool basis.
    #[arg(long)]
    by_pool: bool,

    /// For rows that were not matched on the server by provider rules we will
    /// normally show a site-name in the place of the PROVIDER column.
    ///
    /// When --show-domains is enabled, an additional DOMAINS column will be
    /// added to the output to hold a list of domains which correspond to
    /// that site-name.
    ///
    /// This option is only capable of filling in the list
    /// of domains if any of those domains have delayed messages residing
    /// in their respective scheduled queues at the time that this command
    /// was invoked.
    #[arg(long)]
    show_domains: bool,

    /// Limit results to LIMIT results
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Default)]
struct ProviderMetrics {
    name: String,
    delivered: usize,
    transfail: usize,
    fail: usize,
    queue_size: usize,
    pool: Option<String>,
    connections: usize,
}

impl ProviderMetrics {
    fn volume(&self) -> usize {
        self.queue_size + self.delivered + self.transfail + self.fail
    }
}

impl ProviderSummaryCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let mut provider_metrics = HashMap::new();
        let mut provider_by_pool = HashMap::new();
        let mut domain_resolution = FuturesUnordered::new();
        let limiter = Arc::new(tokio::sync::Semaphore::new(128));
        let _: Vec<()> = get_metrics(endpoint, |m| {
            let name = m.name().as_str();
            match name {
                "scheduled_count" if self.show_domains => {
                    let queue = m.labels().get("queue").unwrap();
                    let components = QueueNameComponents::parse(queue);
                    let domain = components
                        .routing_domain
                        .unwrap_or(components.domain)
                        .to_string();
                    let limiter = limiter.clone();
                    domain_resolution.push(tokio::spawn(async move {
                        match limiter.acquire().await {
                            Ok(permit) => {
                                let mx_result = MailExchanger::resolve(&domain).await;
                                drop(permit);
                                (domain, mx_result)
                            }
                            Err(err) => (domain, Err(err).context("failed to acquire permit")),
                        }
                    }));
                }

                "total_messages_delivered_by_provider"
                | "connection_count_by_provider"
                | "total_messages_fail_by_provider"
                | "total_messages_transfail_by_provider"
                | "queued_count_by_provider" => {
                    if !self.by_pool {
                        let provider = m.labels().get("provider").unwrap();
                        let value = m.value() as usize;

                        let entry =
                            provider_metrics
                                .entry(provider.to_string())
                                .or_insert_with(|| ProviderMetrics {
                                    name: provider.to_string(),
                                    ..Default::default()
                                });

                        match name {
                            "total_messages_delivered_by_provider" => {
                                entry.delivered += value;
                            }
                            "total_messages_fail_by_provider" => {
                                entry.fail += value;
                            }
                            "total_messages_transfail_by_provider" => {
                                entry.transfail += value;
                            }
                            "queued_count_by_provider" => {
                                entry.queue_size += value;
                            }
                            "connection_count_by_provider" => {
                                entry.connections += value;
                            }
                            _ => {}
                        }
                    }
                }
                "total_messages_delivered_by_provider_and_source"
                | "connection_count_by_provider_and_pool"
                | "total_messages_fail_by_provider_and_source"
                | "total_messages_transfail_by_provider_and_source"
                | "queued_count_by_provider_and_pool" => {
                    if self.by_pool {
                        let provider = m.labels().get("provider").unwrap();
                        let pool = m.labels().get("pool").unwrap();
                        let value = m.value() as usize;

                        let entry = provider_by_pool
                            .entry((provider.to_string(), pool.to_string()))
                            .or_insert_with(|| ProviderMetrics {
                                name: provider.to_string(),
                                pool: Some(pool.to_string()),
                                ..Default::default()
                            });

                        match name {
                            "total_messages_delivered_by_provider_and_source" => {
                                entry.delivered += value;
                            }
                            "total_messages_fail_by_provider_and_source" => {
                                entry.fail += value;
                            }
                            "total_messages_transfail_by_provider_and_source" => {
                                entry.transfail += value;
                            }
                            "queued_count_by_provider_and_pool" => {
                                entry.queue_size += value;
                            }
                            "connection_count_by_provider_and_pool" => {
                                entry.connections += value;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            None
        })
        .await?;

        let mut site_to_domains: HashMap<String, BTreeSet<String>> = HashMap::new();
        while let Some(Ok((domain, result))) = domain_resolution.next().await {
            if let Ok(mx) = result {
                site_to_domains
                    .entry(mx.site_name.to_string())
                    .or_default()
                    .insert(domain);
            }
        }

        if self.by_pool {
            let mut provider_by_pool: Vec<_> = provider_by_pool.into_values().collect();

            provider_by_pool.sort_by(|a, b| match b.volume().cmp(&a.volume()) {
                Ordering::Equal => match natural_lexical_cmp(&a.name, &b.name) {
                    Ordering::Equal => {
                        natural_lexical_cmp(a.pool.as_ref().unwrap(), b.pool.as_ref().unwrap())
                    }
                    ordering => ordering,
                },
                ordering => ordering,
            });
            let mut columns = vec![
                Column {
                    name: "PROVIDER".to_string(),
                    alignment: Alignment::Left,
                },
                Column {
                    name: "POOL".to_string(),
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
                    name: "F".to_string(),
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
            ];

            if !site_to_domains.is_empty() {
                columns.push(Column {
                    name: "DOMAINS".to_string(),
                    alignment: Alignment::Left,
                });
            }

            let mut rows = vec![];
            for m in &provider_by_pool {
                if let Some(limit) = self.limit {
                    if rows.len() >= limit {
                        break;
                    }
                }
                let mut row = vec![
                    m.name.to_string(),
                    m.pool.as_ref().unwrap().to_string(),
                    m.delivered.to_formatted_string(&Locale::en),
                    m.transfail.to_formatted_string(&Locale::en),
                    m.fail.to_formatted_string(&Locale::en),
                    m.connections.to_formatted_string(&Locale::en),
                    m.queue_size.to_formatted_string(&Locale::en),
                ];

                if let Some(domains) = resolve_domains(&mut site_to_domains, &m.name) {
                    row.push(domains);
                }

                rows.push(row);
            }

            tabout::tabulate_output(&columns, &rows, &mut std::io::stdout())?;
        } else {
            let mut provider_metrics: Vec<_> = provider_metrics.into_values().collect();
            // Order by queue size DESC, then name
            provider_metrics.sort_by(|a, b| match b.volume().cmp(&a.volume()) {
                Ordering::Equal => natural_lexical_cmp(&a.name, &b.name),
                ordering => ordering,
            });
            let mut columns = vec![
                Column {
                    name: "PROVIDER".to_string(),
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
                    name: "F".to_string(),
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
            ];

            if !site_to_domains.is_empty() {
                columns.push(Column {
                    name: "DOMAINS".to_string(),
                    alignment: Alignment::Left,
                });
            }

            let mut rows = vec![];
            for m in &provider_metrics {
                if let Some(limit) = self.limit {
                    if rows.len() >= limit {
                        break;
                    }
                }
                let mut row = vec![
                    m.name.to_string(),
                    m.delivered.to_formatted_string(&Locale::en),
                    m.transfail.to_formatted_string(&Locale::en),
                    m.fail.to_formatted_string(&Locale::en),
                    m.connections.to_formatted_string(&Locale::en),
                    m.queue_size.to_formatted_string(&Locale::en),
                ];

                if let Some(domains) = resolve_domains(&mut site_to_domains, &m.name) {
                    row.push(domains);
                }

                rows.push(row);
            }

            tabout::tabulate_output(&columns, &rows, &mut std::io::stdout())?;
        }

        Ok(())
    }
}

fn resolve_domains(
    site_to_domains: &mut HashMap<String, BTreeSet<String>>,
    site: &str,
) -> Option<String> {
    if let Some(domains) = site_to_domains.get_mut(site) {
        Some(domains.iter().join(", "))
    } else {
        None
    }
}
