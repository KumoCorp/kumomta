use axum::extract::ws::{close_code, CloseFrame, Message, WebSocket};
use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use kumo_api_types::TraceDetail::{self, *};
use kumo_api_types::{TraceSmtpV1Event, TraceSmtpV1Payload};
use kumo_server_common::http_server::{AppError, AppState, TraceMetaKeyLevels};
use rfc5321::{Command, ReversePath};
use serde::Deserialize;
use serde_json::Value;

pub struct Subscription {
    pub detail: TraceDetail,
    pub inbound_meta: TraceMetaKeyLevels,
    ceiling: TraceDetail,
}

impl Subscription {
    // A filter on a field above the listener's ceiling could disclose it by matching.
    pub async fn allow_filter(
        &self,
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
        let params = state.params();
        let ceiling = params.max_smtp_trace_detail;
        let level = options.detail.unwrap_or(ceiling);
        if level > ceiling {
            return Err(AppError::new(
                StatusCode::FORBIDDEN,
                format!("maximum SMTP trace detail is {ceiling}"),
            ));
        }
        Ok(Self {
            detail: level,
            inbound_meta: params.smtp_trace_meta_detail.inbound_connection.clone(),
            ceiling,
        })
    }
}

pub fn command(cmd: &Command, level: TraceDetail) -> String {
    use Command::*;
    let (verb, full_at) = match cmd {
        Ehlo(_) => ("EHLO", Operational),
        Helo(_) => ("HELO", Operational),
        Lhlo(_) => ("LHLO", Operational),
        MailFrom {
            address: ReversePath::NullSender,
            ..
        } => ("MAIL FROM:<>", Envelope),
        MailFrom { .. } => ("MAIL FROM", Envelope),
        RcptTo { .. } => ("RCPT TO", Envelope),
        Auth { .. } if level < Secrets => return cmd.encode_for_logging().to_string(),
        Auth { .. } => ("AUTH", Secrets),
        XClient(_) => ("XCLIENT", Secrets),
        Unknown(_) => ("[unrecognized command]", Secrets),
        Help(_) => ("HELP", Secrets),
        Vrfy(_) => ("VRFY", Envelope),
        Expn(_) => ("EXPN", Envelope),
        Noop(_) => ("NOOP", Secrets),
        Data => ("DATA", Secrets),
        DataDot => (".", Secrets),
        Rset => ("RSET", Secrets),
        Quit => ("QUIT", Secrets),
        StartTls => ("STARTTLS", Secrets),
    };
    if level >= full_at {
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

pub fn project_meta(
    meta: &mut Value,
    detail: TraceDetail,
    field_level: impl Fn(&str) -> TraceDetail,
) {
    if detail == Secrets {
        return;
    }
    if let Some(obj) = meta.as_object_mut() {
        obj.retain(|key, _| detail >= field_level(key));
    }
}

pub fn inbound_meta_detail(key: &str, levels: &TraceMetaKeyLevels) -> TraceDetail {
    match key {
        "tls_protocol_version" => Protocol,
        "received_from" | "received_via" | "orig_received_from" | "orig_received_via"
        | "reception_protocol" | "hostname" | "ehlo_domain" | "tls_cipher" | "tls_implicit" => {
            Operational
        }
        "authn_id" | "authz_id" | "tls_peer_subject_name" => Envelope,
        _ if key.starts_with("x_") => levels
            .keys
            .get(key)
            .copied()
            .or(levels.default_for_x_keys)
            .unwrap_or(Secrets),
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
        let levels = TraceMetaKeyLevels::default();
        project_meta(&mut inbound, Operational, |k| {
            inbound_meta_detail(k, &levels)
        });
        project_meta(&mut projected, Operational, outbound_meta_detail);
        assert_eq!(inbound, serde_json::json!({}));
        assert_eq!(projected, outbound);
    }
}
