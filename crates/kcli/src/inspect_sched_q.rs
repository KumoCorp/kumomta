use clap::Parser;
use kumo_api_types::{InspectQueueV1Request, InspectQueueV1Response};
use reqwest::Url;

#[derive(Debug, Parser)]
/// Returns information about a scheduled queue.
///
/// Part of the information is a sample of the messages contained
/// within that queue.
///
/// Depending on the configured queue strategy, it may not be possible
/// to sample messages from the queue.
/// At the time of writing, the server side can only provide message
/// information if the strategy is set to "SingletonTimerWheel" (the default).
pub struct InspectQueueCommand {
    /// Whether to include the message body information in the results.
    /// This can be expensive, especially with large or no limits.
    #[arg(long)]
    pub want_body: bool,

    /// How many messages to include in the sample.
    /// The default is 5 messages.
    /// The messages are an unspecified subset of the messages in
    /// the queue and likely do NOT indicate which message(s) will
    /// be next due for delivery.
    #[arg(long, default_value = "5")]
    pub limit: usize,

    /// Instead of guessing at a limit, run with no limit on the number
    /// of messages returned.  This can be expensive, especially if
    /// `--want-body` is also enabled.
    #[arg(long, conflicts_with = "limit")]
    pub no_limit: bool,

    /// The name of the queue that you want to query
    pub queue_name: String,
}

impl InspectQueueCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let mut url = endpoint.join("/api/admin/inspect-sched-q/v1")?;
        let request = InspectQueueV1Request {
            queue_name: self.queue_name.clone(),
            want_body: self.want_body,
            limit: if self.no_limit {
                None
            } else {
                Some(self.limit)
            },
        };
        request.apply_to_url(&mut url);

        let result: InspectQueueV1Response =
            crate::request_with_json_response(reqwest::Method::GET, url, &()).await?;

        println!("{}", serde_json::to_string_pretty(&result)?);

        Ok(())
    }
}
