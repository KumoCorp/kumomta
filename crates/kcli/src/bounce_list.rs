use clap::Parser;
use kumo_api_client::KumoApiClient;
use num_format::{Locale, ToFormattedString};
use reqwest::Url;
use tabout::{Alignment, Column};

#[derive(Debug, Parser)]
/// Returns list of current administrative bounce rules.
///
/// Returns the list of un-expired admin bounces that are
/// currently in effect on the target instance.
pub struct BounceListCommand {
    /// Instead of showing the human readable tabulated output,
    /// return the underlying json data.
    #[arg(long)]
    json: bool,
}

impl BounceListCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let result = client.admin_bounce_list_v1().await?;

        if self.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            let columns = [
                Column {
                    name: "ID".to_string(),
                    alignment: Alignment::Left,
                },
                Column {
                    name: "REASON".to_string(),
                    alignment: Alignment::Left,
                },
                Column {
                    name: "REMAIN".to_string(),
                    alignment: Alignment::Left,
                },
                Column {
                    name: "BOUNCED".to_string(),
                    alignment: Alignment::Right,
                },
                Column {
                    name: "CRITERIA".to_string(),
                    alignment: Alignment::Left,
                },
            ];
            let mut rows = vec![];
            for entry in result {
                let mut criteria = vec![];
                if let Some(c) = &entry.campaign {
                    criteria.push(format!("campaign={c}"));
                }
                if let Some(c) = &entry.tenant {
                    criteria.push(format!("tenant={c}"));
                }
                if let Some(c) = &entry.domain {
                    criteria.push(format!("domain={c}"));
                }
                if let Some(c) = &entry.routing_domain {
                    criteria.push(format!("routing_domain={c}"));
                }
                if criteria.is_empty() {
                    criteria.push("everything".to_string());
                }
                let criteria = criteria.join(", ");

                rows.push(vec![
                    entry.id.to_string(),
                    entry.reason,
                    humantime::format_duration(entry.duration).to_string(),
                    entry.total_bounced.to_formatted_string(&Locale::en),
                    criteria,
                ]);
            }
            tabout::tabulate_output(&columns, &rows, &mut std::io::stdout())?;
        }

        Ok(())
    }
}
