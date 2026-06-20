use clap::Parser;
use kumo_api_client::KumoApiClient;
use kumo_api_types::{DispatcherPhase, InspectReadyQV1Request, InspectReadyQV1Response};
use num_format::{Locale, ToFormattedString};
use reqwest::Url;
use std::io::Write;
use std::time::Duration;
use tabout::{Alignment, Column};

#[derive(Debug, Parser)]
/// Returns information about a ready queue: its egress identity,
/// effective state (throttles, suspensions, ready and connection
/// counts), and, optionally, the dispatcher tasks that are currently
/// handling its connections plus the egress path configuration in
/// effect.
pub struct InspectReadyQCommand {
    /// Show the per-connection dispatcher state (phase, time in
    /// phase, message counters, etc.). Off by default since a busy
    /// queue can have many dispatchers.
    #[arg(long)]
    pub connections: bool,

    /// Include the egress path configuration snapshot. Off by default
    /// because the config can be large.
    #[arg(long)]
    pub config: bool,

    /// Output the response as pretty-printed JSON. Mutually exclusive
    /// with the other flags; the JSON output always carries the full
    /// payload.
    #[arg(long, conflicts_with_all=&["connections", "config"])]
    pub json: bool,

    /// The name of the ready queue to inspect.
    pub queue_name: String,
}

impl InspectReadyQCommand {
    pub async fn run(&self, endpoint: &Url) -> anyhow::Result<()> {
        let client = KumoApiClient::new(endpoint.clone());
        let response = client
            .admin_inspect_ready_q_v1(&InspectReadyQV1Request {
                queue_name: self.queue_name.clone(),
            })
            .await?;

        if self.json {
            println!("{}", serde_json::to_string_pretty(&response)?);
            return Ok(());
        }

        let mut out = std::io::stdout().lock();
        render_summary(&response, &mut out)?;
        render_states(&response, &mut out)?;
        render_ceilings(&response, &mut out)?;
        render_dispatchers(&response, self.connections, &mut out)?;
        if self.config {
            render_config(&response, &mut out)?;
        }
        Ok(())
    }
}

fn render_ceilings(r: &InspectReadyQV1Response, out: &mut dyn Write) -> anyhow::Result<()> {
    writeln!(out)?;
    write!(out, "{}", r.constraints.to_human_string())?;
    Ok(())
}

fn fmt_duration(d: Duration) -> String {
    humantime::format_duration(d).to_string()
}

fn fmt_count(n: u64) -> String {
    n.to_formatted_string(&Locale::en)
}

fn render_summary(r: &InspectReadyQV1Response, out: &mut dyn Write) -> anyhow::Result<()> {
    writeln!(out, "queue: {}", r.queue_name)?;
    writeln!(out)?;

    // Text fields, one per line, label-padded to the longest label.
    let mut text_fields: Vec<(&str, String)> = vec![
        ("egress pool", r.egress_pool.clone()),
        ("egress source", r.egress_source.clone()),
        ("protocol", r.protocol.clone()),
    ];
    if let Some(site) = &r.site_name {
        text_fields.push(("site", site.clone()));
    }
    text_fields.push((
        "watchdog threshold",
        fmt_duration(r.state.watchdog_threshold),
    ));

    let label_w = text_fields
        .iter()
        .map(|(l, _)| l.len() + 1)
        .max()
        .unwrap_or(0);
    for (label, value) in &text_fields {
        writeln!(
            out,
            "{:<label_w$} {}",
            format!("{label}:"),
            value,
            label_w = label_w
        )?;
    }

    // `connections` lives further down with the dispatcher detail.
    writeln!(out)?;
    writeln!(out, "ready: {}", fmt_count(r.state.ready_count as u64))?;
    Ok(())
}

fn render_states(r: &InspectReadyQV1Response, out: &mut dyn Write) -> anyhow::Result<()> {
    let state = &r.state;
    if let Some(s) = &state.connection_rate_throttled {
        writeln!(out)?;
        writeln!(out, "connection rate throttled: {}", s.context)?;
        writeln!(out, "  since: {}", s.since)?;
    }
    if let Some(s) = &state.connection_limited {
        writeln!(out)?;
        writeln!(out, "connection limited: {}", s.context)?;
        writeln!(out, "  since: {}", s.since)?;
    }
    if let Some(s) = &state.suspended {
        writeln!(out)?;
        writeln!(out, "suspended: {}", s.reason)?;
        writeln!(out, "  expires in: {}", fmt_duration(s.duration))?;
    }
    Ok(())
}

fn render_dispatchers(
    r: &InspectReadyQV1Response,
    show_table: bool,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    writeln!(out)?;
    writeln!(
        out,
        "connections: {}",
        fmt_count(r.state.connection_count as u64)
    )?;
    if !show_table || r.dispatchers.is_empty() {
        return Ok(());
    }

    let threshold = r.state.watchdog_threshold;
    let mut dispatchers: Vec<&kumo_api_types::DispatcherSummary> = r.dispatchers.iter().collect();
    // Oldest first; long-lived dispatchers are the more interesting
    // ones when investigating wedges.
    dispatchers.sort_by(|a, b| b.age.cmp(&a.age));

    let columns = [
        Column {
            name: "SESSION".to_string(),
            alignment: Alignment::Left,
        },
        Column {
            name: "DELIV".to_string(),
            alignment: Alignment::Right,
        },
        Column {
            name: "TRANS".to_string(),
            alignment: Alignment::Right,
        },
        Column {
            name: "FAIL".to_string(),
            alignment: Alignment::Right,
        },
        Column {
            name: "AGE".to_string(),
            alignment: Alignment::Right,
        },
        Column {
            name: "RATE".to_string(),
            alignment: Alignment::Right,
        },
        Column {
            name: "".to_string(),
            alignment: Alignment::Left,
        },
    ];

    // Pad the SESSION cell to the longest of session_id, phase line
    // and detail line so the column accommodates the continuation
    // rows beneath each tabular row.
    let phase_line_for = |d: &kumo_api_types::DispatcherSummary| {
        format!(
            "{} for {}",
            phase_label(&d.phase),
            fmt_duration(d.time_in_current_phase)
        )
    };
    let session_col_width = dispatchers
        .iter()
        .map(|d| {
            let session = d.session_id.to_string().len();
            let phase = phase_line_for(d).len();
            let detail = d.detail.as_ref().map(|s| s.len()).unwrap_or(0);
            session.max(phase).max(detail)
        })
        .max()
        .unwrap_or(0);

    let stalled_threshold = threshold.mul_f64(0.75);
    let rows: Vec<Vec<String>> = dispatchers
        .iter()
        .map(|d| {
            let marker = if d.time_in_current_phase >= threshold {
                "⚠ STUCK".to_string()
            } else if d.time_in_current_phase >= stalled_threshold {
                "⚠ STALL?".to_string()
            } else {
                String::new()
            };
            vec![
                format!("{:width$}", d.session_id, width = session_col_width),
                fmt_count(d.messages_delivered),
                fmt_count(d.messages_transfailed),
                fmt_count(d.messages_failed),
                fmt_duration(d.age),
                format!("{:.1}/s", d.overall_rate_per_sec),
                marker,
            ]
        })
        .collect();

    // tabout doesn't support interleaving non-table lines, so render
    // to a buffer and post-process to insert phase/detail lines
    // beneath their owning rows.
    let mut buf = Vec::<u8>::new();
    tabout::tabulate_output(&columns, &rows, &mut buf)?;
    let text = String::from_utf8(buf)?;
    let mut lines = text.lines();
    if let Some(header) = lines.next() {
        writeln!(out, "  {header}")?;
    }
    for (line, d) in lines.zip(dispatchers.iter()) {
        writeln!(out, "  {line}")?;
        writeln!(out, "  {}", phase_line_for(d))?;
        if let Some(detail) = &d.detail {
            writeln!(out, "  {detail}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

fn render_config(r: &InspectReadyQV1Response, out: &mut dyn Write) -> anyhow::Result<()> {
    writeln!(out)?;
    writeln!(out, "config:")?;
    let text = toml::to_string_pretty(&r.path_config)?;
    for line in text.lines() {
        writeln!(out, "  {line}")?;
    }
    Ok(())
}

fn phase_label(p: &DispatcherPhase) -> String {
    match p {
        DispatcherPhase::Starting => "Starting".to_string(),
        DispatcherPhase::AcquiringLease { label } => format!("AcquiringLease({label})"),
        DispatcherPhase::Idle => "Idle".to_string(),
        DispatcherPhase::AccumulatingBatch { have, want } => {
            format!("AccumulatingBatch({have}/{want})")
        }
        DispatcherPhase::ConnectionRateThrottled => "ConnectionRateThrottled".to_string(),
        DispatcherPhase::MessageRateThrottled => "MessageRateThrottled".to_string(),
        DispatcherPhase::AttemptingConnection => "AttemptingConnection".to_string(),
        DispatcherPhase::DeliveringMessage => "DeliveringMessage".to_string(),
        DispatcherPhase::Closing => "Closing".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use kumo_api_types::egress_path::EgressPathConfig;
    use kumo_api_types::{
        DispatcherSummary, InspectReadyQV1Response, QueueState, ReadyQueueStateSnapshot,
        SuspendReadyQueueV1ListEntry,
    };
    use uuid::Uuid;

    fn fixed_time() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .with_ymd_and_hms(2026, 6, 19, 13, 42, 17)
            .unwrap()
    }

    fn build_response() -> InspectReadyQV1Response {
        let path_config = EgressPathConfig::default();
        let constraints = path_config.compute_constraints();
        InspectReadyQV1Response {
            queue_name: "my-source->gmail.com@smtp_client".to_string(),
            site_name: Some("gmail.com".to_string()),
            egress_source: "my-source".to_string(),
            egress_pool: "default".to_string(),
            protocol: "smtp_client".to_string(),
            state: ReadyQueueStateSnapshot {
                ready_count: 1142,
                connection_count: 3,
                connection_rate_throttled: None,
                connection_limited: None,
                suspended: None,
                watchdog_threshold: Duration::from_secs(10 * 60),
            },
            path_config,
            constraints,
            dispatchers: vec![],
            now: fixed_time(),
        }
    }

    fn render_all(r: &InspectReadyQV1Response, connections: bool) -> String {
        let mut out = Vec::<u8>::new();
        render_summary(r, &mut out).unwrap();
        render_states(r, &mut out).unwrap();
        render_ceilings(r, &mut out).unwrap();
        render_dispatchers(r, connections, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn summary_only() {
        let r = build_response();
        k9::snapshot!(
            render_all(&r, false),
            "
queue: my-source->gmail.com@smtp_client

egress pool:        default
egress source:      my-source
protocol:           smtp_client
site:               gmail.com
watchdog threshold: 10m

ready: 1,142

ceilings:
  concurrent dispatchers: 32
    source: connection_limit

connections: 3

"
        );
    }

    #[test]
    fn summary_with_states() {
        let mut r = build_response();
        r.state.connection_rate_throttled = Some(QueueState {
            context: "max_connection_rate throttling for 5s".to_string(),
            since: fixed_time(),
        });
        r.state.suspended = Some(SuspendReadyQueueV1ListEntry {
            id: Uuid::nil(),
            name: r.queue_name.clone(),
            reason: "working with destination postmaster".to_string(),
            duration: Duration::from_secs(60 * 60 + 27 * 60),
            expires: fixed_time(),
        });
        k9::snapshot!(
            render_all(&r, false),
            "
queue: my-source->gmail.com@smtp_client

egress pool:        default
egress source:      my-source
protocol:           smtp_client
site:               gmail.com
watchdog threshold: 10m

ready: 1,142

connection rate throttled: max_connection_rate throttling for 5s
  since: 2026-06-19 13:42:17 UTC

suspended: working with destination postmaster
  expires in: 1h 27m

ceilings:
  concurrent dispatchers: 32
    source: connection_limit

connections: 3

"
        );
    }

    #[test]
    fn with_dispatchers_including_stuck() {
        let mut r = build_response();
        r.state.connection_count = 4;
        r.dispatchers = vec![
            DispatcherSummary {
                session_id: Uuid::parse_str("7f3a8d4e-9c12-4abc-8def-1234567890ab").unwrap(),
                started_at: fixed_time(),
                age: Duration::from_secs(8 * 60 + 12),
                phase: DispatcherPhase::DeliveringMessage,
                detail: Some("send msg with 1 recip(s)".to_string()),
                time_in_current_phase: Duration::from_millis(1200),
                messages_delivered: 142,
                messages_transfailed: 4,
                messages_failed: 0,
                delivered_this_connection: 32,
                overall_rate_per_sec: 18.0,
            },
            DispatcherSummary {
                session_id: Uuid::parse_str("9e4b6a01-2345-6789-abcd-ef0123456789").unwrap(),
                started_at: fixed_time(),
                age: Duration::from_secs(47 * 60 + 8),
                phase: DispatcherPhase::DeliveringMessage,
                detail: Some("send msg with 1 recip(s)".to_string()),
                time_in_current_phase: Duration::from_secs(42 * 60 + 12),
                messages_delivered: 1391,
                messages_transfailed: 0,
                messages_failed: 0,
                delivered_this_connection: 0,
                overall_rate_per_sec: 0.49,
            },
            DispatcherSummary {
                session_id: Uuid::parse_str("b1c2d3e4-1111-2222-3333-444455556666").unwrap(),
                started_at: fixed_time(),
                age: Duration::from_secs(8 * 60),
                phase: DispatcherPhase::DeliveringMessage,
                detail: Some("send msg with 1 recip(s)".to_string()),
                // 80% of the 10-minute threshold => STALLED
                time_in_current_phase: Duration::from_secs(8 * 60),
                messages_delivered: 45,
                messages_transfailed: 1,
                messages_failed: 0,
                delivered_this_connection: 12,
                overall_rate_per_sec: 0.09,
            },
            DispatcherSummary {
                session_id: Uuid::parse_str("c2d35e10-aaaa-bbbb-cccc-dddddddddddd").unwrap(),
                started_at: fixed_time(),
                age: Duration::from_secs(31),
                phase: DispatcherPhase::Idle,
                detail: None,
                time_in_current_phase: Duration::from_secs(31),
                messages_delivered: 0,
                messages_transfailed: 0,
                messages_failed: 0,
                delivered_this_connection: 0,
                overall_rate_per_sec: 0.0,
            },
        ];
        k9::snapshot!(
            render_all(&r, true),
            "
queue: my-source->gmail.com@smtp_client

egress pool:        default
egress source:      my-source
protocol:           smtp_client
site:               gmail.com
watchdog threshold: 10m

ready: 1,142

ceilings:
  concurrent dispatchers: 32
    source: connection_limit

connections: 4
  SESSION                              DELIV TRANS FAIL    AGE   RATE         
  9e4b6a01-2345-6789-abcd-ef0123456789 1,391     0    0 47m 8s  0.5/s ⚠ STUCK 
  DeliveringMessage for 42m 12s
  send msg with 1 recip(s)

  7f3a8d4e-9c12-4abc-8def-1234567890ab   142     4    0 8m 12s 18.0/s         
  DeliveringMessage for 1s 200ms
  send msg with 1 recip(s)

  b1c2d3e4-1111-2222-3333-444455556666    45     1    0     8m  0.1/s ⚠ STALL?
  DeliveringMessage for 8m
  send msg with 1 recip(s)

  c2d35e10-aaaa-bbbb-cccc-dddddddddddd     0     0    0    31s  0.0/s         
  Idle for 31s


"
        );
    }

    #[test]
    fn ceilings_with_reconnect_cycling_annotation() {
        // A configuration that hits the K × C trap: max_message_rate
        // declared as 1000/s but reconnect cycling caps actual
        // throughput at 100/s. The annotation should surface.
        let mut path_config = EgressPathConfig::default();
        path_config.max_message_rate = Some(throttle::ThrottleSpec::try_from("1000/s").unwrap());
        path_config.max_deliveries_per_connection = 10;
        path_config.max_connection_rate = Some(throttle::ThrottleSpec::try_from("10/s").unwrap());
        let constraints = path_config.compute_constraints();
        let mut r = build_response();
        r.path_config = path_config;
        r.constraints = constraints;

        k9::snapshot!(
            render_all(&r, false),
            "
queue: my-source->gmail.com@smtp_client

egress pool:        default
egress source:      my-source
protocol:           smtp_client
site:               gmail.com
watchdog threshold: 10m

ready: 1,142

ceilings:
  concurrent dispatchers: 32
    source: connection_limit
  message rate:           10 × 10/s = 100/s
    source: max_deliveries_per_connection × max_connection_rate
    declared: max_message_rate = 1000/s ← effectively unreachable
  connection rate:        10/s
    source: max_connection_rate

connections: 3

"
        );
    }
}
