use anyhow::Context;
use cidr_map::{AnyIpCidr, CidrSet};
use clap::Parser;
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload, TraceSmtpV1Request};
use reqwest::Url;
use std::collections::HashMap;
use std::str::FromStr;
use tungstenite::{connect, Message};

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
}

impl TraceSmtpServerCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let source_addr = if self.source.is_empty() {
            None
        } else {
            let mut set = CidrSet::new();
            for s in &self.source {
                let cidr =
                    AnyIpCidr::from_str(s).with_context(|| format!("{s} is not a valid CIDR"))?;
                set.insert(cidr);
            }
            Some(set)
        };

        let mut endpoint = endpoint.join("/api/admin/trace-smtp-server/v1")?;
        endpoint.set_scheme("ws").expect("ws to be valid scheme");

        let (mut socket, _response) = connect(endpoint)?;

        socket.send(Message::Text(serde_json::to_string(&TraceSmtpV1Request {
            source_addr,
        })?))?;

        let mut meta_by_conn = HashMap::new();

        fn conn_key(meta: &serde_json::Value) -> anyhow::Result<String> {
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

        loop {
            let msg = socket.read()?;
            match msg {
                Message::Text(s) => {
                    let event: TraceSmtpV1Event = serde_json::from_str(&s)?;

                    let key = conn_key(&event.conn_meta)?;

                    match event.payload {
                        TraceSmtpV1Payload::Connected => {
                            meta_by_conn.insert(key.clone(), event.conn_meta.clone());
                            println!(
                                "[{key}] === Connected {}",
                                serde_json::to_string(&event.conn_meta)?
                            );
                        }
                        TraceSmtpV1Payload::Closed => {
                            meta_by_conn.remove(&key);
                            println!("[{key}] === Closed");
                        }
                        TraceSmtpV1Payload::Read(data) => {
                            for line in data.lines() {
                                println!("[{key}]  -> {}", line.escape_debug());
                            }
                        }
                        TraceSmtpV1Payload::Write(data) => {
                            for line in data.lines() {
                                println!("[{key}] <-  {}", line.escape_debug());
                            }
                        }
                        TraceSmtpV1Payload::Diagnostic { level, message } => {
                            println!("[{key}] === {level}: {message}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: None,
                            error: Some(error),
                        } => {
                            println!("[{key}] === {name}: Error: {error}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: Some(s),
                            error: None,
                        } => {
                            println!("[{key}] === {name}: Ok: {s:?}");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result: None,
                            error: None,
                        } => {
                            println!("[{key}] === {name}: Ok");
                        }
                        TraceSmtpV1Payload::Callback {
                            name,
                            result,
                            error,
                        } => {
                            println!("[{key}] === {name}: Impossible success {result:?} and error: {error:?}");
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
                            println!("[{key}] === Message from={sender} to={recipient} id={id}");
                            println!("[{key}] === Message queue={queue} relay={relay} log_arf={log_arf} log_oob={log_oob}");
                            println!(
                                "[{key}] === Message meta: {}",
                                serde_json::to_string(&meta)?
                            );
                        }
                    }

                    if let Some(prior) = meta_by_conn.get_mut(&key) {
                        if *prior != event.conn_meta {
                            println!(
                                "[{key}] === conn_meta updated to {}",
                                serde_json::to_string(&event.conn_meta)?
                            );
                            *prior = event.conn_meta;
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
