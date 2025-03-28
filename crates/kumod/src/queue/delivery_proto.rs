use crate::queue::LuaDeliveryProtocol;
use crate::smtp_dispatcher::SmtpProtocol;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum DeliveryProto {
    Smtp {
        smtp: SmtpProtocol,
    },
    Maildir {
        maildir_path: String,
        dir_mode: Option<u32>,
        file_mode: Option<u32>,
    },
    Lua {
        custom_lua: LuaDeliveryProtocol,
    },
    HttpInjectionGenerator,
    DeferredSmtpInjection,
    Null,
}

impl DeliveryProto {
    pub fn metrics_protocol_name(&self) -> &'static str {
        match self {
            Self::Smtp { .. } => "smtp_client",
            Self::Maildir { .. } => "maildir",
            Self::Lua { .. } => "lua",
            Self::HttpInjectionGenerator { .. } => "httpinject",
            Self::DeferredSmtpInjection { .. } => "defersmtpinject",
            Self::Null { .. } => "null",
        }
    }

    pub fn ready_queue_name(&self) -> String {
        let proto_name = self.metrics_protocol_name();
        match self {
            Self::Smtp { .. } | Self::Null | Self::DeferredSmtpInjection => proto_name.to_string(),
            Self::Maildir { maildir_path, .. } => {
                format!("{proto_name}:{maildir_path}")
            }
            Self::Lua { custom_lua } => format!("{proto_name}:{}", custom_lua.constructor),
            Self::HttpInjectionGenerator => format!("{proto_name}:generator"),
        }
    }
}

impl Default for DeliveryProto {
    fn default() -> Self {
        Self::Smtp {
            smtp: SmtpProtocol::default(),
        }
    }
}
