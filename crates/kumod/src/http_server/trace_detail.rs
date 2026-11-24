use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket};
use axum::extract::{FromRequestParts, Query};
use axum::http::{request::Parts, StatusCode};
use chrono::{DateTime, Utc};
use kumo_api_types::TraceDetail::{self, *};
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload};
use kumo_server_common::http_server::{AppError, AppState};
use rfc5321::{Command, ReversePath};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy)]
pub struct Subscription {
    pub detail: TraceDetail,
    ceiling: TraceDetail,
}

impl Subscription {
    // A filter on a field above the listener's ceiling could disclose it by matching.
    pub async fn allow_filter(
        self,
        required: TraceDetail,
        ws: &mut WebSocket,
    ) -> anyhow::Result<bool> {
        if required <= self.ceiling {
            return Ok(true);
        }
        ws.send(Message::Close(Some(CloseFrame {
            code: close_code::POLICY,
            reason: format!("trace filter requires {required} detail on this listener").into(),
        })))
        .await?;
        Ok(false)
    }
}

impl FromRequestParts<AppState> for Subscription {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AppError> {
        #[derive(Deserialize)]
        struct Options {
            detail: Option<TraceDetail>,
        }
        let Query(options) = Query::<Options>::try_from_uri(&parts.uri)
            .map_err(|err| AppError::new(StatusCode::BAD_REQUEST, err.to_string()))?;
        let ceiling = state.params().max_smtp_trace_detail;
        let level = options.detail.unwrap_or(ceiling);
        if level > ceiling {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                format!("maximum SMTP trace detail is {ceiling}"),
            ));
        }
        Ok(Self {
            detail: level,
            ceiling,
        })
    }
}

pub fn command(cmd: &Command, level: TraceDetail) -> String {
    use Command::*;
    let verb = match cmd {
        Ehlo(_) => "EHLO",
        Helo(_) => "HELO",
        Lhlo(_) => "LHLO",
        MailFrom {
            address: ReversePath::NullSender,
            ..
        } if level < Envelope => "MAIL FROM:<>",
        MailFrom { .. } => "MAIL FROM",
        RcptTo { .. } => "RCPT TO",
        Auth { .. } if level < Secrets => return cmd.encode_for_logging().to_string(),
        Auth { .. } => "AUTH",
        XClient(_) => "XCLIENT",
        Unknown(_) => "[unrecognized command]",
        Help(_) => "HELP",
        Vrfy(_) => "VRFY",
        Expn(_) => "EXPN",
        Noop(_) => "NOOP",
        Data => "DATA",
        DataDot => ".",
        Rset => "RSET",
        Quit => "QUIT",
        StartTls => "STARTTLS",
    };
    let required = match cmd {
        Ehlo(_) | Helo(_) | Lhlo(_) => Operational,
        MailFrom { .. } | RcptTo { .. } | Vrfy(_) | Expn(_) => Envelope,
        // Unstructured arguments and unknown commands have no lower-tier representation.
        _ => Secrets,
    };
    if level >= required {
        cmd.encode().to_string()
    } else {
        verb.to_owned()
    }
}

pub fn reply(line: String, level: TraceDetail) -> String {
    if level >= Operational && (level == Secrets || !line.starts_with("334")) {
        return line;
    }
    let Ok(code) = line.get(..3).unwrap_or("").parse::<u16>() else {
        return "[malformed reply]".into();
    };
    let message = line.get(4..).unwrap_or("").trim_end();
    if !message.is_empty() && code != 334 {
        if let Some(e) = rfc5321::Response::with_code_and_message(code, message).enhanced_code {
            return format!("{code} {}.{}.{}", e.class, e.subject, e.detail);
        }
    }
    code.to_string()
}

// kcli's WebSocket reader accepts frames up to 16 MiB. Check the JSON after projection
// and terse formatting: the byte count on the SMTP wire isn't the WebSocket frame size.
pub const MAX_FRAME_SIZE: usize = 16 << 20;

pub fn omitted_data(when: DateTime<Utc>, conn_meta: Value) -> serde_json::Result<String> {
    // Diagnostic has the same wire representation in both trace APIs.
    let mut event = TraceSmtpV1Event {
        when,
        conn_meta,
        payload: TraceSmtpV1Payload::Diagnostic {
            level: "WARN".into(),
            message: "SMTP DATA omitted from trace: serialized event exceeds 16 MiB".into(),
        },
    };
    let json = serde_json::to_string(&event)?;
    if json.len() <= MAX_FRAME_SIZE {
        return Ok(json);
    }
    // Even the metadata alone may be too large. Emit a stream-wide notice in that case.
    event.conn_meta = Value::Null;
    serde_json::to_string(&event)
}

pub fn project_meta(meta: &mut Value, detail: TraceDetail, field_level: fn(&str) -> TraceDetail) {
    if detail == Secrets {
        return;
    }
    if let Some(obj) = meta.as_object_mut() {
        obj.retain(|key, _| detail >= field_level(key));
    }
}

pub fn inbound_meta_detail(key: &str) -> TraceDetail {
    match key {
        "tls_protocol_version" => Protocol,
        "received_from" | "received_via" | "orig_received_from" | "orig_received_via"
        | "reception_protocol" | "hostname" | "ehlo_domain" | "tls_cipher" | "tls_implicit" => {
            Operational
        }
        "authn_id" | "authz_id" => Envelope,
        _ => Secrets,
    }
}

pub fn outbound_meta_detail(key: &str) -> TraceDetail {
    match key {
        "id" => Protocol,
        "source_address" | "mx_address" | "mx_host" | "mx_plan" | "egress_pool"
        | "egress_source" => Operational,
        "sender" | "recipient" | "domain" | "routing_domain" | "queue_name"
        | "ready_queue_name" | "tenant" | "campaign" | "message_id" => Envelope,
        _ => Secrets,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn mx_plan_is_operational_only_on_outbound_traces() {
        let outbound = serde_json::json!({"mx_plan": ["example.test"]});
        let mut inbound = outbound.clone();
        let mut projected = outbound.clone();
        project_meta(&mut inbound, Operational, inbound_meta_detail);
        project_meta(&mut projected, Operational, outbound_meta_detail);
        assert_eq!(inbound, serde_json::json!({}));
        assert_eq!(projected, outbound);
    }
}
