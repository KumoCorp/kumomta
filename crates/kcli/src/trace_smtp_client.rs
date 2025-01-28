use crate::ColorMode;
use chrono::{DateTime, Utc};
use cidr_map::CidrSet;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use kumo_api_types::{TraceSmtpClientV1Event, TraceSmtpClientV1Payload, TraceSmtpClientV1Request};
use reqwest::Url;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::IsTerminal;
use tokio::sync::mpsc::unbounded_channel;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Trace outgoing sessions made by the SMTP service.
///
/// This is a diagnostic tool for the server operator.
///
/// Sessions are logged in real time.
///
/// Take care on a busy server with live traffic as this tracing
/// mechanism will by-default match all traffic, but there is limited
/// capacity/resources for tracing.  Outside of initial small scale
/// testing, you will need to carefully consider filtering constraints
/// in order to observe only the intended sessions, otherwise the tracing
/// subsystem will be forced to drop a subset of trace events.
///
/// Filtering works by specifying an allow-list for specific properties
/// of a trace session. If an allow-list for a given property is set,
/// and the session has the corresponding property set, then the session
/// is traced only if is value is contained in your set of allowed values.
///
/// Most session properties are filled out AFTER the session has been
/// initiated, and a given session may attempt to establish a series
/// of connections based on how the MX records are resolved, so you
/// should anticipate seeing a number of session initiations that
/// won't (yet) match your trace parameters.
///
/// The main session property that is known at initiation is the
/// ready queue name, so if you know precisely the ready queue of
/// interest, using `--ready-queue` will be the most focused and
/// efficient filter you can specify.
#[derive(Debug, Parser)]
pub struct TraceSmtpClientCommand {
    /// Add a source (in CIDR format) to the list of source addresses
    /// that we want to filter by. If any are specified, then only
    /// connections made from a matching address will be traced.
    /// If no sources are specified, any/all incoming SMTP connections
    /// will be traced.
    ///
    /// Can be used multiple times to add multiple candidate addresses.
    ///
    /// Eg: --source 10.0.0.1 --source 192.168.1.0/24
    #[arg(long)]
    pub source: Vec<String>,

    /// Add an address (in CIDR format) to the list of MX host addresses
    /// that we want to filter by. If any are specified, then only
    /// connections made from a matching address will be traced.
    /// If no addresses are specified, any/all incoming SMTP connections
    /// will be traced.
    ///
    /// A given session may communicate with multiple MX addresses over
    /// its lifetime. The full list of MX addresses is not known at
    /// session initiation, and is filled in after they have been
    /// resolved.
    ///
    /// Can be used multiple times to add multiple candidate addresses.
    ///
    /// Eg: --mx-addr 10.0.0.1 --mx-addr 192.168.1.0/24
    #[arg(long)]
    pub mx_addr: Vec<String>,

    /// The MX hostname to match.
    /// If omitted, any MX hostname will match!
    ///
    /// A given session may communicate with multiple MX addresses over
    /// its lifetime. The full list of MX addresses is not known at
    /// session initiation, and is filled in after they have been
    /// resolved.
    #[arg(long)]
    pub mx_host: Vec<String>,

    /// The domain name to match.
    /// If omitted, any domains will match!
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    ///
    /// A given connection in a session may transit messages with
    /// a variety of different domains.
    #[arg(long)]
    pub domain: Vec<String>,

    /// The routing_domain name to match.
    /// If omitted, any routing domain will match!
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    #[arg(long)]
    pub routing_domain: Vec<String>,

    /// The campaign name to match.
    /// If omitted, any campaigns will match!
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    ///
    /// A given connection in a session may transit messages with
    /// a variety of different campaigns.
    #[arg(long)]
    pub campaign: Vec<String>,

    /// The tenant name to match.
    /// If omitted, any tenant will match!
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    ///
    /// A given connection in a session may transit messages with
    /// a variety of different tenants.
    #[arg(long)]
    pub tenant: Vec<String>,

    /// The egress pool name to match.
    /// If omitted, any pool will match!
    ///
    /// This property is known at session initiation.
    #[arg(long)]
    pub egress_pool: Vec<String>,

    /// The egress source name to match.
    /// If omitted, any source will match!
    ///
    /// This property is known at session initiation.
    #[arg(long)]
    pub egress_source: Vec<String>,

    /// The envelope sender to match. If omitted, any will match.
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    ///
    /// A given connection in a session may transit messages with
    /// a variety of different envelopes.
    #[arg(long)]
    pub mail_from: Vec<String>,

    /// The envelope recipient to match. If omitted, any will match.
    ///
    /// This is a per-message property, and is unavailable
    /// for matching until after a session has established a successful
    /// connection to a host and is ready to deliver a message.
    /// Until a message is present, this filter is ignored.
    ///
    /// A given connection in a session may transit messages with
    /// a variety of different envelopes.
    #[arg(long)]
    pub rcpt_to: Vec<String>,

    /// The ready queue name to match.
    /// If omitted, any ready queue will match!
    ///
    /// This property is known at session initiation.
    #[arg(long)]
    pub ready_queue: Vec<String>,

    /// Whether to colorize the output
    #[arg(long, default_value = "tty")]
    pub color: ColorMode,

    /// Trace only newly opened sessions; ignore data from previously
    /// opened sessions
    #[arg(long)]
    pub only_new: bool,

    /// Trace the first session that we observe, ignoring all others
    #[arg(long)]
    pub only_one: bool,

    /// Abbreviate especially the write side of the transaction trace,
    /// which is useful when examining high traffic and/or large message
    /// transmission
    #[arg(long)]
    pub terse: bool,
}

impl TraceSmtpClientCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let source_addr = if self.source.is_empty() {
            None
        } else {
            let set = CidrSet::try_from(self.source.clone())
                .map_err(|err| anyhow::anyhow!("invalid --source value(s): {err:#}"))?;
            Some(set)
        };

        let mx_addr = if self.mx_addr.is_empty() {
            None
        } else {
            let set = CidrSet::try_from(self.mx_addr.clone())
                .map_err(|err| anyhow::anyhow!("invalid --mx-host value(s): {err:#}"))?;
            Some(set)
        };

        let mut endpoint = endpoint.join("/api/admin/trace-smtp-client/v1")?;
        endpoint.set_scheme("ws").expect("ws to be valid scheme");

        let (mut socket, _response) = connect_async(endpoint.to_string()).await?;

        socket
            .send(Message::Text(serde_json::to_string(
                &TraceSmtpClientV1Request {
                    domain: self.domain.clone(),
                    routing_domain: self.routing_domain.clone(),
                    campaign: self.campaign.clone(),
                    tenant: self.tenant.clone(),
                    egress_pool: self.egress_pool.clone(),
                    egress_source: self.egress_source.clone(),
                    mail_from: self.mail_from.clone(),
                    rcpt_to: self.rcpt_to.clone(),
                    mx_host: self.mx_host.clone(),
                    ready_queue: self.ready_queue.clone(),
                    source_addr,
                    mx_addr,
                    terse: self.terse,
                },
            )?))
            .await?;

        struct ConnState {
            meta: serde_json::Value,
            opened: DateTime<Utc>,
        }

        let mut meta_by_conn: HashMap<String, ConnState> = HashMap::new();

        fn conn_key(meta: &serde_json::Value) -> anyhow::Result<String> {
            if meta.is_null() {
                return Ok(String::new());
            }

            #[derive(Deserialize, Debug)]
            struct Decoded {
                id: String,
            }

            let Decoded { id } = serde_json::from_value(meta.clone())?;

            Ok(id)
        }

        let color = match self.color {
            ColorMode::Tty => std::io::stdout().is_terminal(),
            ColorMode::Yes => true,
            ColorMode::No => false,
        };

        let cyan = if color { "\u{1b}[36m" } else { "" };
        let green = if color { "\u{1b}[32m" } else { "" };
        let red = if color { "\u{1b}[31m" } else { "" };
        let normal = if color { "\u{1b}[0m" } else { "" };

        let mut wanted_key = None;

        let (tx, mut rx) = unbounded_channel();
        tokio::spawn(async move {
            while let Some(event) = socket.next().await {
                let msg = event?;
                tx.send(msg)?;
            }
            Ok::<(), anyhow::Error>(())
        });

        while let Some(msg) = rx.recv().await {
            match msg {
                Message::Text(s) => {
                    let event: TraceSmtpClientV1Event = serde_json::from_str(&s)?;

                    let key = conn_key(&event.conn_meta)?;

                    if let Some(wanted_key) = &wanted_key {
                        if *wanted_key != key && !key.is_empty() {
                            continue;
                        }
                    }

                    let delta = match meta_by_conn.get(&key).map(|m| event.when - m.opened) {
                        Some(delta) => {
                            let delta = delta.to_std().unwrap();
                            format!("{delta: >5.0?}")
                        }
                        None => {
                            if event.payload != TraceSmtpClientV1Payload::BeginSession {
                                if self.only_new {
                                    // We haven't seen this one before, and we're only tracing new
                                    // sessions, so ignore it
                                    continue;
                                }
                            }

                            if self.only_one && wanted_key.is_none() {
                                wanted_key.replace(key.clone());
                            }

                            // Let's create an entry to indicate when we first observed
                            // this session, so that we can display timing for the rest
                            // of it
                            meta_by_conn.insert(
                                key.clone(),
                                ConnState {
                                    meta: serde_json::json!({}),
                                    opened: event.when,
                                },
                            );

                            "     ".to_string()
                        }
                    };

                    match event.payload {
                        TraceSmtpClientV1Payload::BeginSession => {
                            println!("[{key}] {delta} === BeginSession {}", event.when);
                        }
                        TraceSmtpClientV1Payload::Connected => {
                            println!("[{key}] {delta} === Connected");
                        }
                        TraceSmtpClientV1Payload::MessageObtained => {
                            println!("[{key}] {delta} === MessageObtained");
                        }
                        TraceSmtpClientV1Payload::Closed => {
                            meta_by_conn.remove(&key);
                            println!("[{key}] {delta} === Closed");
                            if self.only_one {
                                return Ok(());
                            }
                        }
                        TraceSmtpClientV1Payload::Read(data) => {
                            for line in data.lines() {
                                println!(
                                    "[{key}] {delta} {cyan} <- {}{normal}",
                                    line.escape_debug()
                                );
                            }
                        }
                        TraceSmtpClientV1Payload::AbbreviatedWrite { snippet, len } => {
                            println!(
                                "[{key}] {delta} {green}->  {}{normal}",
                                snippet.escape_debug()
                            );
                            println!("[{key}] {delta} === bytes written={len}");
                        }
                        TraceSmtpClientV1Payload::Write(data) => {
                            for (idx, line) in data.trim_ascii_end().lines().enumerate() {
                                if idx > 0 && self.terse {
                                    println!("[{key}] {delta} === bytes written={}", data.len());
                                    break;
                                }
                                println!(
                                    "[{key}] {delta} {green}->  {}{normal}",
                                    line.escape_debug()
                                );
                            }
                        }
                        TraceSmtpClientV1Payload::Diagnostic { level, message } => {
                            let level_color = if level == "ERROR" { red } else { normal };
                            println!("[{key}] {delta} === {level_color}{level}: {message}{normal}");
                        }
                    }

                    if let Some(prior) = meta_by_conn.get_mut(&key) {
                        if prior.meta != event.conn_meta {
                            // Diff the values

                            match (&prior.meta, &event.conn_meta) {
                                (
                                    serde_json::Value::Object(prior),
                                    serde_json::Value::Object(new),
                                ) => {
                                    for (meta_key, prior_value) in prior.iter() {
                                        match new.get(meta_key) {
                                            Some(value) if value != prior_value => {
                                                println!(
                                                    "[{key}] {delta} === conn_meta {meta_key}={}",
                                                    serde_json::to_string(value)?
                                                );
                                            }
                                            Some(_) => {
                                                // Unchanged
                                            }
                                            None => {
                                                println!(
                                                    "[{key}] {delta} === conn_meta deleted {meta_key}",
                                                );
                                            }
                                        }
                                    }
                                    for (meta_key, value) in new.iter() {
                                        if !prior.contains_key(meta_key) {
                                            println!(
                                                "[{key}] {delta} === conn_meta {meta_key}={}",
                                                serde_json::to_string(value)?
                                            );
                                        }
                                    }
                                }
                                _ => {
                                    println!(
                                        "[{key}] {delta} === conn_meta updated to {}",
                                        serde_json::to_string(&event.conn_meta)?
                                    );
                                }
                            }

                            prior.meta = event.conn_meta;
                        }
                    }
                }
                _ => {
                    anyhow::bail!("Unexpected {msg:?} response");
                }
            }
        }
        Ok(())
    }
}
