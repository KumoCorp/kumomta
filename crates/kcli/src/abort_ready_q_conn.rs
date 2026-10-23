use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::AbortReadyQConnV1Request;
use reqwest::Url;
use uuid::Uuid;

#[derive(Debug, Parser)]
/// Aborts the dispatcher task within a ready queue identified by its
/// session_id. The dispatcher's drop path returns any in-flight
/// message to the scheduled queue for another delivery attempt.
pub struct AbortReadyQConnCommand {
    /// The name of the ready queue.
    pub queue_name: String,
    /// The session_id of the dispatcher to abort. Obtain this via
    /// `kcli inspect-ready-q`.
    #[arg(value_parser = Uuid::parse_str)]
    pub session_id: Uuid,
}

impl AbortReadyQConnCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_abort_ready_q_conn_v1(&AbortReadyQConnV1Request {
                queue_name: self.queue_name.clone(),
                session_id: self.session_id,
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
