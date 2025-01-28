use crate::ColorMode;
use chrono::{DateTime, Utc};
use cidr_map::CidrSet;
use clap::Parser;
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload, TraceSmtpV1Request};
use reqwest::Url;
use std::collections::HashMap;
use std::io::IsTerminal;
use tokio_tungstenite::tungstenite::{connect, Message};

/// Trace incoming connections made to the SMTP service.
///
/// This is a diagnostic tool for the server operator.
///
/// Connections are logged in real time.
///
/// Take care to use an appropriate `--source` when using this with
/// a live busy server, as you will be overwhelmed by the traffic.
#[derive(Debug, Parser)]
pub struct TraceSmtpServerCommand {
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

    /// Abbreviate especially the read side of the transaction trace,
    /// which is useful when examining high traffic and/or large message
    /// transmission
    #[arg(long)]
    pub terse: bool,
}

impl TraceSmtpServerCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let source_addr = if self.source.is_empty() {
            None
        } else {
            let mut set = CidrSet::new();
            for s in &self.source {
                let cidr = cidr_map::parse_cidr(s)?;
                set.insert(cidr);
            }
            Some(set)
        };

        let mut endpoint = endpoint.join("/api/admin/trace-smtp-server/v1")?;
        endpoint.set_scheme("ws").expect("ws to be valid scheme");

        let (mut socket, _response) = connect(endpoint.to_string())?;

        socket.send(Message::Text(serde_json::to_string(&TraceSmtpV1Request {
            source_addr,
        })?))?;

        struct ConnState {
            meta: serde_json::Value,
            opened: DateTime<Utc>,
        }

        let mut meta_by_conn: HashMap<String, ConnState> = HashMap::new();

        fn conn_key(meta: &serde_json::Value) -> anyhow::Result<String> {
            if meta.is_null() {
                return Ok(String::new());
            }
            let from = meta
                .get("received_from")
                .ok_or_else(|| anyhow::anyhow!("conn_meta is missing received_from"))?
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("received_from is not a string"))?;
            let via = meta
                .get("received_via")
                .ok_or_else(|| anyhow::anyhow!("conn_meta is missing received_via"))?
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("received_via is not a string"))?;
            Ok(format!("{from}->{via}"))
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

        loop {
            let msg = socket.read()?;
            match msg {
                Message::Text(s) => {
                    let event: TraceSmtpV1Event = serde_json::from_str(&s)?;

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
                            if event.payload != TraceSmtpV1Payload::Connected {
                                if self.only_new {
                                    // We haven't seen this one before, and we're only tracing new
                                    // sessions, so ignore it
                                    continue;
                                }
                            }

                            if self.only_one && wanted_key.is_none() {
                                wanted_key.replace(key.clone());
                            }

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
                        TraceSmtpV1Payload::Connected => {
                            println!("[{key}] {delta} === Connected {}", event.when,);
                        }
                        TraceSmtpV1Payload::Closed => {
                            meta_by_conn.remove(&key);
                            println!("[{key}] {delta} === Closed");
                            if self.only_one {
                                return Ok(());
                            }
                        }
                        TraceSmtpV1Payload::Read(data) => {
                            for (idx, line) in data.lines().enumerate() {
                                if idx > 0 && self.terse {
                                    println!("[{key}] {delta} === bytes read={}", data.len());
                                    break;
                                }
                                println!(
                                    "[{key}] {delta} {green} -> {}{normal}",
                                    line.escape_debug()
                                );
                            }
                        }
                        TraceSmtpV1Payload::Write(data) => {
                            for line in data.lines() {
                                println!(
                                    "[{key}] {delta} {cyan}<-  {}{normal}",
                                    line.escape_debug()
                                );
                            }
                        }
                        TraceSmtpV1Payload::Diagnostic { level, message } => {
                            let level_color = if level == "ERROR" { red } else { normal };
                            println!("[{key}] {delta} === {level_color}{level}: {message}{normal}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: None,
                            error: Some(error),
                        } => {
                            println!("[{key}] {delta} === {name}: {red}Error: {error}{normal}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: Some(s),
                            error: None,
                        } => {
                            println!("[{key}] {delta} === {name}: Ok: {s:?}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: None,
                            error: None,
                        } => {
                            println!("[{key}] {delta} === {name}: Ok");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result,
                            error,
                        } => {
                            println!(
                                "[{key}] {delta} === {name}: Impossible success \
                                 {result:?} and {red}error: {error:?}{normal}"
                            );
                        }
                        TraceSmtpV1Payload::MessageDisposition {
                            relay,
                            log_arf,
                            log_oob,
                            queue,
                            meta,
                            sender,
                            recipient,
                            id,
                        } => {
                            println!(
                                "[{key}] {delta} === Message from=<{sender}> to=<{recipient}> id={id}"
                            );
                            println!(
                                "[{key}] {delta} === Message queue={queue} relay={relay} \
                                 log_arf={log_arf} log_oob={log_oob}"
                            );
                            match meta {
                                serde_json::Value::Object(obj) => {
                                    for (meta_key, value) in obj {
                                        println!(
                                            "[{key}] {delta} === Message meta: {meta_key}={}",
                                            serde_json::to_string(&value)?
                                        );
                                    }
                                }
                                _ => {
                                    println!(
                                        "[{key}] {delta} === Message meta: {}",
                                        serde_json::to_string(&meta)?
                                    );
                                }
                            }
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
    }
}
