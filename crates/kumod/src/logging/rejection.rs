use crate::logging::Logger;
use bounce_classify::BounceClass;
use chrono::Utc;
pub use kumo_log_types::*;
use rfc5321::Response;
use std::collections::HashMap;

pub struct LogRejection {
    pub peer_address: ResolvedAddress,
    pub response: Response,
    pub meta: serde_json::Value,
    pub sender: Option<String>,
    pub recipient: Option<String>,
}

pub async fn log_rejection(args: LogRejection) {
    let loggers = Logger::get_loggers();
    if loggers.is_empty() {
        return;
    }
    let now = Utc::now();
    let nodeid = kumo_server_common::nodeid::NodeId::get_uuid();

    let kind = RecordType::Rejection;

    for logger in loggers.iter() {
        if !logger.record_is_enabled(kind) {
            continue;
        }

        let meta = logger.extract_meta(&args.meta);

        let record = JsonLogRecord {
            kind,
            id: "".to_string(),
            size: 0,
            sender: args.sender.clone().unwrap_or_else(String::new),
            recipient: args.recipient.clone().unwrap_or_else(String::new),
            queue: "".to_string(),
            site: "".to_string(),
            peer_address: Some(args.peer_address.clone()),
            response: args.response.clone(),
            timestamp: now,
            created: now,
            num_attempts: 0,
            egress_pool: None,
            egress_source: None,
            bounce_classification: BounceClass::default(),
            feedback_report: None,
            headers: HashMap::new(),
            meta,
            delivery_protocol: None,
            reception_protocol: None,
            nodeid,
            tls_cipher: None,
            tls_protocol_version: None,
            tls_peer_subject_name: None,
            source_address: None,
        };
        if let Err(err) = logger.log(record).await {
            tracing::error!("failed to log: {err:#}");
        }
    }
}
