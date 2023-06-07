use clap::Parser;
use kumo_api_types::SetDiagnosticFilterRequest;
use reqwest::{RequestBuilder, Url};

#[derive(Debug, Parser)]
/// Changes the diagnostic log filter
///
/// See <https://docs.kumomta.com/reference/kumo/set_diagnostic_log_filter/>
/// for more information about the log filter syntax.
pub struct SetLogFilterCommand {
    filter: String,
}

impl SetLogFilterCommand {
    pub async fn run(&self, request: RequestBuilder) -> anyhow::Result<()> {
        let response = request
            .json(&SetDiagnosticFilterRequest {
                filter: self.filter.clone(),
            })
            .send()
            .await?;

        let status = response.status();

        let response = response.text().await?;

        if !status.is_success() {
            anyhow::bail!("{response}");
        }

        if !response.is_empty() {
            println!("{response}");
        } else {
            println!("OK");
        }

        Ok(())
    }

    pub fn url(&self, endpoint: Url) -> anyhow::Result<Url> {
        Ok(endpoint.join("/api/admin/set_diagnostic_log_filter/v1")?)
    }
}
