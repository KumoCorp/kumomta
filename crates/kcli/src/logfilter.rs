use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::SetDiagnosticFilterRequest;
use reqwest::Url;

#[derive(Debug, Parser)]
/// Changes the diagnostic log filter
///
/// See <https://docs.kumomta.com/reference/kumo/set_diagnostic_log_filter/>
/// for more information about the log filter syntax.
pub struct SetLogFilterCommand {
    filter: String,
}

impl SetLogFilterCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_set_diagnostic_log_filter_v1(&SetDiagnosticFilterRequest {
                filter: self.filter.clone(),
            })
            .await?;

        if !response.is_empty() {
            println!("{response}");
        } else {
            println!("OK");
        }

        Ok(())
    }
}
